//! Tempo detection, beat grid helpers, tap tempo and the waveform overview.

/// Tempo and beat grid of a track. Beat `n` sits at
/// `first_beat_secs + n * 60 / bpm` (n may be negative).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Analysis {
    /// Detected tempo; `None` when there is no confident pulse.
    pub bpm: Option<f64>,
    /// Grid anchor: time of a beat near the start of the music, in seconds.
    pub first_beat_secs: f64,
    /// 0..=1, how sure the detector is.
    pub confidence: f32,
}

impl Analysis {
    pub fn beat_len_secs(&self) -> Option<f64> {
        self.bpm.filter(|b| *b > 0.0).map(|b| 60.0 / b)
    }

    /// Fractional beat index at `t_secs` (negative before the anchor).
    pub fn beat_at(&self, t_secs: f64) -> Option<f64> {
        self.beat_len_secs()
            .map(|len| (t_secs - self.first_beat_secs) / len)
    }

    /// Time of (fractional) beat index `beat`.
    pub fn time_of_beat(&self, beat: f64) -> Option<f64> {
        self.beat_len_secs()
            .map(|len| self.first_beat_secs + beat * len)
    }
}

/// Detect tempo and a beat-grid anchor.
///
/// `samples` is interleaved stereo at `sample_rate`. Result tempo is folded
/// into 70..=180 BPM (DJ convention: 140 not 70, 87 not 174 only if the
/// pulse is genuinely half-time) and rounded to 0.01 BPM. Silence or
/// arrhythmic material returns `bpm: None, confidence < 0.3`.
pub fn detect_tempo(samples: &[f32], sample_rate: u32) -> Analysis {
    let _ = (samples, sample_rate);
    todo!("analysis agent")
}

/// One column of the waveform overview. Values are 0..=1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OverviewBin {
    /// Max absolute sample (L/R) in the bin.
    pub peak: f32,
    /// RMS-ish energy of the band (< ~250 Hz), normalized against the
    /// track's loudest bin so the louder bands read as 1.0.
    pub low: f32,
    /// 250 Hz .. 4 kHz.
    pub mid: f32,
    /// > 4 kHz.
    pub high: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Overview {
    pub bins_per_sec: f32,
    pub bins: Vec<OverviewBin>,
}

impl Overview {
    pub fn bin_at(&self, t_secs: f64) -> Option<&OverviewBin> {
        if t_secs < 0.0 || self.bins_per_sec <= 0.0 {
            return None;
        }
        self.bins.get((t_secs * self.bins_per_sec as f64) as usize)
    }
}

/// Resolution used by `load_track`.
pub const OVERVIEW_BINS_PER_SEC: f32 = 100.0;

/// Build the colored waveform overview (`bins_per_sec` columns per second).
pub fn build_overview(samples: &[f32], sample_rate: u32, bins_per_sec: f32) -> Overview {
    let _ = (samples, sample_rate, bins_per_sec);
    todo!("analysis agent")
}

/// Tap tempo: average of the recent tap intervals.
#[derive(Clone, Debug, Default)]
pub struct TapTempo {
    taps: Vec<f64>,
}

impl TapTempo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tap at `t_secs`. Returns the BPM once there are at least
    /// 3 taps. A gap over 2 s starts a new series. Uses the last 8 taps.
    pub fn tap(&mut self, t_secs: f64) -> Option<f64> {
        let _ = t_secs;
        let _ = &self.taps;
        todo!("analysis agent")
    }

    pub fn reset(&mut self) {
        self.taps.clear();
    }
}
