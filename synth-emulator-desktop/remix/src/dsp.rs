//! Mixer DSP: smoothing, biquads, DJ isolator EQ, one-knob filter, echo,
//! crossfader curves and the master limiter.
//!
//! All `process` methods are real-time safe: no allocation, no locks.
//! Every parameter change is smoothed so moving a control never clicks.

use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2, TAU};

/// One-pole parameter smoother.
#[derive(Clone, Copy, Debug)]
pub struct Smoother {
    value: f32,
    target: f32,
    coeff: f32,
}

impl Smoother {
    /// `time_secs` is the ~63% settling time constant.
    pub fn new(sample_rate: f32, time_secs: f32, initial: f32) -> Self {
        let coeff = if time_secs <= 0.0 {
            0.0
        } else {
            (-1.0 / (time_secs * sample_rate)).exp()
        };
        Self {
            value: initial,
            target: initial,
            coeff,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Jump straight to `v` (no glide).
    pub fn snap(&mut self, v: f32) {
        self.value = v;
        self.target = v;
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    // Named `next` in the existing contract; it is not an Iterator.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn next(&mut self) -> f32 {
        self.value = self.target + (self.value - self.target) * self.coeff;
        if (self.value - self.target).abs() < 1e-6 {
            self.value = self.target;
        }
        self.value
    }
}

/// Sample rate used when a caller passes a non-finite or implausibly low one.
const FALLBACK_SAMPLE_RATE: f32 = 48_000.0;
/// Q of a 2nd-order Butterworth section. Two in cascade make a Linkwitz-Riley 4 split.
const BUTTERWORTH_Q: f32 = FRAC_1_SQRT_2;
/// Recursive filter state below this magnitude is flushed to zero.
const DENORMAL_FLOOR: f32 = 1e-20;
/// Lowest centre frequency a biquad will be tuned to.
const MIN_FILTER_HZ: f64 = 10.0;
/// Lowest Q a biquad will be tuned to.
const MIN_Q: f64 = 0.1;
/// Highest Q a biquad will be tuned to. Past this alpha is so small that the pole
/// radius rounds to 1 in f32, so the poles sit on the unit circle and the filter rings
/// forever (+inf and 1e30 both do this).
const MAX_Q: f64 = 50.0;
/// Largest peaking gain a biquad will be tuned to, in dB.
const MAX_PEAK_DB: f32 = 60.0;

/// Sample rate guard: anything non-finite or below 1 kHz falls back to 48 kHz.
#[inline]
fn sample_rate_or_default(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() && sample_rate >= 1_000.0 {
        sample_rate
    } else {
        FALLBACK_SAMPLE_RATE
    }
}

/// Clamp to `lo..=hi`, substituting `nan` for NaN. Unlike `f32::clamp` this
/// never panics, even when `lo > hi`.
#[inline]
fn clamp_or(x: f32, lo: f32, hi: f32, nan: f32) -> f32 {
    if x.is_nan() {
        nan
    } else {
        x.max(lo).min(hi)
    }
}

/// Replace non-finite input samples with silence so NaN never reaches a filter state.
#[inline]
fn finite(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

/// Flush denormal-sized or non-finite recursive state to zero.
#[inline]
fn flush(z: f32) -> f32 {
    if z.abs() < DENORMAL_FLOOR || !z.is_finite() {
        0.0
    } else {
        z
    }
}

/// Shared RBJ front end: returns `(cos w0, alpha)` for the clamped frequency and Q.
fn rbj_terms(sample_rate: f32, freq: f32, q: f32) -> (f64, f64) {
    let fs = f64::from(sample_rate_or_default(sample_rate));
    let hi = 0.49 * fs;
    let f = if freq.is_nan() {
        MIN_FILTER_HZ
    } else {
        f64::from(freq).max(MIN_FILTER_HZ).min(hi)
    };
    let q = if q.is_nan() {
        f64::from(BUTTERWORTH_Q)
    } else {
        f64::from(q).clamp(MIN_Q, MAX_Q)
    };
    let w0 = 2.0 * std::f64::consts::PI * f / fs;
    let alpha = w0.sin() / (2.0 * q);
    (w0.cos(), alpha)
}

/// Normalised biquad coefficients (a0 == 1), RBJ Audio EQ Cookbook.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiquadCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl BiquadCoeffs {
    pub const IDENTITY: Self = Self {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Divides every coefficient by `a0` in f64, then narrows to f32. Using one
    /// shared reciprocal keeps symmetric designs (allpass) exactly symmetric.
    fn normalised(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        let inv = 1.0 / a0;
        Self {
            b0: (b0 * inv) as f32,
            b1: (b1 * inv) as f32,
            b2: (b2 * inv) as f32,
            a1: (a1 * inv) as f32,
            a2: (a2 * inv) as f32,
        }
    }

    pub fn lowpass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let (cos_w0, alpha) = rbj_terms(sample_rate, freq, q);
        let b1 = 1.0 - cos_w0;
        Self::normalised(
            b1 / 2.0,
            b1,
            b1 / 2.0,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        )
    }

    pub fn highpass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let (cos_w0, alpha) = rbj_terms(sample_rate, freq, q);
        let b0 = (1.0 + cos_w0) / 2.0;
        Self::normalised(
            b0,
            -(1.0 + cos_w0),
            b0,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        )
    }

    /// Second-order allpass.
    pub fn allpass(sample_rate: f32, freq: f32, q: f32) -> Self {
        let (cos_w0, alpha) = rbj_terms(sample_rate, freq, q);
        Self::normalised(
            1.0 - alpha,
            -2.0 * cos_w0,
            1.0 + alpha,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        )
    }

    pub fn peaking(sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Self {
        let (cos_w0, alpha) = rbj_terms(sample_rate, freq, q);
        let gain_db = clamp_or(gain_db, -MAX_PEAK_DB, MAX_PEAK_DB, 0.0);
        let a = 10f64.powf(f64::from(gain_db) / 40.0);
        Self::normalised(
            1.0 + alpha * a,
            -2.0 * cos_w0,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos_w0,
            1.0 - alpha / a,
        )
    }

    /// Magnitude response at `freq`, linear. For tests and UI curves.
    pub fn magnitude_at(&self, sample_rate: f32, freq: f32) -> f32 {
        let fs = f64::from(sample_rate_or_default(sample_rate));
        let w = 2.0 * std::f64::consts::PI * f64::from(freq) / fs;
        let (sin1, cos1) = w.sin_cos();
        let (sin2, cos2) = (2.0 * w).sin_cos();
        let (b0, b1, b2) = (f64::from(self.b0), f64::from(self.b1), f64::from(self.b2));
        let (a1, a2) = (f64::from(self.a1), f64::from(self.a2));
        // H(e^jw) = N / D with N = b0 + b1 e^-jw + b2 e^-2jw, D = 1 + a1 e^-jw + a2 e^-2jw.
        let num = (b0 + b1 * cos1 + b2 * cos2).hypot(b1 * sin1 + b2 * sin2);
        let den = (1.0 + a1 * cos1 + a2 * cos2).hypot(a1 * sin1 + a2 * sin2);
        (num / den) as f32
    }
}

/// One transposed direct form II section for one channel.
#[inline]
fn tdf2(c: &BiquadCoeffs, x: f32, z1: &mut f32, z2: &mut f32) -> f32 {
    let y = c.b0 * x + *z1;
    *z1 = flush(c.b1 * x - c.a1 * y + *z2);
    *z2 = flush(c.b2 * x - c.a2 * y);
    y
}

/// Stereo biquad, transposed direct form II.
#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    pub coeffs: BiquadCoeffs,
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Biquad {
    pub fn new(coeffs: BiquadCoeffs) -> Self {
        Self {
            coeffs,
            z1: [0.0; 2],
            z2: [0.0; 2],
        }
    }

    pub fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let c = self.coeffs;
        let (l, r) = (finite(l), finite(r));
        let yl = tdf2(&c, l, &mut self.z1[0], &mut self.z2[0]);
        let yr = tdf2(&c, r, &mut self.z1[1], &mut self.z2[1]);
        (yl, yr)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Band {
    Low,
    Mid,
    High,
}

impl Band {
    pub const ALL: [Band; 3] = [Band::Low, Band::Mid, Band::High];

    pub fn index(self) -> usize {
        match self {
            Band::Low => 0,
            Band::Mid => 1,
            Band::High => 2,
        }
    }
}

/// Crossover points of the isolator.
pub const EQ_LOW_MID_HZ: f32 = 250.0;
pub const EQ_MID_HIGH_HZ: f32 = 3_000.0;
/// Max band gain (linear). 1.0 is flat, 0.0 is a full kill.
pub const EQ_MAX_GAIN: f32 = 2.0;

/// Ramp time of the band gains (and kill switches).
const EQ_GAIN_SMOOTH_SECS: f32 = 0.010;

/// Stages per Linkwitz-Riley 4 filter: two cascaded Butterworth sections.
type Lr4 = [Biquad; 2];

/// DJ isolator: Linkwitz-Riley 4th-order 3-way split, per-band gain, sum.
/// All gains at 1.0 must be magnitude-flat (allpass). Gain 0 fully removes
/// the band. Kill switches mute a band regardless of its gain.
#[derive(Clone, Debug)]
pub struct ThreeBandEq {
    /// Low band: LR4 low-pass at `EQ_LOW_MID_HZ`.
    low_lp: Lr4,
    /// Everything above the low crossover: LR4 high-pass at `EQ_LOW_MID_HZ`.
    rest_hp: Lr4,
    /// Mid band: LR4 low-pass at `EQ_MID_HIGH_HZ`, applied to `rest_hp`.
    mid_lp: Lr4,
    /// High band: LR4 high-pass at `EQ_MID_HIGH_HZ`, applied to `rest_hp`.
    high_hp: Lr4,
    /// Phase match for the low band. LR4 low + high sum to a 2nd-order allpass at
    /// the same frequency, so the low band needs the same allpass for the sum to be flat.
    low_ap: Biquad,
    /// Stored (clamped) gain per band, as set by the caller.
    gains: [f32; 3],
    /// Stored kill switch per band.
    kills: [bool; 3],
    /// Smoothed effective gain per band (0 when killed).
    smooth: [Smoother; 3],
}

impl ThreeBandEq {
    pub fn new(sample_rate: f32) -> Self {
        let fs = sample_rate_or_default(sample_rate);
        let low_lp = BiquadCoeffs::lowpass(fs, EQ_LOW_MID_HZ, BUTTERWORTH_Q);
        let low_hp = BiquadCoeffs::highpass(fs, EQ_LOW_MID_HZ, BUTTERWORTH_Q);
        let mid_lp = BiquadCoeffs::lowpass(fs, EQ_MID_HIGH_HZ, BUTTERWORTH_Q);
        let high_hp = BiquadCoeffs::highpass(fs, EQ_MID_HIGH_HZ, BUTTERWORTH_Q);
        let ap = BiquadCoeffs::allpass(fs, EQ_MID_HIGH_HZ, BUTTERWORTH_Q);
        Self {
            low_lp: [Biquad::new(low_lp); 2],
            rest_hp: [Biquad::new(low_hp); 2],
            mid_lp: [Biquad::new(mid_lp); 2],
            high_hp: [Biquad::new(high_hp); 2],
            low_ap: Biquad::new(ap),
            gains: [1.0; 3],
            kills: [false; 3],
            smooth: [Smoother::new(fs, EQ_GAIN_SMOOTH_SECS, 1.0); 3],
        }
    }

    /// Pushes the effective (kill-aware) target of band `i` into its smoother.
    fn retarget(&mut self, i: usize) {
        let target = if self.kills[i] { 0.0 } else { self.gains[i] };
        self.smooth[i].set_target(target);
    }

    /// `gain` linear 0..=EQ_MAX_GAIN, smoothed. NaN is treated as unity.
    pub fn set_gain(&mut self, band: Band, gain: f32) {
        let i = band.index();
        self.gains[i] = clamp_or(gain, 0.0, EQ_MAX_GAIN, 1.0);
        self.retarget(i);
    }

    pub fn set_kill(&mut self, band: Band, on: bool) {
        let i = band.index();
        self.kills[i] = on;
        self.retarget(i);
    }

    pub fn gain(&self, band: Band) -> f32 {
        self.gains[band.index()]
    }

    pub fn kill(&self, band: Band) -> bool {
        self.kills[band.index()]
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (l, r) = (finite(l), finite(r));
        let (low_l, low_r) = run_lr4(&mut self.low_lp, l, r);
        let (rest_l, rest_r) = run_lr4(&mut self.rest_hp, l, r);
        let (mid_l, mid_r) = run_lr4(&mut self.mid_lp, rest_l, rest_r);
        let (high_l, high_r) = run_lr4(&mut self.high_hp, rest_l, rest_r);
        let (low_l, low_r) = self.low_ap.process(low_l, low_r);

        let g_low = self.smooth[Band::Low.index()].next();
        let g_mid = self.smooth[Band::Mid.index()].next();
        let g_high = self.smooth[Band::High.index()].next();
        (
            low_l * g_low + mid_l * g_mid + high_l * g_high,
            low_r * g_low + mid_r * g_mid + high_r * g_high,
        )
    }
}

#[inline]
fn run_lr4(stages: &mut Lr4, l: f32, r: f32) -> (f32, f32) {
    let (l, r) = stages[0].process(l, r);
    stages[1].process(l, r)
}

/// Position range of the DJ filter.
const DJ_POS_SMOOTH_SECS: f32 = 0.020;
/// Positions with |p| below this are bypassed bit-exactly.
const DJ_DEAD_ZONE: f32 = 0.02;
/// Width over which the wet/dry blend ramps in once outside the dead zone.
const DJ_WET_RAMP: f32 = 0.06;
/// Coefficients are retuned every this many samples.
const DJ_RETUNE_INTERVAL: u32 = 32;
const DJ_LP_TOP_HZ: f32 = 20_000.0;
const DJ_LP_BOTTOM_HZ: f32 = 60.0;
const DJ_HP_BOTTOM_HZ: f32 = 20.0;
const DJ_HP_TOP_HZ: f32 = 8_000.0;
const DJ_Q_BASE: f32 = 0.75;
const DJ_Q_SLOPE: f32 = 0.35;

/// One-knob DJ filter. Position -1..=1: 0 is bypass, negative sweeps a
/// low-pass down, positive sweeps a high-pass up.
#[derive(Clone, Debug)]
pub struct DjFilter {
    /// Smoothed position; its target is the position the caller set.
    pos: Smoother,
    lp: Biquad,
    hp: Biquad,
    /// Coefficients at the start and end of the current retune interval. The live
    /// coefficients interpolate between them every sample, so a retune never steps
    /// the output.
    lp_from: BiquadCoeffs,
    lp_to: BiquadCoeffs,
    hp_from: BiquadCoeffs,
    hp_to: BiquadCoeffs,
    sample_rate: f32,
    /// Samples left until the next coefficient retune.
    countdown: u32,
}

/// Linear interpolation of two coefficient sets, `t` in 0..=1.
#[inline]
fn lerp_coeffs(a: &BiquadCoeffs, b: &BiquadCoeffs, t: f32) -> BiquadCoeffs {
    BiquadCoeffs {
        b0: a.b0 + (b.b0 - a.b0) * t,
        b1: a.b1 + (b.b1 - a.b1) * t,
        b2: a.b2 + (b.b2 - a.b2) * t,
        a1: a.a1 + (b.a1 - a.a1) * t,
        a2: a.a2 + (b.a2 - a.a2) * t,
    }
}

impl DjFilter {
    pub fn new(sample_rate: f32) -> Self {
        let fs = sample_rate_or_default(sample_rate);
        let lp = BiquadCoeffs::lowpass(fs, DJ_LP_TOP_HZ, DJ_Q_BASE);
        let hp = BiquadCoeffs::highpass(fs, DJ_HP_BOTTOM_HZ, DJ_Q_BASE);
        Self {
            pos: Smoother::new(fs, DJ_POS_SMOOTH_SECS, 0.0),
            lp: Biquad::new(lp),
            hp: Biquad::new(hp),
            lp_from: lp,
            lp_to: lp,
            hp_from: hp,
            hp_to: hp,
            sample_rate: fs,
            countdown: 0,
        }
    }

    /// Clamped to -1..=1. NaN is treated as bypass (0).
    pub fn set_position(&mut self, pos: f32) {
        self.pos.set_target(clamp_or(pos, -1.0, 1.0, 0.0));
    }

    pub fn position(&self) -> f32 {
        self.pos.target()
    }

    /// Sets new target coefficients for both filters from the smoothed position. The inactive side is parked
    /// at its transparent setting (LP at 20 kHz, or HP at 20 Hz) so it keeps tracking
    /// the input and never has to ramp in from silence.
    fn retune(&mut self, p: f32) {
        let mag = p.abs();
        let q = DJ_Q_BASE + DJ_Q_SLOPE * mag;
        let lp_hz = if p < 0.0 {
            DJ_LP_TOP_HZ * (DJ_LP_BOTTOM_HZ / DJ_LP_TOP_HZ).powf(mag)
        } else {
            DJ_LP_TOP_HZ
        };
        let hp_hz = if p > 0.0 {
            DJ_HP_BOTTOM_HZ * (DJ_HP_TOP_HZ / DJ_HP_BOTTOM_HZ).powf(p)
        } else {
            DJ_HP_BOTTOM_HZ
        };
        // The previous target is where the live coefficients sit now, so the ramp is continuous.
        self.lp_from = self.lp_to;
        self.hp_from = self.hp_to;
        self.lp_to = BiquadCoeffs::lowpass(self.sample_rate, lp_hz, q);
        self.hp_to = BiquadCoeffs::highpass(self.sample_rate, hp_hz, q);
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (l, r) = (finite(l), finite(r));
        let p = self.pos.next();
        if self.countdown == 0 {
            self.countdown = DJ_RETUNE_INTERVAL;
            self.retune(p);
        }
        self.countdown -= 1;

        // Glide the live coefficients across the interval (reaching the target on its last sample).
        let t = (DJ_RETUNE_INTERVAL - self.countdown) as f32 / DJ_RETUNE_INTERVAL as f32;
        self.lp.coeffs = lerp_coeffs(&self.lp_from, &self.lp_to, t);
        self.hp.coeffs = lerp_coeffs(&self.hp_from, &self.hp_to, t);

        if p.abs() < DJ_DEAD_ZONE && self.pos.target().abs() < DJ_DEAD_ZONE {
            // Bypass. Clear the filter memory so re-entering the sweep starts clean.
            self.lp.reset();
            self.hp.reset();
            return (l, r);
        }

        let (lp_l, lp_r) = self.lp.process(l, r);
        let (hp_l, hp_r) = self.hp.process(l, r);
        // `p` is always finite (smoothed from clamped targets), so clamp cannot see NaN.
        let wet = ((p.abs() - DJ_DEAD_ZONE) / DJ_WET_RAMP).clamp(0.0, 1.0);
        let (fl, fr) = if p < 0.0 { (lp_l, lp_r) } else { (hp_l, hp_r) };
        (l * (1.0 - wet) + fl * wet, r * (1.0 - wet) + fr * wet)
    }
}

/// Echo tuning.
const ECHO_MIN_SECS: f32 = 0.01;
const ECHO_DEFAULT_SECS: f32 = 0.25;
const ECHO_DEFAULT_MAX_SECS: f32 = 2.0;
const ECHO_MIN_MAX_SECS: f32 = 0.02;
const ECHO_MAX_MAX_SECS: f32 = 60.0;
const ECHO_DEFAULT_FEEDBACK: f32 = 0.5;
const ECHO_MAX_FEEDBACK: f32 = 0.95;
/// Glide time of the delay time (in samples, so the change is tape-like).
const ECHO_DELAY_SMOOTH_SECS: f32 = 0.060;
/// Glide time of the send level and feedback gain. These scale what is written into
/// the buffer, so a jump there would replay as a click one delay later.
const ECHO_PARAM_SMOOTH_SECS: f32 = 0.015;
/// Feedback-path damping: one-pole low-pass, then one-pole high-pass.
const ECHO_DAMP_LP_HZ: f32 = 6_000.0;
const ECHO_DAMP_HP_HZ: f32 = 150.0;

/// Coefficient of a one-pole filter with cutoff `hz`.
fn one_pole_coef(sample_rate: f32, hz: f32) -> f32 {
    1.0 - (-TAU * hz / sample_rate).exp()
}

/// Soft clip: identity up to 1.0, then a continuous knee that approaches +-2.
#[inline]
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 1.0 {
        x
    } else {
        x.signum() * (1.0 + (a - 1.0).tanh())
    }
}

/// Per-channel state of the feedback-path damping filters.
#[derive(Clone, Copy, Debug, Default)]
struct FeedbackDamping {
    lp: f32,
    hp_lp: f32,
}

impl FeedbackDamping {
    #[inline]
    fn process(&mut self, x: f32, lp_coef: f32, hp_coef: f32) -> f32 {
        self.lp = flush(self.lp + lp_coef * (x - self.lp));
        self.hp_lp = flush(self.hp_lp + hp_coef * (self.lp - self.hp_lp));
        self.lp - self.hp_lp
    }
}

/// Stereo echo with damped feedback, for transitions ("echo out").
#[derive(Clone, Debug)]
pub struct Echo {
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    /// Index the next input sample is written to.
    write: usize,
    sample_rate: f32,
    /// Largest delay time in seconds the buffers can hold.
    max_secs: f32,
    /// Delay time in samples, gliding towards its target. The smoother is f32, so near
    /// a large target it settles a few samples short (about 1e-4 relative); inaudible.
    delay: Smoother,
    /// Feedback gain 0..=0.95, gliding so a change never steps the buffer write.
    feedback: Smoother,
    /// Wet send 0..=1, gliding for the same reason.
    mix: Smoother,
    damp: [FeedbackDamping; 2],
    lp_coef: f32,
    hp_coef: f32,
}

impl Echo {
    /// Buffers are allocated here, sized for `max_secs`.
    pub fn new(sample_rate: f32, max_secs: f32) -> Self {
        let fs = sample_rate_or_default(sample_rate);
        let max_secs = clamp_or(
            max_secs,
            ECHO_MIN_MAX_SECS,
            ECHO_MAX_MAX_SECS,
            ECHO_DEFAULT_MAX_SECS,
        );
        let len = (max_secs * fs).ceil() as usize + 4;
        let default_delay = ECHO_DEFAULT_SECS.min(max_secs) * fs;
        Self {
            buf_l: vec![0.0; len],
            buf_r: vec![0.0; len],
            write: 0,
            sample_rate: fs,
            max_secs,
            delay: Smoother::new(fs, ECHO_DELAY_SMOOTH_SECS, default_delay),
            feedback: Smoother::new(fs, ECHO_PARAM_SMOOTH_SECS, ECHO_DEFAULT_FEEDBACK),
            mix: Smoother::new(fs, ECHO_PARAM_SMOOTH_SECS, 0.0),
            damp: [FeedbackDamping::default(); 2],
            lp_coef: one_pole_coef(fs, ECHO_DAMP_LP_HZ),
            hp_coef: one_pole_coef(fs, ECHO_DAMP_HP_HZ),
        }
    }

    /// Delay time, clamped to 0.01..=max_secs. Always glides, on the first call
    /// as well, so the read head never jumps and changing it never clicks.
    pub fn set_time_secs(&mut self, secs: f32) {
        let secs = clamp_or(secs, ECHO_MIN_SECS, self.max_secs, ECHO_DEFAULT_SECS);
        self.delay.set_target(secs * self.sample_rate);
    }

    /// 0..=0.95. Glides, so a jump never clicks.
    pub fn set_feedback(&mut self, fb: f32) {
        self.feedback
            .set_target(clamp_or(fb, 0.0, ECHO_MAX_FEEDBACK, 0.0));
    }

    /// Wet send 0..=1. At 0 the output is exactly the input, and the tail
    /// that is already in the buffer keeps ringing out (send, not wet/dry).
    /// Glides, so a jump never clicks.
    pub fn set_mix(&mut self, mix: f32) {
        self.mix.set_target(clamp_or(mix, 0.0, 1.0, 0.0));
    }

    /// Silence the buffer and the filter memory. Every smoother snaps to its
    /// target, so the next block sees the settings exactly as last set.
    pub fn clear(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.damp = [FeedbackDamping::default(); 2];
        self.delay.snap(self.delay.target());
        self.feedback.snap(self.feedback.target());
        self.mix.snap(self.mix.target());
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (l, r) = (finite(l), finite(r));
        let len = self.buf_l.len();

        // Fractional read `delay` samples behind the write head, linearly interpolated.
        let delay = f64::from(self.delay.next());
        let mut read = self.write as f64 - delay;
        if read < 0.0 {
            read += len as f64;
        }
        let i0 = (read as usize).min(len - 1);
        let frac = ((read - i0 as f64) as f32).min(1.0);
        let i1 = if i0 + 1 == len { 0 } else { i0 + 1 };

        let dl = self.buf_l[i0] + (self.buf_l[i1] - self.buf_l[i0]) * frac;
        let dr = self.buf_r[i0] + (self.buf_r[i1] - self.buf_r[i0]) * frac;

        let fl = self.damp[0].process(dl, self.lp_coef, self.hp_coef);
        let fr = self.damp[1].process(dr, self.lp_coef, self.hp_coef);

        // One smoothed step per frame, shared by both channels.
        let (m, g) = (self.mix.next(), self.feedback.next());
        self.buf_l[self.write] = soft_clip(l * m + fl * g);
        self.buf_r[self.write] = soft_clip(r * m + fr * g);
        self.write += 1;
        if self.write == len {
            self.write = 0;
        }

        (l + dl, r + dr)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CrossfaderCurve {
    /// Constant power: both at -3 dB in the middle.
    #[default]
    Smooth,
    /// Scratch cut: both at full level except the last ~5% at each end.
    Sharp,
}

/// Gains for deck A and B. `pos` 0 = only A, 1 = only B.
pub fn crossfader_gains(pos: f32, curve: CrossfaderCurve) -> (f32, f32) {
    let p = clamp_or(pos, 0.0, 1.0, 0.0);
    match curve {
        CrossfaderCurve::Smooth => {
            let t = p * FRAC_PI_2;
            (t.cos(), t.sin())
        }
        CrossfaderCurve::Sharp => {
            let a = if p <= 0.95 { 1.0 } else { (1.0 - p) / 0.05 };
            let b = if p >= 0.05 { 1.0 } else { p / 0.05 };
            (a, b)
        }
    }
}

/// Stereo-linked peak limiter on the master. Instant attack, ~120 ms
/// release, ceiling -0.3 dBFS. Output never exceeds the ceiling.
#[derive(Clone, Debug)]
pub struct Limiter {
    /// Current gain, 0..=1.
    gain: f32,
    /// Per-sample recovery factor towards unity gain.
    release: f32,
}

pub const LIMITER_CEILING: f32 = 0.966; // -0.3 dBFS

/// Release time constant of the limiter, in seconds.
const LIMITER_RELEASE_SECS: f32 = 0.120;

impl Limiter {
    pub fn new(sample_rate: f32) -> Self {
        let fs = sample_rate_or_default(sample_rate);
        Self {
            gain: 1.0,
            release: (-1.0 / (LIMITER_RELEASE_SECS * fs)).exp(),
        }
    }

    /// Current gain reduction, linear (1.0 = none). For a meter.
    pub fn reduction(&self) -> f32 {
        self.gain
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (l, r) = (finite(l), finite(r));
        let peak = l.abs().max(r.abs());
        let required = if peak > LIMITER_CEILING {
            LIMITER_CEILING / peak
        } else {
            1.0
        };
        // Instant attack to the required gain, then recover towards unity.
        let recovering = 1.0 + (self.gain - 1.0) * self.release;
        self.gain = recovering.min(required);

        (
            clamp_or(l * self.gain, -LIMITER_CEILING, LIMITER_CEILING, 0.0),
            clamp_or(r * self.gain, -LIMITER_CEILING, LIMITER_CEILING, 0.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48_000.0;
    /// 0.3 s of settling before measuring.
    const SETTLE: usize = 14_400;
    /// 0.5 s of measuring; a whole number of cycles for every test tone used.
    const MEASURE: usize = 24_000;

    /// Sine at `freq`, sample index `i`, computed in f64 so long runs stay accurate.
    fn tone(i: usize, freq: f32, amp: f32) -> f32 {
        let phase = std::f64::consts::TAU * f64::from(freq) * i as f64 / f64::from(FS);
        (f64::from(amp) * phase.sin()) as f32
    }

    /// Deterministic noise in -1..1, so every run is repeatable.
    struct Noise(u32);

    impl Noise {
        fn sample(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        }
    }

    /// Steady-state gain in dB of a stereo process at `freq` (left channel, sine of amplitude 0.5).
    fn tone_gain_db(process: &mut impl FnMut(f32, f32) -> (f32, f32), freq: f32) -> f32 {
        let (mut in_pow, mut out_pow) = (0.0f64, 0.0f64);
        for i in 0..SETTLE + MEASURE {
            let x = tone(i, freq, 0.5);
            let (y, _) = process(x, x);
            if i >= SETTLE {
                in_pow += f64::from(x) * f64::from(x);
                out_pow += f64::from(y) * f64::from(y);
            }
        }
        (10.0 * (out_pow / in_pow).log10()) as f32
    }

    fn db(mag: f32) -> f32 {
        20.0 * mag.log10()
    }

    /// Largest absolute difference between consecutive samples.
    fn max_step(v: &[f32]) -> f32 {
        v.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    }

    // ---------- BiquadCoeffs ----------

    #[test]
    fn lowpass_1k_shape_at_48k() {
        let c = BiquadCoeffs::lowpass(FS, 1_000.0, FRAC_1_SQRT_2);
        assert!(db(c.magnitude_at(FS, 100.0)).abs() < 0.1);
        assert!((db(c.magnitude_at(FS, 1_000.0)) + 3.0).abs() < 0.3);
        assert!(db(c.magnitude_at(FS, 10_000.0)) < -35.0);
    }

    #[test]
    fn highpass_1k_shape_at_48k() {
        let c = BiquadCoeffs::highpass(FS, 1_000.0, FRAC_1_SQRT_2);
        assert!(db(c.magnitude_at(FS, 10_000.0)).abs() < 0.1);
        assert!((db(c.magnitude_at(FS, 1_000.0)) + 3.0).abs() < 0.3);
        assert!(db(c.magnitude_at(FS, 100.0)) < -35.0);
    }

    #[test]
    fn allpass_has_unity_magnitude_across_audio_band() {
        let c = BiquadCoeffs::allpass(FS, 1_000.0, FRAC_1_SQRT_2);
        for k in 0..=300 {
            let f = 20.0 * 1_000f32.powf(k as f32 / 300.0);
            let m = c.magnitude_at(FS, f);
            assert!((m - 1.0).abs() < 1e-4, "allpass |H| at {f} Hz = {m}");
        }
    }

    #[test]
    fn peaking_centre_gain_is_six_db() {
        let c = BiquadCoeffs::peaking(FS, 1_000.0, 1.0, 6.0);
        assert!((db(c.magnitude_at(FS, 1_000.0)) - 6.0).abs() < 0.1);
        // Far from the centre the filter is back to unity.
        assert!(db(c.magnitude_at(FS, 20.0)).abs() < 0.1);
        assert!(db(c.magnitude_at(FS, 20_000.0)).abs() < 0.1);
    }

    #[test]
    fn peaking_zero_db_is_flat() {
        let c = BiquadCoeffs::peaking(FS, 1_000.0, 1.0, 0.0);
        for f in [50.0, 500.0, 1_000.0, 5_000.0] {
            assert!(db(c.magnitude_at(FS, f)).abs() < 1e-3, "{f} Hz");
        }
    }

    #[test]
    fn coefficient_constructors_clamp_bad_inputs() {
        let cases = [
            BiquadCoeffs::lowpass(FS, f32::NAN, f32::NAN),
            BiquadCoeffs::lowpass(FS, 0.0, 0.0),
            BiquadCoeffs::lowpass(FS, 1e9, -1.0),
            BiquadCoeffs::highpass(FS, -5.0, f32::INFINITY),
            BiquadCoeffs::highpass(FS, f32::INFINITY, 0.0),
            BiquadCoeffs::allpass(FS, f32::NAN, 0.0),
            BiquadCoeffs::peaking(FS, 0.0, 0.0, f32::NAN),
            BiquadCoeffs::peaking(FS, 1_000.0, 1.0, 1e6),
            BiquadCoeffs::peaking(FS, 1_000.0, 1.0, -1e6),
            BiquadCoeffs::lowpass(f32::NAN, 1_000.0, 0.7),
        ];
        for c in cases {
            for v in [c.b0, c.b1, c.b2, c.a1, c.a2] {
                assert!(v.is_finite(), "{c:?}");
            }
            assert!(c.magnitude_at(FS, 1_000.0).is_finite());
        }
    }

    #[test]
    fn huge_q_is_clamped_so_the_filter_still_decays() {
        assert_eq!(
            BiquadCoeffs::highpass(FS, 1_000.0, f32::INFINITY),
            BiquadCoeffs::highpass(FS, 1_000.0, 50.0),
        );
        for q in [f32::INFINITY, 1e30] {
            let mut b = Biquad::new(BiquadCoeffs::highpass(FS, 1_000.0, q));
            b.process(1.0, 1.0);
            let mut energy = 0.0f64;
            for n in 1..96_000usize {
                let (y, _) = b.process(0.0, 0.0);
                if n >= 48_000 {
                    energy += f64::from(y) * f64::from(y);
                }
            }
            assert!(energy < 1e-6, "q {q}: energy after 1 s {energy}");
        }
    }

    #[test]
    fn lowpass_clamps_frequency_to_nyquist_margin() {
        // 1e9 Hz clamps to 0.49 fs, so the response is still a valid low-pass
        // with the cutoff near Nyquist: unity at 1 kHz.
        let c = BiquadCoeffs::lowpass(FS, 1e9, FRAC_1_SQRT_2);
        assert!(db(c.magnitude_at(FS, 1_000.0)).abs() < 0.1);
    }

    // ---------- Biquad ----------

    #[test]
    fn biquad_identity_is_transparent() {
        let mut b = Biquad::new(BiquadCoeffs::IDENTITY);
        let mut noise = Noise(3);
        for _ in 0..1_000 {
            let (x, y) = (noise.sample(), noise.sample());
            assert_eq!(b.process(x, y), (x, y));
        }
    }

    #[test]
    fn biquad_channels_are_independent() {
        let mut b = Biquad::new(BiquadCoeffs::lowpass(FS, 500.0, FRAC_1_SQRT_2));
        for _ in 0..500 {
            let (_, r) = b.process(1.0, 0.0);
            assert_eq!(r, 0.0);
        }
    }

    #[test]
    fn biquad_flushes_decayed_state_to_zero() {
        let mut b = Biquad::new(BiquadCoeffs::lowpass(FS, 1_000.0, FRAC_1_SQRT_2));
        b.process(1.0, 1.0);
        let mut last = (1.0, 1.0);
        for _ in 0..2_000 {
            last = b.process(0.0, 0.0);
        }
        assert_eq!(last, (0.0, 0.0));
        assert_eq!(b.z1, [0.0, 0.0]);
        assert_eq!(b.z2, [0.0, 0.0]);
    }

    #[test]
    fn biquad_nan_input_does_not_poison_state() {
        let c = BiquadCoeffs::lowpass(FS, 1_000.0, FRAC_1_SQRT_2);
        let mut poisoned = Biquad::new(c);
        let mut clean = Biquad::new(c);
        let (y0, y1) = poisoned.process(f32::NAN, f32::INFINITY);
        assert_eq!((y0, y1), clean.process(0.0, 0.0));
        for i in 0..200 {
            let x = tone(i, 300.0, 0.5);
            let a = poisoned.process(x, x);
            assert!(a.0.is_finite() && a.1.is_finite());
            assert_eq!(a, clean.process(x, x));
        }
    }

    #[test]
    fn biquad_reset_clears_memory() {
        let mut b = Biquad::new(BiquadCoeffs::lowpass(FS, 1_000.0, FRAC_1_SQRT_2));
        for _ in 0..100 {
            b.process(1.0, 1.0);
        }
        b.reset();
        assert_eq!(b.process(0.0, 0.0), (0.0, 0.0));
    }

    // ---------- ThreeBandEq ----------

    #[test]
    fn eq_unity_gains_are_flat_across_the_spectrum() {
        for freq in [30.0, 100.0, 250.0, 1_000.0, 3_000.0, 8_000.0, 16_000.0] {
            let mut eq = ThreeBandEq::new(FS);
            let db = tone_gain_db(&mut |l, r| eq.process(l, r), freq);
            assert!(db.abs() <= 0.25, "{freq} Hz: {db} dB");
        }
    }

    #[test]
    fn eq_kill_low_removes_bass_and_keeps_mids() {
        let mut eq = ThreeBandEq::new(FS);
        eq.set_kill(Band::Low, true);
        let bass = tone_gain_db(&mut |l, r| eq.process(l, r), 60.0);
        assert!(bass < -40.0, "60 Hz with low killed: {bass} dB");

        let mut eq = ThreeBandEq::new(FS);
        eq.set_kill(Band::Low, true);
        let mid = tone_gain_db(&mut |l, r| eq.process(l, r), 1_000.0);
        assert!(mid.abs() <= 0.5, "1 kHz with low killed: {mid} dB");
    }

    #[test]
    fn eq_kill_high_removes_treble() {
        let mut eq = ThreeBandEq::new(FS);
        eq.set_kill(Band::High, true);
        let treble = tone_gain_db(&mut |l, r| eq.process(l, r), 10_000.0);
        assert!(treble < -40.0, "10 kHz with high killed: {treble} dB");
    }

    #[test]
    fn eq_zero_gain_removes_band_without_kill() {
        let mut eq = ThreeBandEq::new(FS);
        eq.set_gain(Band::High, 0.0);
        assert!(!eq.kill(Band::High));
        let treble = tone_gain_db(&mut |l, r| eq.process(l, r), 16_000.0);
        assert!(treble < -40.0, "16 kHz with high gain 0: {treble} dB");
    }

    #[test]
    fn eq_low_boost_is_six_db_at_max_gain() {
        let mut eq = ThreeBandEq::new(FS);
        eq.set_gain(Band::Low, EQ_MAX_GAIN);
        let db = tone_gain_db(&mut |l, r| eq.process(l, r), 60.0);
        assert!((db - 6.02).abs() < 0.2, "{db} dB");
    }

    #[test]
    fn eq_setters_clamp_and_store() {
        let mut eq = ThreeBandEq::new(FS);
        assert_eq!(eq.gain(Band::Mid), 1.0);
        assert!(!eq.kill(Band::Mid));

        eq.set_gain(Band::Low, 10.0);
        assert_eq!(eq.gain(Band::Low), EQ_MAX_GAIN);
        eq.set_gain(Band::Mid, -1.0);
        assert_eq!(eq.gain(Band::Mid), 0.0);
        eq.set_gain(Band::High, f32::NAN);
        assert_eq!(eq.gain(Band::High), 1.0);

        eq.set_kill(Band::High, true);
        assert!(eq.kill(Band::High));
        assert!(!eq.kill(Band::Low));
        eq.set_kill(Band::High, false);
        assert!(!eq.kill(Band::High));
    }

    #[test]
    fn eq_gain_change_glides_rather_than_jumps() {
        let mut eq = ThreeBandEq::new(FS);
        eq.set_gain(Band::Low, 0.0);
        eq.process(0.0, 0.0);
        let g = eq.smooth[Band::Low.index()].value();
        assert!(g > 0.5 && g < 1.0, "first sample gain {g}");
        for _ in 0..(FS as usize / 2) {
            eq.process(0.0, 0.0);
        }
        assert_eq!(eq.smooth[Band::Low.index()].value(), 0.0);
    }

    #[test]
    fn eq_nan_and_inf_inputs_stay_finite() {
        let mut eq = ThreeBandEq::new(FS);
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5] {
            let (l, r) = eq.process(x, x);
            assert!(l.is_finite() && r.is_finite());
        }
    }

    // ---------- DjFilter ----------

    #[test]
    fn dj_centre_position_is_bit_identical() {
        let mut f = DjFilter::new(FS);
        let mut noise = Noise(11);
        for _ in 0..48_000 {
            let x = noise.sample();
            let y = -x * 0.5;
            assert_eq!(f.process(x, y), (x, y));
        }
    }

    #[test]
    fn dj_negative_position_lowpasses() {
        let mut f = DjFilter::new(FS);
        f.set_position(-1.0);
        let g = tone_gain_db(&mut |l, r| f.process(l, r), 1_000.0);
        assert!(g < -24.0, "1 kHz at -1: {g} dB");
    }

    #[test]
    fn dj_positive_position_highpasses() {
        let mut f = DjFilter::new(FS);
        f.set_position(1.0);
        let g = tone_gain_db(&mut |l, r| f.process(l, r), 100.0);
        assert!(g < -24.0, "100 Hz at +1: {g} dB");
    }

    #[test]
    fn dj_jump_to_minus_0_6_has_no_click() {
        let mut f = DjFilter::new(FS);
        let freq = 200.0;
        let warm = 9_600;
        let total = warm + 48_000;
        let xs: Vec<f32> = (0..total).map(|i| tone(i, freq, 0.5)).collect();
        let mut ys = Vec::with_capacity(total);
        for (i, &x) in xs.iter().enumerate() {
            if i == warm {
                f.set_position(-0.6);
            }
            ys.push(f.process(x, x).0);
        }
        let max_step = |v: &[f32]| {
            v.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max)
        };
        let in_step = max_step(&xs[warm - 1..]);
        let out_step = max_step(&ys[warm - 1..]);
        assert!(in_step > 0.0);
        assert!(
            out_step <= 1.5 * in_step,
            "output step {out_step} vs sine step {in_step}"
        );
    }

    #[test]
    fn dj_returns_to_bit_identical_bypass_after_settling_at_zero() {
        let mut f = DjFilter::new(FS);
        let mut noise = Noise(5);
        f.set_position(-1.0);
        for _ in 0..14_400 {
            f.process(noise.sample(), noise.sample());
        }
        f.set_position(0.0);
        for _ in 0..14_400 {
            f.process(noise.sample(), noise.sample());
        }
        for _ in 0..2_000 {
            let (x, y) = (noise.sample(), noise.sample());
            assert_eq!(f.process(x, y), (x, y));
        }
    }

    #[test]
    fn dj_position_clamps_and_nan_is_bypass() {
        let mut f = DjFilter::new(FS);
        f.set_position(5.0);
        assert_eq!(f.position(), 1.0);
        f.set_position(-5.0);
        assert_eq!(f.position(), -1.0);
        f.set_position(f32::NAN);
        assert_eq!(f.position(), 0.0);
    }

    // ---------- Echo ----------

    #[test]
    fn echo_first_repeat_lands_at_quarter_second_and_later_repeats_decay() {
        let mut e = Echo::new(FS, 2.0);
        e.set_time_secs(0.25);
        e.set_feedback(0.5);
        e.set_mix(1.0);
        // Snap the glides so the impulse at sample 0 is written at full level.
        e.clear();
        let mut out = vec![0.0f32; 40_000];
        for (i, o) in out.iter_mut().enumerate() {
            let x = if i == 0 { 1.0 } else { 0.0 };
            *o = e.process(x, x).0;
        }
        let peak_near = |centre: usize, half: usize| {
            (centre - half..=centre + half)
                .max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs()))
                .unwrap()
        };

        // Sample 0 is the dry impulse itself, so the search starts after it.
        let first_at = (1..20_000)
            .find(|&i| out[i].abs() > 0.5)
            .expect("no echo above 0.5 within 20000 samples");
        assert!(first_at.abs_diff(12_000) <= 2, "first echo at {first_at}");
        let first = out[first_at].abs();
        assert!(first > 0.6, "first echo magnitude {first}");

        let second = out[peak_near(24_000, 4)].abs();
        let third = out[peak_near(36_000, 6)].abs();
        assert!(second > 0.0 && second < 0.6, "second echo {second}");
        assert!(third < second, "third {third} not below second {second}");
    }

    #[test]
    fn echo_default_mix_is_exact_passthrough() {
        let mut e = Echo::new(FS, 2.0);
        let mut noise = Noise(9);
        for _ in 0..48_000 {
            let (x, y) = (noise.sample(), noise.sample());
            assert_eq!(e.process(x, y), (x, y));
        }
    }

    #[test]
    fn echo_mix_zero_after_clear_is_exact_passthrough() {
        let mut e = Echo::new(FS, 2.0);
        e.set_time_secs(0.25);
        e.set_feedback(0.9);
        e.set_mix(1.0);
        let mut noise = Noise(21);
        for _ in 0..48_000 {
            e.process(noise.sample(), noise.sample());
        }
        e.set_mix(0.0);
        e.clear();
        for _ in 0..48_000 {
            let (x, y) = (noise.sample(), noise.sample());
            assert_eq!(e.process(x, y), (x, y));
        }
    }

    #[test]
    fn echo_mix_zero_lets_existing_tail_ring_out() {
        let mut e = Echo::new(FS, 2.0);
        e.set_time_secs(0.25);
        e.set_feedback(0.5);
        e.set_mix(1.0);
        e.clear();
        e.process(1.0, 1.0);
        e.set_mix(0.0);
        let mut at_repeat = 0.0;
        for i in 1..=12_000 {
            let (l, _) = e.process(0.0, 0.0);
            if i == 12_000 {
                at_repeat = l;
            }
        }
        assert!(
            (at_repeat - 1.0).abs() < 1e-6,
            "tail at repeat: {at_repeat}"
        );
    }

    #[test]
    fn echo_thirty_seconds_of_noise_stays_bounded() {
        let mut e = Echo::new(FS, 2.0);
        e.set_time_secs(0.5);
        e.set_feedback(0.95);
        e.set_mix(1.0);
        let mut noise = Noise(77);
        let mut worst = 0.0f32;
        for _ in 0..(30 * 48_000) {
            let (l, r) = e.process(noise.sample(), noise.sample());
            assert!(l.is_finite() && r.is_finite());
            worst = worst.max(l.abs()).max(r.abs());
        }
        assert!(worst < 6.0, "worst |out| {worst}");
        assert!(worst > 0.5, "echo never sounded: {worst}");
    }

    #[test]
    fn echo_time_and_feedback_are_clamped() {
        let mut e = Echo::new(FS, 2.0);
        e.set_time_secs(0.0);
        assert_eq!(e.delay.target(), 480.0);
        e.set_time_secs(100.0);
        assert_eq!(e.delay.target(), 2.0 * FS);
        e.set_time_secs(f32::NAN);
        assert_eq!(e.delay.target(), 0.25 * FS);

        e.set_feedback(2.0);
        assert_eq!(e.feedback.target(), ECHO_MAX_FEEDBACK);
        e.set_feedback(-1.0);
        assert_eq!(e.feedback.target(), 0.0);
        e.set_mix(7.0);
        assert_eq!(e.mix.target(), 1.0);
        e.set_mix(f32::NAN);
        assert_eq!(e.mix.target(), 0.0);
    }

    #[test]
    fn echo_minimum_delay_repeats_after_ten_milliseconds() {
        let mut e = Echo::new(FS, 2.0);
        e.set_time_secs(0.0);
        e.set_feedback(0.5);
        e.set_mix(1.0);
        e.clear();
        e.process(1.0, 1.0);
        let mut at = 0.0;
        for i in 1..=480 {
            let (l, _) = e.process(0.0, 0.0);
            if i == 480 {
                at = l;
            }
        }
        assert!((at - 1.0).abs() < 1e-6, "10 ms repeat: {at}");
    }

    #[test]
    fn echo_time_change_glides_after_first_set() {
        let mut e = Echo::new(FS, 2.0);
        e.set_time_secs(0.25);
        assert_eq!(e.delay.value(), 12_000.0);
        e.set_time_secs(0.5);
        for _ in 0..(FS as usize / 100) {
            e.process(0.0, 0.0);
        }
        let mid = e.delay.value();
        assert!(mid > 12_000.0 && mid < 24_000.0, "mid-glide delay {mid}");
        for _ in 0..(FS as usize) {
            e.process(0.0, 0.0);
        }
        // f32 smoothing stalls a few samples short of a large target; allow 2e-4 relative.
        assert!(
            (e.delay.value() - 24_000.0).abs() < 24_000.0 * 2e-4,
            "settled at {}",
            e.delay.value()
        );
    }

    #[test]
    fn echo_first_time_change_glides_after_audio_has_passed() {
        let mut e = Echo::new(FS, 2.0);
        e.set_feedback(0.0);
        e.set_mix(1.0);
        // Audio goes through at the default 0.25 s delay before the first change.
        let warm = 24_000;
        for i in 0..warm {
            let x = tone(i, 230.0, 0.5);
            e.process(x, x);
        }
        let before = e.delay.value();
        e.set_time_secs(0.1);
        assert_eq!(e.delay.value(), before, "first change must not snap");

        let mut xs = Vec::new();
        let mut sends = Vec::new();
        for i in warm..warm + 24_000 {
            let x = tone(i, 230.0, 0.5);
            let (y, _) = e.process(x, x);
            xs.push(x);
            sends.push(y - x);
        }
        let now = e.delay.value();
        assert!(now < before && now > 4_800.0, "mid-glide delay {now}");
        // The read head speeds up at most ~3.5x while gliding from 0.25 s to 0.1 s.
        // A snap would move it by 7200 samples, which is 34.5 cycles of 230 Hz: the
        // replayed sine is half a cycle out of phase and steps by about 1.0.
        let sine_step = max_step(&xs);
        let send_step = max_step(&sends);
        assert!(
            send_step <= 5.0 * sine_step,
            "send step {send_step} vs sine step {sine_step}"
        );
    }

    #[test]
    fn echo_mix_and_feedback_jumps_do_not_click() {
        let mut e = Echo::new(FS, 2.0);
        e.set_time_secs(0.25);
        e.clear();
        let mut xs = Vec::with_capacity(96_000);
        let mut sends = Vec::with_capacity(96_000);
        // The jumps land on the 200 Hz crest, where an unsmoothed step is largest
        // (sample 24000 is a zero crossing and would hide the bug).
        for i in 0..96_000 {
            if i == 24_060 {
                e.set_mix(1.0);
            }
            if i == 48_060 {
                e.set_feedback(0.95);
            }
            let x = tone(i, 200.0, 0.5);
            let (y, _) = e.process(x, x);
            xs.push(x);
            sends.push(y - x);
        }
        let echo_peak = sends.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(echo_peak > 0.3, "echo never sounded: {echo_peak}");
        // Feedback 0.95 on a 200 Hz tone builds the echo past the input level (peak
        // about 1.08 against 0.5). That is real loop gain, not a click, so the bound
        // is the 200 Hz slope at the echo's own peak: a clean sine never exceeds it.
        // An unsmoothed jump on the crest writes a step of about the input's
        // amplitude (0.5), far above this bound.
        let slope_per_amp = max_step(&xs) / 0.5;
        let send_step = max_step(&sends);
        assert!(
            send_step <= 1.5 * echo_peak * slope_per_amp,
            "echo step {send_step} vs slope of a {echo_peak} peak {}",
            echo_peak * slope_per_amp
        );
    }

    #[test]
    fn echo_nan_and_inf_inputs_stay_finite() {
        let mut e = Echo::new(FS, 1.0);
        e.set_time_secs(0.05);
        e.set_mix(1.0);
        e.set_feedback(0.95);
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e30] {
            let (l, r) = e.process(x, x);
            assert!(l.is_finite() && r.is_finite());
        }
        // Huge but finite input passes straight through; the buffer stays finite.
        for _ in 0..10_000 {
            let (l, r) = e.process(1.0e30, -1.0e30);
            assert!(l.is_finite() && r.is_finite());
        }
        for _ in 0..10_000 {
            let (l, r) = e.process(0.0, 0.0);
            assert!(l.is_finite() && r.is_finite());
        }
    }

    // ---------- Crossfader ----------

    #[test]
    fn crossfader_smooth_is_constant_power_with_minus_3_db_centre() {
        let (a, b) = crossfader_gains(0.5, CrossfaderCurve::Smooth);
        assert!((a - FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((b - FRAC_1_SQRT_2).abs() < 1e-6);
        for k in 0..=1_000 {
            let (a, b) = crossfader_gains(k as f32 / 1_000.0, CrossfaderCurve::Smooth);
            assert!((a * a + b * b - 1.0).abs() < 1e-6, "pos {k}");
        }
        let (a, b) = crossfader_gains(0.0, CrossfaderCurve::Smooth);
        assert!((a - 1.0).abs() < 1e-6 && b.abs() < 1e-6);
        let (a, b) = crossfader_gains(1.0, CrossfaderCurve::Smooth);
        assert!(a.abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
    }

    #[test]
    fn crossfader_sharp_is_full_level_until_the_ends() {
        assert_eq!(crossfader_gains(0.5, CrossfaderCurve::Sharp), (1.0, 1.0));
        assert_eq!(crossfader_gains(1.0, CrossfaderCurve::Sharp), (0.0, 1.0));
        assert_eq!(crossfader_gains(0.0, CrossfaderCurve::Sharp), (1.0, 0.0));
        let (a, b) = crossfader_gains(0.97, CrossfaderCurve::Sharp);
        assert!((a - 0.6).abs() < 1e-5 && b == 1.0);
        let (a, b) = crossfader_gains(0.03, CrossfaderCurve::Sharp);
        assert!(a == 1.0 && (b - 0.6).abs() < 1e-5);
    }

    #[test]
    fn crossfader_clamps_position_and_survives_nan() {
        for curve in [CrossfaderCurve::Smooth, CrossfaderCurve::Sharp] {
            assert_eq!(crossfader_gains(-3.0, curve), crossfader_gains(0.0, curve));
            assert_eq!(crossfader_gains(3.0, curve), crossfader_gains(1.0, curve));
            let (a, b) = crossfader_gains(f32::NAN, curve);
            assert!(a.is_finite() && b.is_finite());
        }
    }

    // ---------- Limiter ----------

    #[test]
    fn limiter_holds_two_amplitude_sine_under_ceiling() {
        let mut lim = Limiter::new(FS);
        let mut worst = 0.0f32;
        for i in 0..48_000 {
            let x = tone(i, 1_000.0, 2.0);
            let (l, r) = lim.process(x, x);
            worst = worst.max(l.abs()).max(r.abs());
            assert!(l.abs() <= LIMITER_CEILING && r.abs() <= LIMITER_CEILING);
        }
        assert!(worst > 0.9 * LIMITER_CEILING, "limited peak {worst}");
        assert!(lim.reduction() < 0.6);
    }

    #[test]
    fn limiter_returns_to_unity_after_release() {
        let mut lim = Limiter::new(FS);
        for i in 0..48_000 {
            let x = tone(i, 1_000.0, 2.0);
            lim.process(x, x);
        }
        assert!(lim.reduction() < 0.6);
        for i in 0..72_000 {
            let x = tone(i, 1_000.0, 0.3);
            let (l, _) = lim.process(x, x);
            assert!(l.abs() <= LIMITER_CEILING);
        }
        assert!(
            (lim.reduction() - 1.0).abs() < 0.01,
            "gain after 1.5 s: {}",
            lim.reduction()
        );
    }

    #[test]
    fn limiter_leaves_quiet_signal_untouched() {
        let mut lim = Limiter::new(FS);
        let mut noise = Noise(31);
        for _ in 0..48_000 {
            let (x, y) = (noise.sample() * 0.9, noise.sample() * 0.9);
            assert_eq!(lim.process(x, y), (x, y));
        }
        assert_eq!(lim.reduction(), 1.0);
    }

    #[test]
    fn limiter_links_channels() {
        let mut lim = Limiter::new(FS);
        // Right channel is loud, left is quiet: left must be pulled down too.
        let (l, r) = lim.process(0.5, 2.0);
        assert!(lim.reduction() < 0.5);
        assert!(r.abs() <= LIMITER_CEILING);
        assert!(l.abs() < 0.5);
    }

    #[test]
    fn limiter_nan_and_inf_inputs_stay_finite() {
        let mut lim = Limiter::new(FS);
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e38] {
            let (l, r) = lim.process(x, -x);
            assert!(l.is_finite() && r.is_finite());
            assert!(l.abs() <= LIMITER_CEILING && r.abs() <= LIMITER_CEILING);
        }
        assert!(lim.reduction().is_finite());
    }
}
