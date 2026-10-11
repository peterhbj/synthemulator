//! File decoding (symphonia) and resampling (rubato).

use std::fmt;
use std::path::Path;

use crate::analysis::{build_overview, detect_tempo, OVERVIEW_BINS_PER_SEC};
use crate::track::Track;

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
    let _ = path;
    todo!("decode agent")
}

/// Resample interleaved stereo from `from` Hz to `to` Hz with a
/// band-limited sinc resampler. Output length is
/// `round(frames * to / from)` frames (±1). `from == to` returns a copy.
pub fn resample_stereo(samples: &[f32], from: u32, to: u32) -> Result<Vec<f32>, LoadError> {
    let _ = (samples, from, to);
    todo!("decode agent")
}

/// Decode, resample to `target_rate`, detect tempo and build the overview.
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
    Ok(Track::new(
        title,
        path.to_path_buf(),
        target_rate,
        samples,
        analysis,
        overview,
    ))
}
