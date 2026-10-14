//! File decoding (symphonia) and resampling (rubato).

use std::fmt;
use std::path::Path;

use rubato::{FftFixedIn, Resampler};
use symphonia::core::audio::{Channels, SampleBuffer};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::analysis::{build_overview, detect_tempo, OVERVIEW_BINS_PER_SEC};
use crate::key::detect_key;
use crate::track::Track;

/// Front-centre weight when a source has three or more channels (-3 dB, ~0.7071).
const CENTER_MIX: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// Keeps a full-scale front pair plus centre inside [-1, 1].
const DOWNMIX_SCALE: f32 = 1.0 / (1.0 + CENTER_MIX);
/// Corrupt packets tolerated in a row before the file is treated as broken.
const MAX_SKIPPED_PACKETS: u32 = 1_000;
/// Upper bound on the up-front reservation taken from `n_frames` (10 min at 48 kHz).
const MAX_RESERVE_FRAMES: u64 = 48_000 * 600;
/// Input frames per resampler call.
const RESAMPLE_CHUNK: usize = 1024;
/// Largest magnitude a sample may have before resampling. Real audio sits well
/// inside [-1, 1]; this keeps the resampler's FFT far from f32 overflow.
const SAMPLE_LIMIT: f32 = 16.0;
/// Sample rates accepted from a file; keeps resampler buffers bounded.
const MIN_RATE: u32 = 1_000;
const MAX_RATE: u32 = 768_000;

#[derive(Debug, Clone, PartialEq)]
pub enum LoadError {
    Io(String),
    Unsupported(String),
    Decode(String),
    Empty,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "não consegui abrir o arquivo: {e}"),
            LoadError::Unsupported(e) => write!(f, "formato não suportado: {e}"),
            LoadError::Decode(e) => write!(f, "erro ao decodificar: {e}"),
            LoadError::Empty => write!(f, "arquivo sem áudio"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Lowercase extensions the library browser lists.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "wave", "ogg", "oga", "m4a", "mp4", "aac", "aif", "aiff",
];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Decode a whole file to interleaved stereo f32 at its native rate.
/// Mono is duplicated to both sides; more than two channels are downmixed
/// to L/R (front pair). Returns `(samples, sample_rate)`.
pub fn decode_file(path: &Path) -> Result<(Vec<f32>, u32), LoadError> {
    let file = std::fs::File::open(path).map_err(|e| LoadError::Io(e.to_string()))?;

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    guard_panics(|| decode_stream(file, &hint))
}

/// Probe, decode and map an opened file. Runs under `guard_panics`.
fn decode_stream(file: std::fs::File, hint: &Hint) -> Result<(Vec<f32>, u32), LoadError> {
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let format_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let probed = symphonia::default::get_probe()
        .format(hint, stream, &format_opts, &MetadataOptions::default())
        .map_err(|e| LoadError::Unsupported(e.to_string()))?;
    let mut format = probed.format;

    let (track_id, params) = {
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| LoadError::Unsupported("nenhuma trilha de áudio".into()))?;
        (track.id, track.codec_params.clone())
    };
    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| LoadError::Unsupported(e.to_string()))?;

    let mut rate: Option<u32> = params.sample_rate.filter(|&r| r > 0);
    let mut out: Vec<f32> = Vec::new();
    if let Some(frames) = params.n_frames {
        out.reserve(frames.min(MAX_RESERVE_FRAMES) as usize * 2);
    }

    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut skipped = 0u32;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(err) => match recover(err, out.len() / 2, rate, &mut skipped)? {
                Recover::Skip => continue,
                Recover::Stop => break,
            },
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(err) => match recover(err, out.len() / 2, rate, &mut skipped)? {
                Recover::Skip => continue,
                Recover::Stop => break,
            },
        };
        skipped = 0;

        let spec = *decoded.spec();
        let channels = spec.channels.count();
        if rate.is_none() && spec.rate > 0 {
            rate = Some(spec.rate);
        }
        let frames = decoded.frames();
        if channels == 0 || frames == 0 {
            continue;
        }

        let needed = decoded.capacity().max(frames);
        if sbuf
            .as_ref()
            .is_none_or(|b| b.capacity() < needed * channels)
        {
            sbuf = Some(SampleBuffer::<f32>::new(needed as u64, spec));
        }
        if let Some(sb) = sbuf.as_mut() {
            sb.copy_interleaved_ref(decoded);
            push_stereo(&mut out, sb.samples(), spec.channels);
        }
    }

    let rate =
        rate.ok_or_else(|| LoadError::Unsupported("taxa de amostragem desconhecida".into()))?;
    if !valid_rate(rate) {
        return Err(LoadError::Unsupported(format!(
            "taxa de amostragem fora do intervalo: {rate} Hz"
        )));
    }
    Ok((out, rate))
}

/// Resample interleaved stereo from `from` Hz to `to` Hz with a
/// band-limited sinc resampler. Output length is
/// `round(frames * to / from)` frames (±1). `from == to` returns a copy.
pub fn resample_stereo(samples: &[f32], from: u32, to: u32) -> Result<Vec<f32>, LoadError> {
    if from == to || samples.is_empty() {
        // A copy, with non-finite samples silenced and huge ones clamped as in
        // the resampling path, so no caller ever gets NaN or inf back.
        return Ok(samples.iter().map(|&x| sanitize(x)).collect());
    }
    if from == 0 || to == 0 {
        return Err(LoadError::Decode("taxa de amostragem zero".into()));
    }

    let frames = samples.len() / 2;
    if frames == 0 {
        return Ok(Vec::new());
    }
    let expected = output_frames(frames, from, to);

    let mut resampler =
        FftFixedIn::<f32>::new(from as usize, to as usize, RESAMPLE_CHUNK, 2, 2).map_err(rs_err)?;
    let delay = resampler.output_delay();
    let chunk = resampler.input_frames_next();
    let mut out_buf = resampler.output_buffer_allocate(true);
    let mut in_l = vec![0.0f32; chunk];
    let mut in_r = vec![0.0f32; chunk];
    let mut placer = Placer::new(delay, expected);

    let mut pos = 0;
    while pos < frames {
        let n = (frames - pos).min(chunk);
        for (i, f) in samples[pos * 2..(pos + n) * 2].chunks_exact(2).enumerate() {
            in_l[i] = sanitize(f[0]);
            in_r[i] = sanitize(f[1]);
        }
        let input = [&in_l[..n], &in_r[..n]];
        let call = if n == chunk {
            resampler.process_into_buffer(&input[..], &mut out_buf[..], None)
        } else {
            resampler.process_partial_into_buffer(Some(&input[..]), &mut out_buf[..], None)
        };
        let (_, produced) = call.map_err(rs_err)?;
        placer.push(&out_buf[0][..produced], &out_buf[1][..produced]);
        pos += n;
    }

    // Push the filter tail out with silence until every real output frame has
    // been seen. The resampler only emits a block once `from / gcd` input
    // frames (plus at most 511) have built up, and the tail needs about 1.5
    // blocks of silence, so a budget of `2 * from` frames always completes the
    // output. Each call feeds one chunk, and the loop stops as soon as done.
    let mut silence = 2 * from as usize + 2 * chunk;
    while !placer.done() && silence > 0 {
        let (_, produced) = resampler
            .process_partial_into_buffer(None::<&[&[f32]]>, &mut out_buf[..], None)
            .map_err(rs_err)?;
        placer.push(&out_buf[0][..produced], &out_buf[1][..produced]);
        silence = silence.saturating_sub(chunk);
    }
    Ok(placer.result)
}

/// Collects resampler output into the final interleaved buffer. The first
/// `delay` raw frames are the resampler's group delay and are dropped, so
/// output frame k lines up with input time k * from / to.
struct Placer {
    delay: usize,
    raw: usize,
    result: Vec<f32>,
}

impl Placer {
    fn new(delay: usize, expected: usize) -> Self {
        Self {
            delay,
            raw: 0,
            result: vec![0.0; expected * 2],
        }
    }

    /// True once every output frame has been written.
    fn done(&self) -> bool {
        self.raw >= self.delay + self.result.len() / 2
    }

    fn push(&mut self, left: &[f32], right: &[f32]) {
        let frames = self.result.len() / 2;
        for (i, (&l, &r)) in left.iter().zip(right).enumerate() {
            let k = self.raw + i;
            if k >= self.delay && k - self.delay < frames {
                let o = (k - self.delay) * 2;
                self.result[o] = finite(l);
                self.result[o + 1] = finite(r);
            }
        }
        self.raw += left.len().min(right.len());
    }
}

/// What the packet loop does after a demux or decode error.
enum Recover {
    /// Corrupt data: drop this packet and carry on.
    Skip,
    /// End of stream, or a reset we do not follow: keep what was decoded.
    Stop,
}

/// Maps a symphonia error to the loop action. Anything else is fatal unless
/// at least one second of audio is already decoded.
fn recover(
    err: SymError,
    decoded_frames: usize,
    rate: Option<u32>,
    skipped: &mut u32,
) -> Result<Recover, LoadError> {
    match err {
        SymError::IoError(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            Ok(Recover::Stop)
        }
        SymError::ResetRequired => Ok(Recover::Stop),
        SymError::DecodeError(_) if *skipped < MAX_SKIPPED_PACKETS => {
            *skipped += 1;
            Ok(Recover::Skip)
        }
        other => {
            if rate.is_some_and(|r| decoded_frames >= r as usize) {
                Ok(Recover::Stop)
            } else {
                Err(LoadError::Decode(other.to_string()))
            }
        }
    }
}

/// Makes one sample safe to resample or play: non-finite becomes silence and
/// anything beyond `SAMPLE_LIMIT` is clamped.
fn sanitize(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-SAMPLE_LIMIT, SAMPLE_LIMIT)
    } else {
        0.0
    }
}

/// Appends interleaved `samples` to `out` as L/R. `layout` names the channels
/// of each frame in interleaved order. Mono is duplicated. With three or more
/// channels the front pair is taken by position, and the front centre is mixed
/// in at -3 dB, scaled by `DOWNMIX_SCALE`, only when the layout has one.
/// Every sample is sanitized on the way in.
fn push_stereo(out: &mut Vec<f32>, samples: &[f32], layout: Channels) {
    let n = layout.count();
    match n {
        0 => {}
        1 => {
            for &s in samples {
                let s = sanitize(s);
                out.push(s);
                out.push(s);
            }
        }
        2 => out.extend(samples.iter().map(|&s| sanitize(s))),
        _ => {
            let (left, right) = front_pair(layout);
            match index_of(layout, Channels::FRONT_CENTRE) {
                Some(centre) => {
                    for frame in samples.chunks_exact(n) {
                        let c = CENTER_MIX * sanitize(frame[centre]);
                        out.push((sanitize(frame[left]) + c) * DOWNMIX_SCALE);
                        out.push((sanitize(frame[right]) + c) * DOWNMIX_SCALE);
                    }
                }
                None => {
                    for frame in samples.chunks_exact(n) {
                        out.push(sanitize(frame[left]));
                        out.push(sanitize(frame[right]));
                    }
                }
            }
        }
    }
}

/// Positions of front-left and front-right in `layout`, or channels 0 and 1
/// when either is missing.
fn front_pair(layout: Channels) -> (usize, usize) {
    match (
        index_of(layout, Channels::FRONT_LEFT),
        index_of(layout, Channels::FRONT_RIGHT),
    ) {
        (Some(left), Some(right)) => (left, right),
        _ => (0, 1),
    }
}

/// Position of the single channel `flag` in interleaved order, if `layout`
/// has it. Channels are interleaved in ascending bit order, so the position
/// is the number of channels below `flag`.
fn index_of(layout: Channels, flag: Channels) -> Option<usize> {
    layout
        .contains(flag)
        .then(|| (layout.bits() & (flag.bits() - 1)).count_ones() as usize)
}

/// Runs a probe or decode step and turns a panic inside an upstream demuxer or
/// codec (some index without bounds checks on malformed input) into a decode
/// error, so a bad file cannot take the loader down. The default panic hook
/// still reports the message on stderr.
fn guard_panics<T>(f: impl FnOnce() -> Result<T, LoadError>) -> Result<T, LoadError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(LoadError::Decode("decoder panicked".into())))
}

/// `round(frames * to / from)`, half up, in exact integer arithmetic.
fn output_frames(frames: usize, from: u32, to: u32) -> usize {
    let num = frames as u128 * to as u128;
    let from = from as u128;
    ((2 * num + from) / (2 * from)) as usize
}

fn finite(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

fn rs_err(e: impl fmt::Display) -> LoadError {
    LoadError::Decode(e.to_string())
}

fn valid_rate(rate: u32) -> bool {
    (MIN_RATE..=MAX_RATE).contains(&rate)
}

/// Decode, resample to `target_rate`, detect tempo and key and build the overview.
/// Runs on a loader thread, never on the audio thread.
pub fn load_track(path: &Path, target_rate: u32) -> Result<Track, LoadError> {
    let (raw, rate) = decode_file(path)?;
    if raw.is_empty() {
        return Err(LoadError::Empty);
    }
    let samples = resample_stereo(&raw, rate, target_rate)?;
    drop(raw);
    let analysis = detect_tempo(&samples, target_rate);
    let overview = build_overview(&samples, target_rate, OVERVIEW_BINS_PER_SEC);
    let title = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sem nome")
        .to_string();
    let key = detect_key(&samples, target_rate);
    let mut track = Track::new(
        title,
        path.to_path_buf(),
        target_rate,
        samples,
        analysis,
        overview,
    );
    track.key = key;
    Ok(track)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn split(interleaved: &[f32]) -> (Vec<f32>, Vec<f32>) {
        interleaved.chunks_exact(2).map(|f| (f[0], f[1])).unzip()
    }

    /// Frequency estimate from sign changes in `channel[start..end]`.
    fn freq_by_zero_crossings(channel: &[f32], rate: u32, start: usize, end: usize) -> f64 {
        let seg = &channel[start..end];
        let crossings = seg
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        let secs = seg.len() as f64 / rate as f64;
        crossings as f64 / (2.0 * secs)
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / x.len() as f64).sqrt()
    }

    fn sine_stereo(rate: u32, secs: f64, freq: f64, amp_l: f64, amp_r: f64) -> Vec<f32> {
        let frames = (rate as f64 * secs).round() as usize;
        let mut v = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let t = n as f64 / rate as f64;
            let s = (2.0 * std::f64::consts::PI * freq * t).sin();
            v.push((amp_l * s) as f32);
            v.push((amp_r * s) as f32);
        }
        v
    }

    fn check_tone_stereo(name: &str) {
        let (s, rate) = decode_file(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(rate, 44100, "{name}: rate");
        let frames = s.len() / 2;
        let secs = frames as f64 / rate as f64;
        assert!((0.45..=0.62).contains(&secs), "{name}: duration {secs} s");

        let (l, r) = split(&s);
        let start = (0.1 * rate as f64) as usize;
        let end = (0.4 * rate as f64) as usize;
        assert!(rms(&l[start..end]) > 0.05, "{name}: left channel is silent");
        assert!(
            rms(&r[start..end]) > 0.05,
            "{name}: right channel is silent"
        );

        let fl = freq_by_zero_crossings(&l, rate, start, end);
        let fr = freq_by_zero_crossings(&r, rate, start, end);
        assert!(
            (fl - 440.0).abs() <= 440.0 * 0.06,
            "{name}: left {fl:.1} Hz, expected 440"
        );
        assert!(
            (fr - 660.0).abs() <= 660.0 * 0.06,
            "{name}: right {fr:.1} Hz, expected 660"
        );
    }

    #[test]
    fn decodes_stereo_wav() {
        check_tone_stereo("tone_stereo.wav");
    }

    #[test]
    fn decodes_stereo_flac() {
        check_tone_stereo("tone_stereo.flac");
    }

    #[test]
    fn decodes_stereo_mp3() {
        check_tone_stereo("tone_stereo.mp3");
    }

    #[test]
    fn decodes_stereo_ogg() {
        check_tone_stereo("tone_stereo.ogg");
    }

    #[test]
    fn decodes_stereo_m4a() {
        check_tone_stereo("tone_stereo.m4a");
    }

    #[test]
    fn mono_48k_wav_is_duplicated_to_both_sides() {
        let (s, rate) = decode_file(&fixture("tone_mono_48k.wav")).unwrap();
        assert_eq!(rate, 48000);
        let frames = s.len() / 2;
        let secs = frames as f64 / rate as f64;
        assert!((0.45..=0.62).contains(&secs), "duration {secs} s");
        let (l, r) = split(&s);
        assert!(l.iter().zip(&r).all(|(a, b)| a == b), "L != R somewhere");
        assert!(rms(&l) > 0.05, "mono source decoded as silence");
    }

    #[test]
    fn missing_file_is_io_error() {
        let result = decode_file(&fixture("does_not_exist.mp3"));
        assert!(matches!(result, Err(LoadError::Io(_))), "{result:?}");
    }

    #[test]
    fn garbage_bytes_are_rejected_without_panicking() {
        let dir = std::env::temp_dir().join(format!("helix-remix-decode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.mp3");
        let mut seed: u32 = 0x1234_5678;
        let bytes: Vec<u8> = (0..64 * 1024)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect();
        std::fs::write(&path, &bytes).unwrap();

        let result = decode_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            matches!(
                result,
                Err(LoadError::Unsupported(_)) | Err(LoadError::Decode(_))
            ),
            "{result:?}"
        );
    }

    #[test]
    fn empty_file_is_rejected_without_panicking() {
        let dir = std::env::temp_dir().join(format!("helix-remix-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.wav");
        std::fs::write(&path, b"").unwrap();

        let result = decode_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn is_supported_checks_extension_case_insensitively() {
        assert!(is_supported(Path::new("a.MP3")));
        assert!(is_supported(Path::new("music/track.Flac")));
        assert!(is_supported(Path::new("x.m4a")));
        assert!(!is_supported(Path::new("a.txt")));
        assert!(!is_supported(Path::new("no_extension")));
    }

    #[test]
    fn resample_44100_to_48000_keeps_length_frequency_and_level() {
        let input = sine_stereo(44100, 1.0, 1000.0, 0.5, 0.25);
        let out = resample_stereo(&input, 44100, 48000).unwrap();
        let frames = out.len() / 2;
        assert!((frames as i64 - 48000).abs() <= 1, "frames {frames}");
        assert!(out.iter().all(|x| x.is_finite()), "NaN or inf in output");

        let (l_in, r_in) = split(&input);
        let (l_out, r_out) = split(&out);
        let in_frames = l_in.len();
        for (name, inp, outp) in [("left", &l_in, &l_out), ("right", &r_in, &r_out)] {
            let f = freq_by_zero_crossings(outp, 48000, 2000, frames - 2000);
            assert!(
                (f - 1000.0).abs() <= 1000.0 * 0.005,
                "{name}: {f:.2} Hz after resampling"
            );
            let db = 20.0
                * (rms(&outp[2000..frames - 2000]) / rms(&inp[2000..in_frames - 2000])).log10();
            assert!(db.abs() <= 0.5, "{name}: level changed by {db:.3} dB");
        }
    }

    #[test]
    fn resample_impulse_lands_at_scaled_position() {
        let mut input = vec![0.0f32; 44100 * 2];
        let at = 10_000;
        input[at * 2] = 1.0;
        input[at * 2 + 1] = 1.0;
        let out = resample_stereo(&input, 44100, 48000).unwrap();
        let (l, _) = split(&out);
        let (peak, _) = l
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap();
        let expected = output_frames(at, 44100, 48000);
        assert!(
            (peak as i64 - expected as i64).abs() <= 2,
            "peak at {peak}, expected {expected}"
        );
    }

    /// Minimal IEEE-float WAV with the given interleaved samples. With `mask`
    /// it is WAVE_FORMAT_EXTENSIBLE and carries that channel mask. Returns the
    /// path; the caller removes `path.parent()` when done.
    fn write_float_wav(
        tag: &str,
        rate: u32,
        channels: u16,
        mask: Option<u32>,
        samples: &[f32],
    ) -> std::path::PathBuf {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let block_align = channels as u32 * 4;
        let mut fmt = Vec::new();
        let format_tag: u16 = if mask.is_some() { 0xFFFE } else { 3 };
        fmt.extend_from_slice(&format_tag.to_le_bytes());
        fmt.extend_from_slice(&channels.to_le_bytes());
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * block_align).to_le_bytes());
        fmt.extend_from_slice(&(block_align as u16).to_le_bytes());
        fmt.extend_from_slice(&32u16.to_le_bytes());
        if let Some(mask) = mask {
            fmt.extend_from_slice(&22u16.to_le_bytes()); // cbSize
            fmt.extend_from_slice(&32u16.to_le_bytes()); // valid bits
            fmt.extend_from_slice(&mask.to_le_bytes());
            // KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
            fmt.extend_from_slice(&[
                0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38,
                0x9b, 0x71,
            ]);
        }
        let mut wav = Vec::with_capacity(44 + fmt.len() + data.len());
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(4 + 8 + fmt.len() as u32 + 8 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        wav.extend_from_slice(&fmt);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);

        let dir = std::env::temp_dir().join(format!("helix-remix-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("float.wav");
        std::fs::write(&path, &wav).unwrap();
        path
    }

    #[test]
    fn quad_file_maps_front_pair_and_ignores_rear_channels() {
        // WAVE_FORMAT_EXTENSIBLE with the quad mask FL FR RL RR (0x33).
        let samples = [0.1, 0.2, 0.9, 0.8, -0.3, 0.7, 0.5, 0.5];
        let path = write_float_wav("float-quad", 48000, 4, Some(0x33), &samples);
        let result = decode_file(&path);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        let (s, rate) = result.unwrap();
        assert_eq!(rate, 48000);
        assert_eq!(s, [0.1, 0.2, -0.3, 0.7]);
    }

    #[test]
    fn float_wav_with_non_finite_samples_decodes_to_finite_samples() {
        let samples = [
            0.5,
            -0.5, // ordinary frame
            f32::NAN,
            0.25, // NaN on the left
            f32::INFINITY,
            f32::NEG_INFINITY, // both infinite
            3e38,
            -3e38, // finite but beyond any sane level
            0.1,
            0.2,
        ];
        let path = write_float_wav("float-nan", 44100, 2, None, &samples);
        let result = decode_file(&path);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());

        let (s, rate) = result.unwrap();
        assert_eq!(rate, 44100);
        assert!(s.iter().all(|x| x.is_finite()), "{s:?}");
        assert_eq!(s, [0.5, -0.5, 0.0, 0.25, 0.0, 0.0, 16.0, -16.0, 0.1, 0.2]);
    }

    #[test]
    fn resample_survives_non_finite_and_huge_samples() {
        let mut input = sine_stereo(44100, 0.2, 1000.0, 0.5, 0.5);
        input[2000] = f32::NAN;
        input[2001] = f32::INFINITY;
        input[4000] = f32::NEG_INFINITY;
        input[4001] = 3e38;
        input[6000] = -3e38;

        for (from, to) in [(44100, 48000), (48000, 44100), (44100, 44100)] {
            let out =
                resample_stereo(&input, from, to).unwrap_or_else(|e| panic!("{from}->{to}: {e}"));
            assert!(
                out.iter().all(|x| x.is_finite()),
                "{from}->{to}: non-finite"
            );
        }

        // Same-rate path returns a copy with the bad samples cleaned.
        let same = resample_stereo(&input, 44100, 44100).unwrap();
        assert_eq!(same.len(), input.len());
        assert_eq!(same[2000], 0.0);
        assert_eq!(same[2001], 0.0);
        assert_eq!(same[4000], 0.0);
        assert_eq!(same[4001], 16.0);
        assert_eq!(same[6000], -16.0);
    }

    #[test]
    fn resample_70001_to_48000_keeps_the_tail() {
        // gcd is 1, so one FFT block is 70001 input frames: longer than the
        // old fixed limit of idle flush calls, which left the tail silent.
        let input = sine_stereo(70001, 1.0, 1000.0, 0.5, 0.5);
        let out = resample_stereo(&input, 70001, 48000).unwrap();
        assert_eq!(out.len() / 2, 48000);
        assert!(out.iter().all(|x| x.is_finite()));

        let (l, _) = split(&out);
        let tail = &l[l.len() - 4800..l.len() - 500];
        let expected = 0.5 / std::f64::consts::SQRT_2;
        let db = 20.0 * (rms(tail) / expected).log10();
        assert!(db.abs() <= 1.0, "tail level off by {db:.2} dB");
    }

    #[test]
    fn resample_tail_is_not_silent_for_large_block_pairs() {
        for (from, to) in [
            (70001u32, 48000u32),
            (48000, 70001),
            (65537, 44100),
            (44100, 768000),
        ] {
            let input = sine_stereo(from, 0.5, 1000.0, 0.5, 0.5);
            let out = resample_stereo(&input, from, to).unwrap();
            let expected = output_frames(input.len() / 2, from, to);
            assert_eq!(out.len() / 2, expected, "{from}->{to}");
            let (l, _) = split(&out);
            let tail = &l[l.len() - expected / 10..l.len() - expected / 50];
            let db = 20.0 * (rms(tail) / (0.5 / std::f64::consts::SQRT_2)).log10();
            assert!(
                db.abs() <= 1.0,
                "{from}->{to}: tail level off by {db:.2} dB"
            );
        }
    }

    #[test]
    fn resample_48000_to_44100_length() {
        let input = sine_stereo(48000, 1.0, 1000.0, 0.5, 0.5);
        let out = resample_stereo(&input, 48000, 44100).unwrap();
        let frames = out.len() / 2;
        assert!((frames as i64 - 44100).abs() <= 1, "frames {frames}");
        assert!(out.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn resample_length_is_exact_rounding_for_odd_sizes() {
        for frames in [1usize, 2, 1023, 1024, 1025, 5000, 12345] {
            let input = sine_stereo(44100, frames as f64 / 44100.0, 440.0, 0.5, 0.5);
            let out = resample_stereo(&input, 44100, 48000).unwrap();
            let expected = (frames as f64 * 48000.0 / 44100.0).round() as usize;
            assert_eq!(out.len() / 2, expected, "input frames {frames}");
            assert_eq!(out.len() % 2, 0);
            assert!(out.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn resample_same_rate_and_empty_input() {
        let input = sine_stereo(44100, 0.01, 440.0, 0.5, 0.5);
        assert_eq!(resample_stereo(&input, 44100, 44100).unwrap(), input);
        assert!(resample_stereo(&[], 44100, 48000).unwrap().is_empty());
    }

    #[test]
    fn resample_zero_rate_is_an_error() {
        let input = sine_stereo(44100, 0.01, 440.0, 0.5, 0.5);
        assert!(matches!(
            resample_stereo(&input, 0, 48000),
            Err(LoadError::Decode(_))
        ));
        assert!(matches!(
            resample_stereo(&input, 44100, 0),
            Err(LoadError::Decode(_))
        ));
    }

    const STEREO: Channels = Channels::FRONT_LEFT.union(Channels::FRONT_RIGHT);
    const SURROUND_5_1: Channels = STEREO
        .union(Channels::FRONT_CENTRE)
        .union(Channels::LFE1)
        .union(Channels::REAR_LEFT)
        .union(Channels::REAR_RIGHT);

    #[test]
    fn mono_and_stereo_frames_map_to_lr() {
        let mut out = Vec::new();
        push_stereo(&mut out, &[0.25, -0.5], Channels::FRONT_LEFT);
        assert_eq!(out, [0.25, 0.25, -0.5, -0.5]);

        out.clear();
        push_stereo(&mut out, &[0.1, 0.2, 0.3, 0.4], STEREO);
        assert_eq!(out, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn surround_downmix_keeps_full_scale_centre_and_front_under_one() {
        // Three channels: L, R, centre.
        let three = STEREO.union(Channels::FRONT_CENTRE);
        let mut out = Vec::new();
        push_stereo(&mut out, &[0.0, 0.0, 1.0], three);
        // 0.7071 / 1.7071, the documented mix for a centre-only frame.
        let centre_only = 0.4142_f32;
        assert!((out[0] - centre_only).abs() < 1e-3, "{out:?}");
        assert!((out[1] - centre_only).abs() < 1e-3, "{out:?}");

        // Six channels: L, R, C, LFE, Ls, Rs. Full-scale L plus C stays at 1.0.
        out.clear();
        push_stereo(&mut out, &[1.0, -1.0, 1.0, 0.0, 0.0, 0.0], SURROUND_5_1);
        assert!((out[0] - 1.0).abs() < 1e-6, "{out:?}");
        assert!(out.iter().all(|x| x.abs() <= 1.0 + 1e-6), "{out:?}");
    }

    #[test]
    fn quad_layout_is_front_pair_without_centre_or_rear_mix() {
        // Quad is FL FR RL RR: index 2 is rear-left, not a centre. Rear values
        // must not leak into L/R, and nothing is scaled.
        let quad = STEREO
            .union(Channels::REAR_LEFT)
            .union(Channels::REAR_RIGHT);
        let mut out = Vec::new();
        push_stereo(&mut out, &[0.1, 0.2, 0.9, 0.8, -0.3, 0.7, 0.5, 0.5], quad);
        assert_eq!(out, [0.1, 0.2, -0.3, 0.7]);
    }

    #[test]
    fn two_point_one_drops_lfe_and_keeps_front_pair() {
        let layout = STEREO.union(Channels::LFE1);
        let mut out = Vec::new();
        push_stereo(&mut out, &[0.1, 0.2, 0.9, -0.4, 0.3, 0.9], layout);
        assert_eq!(out, [0.1, 0.2, -0.4, 0.3]);
    }

    #[test]
    fn channel_positions_follow_bit_order() {
        // Interleaved order is ascending bit order: in 5.1 the centre is the
        // third channel and rear-left the fifth.
        assert_eq!(index_of(SURROUND_5_1, Channels::FRONT_CENTRE), Some(2));
        assert_eq!(index_of(SURROUND_5_1, Channels::REAR_LEFT), Some(4));
        assert_eq!(index_of(STEREO, Channels::FRONT_CENTRE), None);
        assert_eq!(front_pair(SURROUND_5_1), (0, 1));
    }

    #[test]
    fn push_stereo_sanitizes_every_sample() {
        let mut out = Vec::new();
        push_stereo(&mut out, &[f32::NAN, f32::INFINITY, 3e38, -3e38], STEREO);
        assert_eq!(out, [0.0, 0.0, 16.0, -16.0]);

        out.clear();
        push_stereo(&mut out, &[f32::NEG_INFINITY, 1e30], Channels::FRONT_LEFT);
        assert_eq!(out, [0.0, 0.0, 16.0, 16.0]);

        out.clear();
        push_stereo(
            &mut out,
            &[f32::NAN, 0.5, 0.25],
            STEREO.union(Channels::FRONT_CENTRE),
        );
        assert!(out.iter().all(|x| x.is_finite()), "{out:?}");
    }

    #[test]
    fn panics_inside_the_decoder_become_decode_errors() {
        let caught: Result<(), LoadError> = guard_panics(|| panic!("upstream demuxer panicked"));
        assert_eq!(caught, Err(LoadError::Decode("decoder panicked".into())));

        let passed: Result<u32, LoadError> = guard_panics(|| Ok(7));
        assert_eq!(passed, Ok(7));
    }

    #[test]
    fn output_frames_rounds_half_up_exactly() {
        assert_eq!(output_frames(44100, 44100, 48000), 48000);
        assert_eq!(output_frames(48000, 48000, 44100), 44100);
        assert_eq!(output_frames(10_000, 44100, 48000), 10_884);
        assert_eq!(output_frames(1, 48000, 44100), 1);
        assert_eq!(output_frames(0, 44100, 48000), 0);
    }
}
