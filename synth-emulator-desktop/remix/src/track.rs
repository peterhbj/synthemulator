use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::analysis::{Analysis, Overview};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A decoded track held fully in memory, already resampled to the engine
/// rate. Shared between the UI and the audio thread behind an `Arc`.
#[derive(Debug)]
pub struct Track {
    /// Unique per load, so the UI can match deck state to its own copy.
    pub id: u64,
    pub title: String,
    pub path: PathBuf,
    pub sample_rate: u32,
    /// Interleaved stereo `L, R, L, R, ...` at `sample_rate`.
    pub samples: Vec<f32>,
    pub analysis: Analysis,
    pub overview: Overview,
}

impl Track {
    pub fn new(
        title: String,
        path: PathBuf,
        sample_rate: u32,
        samples: Vec<f32>,
        analysis: Analysis,
        overview: Overview,
    ) -> Self {
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            title,
            path,
            sample_rate,
            samples,
            analysis,
            overview,
        }
    }

    /// Track built straight from samples, for tests and generated material.
    pub fn from_samples(title: &str, samples: Vec<f32>, sample_rate: u32, analysis: Analysis) -> Self {
        Self::new(
            title.to_string(),
            PathBuf::new(),
            sample_rate,
            samples,
            analysis,
            Overview::default(),
        )
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames() as f64 / self.sample_rate as f64
    }

    /// Stereo frame `i`, or silence outside the track.
    #[inline]
    pub fn frame(&self, i: isize) -> (f32, f32) {
        if i < 0 {
            return (0.0, 0.0);
        }
        let i = i as usize * 2;
        match (self.samples.get(i), self.samples.get(i + 1)) {
            (Some(l), Some(r)) => (*l, *r),
            _ => (0.0, 0.0),
        }
    }
}
