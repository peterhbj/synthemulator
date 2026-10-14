//! Musical key detection (chromagram against key profiles) and the Camelot
//! wheel arithmetic a DJ uses to pick compatible keys.

use std::f64::consts::PI;

use realfft::RealFftPlanner;

/// Pitch class names, sharps only (C = 0).
const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// A key: tonic pitch class (0 = C) and mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub tonic: u8,
    pub minor: bool,
}

impl Key {
    pub fn new(tonic: i32, minor: bool) -> Self {
        Self {
            tonic: tonic.rem_euclid(12) as u8,
            minor,
        }
    }

    /// Musical name, e.g. `Am`, `F#`, `C#m`.
    pub fn name(self) -> String {
        format!("{}{}", NAMES[self.tonic as usize], if self.minor { "m" } else { "" })
    }

    /// Camelot code, e.g. `8A` (A minor) or `8B` (C major).
    pub fn camelot(self) -> String {
        let (n, letter) = self.camelot_parts();
        format!("{n}{letter}")
    }

    /// Camelot number 1..=12 and letter (`A` minor, `B` major).
    pub fn camelot_parts(self) -> (u8, char) {
        // Majors: C = 8B, each fifth up is +1. Minors: A = 8A.
        let base = if self.minor { 9 } else { 0 };
        let fifths = (7 * (self.tonic as i32 - base)).rem_euclid(12);
        let n = (fifths + 7).rem_euclid(12) + 1;
        (n as u8, if self.minor { 'A' } else { 'B' })
    }

    /// The key `semitones` higher (negative = lower), same mode.
    pub fn transposed(self, semitones: i32) -> Self {
        Self::new(self.tonic as i32 + semitones, self.minor)
    }

    /// Keys that mix harmonically with this one on the Camelot wheel: the
    /// same key, one step either way, and the relative major/minor.
    pub fn compatible(self) -> [Key; 4] {
        let relative = if self.minor {
            Key::new(self.tonic as i32 + 3, false)
        } else {
            Key::new(self.tonic as i32 - 3, true)
        };
        [self, self.transposed(7), self.transposed(-7), relative]
    }

    pub fn is_compatible(self, other: Key) -> bool {
        self.compatible().contains(&other)
    }
}

/// Result of key detection.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct KeyAnalysis {
    /// Detected key; `None` for silence or material with no tonal content.
    pub key: Option<Key>,
    /// Offset of the track's tuning from A = 440 Hz, in cents (-50..50).
    pub tuning_cents: f64,
    /// 0..=1, margin of the winning key over the runner-up.
    pub confidence: f32,
}

/// Below this confidence the UI marks the key as a guess.
pub const KEY_UNSURE: f32 = 0.05;

/// Pitch of a key at a playback speed, in fractional semitones above C,
/// tuning included. Two decks sound in the same key when their pitches
/// differ by a whole number of semitones that maps one key onto the other.
pub fn pitch_at_speed(key: Key, tuning_cents: f64, speed: f64) -> f64 {
    let shift = if speed > 0.0 { 12.0 * speed.log2() } else { 0.0 };
    key.tonic as f64 + tuning_cents / 100.0 + shift
}

/// Key heard at `speed` (no key lock: speed moves pitch), rounded to the
/// nearest semitone, and how far off that semitone it sounds, in cents.
pub fn key_at_speed(key: Key, tuning_cents: f64, speed: f64) -> (Key, f64) {
    let p = pitch_at_speed(key, tuning_cents, speed);
    let r = p.round();
    (Key::new(r as i32, key.minor), (p - r) * 100.0)
}

/// Speed for deck `b` (key `b_key`, tuning `b_tuning`) that makes it sound
/// in a key compatible with `target` (the key the other deck sounds in,
/// at `target_pitch` fractional semitones). Picks the compatible key needing
/// the smallest change from `b_speed`. Returns `(speed, key reached)`.
pub fn speed_to_match(
    b_key: Key,
    b_tuning: f64,
    b_speed: f64,
    target: Key,
    target_pitch: f64,
) -> Option<(f64, Key)> {
    let detune = target_pitch - target_pitch.round();
    let b_now = pitch_at_speed(b_key, b_tuning, b_speed);
    let mut best: Option<(f64, f64, Key)> = None;
    for want in target.compatible() {
        if want.minor != b_key.minor {
            continue;
        }
        // Wanted pitch (same detune as the target), the closest octave to now.
        let base = want.tonic as f64 + detune;
        let mut delta = (base - b_now).rem_euclid(12.0);
        if delta > 6.0 {
            delta -= 12.0;
        }
        if best.is_none_or(|(d, _, _)| delta.abs() < d.abs()) {
            best = Some((delta, 0.0, want));
        }
    }
    best.map(|(delta, _, want)| (b_speed * 2f64.powf(delta / 12.0), want))
}

// ---------------------------------------------------------------------------

/// Analysis rate after decimation; 6 kHz Nyquist is plenty for chroma.
const RATE_DIV: usize = 4;
const FFT_LEN: usize = 8192;
const HOP: usize = 4096;
/// Pitch range for the chroma, A1 to A6.
const F_LO: f64 = 55.0;
const F_HI: f64 = 1760.0;
/// Peaks quieter than this share of the frame's loudest are ignored.
const PEAK_FLOOR: f64 = 0.02;
/// Longest stretch analysed (from the middle of longer tracks).
const MAX_SECS: f64 = 360.0;
const SILENCE_RMS: f64 = 1e-4;

/// Key profiles from libKeyFinder (Sha'ath 2011), tuned on popular and
/// electronic music; index 0 is the tonic.
const MAJOR: [f64; 12] = [
    7.239_005_02, 3.507_823_88, 3.584_354_53, 2.845_115_42, 5.819_150_97, 4.559_629_57,
    2.446_706_37, 6.999_380_51, 3.399_341_69, 4.556_235_18, 4.075_956_17, 4.459_094_04,
];
const MINOR: [f64; 12] = [
    7.002_557_76, 3.144_224_89, 4.361_205_09, 5.403_712_53, 3.673_330_15, 4.089_316_48,
    3.912_080_86, 6.203_195_57, 3.863_180_81, 2.886_374_36, 5.354_251_33, 3.830_020_98,
];

/// Detect the key of interleaved stereo `samples` at `sample_rate`.
pub fn detect_key(samples: &[f32], sample_rate: u32) -> KeyAnalysis {
    let none = KeyAnalysis::default();
    let frames = samples.len() / 2;
    if sample_rate == 0 || frames == 0 {
        return none;
    }
    let mono = decimated_mono(samples, sample_rate as f64);
    let sr = sample_rate as f64 / RATE_DIV as f64;
    let rms = (mono.iter().map(|x| x * x).sum::<f64>() / mono.len().max(1) as f64).sqrt();
    if rms.is_nan() || rms < SILENCE_RMS || mono.len() < FFT_LEN {
        return none;
    }

    // Middle `MAX_SECS` of long tracks.
    let max_len = (MAX_SECS * sr) as usize;
    let mono = if mono.len() > max_len {
        let start = (mono.len() - max_len) / 2;
        &mono[start..start + max_len]
    } else {
        &mono[..]
    };

    let peaks = spectral_peaks(mono, sr);
    if peaks.is_empty() {
        return none;
    }
    let tuning = estimate_tuning(&peaks);
    let chroma = chromagram(&peaks, tuning);
    if chroma.iter().all(|&c| c <= 0.0) {
        return none;
    }

    let mut scores: Vec<(f64, Key)> = Vec::with_capacity(24);
    for tonic in 0..12 {
        for minor in [false, true] {
            let profile = if minor { &MINOR } else { &MAJOR };
            let rotated: [f64; 12] = std::array::from_fn(|i| profile[(i + 12 - tonic) % 12]);
            scores.push((correlation(&chroma, &rotated), Key::new(tonic as i32, minor)));
        }
    }
    scores.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (best, key) = scores[0];
    let second = scores[1].0;
    if !best.is_finite() || best <= 0.0 {
        return none;
    }
    KeyAnalysis {
        key: Some(key),
        tuning_cents: tuning * 100.0,
        confidence: (best - second).clamp(0.0, 1.0) as f32,
    }
}

/// Mono mixdown, low-passed (two 4th-order sections at 2.5 kHz) and
/// decimated by `RATE_DIV`.
fn decimated_mono(samples: &[f32], sr: f64) -> Vec<f64> {
    let mut lp = [Lp::new(sr, 2500.0), Lp::new(sr, 2500.0)];
    let frames = samples.len() / 2;
    let mut out = Vec::with_capacity(frames / RATE_DIV + 1);
    for f in 0..frames {
        let l = samples[2 * f];
        let r = samples[2 * f + 1];
        let m = 0.5 * (fin(l) + fin(r));
        let y0 = lp[0].process(m);
        let y = lp[1].process(y0);
        if f % RATE_DIV == 0 {
            out.push(y);
        }
    }
    out
}

#[inline]
fn fin(x: f32) -> f64 {
    if x.is_finite() {
        x as f64
    } else {
        0.0
    }
}

/// Butterworth biquad lowpass.
struct Lp {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Lp {
    fn new(sr: f64, freq: f64) -> Self {
        let w0 = 2.0 * PI * freq.min(0.45 * sr) / sr;
        let (cw, sw) = (w0.cos(), w0.sin());
        let alpha = sw * std::f64::consts::FRAC_1_SQRT_2;
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - cw) * 0.5 / a0,
            b1: (1.0 - cw) / a0,
            b2: (1.0 - cw) * 0.5 / a0,
            a1: -2.0 * cw / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// A spectral peak: frame index, fractional MIDI-style pitch (A4 = 69) and
/// magnitude normalised to the frame's loudest peak.
#[derive(Clone, Copy, Debug)]
struct Peak {
    frame: usize,
    pitch: f64,
    mag: f64,
}

/// Local maxima of the Hann-windowed magnitude spectrum of every frame,
/// with parabolic (log-magnitude) interpolation of their frequency.
fn spectral_peaks(mono: &[f64], sr: f64) -> Vec<Peak> {
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(FFT_LEN);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    let window: Vec<f64> = (0..FFT_LEN)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / FFT_LEN as f64).cos())
        .collect();
    let bin_hz = sr / FFT_LEN as f64;
    let k_lo = ((F_LO * 0.97) / bin_hz).floor().max(1.0) as usize;
    let k_hi = ((F_HI * 1.03) / bin_hz).ceil() as usize;
    let k_hi = k_hi.min(FFT_LEN / 2 - 1);

    let mut peaks = Vec::new();
    let mut mag = vec![0.0f64; FFT_LEN / 2 + 1];
    let mut frame = 0usize;
    let mut start = 0usize;
    while start + FFT_LEN <= mono.len() {
        for (i, x) in input.iter_mut().enumerate() {
            *x = mono[start + i] * window[i];
        }
        if fft.process(&mut input, &mut spectrum).is_err() {
            break;
        }
        for (m, c) in mag.iter_mut().zip(&spectrum) {
            *m = c.norm();
        }
        let loudest = mag[k_lo..=k_hi].iter().copied().fold(0.0, f64::max);
        if loudest > 1e-9 {
            for k in k_lo..=k_hi {
                let m = mag[k];
                if m < PEAK_FLOOR * loudest || m <= mag[k - 1] || m < mag[k + 1] {
                    continue;
                }
                let (a, b, c) = (mag[k - 1].max(1e-12).ln(), m.ln(), mag[k + 1].max(1e-12).ln());
                let denom = a - 2.0 * b + c;
                let delta = if denom < 0.0 { (0.5 * (a - c) / denom).clamp(-0.5, 0.5) } else { 0.0 };
                let hz = (k as f64 + delta) * bin_hz;
                if !(F_LO..=F_HI).contains(&hz) {
                    continue;
                }
                peaks.push(Peak {
                    frame,
                    pitch: 69.0 + 12.0 * (hz / 440.0).log2(),
                    mag: m / loudest,
                });
            }
        }
        frame += 1;
        start += HOP;
    }
    peaks
}

/// Tuning offset in semitones (-0.5..0.5): the magnitude-weighted circular
/// mean of every peak's distance from the nearest equal-tempered pitch.
fn estimate_tuning(peaks: &[Peak]) -> f64 {
    let (mut s, mut c) = (0.0, 0.0);
    for p in peaks {
        let a = 2.0 * PI * p.pitch;
        s += p.mag * a.sin();
        c += p.mag * a.cos();
    }
    if s == 0.0 && c == 0.0 {
        return 0.0;
    }
    s.atan2(c) / (2.0 * PI)
}

/// Pitch-class profile: each peak adds its magnitude to its pitch class,
/// weighted down as it strays from the tuned semitone centre. Each frame is
/// normalised first so loud passages do not outvote the rest.
fn chromagram(peaks: &[Peak], tuning: f64) -> [f64; 12] {
    let mut total = [0.0f64; 12];
    let mut frame_chroma = [0.0f64; 12];
    let mut current = usize::MAX;
    let flush = |fc: &mut [f64; 12], total: &mut [f64; 12]| {
        let norm = fc.iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm > 0.0 {
            for (t, v) in total.iter_mut().zip(fc.iter()) {
                *t += v / norm;
            }
        }
        *fc = [0.0; 12];
    };
    for p in peaks {
        if p.frame != current {
            flush(&mut frame_chroma, &mut total);
            current = p.frame;
        }
        let x = p.pitch - tuning;
        let r = x.round();
        let dev = x - r;
        let w = (-(dev * dev) / (2.0 * 0.2 * 0.2)).exp();
        frame_chroma[(r as i64).rem_euclid(12) as usize] += p.mag * w;
    }
    flush(&mut frame_chroma, &mut total);
    total
}

/// Pearson correlation of two 12-vectors.
fn correlation(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let ma = a.iter().sum::<f64>() / 12.0;
    let mb = b.iter().sum::<f64>() / 12.0;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        let x = a[i] - ma;
        let y = b[i] - mb;
        num += x * y;
        da += x * x;
        db += y * y;
    }
    let den = (da * db).sqrt();
    if den > 0.0 {
        num / den
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;

    /// Stereo track of sustained chords, each a list of MIDI notes with
    /// three harmonics, `secs_each` long, tuned `cents` off A440.
    fn chords(progression: &[&[i32]], secs_each: f64, repeats: usize, cents: f64) -> Vec<f32> {
        let fs = SR as f64;
        let mut out = Vec::new();
        for _ in 0..repeats {
            for notes in progression {
                let n = (secs_each * fs) as usize;
                for i in 0..n {
                    let t = i as f64 / fs;
                    let env = (t / 0.02).min(1.0) * (-(t) / 3.0).exp();
                    let mut v = 0.0;
                    for &m in *notes {
                        let f = 440.0 * 2f64.powf((m as f64 - 69.0 + cents / 100.0) / 12.0);
                        for h in 1..=3 {
                            v += (2.0 * PI * f * h as f64 * t).sin() / h as f64;
                        }
                    }
                    let s = (0.08 * env * v) as f32;
                    out.push(s);
                    out.push(s);
                }
            }
        }
        out
    }

    #[test]
    fn camelot_codes() {
        assert_eq!(Key::new(0, false).camelot(), "8B"); // C
        assert_eq!(Key::new(9, true).camelot(), "8A"); // Am
        assert_eq!(Key::new(7, false).camelot(), "9B"); // G
        assert_eq!(Key::new(5, false).camelot(), "7B"); // F
        assert_eq!(Key::new(4, true).camelot(), "9A"); // Em
        assert_eq!(Key::new(6, false).camelot(), "2B"); // F#
        assert_eq!(Key::new(8, true).camelot(), "1A"); // G#m
        assert_eq!(Key::new(2, true).camelot(), "7A"); // Dm
        assert_eq!(Key::new(11, false).camelot(), "1B"); // B
    }

    #[test]
    fn names() {
        assert_eq!(Key::new(9, true).name(), "Am");
        assert_eq!(Key::new(1, false).name(), "C#");
        assert_eq!(Key::new(-1, true).name(), "Bm");
    }

    #[test]
    fn compatible_keys_are_wheel_neighbours() {
        let am = Key::new(9, true);
        let names: Vec<String> = am.compatible().iter().map(|k| k.camelot()).collect();
        assert_eq!(names, ["8A", "9A", "7A", "8B"]);
        assert!(am.is_compatible(Key::new(0, false)));
        assert!(!am.is_compatible(Key::new(2, false)));
    }

    #[test]
    fn speed_moves_the_key() {
        let am = Key::new(9, true);
        // One semitone up is +5.95 %.
        let (k, cents) = key_at_speed(am, 0.0, 2f64.powf(1.0 / 12.0));
        assert_eq!(k.name(), "A#m");
        assert!(cents.abs() < 1e-6);
        let (k, cents) = key_at_speed(am, 0.0, 1.02);
        assert_eq!(k.name(), "Am");
        assert!((cents - 34.28).abs() < 0.1, "{cents}");
    }

    #[test]
    fn speed_to_match_picks_the_nearest_compatible_key() {
        // B in Gm at speed 1, A in Am at speed 1: Gm -> Am is +2 st, but
        // Gm -> Dm (7A, a neighbour of 8A) is -5 / +7, and Gm -> Am is the
        // nearest compatible minor key... Em (9A) is -3. Nearest is +2 (Am).
        let gm = Key::new(7, true);
        let am = Key::new(9, true);
        let (speed, want) = speed_to_match(gm, 0.0, 1.0, am, 9.0).unwrap();
        assert_eq!(want, am);
        assert!((speed - 2f64.powf(2.0 / 12.0)).abs() < 1e-9);
        // A semitone away: Bbm -> Am needs -1 st.
        let bbm = Key::new(10, true);
        let (speed, _) = speed_to_match(bbm, 0.0, 1.0, am, 9.0).unwrap();
        assert!((speed - 2f64.powf(-1.0 / 12.0)).abs() < 1e-9);
    }

    #[test]
    fn detects_c_major_progression() {
        // I - IV - V - I in C.
        let s = chords(&[&[48, 60, 64, 67], &[53, 57, 60, 65], &[55, 59, 62, 67], &[48, 60, 64, 67]], 2.0, 3, 0.0);
        let k = detect_key(&s, SR);
        assert_eq!(k.key, Some(Key::new(0, false)), "{k:?}");
        assert!(k.tuning_cents.abs() < 5.0, "{k:?}");
    }

    #[test]
    fn detects_a_minor_progression() {
        // i - iv - V - i in A minor (E major chord with G#).
        let s = chords(&[&[45, 57, 60, 64], &[50, 57, 62, 65], &[52, 56, 59, 64], &[45, 57, 60, 64]], 2.0, 3, 0.0);
        let k = detect_key(&s, SR);
        assert_eq!(k.key, Some(Key::new(9, true)), "{k:?}");
    }

    #[test]
    fn detects_detuned_f_sharp_minor_and_its_tuning() {
        // i - VI - VII - i in F# minor, 30 cents sharp.
        let s = chords(&[&[42, 54, 57, 61], &[50, 54, 57, 62], &[52, 56, 59, 64], &[42, 54, 57, 61]], 2.0, 3, 30.0);
        let k = detect_key(&s, SR);
        assert_eq!(k.key, Some(Key::new(6, true)), "{k:?}");
        assert!((k.tuning_cents - 30.0).abs() < 6.0, "{k:?}");
    }

    #[test]
    fn silence_has_no_key() {
        assert_eq!(detect_key(&vec![0.0; 2 * SR as usize * 5], SR).key, None);
        assert_eq!(detect_key(&[], SR).key, None);
        assert_eq!(detect_key(&[0.1, 0.1], 0).key, None);
    }
}
