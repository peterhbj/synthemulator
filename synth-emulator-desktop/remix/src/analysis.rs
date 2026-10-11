//! Tempo detection, beat grid helpers, tap tempo and the waveform overview.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

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

/// Analysis window length; longer tracks are analysed from a window that
/// starts at `ANALYSIS_WINDOW_START` of the track (skips intros).
const ANALYSIS_WINDOW_SECS: f64 = 120.0;
const ANALYSIS_WINDOW_START: f64 = 0.25;
/// Onset envelope frame rate. The hop is `round(sample_rate / ENVELOPE_RATE)`.
const ENVELOPE_RATE: f64 = 200.0;
/// Whole-track RMS of the mono mixdown below this is silence.
const SILENCE_RMS: f64 = 1e-4;
/// Confidence below this reports no tempo.
const MIN_CONFIDENCE: f64 = 0.25;
/// Phase offsets tried per candidate tempo in the coarse refinement.
const PHASE_STEPS: usize = 32;
/// The faster octave wins when its comb score reaches this share of the best.
const OCTAVE_RATIO: f64 = 0.6;
/// Reported tempo range (the contract folds results into it).
const BPM_MIN: f64 = 70.0;
const BPM_MAX: f64 = 180.0;
/// Coarse lag range for the confidence statistic.
const COARSE_BPM_MIN: f64 = 60.0;
const COARSE_BPM_MAX: f64 = 200.0;
/// A refined tempo further than this outside `BPM_MIN..=BPM_MAX` is folded by
/// an octave; closer ones are clamped (a 180.0 pulse must not become 90).
const FOLD_MARGIN: f64 = 0.5;
/// Half-width, in 0.01 BPM steps, of the 1 ms-phase re-search around the
/// winner of the 32-phase pass.
const FINE_STEPS: i32 = 5;
/// Snap to the nearest integer tempo only within this distance...
const SNAP_BPM: f64 = 0.03;
/// ...and only when the integer grid scores at least this share of the best.
const SNAP_SCORE_RATIO: f64 = 0.99;

/// Detect tempo and a beat-grid anchor.
///
/// `samples` is interleaved stereo at `sample_rate`. Result tempo is folded
/// into 70..=180 BPM (DJ convention: 140 not 70, 87 not 174 only if the
/// pulse is genuinely half-time) and rounded to 0.01 BPM. Silence or
/// arrhythmic material returns `bpm: None, confidence < 0.3`.
pub fn detect_tempo(samples: &[f32], sample_rate: u32) -> Analysis {
    let silent = Analysis {
        bpm: None,
        first_beat_secs: 0.0,
        confidence: 0.0,
    };
    let frames = samples.len() / 2;
    if sample_rate == 0 || frames == 0 {
        return silent;
    }
    let sr = sample_rate as f64;

    let energy: f64 = (0..frames)
        .map(|f| {
            let m = mono_at(samples, f);
            m * m
        })
        .sum();
    if (energy / frames as f64).sqrt() < SILENCE_RMS {
        return silent;
    }

    let hop = ((sr / ENVELOPE_RATE).round() as usize).max(1);
    let fr = sr / hop as f64;
    let total = frames / hop;

    // Analysis window in envelope frames: the whole track, or the 120 s
    // starting at 25 % of a longer one (pulled back so it is always 120 s).
    let max_frames = (ANALYSIS_WINDOW_SECS * fr).round() as usize;
    let (w0, n) = analysis_window(total, max_frames);

    let ((l_lo, l_hi), (c_lo, c_hi)) = lag_ranges(fr);
    if l_lo < 2 || c_lo < 1 || c_hi < c_lo || n < 4 * l_hi {
        return silent;
    }

    // Onset envelope of the whole track. The music start is searched over all
    // of it, and the analysis window is a hop-aligned slice, so the window has
    // real band history at its first frame (no fake onset at its start).
    let full = onset_envelope(samples, hop, total, sr, fr);
    let env = &full[w0..w0 + n];
    let vmax = env.iter().copied().fold(0.0f64, f64::max);
    let mean_energy = env.iter().map(|v| v * v).sum::<f64>() / n as f64;
    if !vmax.is_finite() || vmax <= 0.0 || !mean_energy.is_finite() || mean_energy <= 0.0 {
        return silent;
    }

    // Comb-filtered autocorrelation. Beats rarely fall on whole frames, so a
    // sharp onset would be counted only at the lags that happen to round onto
    // it. A short triangle kernel widens each onset by a frame on either side
    // for this step only; the grid refinement below keeps the unsmoothed
    // envelope for timing.
    let smooth = triangle_smooth(env);
    let smooth_energy = smooth.iter().map(|v| v * v).sum::<f64>() / n as f64;
    let max_lag = 2 * (c_hi.max(l_hi) + 1);
    let ac = autocorrelation(&smooth, max_lag.min(n - 1), smooth_energy);
    let mut best_lag = l_lo;
    let mut best_score = f64::NEG_INFINITY;
    for lag in l_lo..=l_hi {
        let s = comb_score(&ac, lag);
        if s > best_score {
            best_score = s;
            best_lag = lag;
        }
    }
    let mean_score =
        (c_lo..=c_hi).map(|lag| comb_score(&ac, lag)).sum::<f64>() / (c_hi - c_lo + 1) as f64;
    let confidence = ((best_score - mean_score) / best_score.max(1e-9)).clamp(0.0, 1.0);
    if confidence < MIN_CONFIDENCE {
        return Analysis {
            bpm: None,
            first_beat_secs: 0.0,
            confidence: confidence as f32,
        };
    }

    // Octave preference: when the half lag (double tempo) of the best peak is
    // itself a scored lag, take it if it keeps enough of the score.
    let mut lag = best_lag;
    let centre = (best_lag as f64 / 2.0).round() as usize;
    let mut fast: Option<(usize, f64)> = None;
    for cand in centre.saturating_sub(1)..=centre + 1 {
        if cand < l_lo || cand > l_hi {
            continue;
        }
        let s = comb_score(&ac, cand);
        if fast.is_none_or(|(_, best)| s > best) {
            fast = Some((cand, s));
        }
    }
    if let Some((cand, s)) = fast {
        if s >= OCTAVE_RATIO * best_score {
            lag = cand;
        }
    }

    // Parabolic interpolation around the chosen lag for sub-frame precision.
    let s_m = comb_score(&ac, lag - 1);
    let s_0 = comb_score(&ac, lag);
    let s_p = comb_score(&ac, lag + 1);
    let denom = s_m - 2.0 * s_0 + s_p;
    let delta = if denom < 0.0 {
        (0.5 * (s_m - s_p) / denom).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let coarse_bpm = 60.0 * fr / (lag as f64 + delta);

    // Refine: +-1 BPM in 0.01 steps, 32 phases per beat, mean envelope on the grid.
    let mut best = (f64::NEG_INFINITY, coarse_bpm);
    for i in 0..=200 {
        let bpm = coarse_bpm - 1.0 + i as f64 * 0.01;
        let beat = 60.0 / bpm;
        for p in 0..PHASE_STEPS {
            let phase = beat * p as f64 / PHASE_STEPS as f64;
            let s = grid_mean(env, fr, phase, beat);
            if s > best.0 {
                best = (s, bpm);
            }
        }
    }
    let refined = fold_bpm(best.1);

    // The 32 phases are 15-27 ms apart while envelope peaks are 1-2 frames
    // wide, so at the true tempo the grid can miss every peak while a tempo
    // 0.01-0.04 BPM away wins by drifting across them. Re-score the 0.01 BPM
    // lattice around the winner with 1 ms phases and keep the best.
    let (fine_score, fine_bpm) = fine_search(env, fr, refined);
    let mut bpm = round_bpm(fine_bpm.clamp(BPM_MIN, BPM_MAX));
    let score = if bpm == fine_bpm {
        fine_score
    } else {
        best_phase_ms(env, fr, 60.0 / bpm).0
    };

    // Snap to the integer tempo when it is within 0.03 BPM and its grid fits
    // the envelope practically as well as the fractional winner.
    let nearest = bpm.round();
    if nearest != bpm
        && (bpm - nearest).abs() <= SNAP_BPM
        && best_phase_ms(env, fr, 60.0 / nearest).0 >= SNAP_SCORE_RATIO * score
    {
        bpm = nearest;
    }
    let beat = 60.0 / bpm;
    // Phase at 1 ms resolution for the final tempo (window-relative seconds).
    let phase = best_phase_ms(env, fr, beat).1;

    // Anchor: earliest grid beat at or after the music start (searched over
    // the whole track), never negative.
    let win_start = (w0 * hop) as f64 / sr;
    let full_max = full.iter().copied().fold(0.0f64, f64::max);
    let Some(first_frame) = full.iter().position(|&v| v > 0.1 * full_max) else {
        return silent;
    };
    let music_start = (first_frame as f64 + 0.5) / fr - 0.030;
    let first = anchor_secs(win_start + phase, beat, music_start, hop as f64 / sr);

    Analysis {
        bpm: Some(bpm),
        first_beat_secs: first,
        confidence: confidence as f32,
    }
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
    if sample_rate == 0 || !(bins_per_sec.is_finite() && bins_per_sec > 0.0) {
        return Overview::default();
    }
    let sr = sample_rate as f64;
    let frames = samples.len() / 2;
    let per_bin = ((sr / bins_per_sec as f64).round() as usize).max(1);
    let n_bins = frames.div_ceil(per_bin);
    let mut bins = vec![OverviewBin::default(); n_bins];
    if n_bins == 0 {
        return Overview { bins_per_sec, bins };
    }

    let mut sums = vec![[0.0f64; 3]; n_bins];
    let mut bands = Bands::new(sr, 250.0, 250.0, 4000.0, 4000.0);
    for frame in 0..frames {
        let l = finite_or_zero(samples[2 * frame]);
        let r = finite_or_zero(samples[2 * frame + 1]);
        let b = frame / per_bin;
        let peak = l.abs().max(r.abs()) as f32;
        if peak > bins[b].peak {
            bins[b].peak = peak;
        }
        let [lo, mid, hi] = bands.process(0.5 * (l + r));
        sums[b][0] += lo * lo;
        sums[b][1] += mid * mid;
        sums[b][2] += hi * hi;
    }

    for (b, bin) in bins.iter_mut().enumerate() {
        bin.peak = bin.peak.min(1.0);
        let len = (frames - b * per_bin).min(per_bin) as f64;
        bin.low = (sums[b][0] / len).sqrt() as f32;
        bin.mid = (sums[b][1] / len).sqrt() as f32;
        bin.high = (sums[b][2] / len).sqrt() as f32;
    }

    // Each band is normalised by its own 99th percentile and clamped to 1.
    for band in 0..3 {
        let mut sorted: Vec<f32> = bins.iter_mut().map(|bin| *band_slot(bin, band)).collect();
        sorted.sort_by(f32::total_cmp);
        let p99 = sorted[((sorted.len() - 1) as f64 * 0.99).round() as usize];
        if p99 > 0.0 {
            for bin in bins.iter_mut() {
                let slot = band_slot(bin, band);
                *slot = (*slot / p99).min(1.0);
            }
        }
    }

    Overview { bins_per_sec, bins }
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
        if !t_secs.is_finite() {
            return None;
        }
        if let Some(&last) = self.taps.last() {
            if t_secs < last || t_secs - last > 2.0 {
                self.taps.clear();
            }
        }
        self.taps.push(t_secs);
        if self.taps.len() > 8 {
            let excess = self.taps.len() - 8;
            self.taps.drain(..excess);
        }
        if self.taps.len() < 3 {
            return None;
        }
        let first = self.taps[0];
        let last = self.taps[self.taps.len() - 1];
        let mean_interval = (last - first) / (self.taps.len() - 1) as f64;
        if mean_interval > 0.0 && mean_interval.is_finite() {
            Some(60.0 / mean_interval)
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.taps.clear();
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Sample value as f64, with NaN and infinities treated as silence.
#[inline]
fn finite_or_zero(x: f32) -> f64 {
    if x.is_finite() {
        x as f64
    } else {
        0.0
    }
}

/// Mono mixdown `(L + R) / 2` of interleaved stereo frame `frame`.
#[inline]
fn mono_at(samples: &[f32], frame: usize) -> f64 {
    0.5 * (finite_or_zero(samples[2 * frame]) + finite_or_zero(samples[2 * frame + 1]))
}

/// Second-order biquad (RBJ cookbook, Butterworth Q), transposed direct form II.
#[derive(Clone, Copy, Debug)]
struct Bq {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Bq {
    fn lowpass(sr: f64, freq: f64) -> Self {
        Self::design(sr, freq, false)
    }

    fn highpass(sr: f64, freq: f64) -> Self {
        Self::design(sr, freq, true)
    }

    fn design(sr: f64, freq: f64, high: bool) -> Self {
        let f = freq.clamp(1.0, 0.45 * sr);
        let w0 = 2.0 * PI * f / sr;
        let (cw, sw) = (w0.cos(), w0.sin());
        // alpha = sin(w0) / (2Q) with Q = 1/sqrt(2).
        let alpha = sw * FRAC_1_SQRT_2;
        let (b0, b1, b2) = if high {
            ((1.0 + cw) * 0.5, -(1.0 + cw), (1.0 + cw) * 0.5)
        } else {
            ((1.0 - cw) * 0.5, 1.0 - cw, (1.0 - cw) * 0.5)
        };
        let a0 = 1.0 + alpha;
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
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

/// Three-band split: low = lowpass, mid = highpass then lowpass, high = highpass.
struct Bands {
    low: Bq,
    mid_hp: Bq,
    mid_lp: Bq,
    high: Bq,
}

impl Bands {
    fn new(sr: f64, low_cut: f64, mid_lo: f64, mid_hi: f64, high_cut: f64) -> Self {
        Self {
            low: Bq::lowpass(sr, low_cut),
            mid_hp: Bq::highpass(sr, mid_lo),
            mid_lp: Bq::lowpass(sr, mid_hi),
            high: Bq::highpass(sr, high_cut),
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> [f64; 3] {
        let lo = self.low.process(x);
        let mid = self.mid_lp.process(self.mid_hp.process(x));
        let hi = self.high.process(x);
        [lo, mid, hi]
    }
}

fn band_slot(bin: &mut OverviewBin, band: usize) -> &mut f32 {
    match band {
        0 => &mut bin.low,
        1 => &mut bin.mid,
        _ => &mut bin.high,
    }
}

/// Onset strength per envelope frame for the first `n` hops of the track.
/// Frame `i` covers samples `[i*hop, (i+1)*hop)` and is timed at its centre,
/// which is the end-of-hop label shifted back by half a hop.
fn onset_envelope(samples: &[f32], hop: usize, n: usize, sr: f64, fr: f64) -> Vec<f64> {
    const WEIGHTS: [f64; 3] = [1.0, 0.6, 0.4];
    let mut bands = Bands::new(sr, 150.0, 150.0, 2000.0, 4000.0);
    let mut prev = [0.0f64; 3];
    let mut onset = Vec::with_capacity(n);
    for h in 0..n {
        let mut acc = [0.0f64; 3];
        let base = h * hop;
        for frame in base..base + hop {
            let [lo, mid, hi] = bands.process(mono_at(samples, frame));
            acc[0] += lo * lo;
            acc[1] += mid * mid;
            acc[2] += hi * hi;
        }
        let mut cur = [0.0f64; 3];
        let mut o = 0.0;
        for b in 0..3 {
            cur[b] = (1000.0 * acc[b] / hop as f64).ln_1p();
            o += WEIGHTS[b] * (cur[b] - prev[b]).max(0.0);
        }
        prev = cur;
        onset.push(o);
    }

    // Subtract a centred ~0.5 s moving average and half-wave rectify.
    let half = ((0.25 * fr).round() as usize).max(1);
    let mut prefix = vec![0.0f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + onset[i];
    }
    (0..n)
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(n);
            let mean = (prefix[hi] - prefix[lo]) / (hi - lo) as f64;
            (onset[i] - mean).max(0.0)
        })
        .collect()
}

/// Autocorrelation normalised so that `ac[0] == 1`. `mean_energy` is the
/// mean of `env[i]^2`. Lags past the data are left at zero.
fn autocorrelation(env: &[f64], max_lag: usize, mean_energy: f64) -> Vec<f64> {
    let n = env.len();
    let mut ac = vec![0.0f64; max_lag + 1];
    for lag in 0..=max_lag.min(n.saturating_sub(1)) {
        let s: f64 = env[..n - lag]
            .iter()
            .zip(&env[lag..])
            .map(|(a, b)| a * b)
            .sum();
        ac[lag] = s / (n - lag) as f64 / mean_energy;
    }
    ac
}

/// Three-tap triangle smoothing `[1/4, 1/2, 1/4]`; the ends are held.
fn triangle_smooth(env: &[f64]) -> Vec<f64> {
    let n = env.len();
    (0..n)
        .map(|i| {
            let prev = env[i.saturating_sub(1)];
            let next = env[(i + 1).min(n - 1)];
            0.25 * prev + 0.5 * env[i] + 0.25 * next
        })
        .collect()
}

/// Comb score for a lag in envelope frames: `ac(lag) + 0.5 ac(2 lag) +
/// 0.25 ac(lag / 2)`, rewarding consistent pulses over sub-multiples.
fn comb_score(ac: &[f64], lag: usize) -> f64 {
    let at = |l: usize| ac.get(l).copied().unwrap_or(0.0);
    at(lag) + 0.5 * at(2 * lag) + 0.25 * at(((lag as f64) / 2.0).round() as usize)
}

/// Mean envelope value on a beat grid with the given phase and beat length
/// (both in seconds, window-relative). Only beats inside the envelope count,
/// with linear interpolation between frame centres.
fn grid_mean(env: &[f64], fr: f64, phase: f64, beat: f64) -> f64 {
    let n = env.len();
    if n == 0 || !beat.is_finite() || beat <= 0.0 {
        return 0.0;
    }
    let max_f = (n - 1) as f64;
    let mut sum = 0.0;
    let mut count = 0usize;
    let mut tau = phase;
    loop {
        let f = tau * fr - 0.5;
        if f > max_f {
            break;
        }
        if f >= 0.0 {
            let i0 = f.floor() as usize;
            let frac = f - i0 as f64;
            let v = if i0 + 1 < n {
                env[i0] * (1.0 - frac) + env[i0 + 1] * frac
            } else {
                env[i0]
            };
            sum += v;
            count += 1;
        }
        tau += beat;
    }
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

/// Phase in `[0, beat)` at 1 ms resolution that maximises `grid_mean`, as
/// `(score, phase)`.
fn best_phase_ms(env: &[f64], fr: f64, beat: f64) -> (f64, f64) {
    let steps = ((beat * 1000.0).floor() as usize).max(1);
    let mut best = (f64::NEG_INFINITY, 0.0);
    for m in 0..steps {
        let phase = m as f64 * 0.001;
        let s = grid_mean(env, fr, phase, beat);
        if s > best.0 {
            best = (s, phase);
        }
    }
    best
}

/// Re-score the 0.01 BPM lattice within `FINE_STEPS` steps of `bpm`, each
/// tempo with its best 1 ms phase. Returns `(score, bpm)` of the winner.
fn fine_search(env: &[f64], fr: f64, bpm: f64) -> (f64, f64) {
    let centre = round_bpm(bpm);
    let mut best = (f64::NEG_INFINITY, centre);
    for d in -FINE_STEPS..=FINE_STEPS {
        let cand = round_bpm(centre + d as f64 * 0.01);
        if !(cand.is_finite() && cand > 0.0) {
            continue;
        }
        let (s, _) = best_phase_ms(env, fr, 60.0 / cand);
        if s > best.0 {
            best = (s, cand);
        }
    }
    best
}

/// Round to 0.01 BPM.
fn round_bpm(bpm: f64) -> f64 {
    (bpm * 100.0).round() / 100.0
}

/// Fold a tempo that is clearly outside `BPM_MIN..=BPM_MAX` by octaves.
/// Values within `FOLD_MARGIN` of the range are left for the final clamp.
fn fold_bpm(mut bpm: f64) -> f64 {
    for _ in 0..8 {
        if bpm > BPM_MAX + FOLD_MARGIN {
            bpm *= 0.5;
        } else if bpm < BPM_MIN - FOLD_MARGIN {
            bpm *= 2.0;
        } else {
            break;
        }
    }
    bpm
}

/// Lag ranges in envelope frames at frame rate `fr`: the scored lags
/// (70..=180 BPM; `floor` at the fast end keeps a 180 BPM pulse, whose lag
/// falls between two whole frames, reachable) and the coarse lags used for
/// the confidence statistic (60..=200 BPM).
fn lag_ranges(fr: f64) -> ((usize, usize), (usize, usize)) {
    let lag = |bpm: f64| 60.0 * fr / bpm;
    (
        (lag(BPM_MAX).floor() as usize, lag(BPM_MIN).floor() as usize),
        (
            lag(COARSE_BPM_MAX).ceil() as usize,
            lag(COARSE_BPM_MIN).floor() as usize,
        ),
    )
}

/// Analysis window `(start, len)` in envelope frames for a track of `total`
/// frames: the whole track when it fits in `max_len`, else `max_len` frames
/// starting at `ANALYSIS_WINDOW_START` of the track, pulled back so that the
/// window never runs past the end.
fn analysis_window(total: usize, max_len: usize) -> (usize, usize) {
    if total <= max_len {
        (0, total)
    } else {
        let start = ((total as f64 * ANALYSIS_WINDOW_START) as usize).min(total - max_len);
        (start, max_len)
    }
}

/// Earliest beat of the grid through `grid0` (spacing `beat`) at or after
/// `music_start`, never negative. A beat less than `tolerance` before 0 (the
/// grid of a track that opens on a kick) is clamped to 0 instead of skipping
/// a whole beat.
fn anchor_secs(grid0: f64, beat: f64, music_start: f64, tolerance: f64) -> f64 {
    if !(grid0.is_finite() && beat.is_finite() && beat > 0.0 && music_start.is_finite()) {
        return 0.0;
    }
    let mut first = grid0 + ((music_start - grid0) / beat).ceil() * beat;
    if first < 0.0 && first >= -tolerance {
        return 0.0;
    }
    if first < 0.0 {
        first += ((-first) / beat).ceil() * beat;
    }
    first.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift64* PRNG, no external crates.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }

        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        /// Uniform in [-1, 1).
        fn uniform(&mut self) -> f32 {
            let unit = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
            unit * 2.0 - 1.0
        }
    }

    fn stereo(mono: &[f32]) -> Vec<f32> {
        mono.iter().flat_map(|&x| [x, x]).collect()
    }

    /// Kick on every beat (60 Hz sine, amplitude 0.8, tau 40 ms, 120 ms long)
    /// and a 30 ms white-noise hat (amplitude 0.15) on every off-beat.
    /// `first_kick` is the time of the first kick in seconds.
    fn kick_track(bpm: f64, secs: f64, sr: u32, first_kick: f64) -> Vec<f32> {
        let fs = sr as f64;
        let frames = (secs * fs).round() as usize;
        let mut mono = vec![0.0f32; frames];
        let beat = 60.0 / bpm;
        let kick_len = (0.120 * fs).round() as usize;
        let hat_len = (0.030 * fs).round() as usize;
        let mut rng = Rng::new(0x5EED);

        let mut k = 0usize;
        loop {
            let t0 = first_kick + k as f64 * beat;
            if t0 >= secs {
                break;
            }
            let start = (t0 * fs).round() as usize;
            for i in 0..kick_len {
                let idx = start + i;
                if idx >= frames {
                    break;
                }
                let t = i as f64 / fs;
                let v = 0.8 * (-t / 0.040).exp() * (2.0 * PI * 60.0 * t).sin();
                mono[idx] += v as f32;
            }
            k += 1;
        }

        let mut k = 0usize;
        loop {
            let t0 = first_kick + (k as f64 + 0.5) * beat;
            if t0 >= secs {
                break;
            }
            let start = (t0 * fs).round() as usize;
            let end = (start + hat_len).min(frames);
            for s in &mut mono[start.min(end)..end] {
                *s += 0.15 * rng.uniform();
            }
            k += 1;
        }
        stereo(&mono)
    }

    fn assert_tempo(bpm: f64, got: Analysis) {
        let detected = got
            .bpm
            .unwrap_or_else(|| panic!("no tempo for {bpm} BPM (confidence {})", got.confidence));
        assert!(
            (detected - bpm).abs() <= 0.05,
            "{bpm} BPM detected as {detected} (confidence {})",
            got.confidence
        );
    }

    #[test]
    fn kick_track_120_bpm() {
        let s = kick_track(120.0, 60.0, 44_100, 0.0);
        assert_tempo(120.0, detect_tempo(&s, 44_100));
    }

    #[test]
    fn kick_track_128_bpm() {
        let s = kick_track(128.0, 60.0, 44_100, 0.0);
        assert_tempo(128.0, detect_tempo(&s, 44_100));
    }

    #[test]
    fn kick_track_140_bpm() {
        let s = kick_track(140.0, 60.0, 44_100, 0.0);
        assert_tempo(140.0, detect_tempo(&s, 44_100));
    }

    #[test]
    fn kick_track_174_bpm() {
        let s = kick_track(174.0, 60.0, 44_100, 0.0);
        assert_tempo(174.0, detect_tempo(&s, 44_100));
    }

    #[test]
    fn kick_track_90_bpm_is_not_doubled() {
        let s = kick_track(90.0, 60.0, 44_100, 0.0);
        assert_tempo(90.0, detect_tempo(&s, 44_100));
    }

    #[test]
    fn kick_track_fractional_bpm_is_not_snapped() {
        let s = kick_track(123.45, 60.0, 44_100, 0.0);
        let got = detect_tempo(&s, 44_100);
        assert_tempo(123.45, got);
        assert!(got.bpm.unwrap().fract().abs() > 0.1);
    }

    #[test]
    fn first_beat_anchor_and_grid_do_not_drift() {
        // 100 s so that beat 200 (at ~94.1 s) is inside the track.
        let s = kick_track(128.0, 100.0, 44_100, 0.37);
        let a = detect_tempo(&s, 44_100);
        assert_tempo(128.0, a);
        assert!(
            (a.first_beat_secs - 0.37).abs() < 0.015,
            "first beat at {}",
            a.first_beat_secs
        );
        let predicted = a.time_of_beat(200.0).unwrap();
        let true_200 = 0.37 + 200.0 * 60.0 / 128.0;
        assert!(
            (predicted - true_200).abs() < 0.02,
            "beat 200 at {predicted}, expected {true_200}"
        );
    }

    #[test]
    fn kick_track_48k_128_bpm() {
        let s = kick_track(128.0, 100.0, 48_000, 0.37);
        let a = detect_tempo(&s, 48_000);
        assert_tempo(128.0, a);
        assert!((a.first_beat_secs - 0.37).abs() < 0.015);
        let predicted = a.time_of_beat(200.0).unwrap();
        assert!((predicted - (0.37 + 200.0 * 60.0 / 128.0)).abs() < 0.02);
    }

    #[test]
    fn long_track_uses_window_from_quarter_point() {
        // 150 s: analysed from a 120 s window starting at 37.5 s.
        let s = kick_track(128.0, 150.0, 44_100, 0.0);
        let a = detect_tempo(&s, 44_100);
        assert_tempo(128.0, a);
        // The grid still sits on the track's own beats (kicks at k * beat).
        let beat = 60.0 / 128.0;
        let frac = (a.first_beat_secs / beat).fract();
        assert!(frac.min(1.0 - frac) < 0.02, "grid offset {frac} beats");
        // The anchor is the track's first kick, not a beat inside the window.
        assert!(
            a.first_beat_secs < 0.015,
            "first beat at {}",
            a.first_beat_secs
        );
    }

    /// Signed error (s) of the grid beat nearest to a true beat at `t`.
    fn grid_error(a: &Analysis, t: f64) -> f64 {
        let n = a.beat_at(t).unwrap().round();
        a.time_of_beat(n).unwrap() - t
    }

    #[test]
    fn long_track_anchor_is_at_music_start() {
        // 200 s: the tempo window starts at 50 s, but the anchor must be the
        // first kick at 0.37 s (the deck cues there on load).
        let bpm = 128.0;
        let s = kick_track(bpm, 200.0, 44_100, 0.37);
        let a = detect_tempo(&s, 44_100);
        assert_tempo(bpm, a);
        assert!(
            (a.first_beat_secs - 0.37).abs() < 0.015,
            "first beat at {}",
            a.first_beat_secs
        );
        let last = 0.37 + ((200.0 - 0.37) * bpm / 60.0).floor() * 60.0 / bpm;
        assert!(grid_error(&a, last).abs() < 0.005);
    }

    #[test]
    fn fractional_tempo_long_track_is_exact() {
        // A 0.01 BPM error drifts this grid by ~9 ms per 120 s; the result
        // must be the exact tempo and the grid within 5 ms at both ends.
        let bpm = 126.37;
        let secs = 240.0;
        let s = kick_track(bpm, secs, 44_100, 0.0);
        let a = detect_tempo(&s, 44_100);
        let got = a.bpm.expect("tempo");
        assert!((got - bpm).abs() < 1e-9, "detected {got}");
        let beat = 60.0 / bpm;
        let last = ((secs - 0.2) / beat).floor() * beat;
        for t in [0.0, last] {
            let e = grid_error(&a, t);
            assert!(e.abs() < 0.005, "grid error {e} s at {t} s");
        }
        assert!(
            a.first_beat_secs < 0.005,
            "first beat at {}",
            a.first_beat_secs
        );
    }

    #[test]
    fn tempo_180_is_not_halved() {
        for sr in [44_100, 48_000] {
            let s = kick_track(180.0, 60.0, sr, 0.0);
            assert_tempo(180.0, detect_tempo(&s, sr));
        }
    }

    #[test]
    fn integer_tempo_is_exact_and_near_integer_is_not_snapped() {
        let a = detect_tempo(&kick_track(128.0, 60.0, 44_100, 0.0), 44_100);
        assert_eq!(a.bpm, Some(128.0));
        for bpm in [111.02, 129.99] {
            let a = detect_tempo(&kick_track(bpm, 60.0, 44_100, 0.0), 44_100);
            let got = a.bpm.expect("tempo");
            assert!((got - bpm).abs() < 1e-9, "{bpm} detected as {got}");
        }
    }

    #[test]
    fn track_opening_on_a_kick_anchors_at_zero() {
        // The grid beat for a kick at 0 s may land a millisecond before 0; it
        // must clamp to 0, not skip to the second beat.
        for (bpm, sr) in [(71.3, 44_100), (70.6, 48_000), (128.0, 44_100)] {
            let a = detect_tempo(&kick_track(bpm, 60.0, sr, 0.0), sr);
            assert_tempo(bpm, a);
            assert!(
                a.first_beat_secs < 0.01,
                "{bpm} BPM at {sr}: first beat {}",
                a.first_beat_secs
            );
        }
    }

    #[test]
    fn anchor_clamps_only_a_beat_just_before_zero() {
        let beat = 0.8415;
        // Grid beat at -1.5 ms: clamp to 0.
        assert_eq!(anchor_secs(0.840, beat, -0.0275, 0.005), 0.0);
        // Grid beat at -21.5 ms: the next beat.
        assert!((anchor_secs(0.820, beat, -0.0275, 0.005) - 0.820).abs() < 1e-12);
        // Music starting later: the earliest beat at or after it.
        assert!((anchor_secs(0.1, 0.5, 3.0, 0.005) - 3.1).abs() < 1e-12);
        // A grid anchored later in the track steps back to the music start.
        assert!((anchor_secs(90.002, 0.5, 0.34, 0.005) - 0.502).abs() < 1e-9);
        assert_eq!(anchor_secs(f64::NAN, beat, 0.0, 0.005), 0.0);
    }

    #[test]
    fn analysis_window_is_full_length_and_starts_at_a_quarter() {
        let max = 120 * 200;
        assert_eq!(analysis_window(100 * 200, max), (0, 100 * 200));
        // 121 s: 25 % would leave only 91 s, so the window is pulled back.
        assert_eq!(analysis_window(121 * 200, max), (200, max));
        assert_eq!(analysis_window(160 * 200, max), (40 * 200, max));
        assert_eq!(analysis_window(300 * 200, max), (75 * 200, max));
    }

    #[test]
    fn lag_ranges_cover_180_bpm_and_the_coarse_range() {
        // 200 frames/s: lag 66 is 181.8 BPM, so 180 BPM (66.7) is reachable;
        // confidence averages over 60..=200 BPM (lags 60..=200).
        assert_eq!(lag_ranges(200.0), ((66, 171), (60, 200)));
        let ((lo, hi), (clo, chi)) = lag_ranges(44_100.0 / 221.0);
        assert!(60.0 * (44_100.0 / 221.0) / lo as f64 >= 180.0);
        assert!(60.0 * (44_100.0 / 221.0) / hi as f64 >= 70.0);
        assert!(clo < lo && chi > hi);
    }

    #[test]
    fn fold_keeps_edge_tempos_and_folds_far_ones() {
        assert_eq!(fold_bpm(180.3), 180.3);
        assert_eq!(fold_bpm(69.7), 69.7);
        assert!((fold_bpm(183.0) - 91.5).abs() < 1e-12);
        assert!((fold_bpm(60.0) - 120.0).abs() < 1e-12);
    }

    /// House-style track: four-on-the-floor pitch-swept kick, snares on 2 and
    /// 4 plus syncopated ghost snares, an off-beat bassline with a syncopated
    /// 16th, 16th hats, and `intro` seconds of beatless pad and noise swells
    /// before the first kick. Returns the stereo track and the kick times.
    fn house_track(bpm: f64, secs: f64, sr: u32, intro: f64) -> (Vec<f32>, Vec<f64>) {
        let fs = sr as f64;
        let frames = (secs * fs) as usize;
        let mut mono = vec![0.0f32; frames];
        let mut rng = Rng::new(0xC0FFEE);
        let beat = 60.0 / bpm;
        let add = |mono: &mut [f32], t0: f64, len: f64, f: &mut dyn FnMut(f64) -> f64| {
            let start = (t0 * fs).round() as usize;
            let end = (start + (len * fs) as usize).min(frames);
            for (i, v) in mono[start.min(end)..end].iter_mut().enumerate() {
                *v += f(i as f64 / fs) as f32;
            }
        };
        // Intro: a slow pad (two detuned sines with tremolo) and noise swells.
        add(&mut mono, 0.0, intro, &mut |t| {
            let trem = 0.6 + 0.4 * (2.0 * PI * 0.3 * t).sin();
            0.12 * trem * ((2.0 * PI * 220.0 * t).sin() + (2.0 * PI * 330.7 * t).sin())
        });
        let mut t = 2.3;
        while t + 3.0 < intro {
            let mut nz = Rng::new(t.to_bits());
            add(&mut mono, t, 2.0, &mut |u| {
                0.08 * (u / 2.0) * nz.uniform() as f64
            });
            t += 4.7;
        }
        let mut kicks = Vec::new();
        let mut k = 0usize;
        loop {
            let t0 = intro + k as f64 * beat;
            if t0 >= secs {
                break;
            }
            kicks.push(t0);
            let mut ph = 0.0;
            add(&mut mono, t0, 0.25, &mut |u| {
                ph += 2.0 * PI * (48.0 + 110.0 * (-u / 0.025).exp()) / fs;
                0.8 * (-u / 0.12).exp() * ph.sin()
            });
            let pos = k % 4;
            let snare = |mono: &mut [f32], at: f64, amp: f64, rng: &mut Rng| {
                add(mono, at, 0.15, &mut |u| {
                    amp * (-u / 0.06).exp()
                        * (0.7 * rng.uniform() as f64 + 0.4 * (2.0 * PI * 185.0 * u).sin())
                });
            };
            if pos == 1 || pos == 3 {
                snare(&mut mono, t0, 0.5, &mut rng);
            }
            if pos == 1 {
                snare(&mut mono, t0 + 0.75 * beat, 0.3, &mut rng);
            }
            if pos == 3 && (k / 4) % 2 == 1 {
                snare(&mut mono, t0 + 0.25 * beat, 0.3, &mut rng);
            }
            let note = [55.0, 55.0, 65.41, 49.0][(k / 2) % 4];
            let bass = |mono: &mut [f32], at: f64, len: f64, f: f64| {
                add(mono, at, len, &mut |u| {
                    let saw = 2.0 * (u * f).fract() - 1.0;
                    0.4 * saw * (u / 0.004).min(1.0) * (-u / len).exp()
                });
            };
            bass(&mut mono, t0 + 0.5 * beat, 0.4 * beat, note);
            if pos == 2 {
                bass(&mut mono, t0 + 0.75 * beat, 0.2 * beat, note * 1.5);
            }
            for s in 0..4 {
                let (amp, dur) = if s == 2 { (0.2, 0.05) } else { (0.06, 0.012) };
                add(
                    &mut mono,
                    t0 + s as f64 * 0.25 * beat,
                    3.0 * dur,
                    &mut |u| amp * (-u / dur).exp() * rng.uniform() as f64,
                );
            }
            k += 1;
        }
        (stereo(&mono), kicks)
    }

    #[test]
    fn house_track_with_beatless_intro() {
        // 150 s with a 30 s beatless intro: tempo window 30..150 s.
        let bpm = 124.63;
        let (s, kicks) = house_track(bpm, 150.0, 44_100, 30.0);
        let a = detect_tempo(&s, 44_100);
        let got = a.bpm.expect("tempo");
        assert!((got - bpm).abs() < 1e-9, "detected {got}");
        let worst = kicks
            .iter()
            .map(|&t| grid_error(&a, t).abs())
            .fold(0.0f64, f64::max);
        assert!(worst < 0.005, "worst grid error {worst} s");
        // Anchored on the grid no later than the first kick, never at the
        // window start.
        assert!(
            a.first_beat_secs <= 30.0 + 0.015,
            "first beat at {}",
            a.first_beat_secs
        );
        assert!(a.first_beat_secs >= 0.0);
    }

    #[test]
    fn silence_has_no_tempo() {
        let s = vec![0.0f32; 2 * 10 * 44_100];
        let a = detect_tempo(&s, 44_100);
        assert_eq!(a.bpm, None);
        assert_eq!(a.confidence, 0.0);
    }

    #[test]
    fn empty_input_has_no_tempo() {
        assert_eq!(detect_tempo(&[], 44_100).bpm, None);
        assert_eq!(detect_tempo(&[0.5, 0.5], 0).bpm, None);
    }

    #[test]
    fn white_noise_has_no_confident_tempo() {
        let mut rng = Rng::new(42);
        let mono: Vec<f32> = (0..30 * 44_100).map(|_| 0.3 * rng.uniform()).collect();
        let a = detect_tempo(&stereo(&mono), 44_100);
        assert!(
            a.bpm.is_none() || a.confidence < 0.3,
            "noise gave {:?} at confidence {}",
            a.bpm,
            a.confidence
        );
    }

    #[test]
    fn non_finite_samples_do_not_panic() {
        let s = vec![f32::NAN; 2 * 44_100];
        assert_eq!(detect_tempo(&s, 44_100).bpm, None);
        let ov = build_overview(&s, 44_100, 100.0);
        assert!(ov.bins.iter().all(|b| b.peak == 0.0));
    }

    #[test]
    fn overview_low_then_high() {
        let sr = 44_100usize;
        let mut mono = Vec::with_capacity(2 * sr);
        for i in 0..sr {
            mono.push(0.5 * (2.0 * PI * 60.0 * i as f64 / sr as f64).sin() as f32);
        }
        for i in 0..sr {
            mono.push(0.5 * (2.0 * PI * 8000.0 * i as f64 / sr as f64).sin() as f32);
        }
        let ov = build_overview(&stereo(&mono), sr as u32, 100.0);
        assert_eq!(ov.bins.len(), 200);
        assert_eq!(ov.bins_per_sec, 100.0);
        for (i, b) in ov.bins.iter().enumerate() {
            assert!(b.peak <= 1.0 && b.low <= 1.0 && b.mid <= 1.0 && b.high <= 1.0);
            if (5..95).contains(&i) {
                assert!(b.low > b.high, "bin {i}: low {} high {}", b.low, b.high);
            }
            if (105..195).contains(&i) {
                assert!(b.high > b.low, "bin {i}: low {} high {}", b.low, b.high);
            }
        }
    }

    #[test]
    fn overview_peak_is_clamped_to_one() {
        let ov = build_overview(&stereo(&[3.0, -2.0, 0.25]), 44_100, 100.0);
        assert_eq!(ov.bins.len(), 1);
        assert_eq!(ov.bins[0].peak, 1.0);
    }

    #[test]
    fn overview_non_positive_rate_is_empty() {
        let s = stereo(&vec![0.5f32; 1000]);
        let ov = build_overview(&s, 44_100, 0.0);
        assert!(ov.bins.is_empty());
        assert!(ov.bin_at(0.0).is_none());
    }

    #[test]
    fn tap_tempo_steady_taps_give_120() {
        let mut tap = TapTempo::new();
        assert_eq!(tap.tap(0.0), None);
        assert_eq!(tap.tap(0.5), None);
        let bpm = tap.tap(1.0).expect("third tap");
        assert!((bpm - 120.0).abs() <= 0.01, "{bpm}");
        let bpm = tap.tap(1.5).expect("fourth tap");
        assert!((bpm - 120.0).abs() <= 0.01, "{bpm}");
    }

    #[test]
    fn tap_tempo_gap_over_two_seconds_starts_over() {
        let mut tap = TapTempo::new();
        tap.tap(0.0);
        tap.tap(0.5);
        assert!((tap.tap(1.0).unwrap() - 120.0).abs() <= 0.01);
        assert_eq!(tap.tap(4.0), None, "3 s gap must reset the series");
        assert_eq!(tap.tap(4.5), None);
        assert!((tap.tap(5.0).unwrap() - 120.0).abs() <= 0.01);
    }

    #[test]
    fn tap_tempo_backwards_time_starts_over() {
        let mut tap = TapTempo::new();
        tap.tap(10.0);
        tap.tap(10.5);
        assert!((tap.tap(11.0).unwrap() - 120.0).abs() <= 0.01);
        assert_eq!(tap.tap(9.0), None);
    }

    #[test]
    fn tap_tempo_uses_last_eight_taps() {
        let mut tap = TapTempo::new();
        // Six taps at 0.4 s spacing (0.0 ..= 2.0), then eight at 0.6 s.
        for i in 0..6 {
            tap.tap(0.4 * i as f64);
        }
        // Only the last eight taps count, so the 0.4 s taps drop out. Averaging
        // all fourteen would give about 114.7 BPM instead of 100.
        let mut last = None;
        for i in 0..8 {
            last = tap.tap(2.6 + 0.6 * i as f64);
        }
        assert!((last.unwrap() - 100.0).abs() <= 0.01, "{last:?}");
    }

    #[test]
    fn analysis_grid_helpers_round_trip() {
        let a = Analysis {
            bpm: Some(120.0),
            first_beat_secs: 0.25,
            confidence: 1.0,
        };
        assert_eq!(a.beat_len_secs(), Some(0.5));
        assert!((a.time_of_beat(4.0).unwrap() - 2.25).abs() < 1e-12);
        assert!((a.beat_at(2.25).unwrap() - 4.0).abs() < 1e-12);
    }
}
