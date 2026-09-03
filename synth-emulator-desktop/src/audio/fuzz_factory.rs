//! Reduced-order analog Fuzz Factory.
//!
//! Full 12-node Newton was unstable (rail chatter = low rumble) and too heavy.
//! This keeps the Factory topology as audio-rate nonlinear stages:
//!   LPB-1 clip → Ge pair with emitter degeneration (Drive/Comp) and collector
//!   starve (Gate) → DC-coupled negative feedback (47k) → Stab sags headroom
//!   until the phase-shift of that loop howls through the same clippers.
//! Low velcro is the 100µ supply path motorboating, not a sine oscillator.

const OS: usize = 4;
const DELAY: usize = 1024;
const DELAY_MASK: usize = DELAY - 1;

#[derive(Clone, Copy, Debug)]
pub struct FuzzKnobs {
    pub vol: f32,
    pub gate: f32,
    pub comp: f32,
    pub drive: f32,
    pub stab: f32,
}

impl Default for FuzzKnobs {
    fn default() -> Self {
        Self {
            vol: 0.68,
            gate: 0.10,
            comp: 0.30,
            drive: 0.72,
            stab: 0.90,
        }
    }
}

impl FuzzKnobs {
    pub fn clamp(mut self) -> Self {
        self.vol = self.vol.clamp(0.0, 1.0);
        self.gate = self.gate.clamp(0.0, 1.0);
        self.comp = self.comp.clamp(0.0, 1.0);
        self.drive = self.drive.clamp(0.0, 1.0);
        self.stab = self.stab.clamp(0.0, 1.0);
        self
    }

    /// Open Gate, Stab CW — thick Face-ish fuzz.
    pub fn smooth() -> Self {
        Self {
            vol: 0.72,
            gate: 0.04,
            comp: 0.22,
            drive: 0.60,
            stab: 0.94,
        }
    }

    /// ZVex “Velcro fuzz”: Gate open, Comp pegged, Stab ~2:00.
    pub fn velcro() -> Self {
        Self {
            vol: 0.74,
            gate: 0.10,
            comp: 0.94,
            drive: 0.90,
            stab: 0.68,
        }
    }

    /// Stab CCW — self-oscillating squeal with no input.
    pub fn squeal() -> Self {
        Self {
            vol: 0.70,
            gate: 0.18,
            comp: 0.58,
            drive: 0.86,
            stab: 0.10,
        }
    }

    /// Comp pegged — pinched, spatty, less sustain.
    pub fn gated_hi() -> Self {
        Self {
            vol: 0.76,
            gate: 0.28,
            comp: 0.92,
            drive: 0.64,
            stab: 0.80,
        }
    }
}

#[inline]
fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-4.5, 4.5);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Asymmetric germanium-ish clip. Headroom tracks the sagged rail.
#[inline]
fn ge_clip(x: f32, head: f32) -> f32 {
    let s = head.max(0.07);
    let y = fast_tanh(x / s);
    (y + 0.36 * y * y - 0.09 * y * y * y) * s
}

pub struct FuzzFactory {
    sr: f32,
    knobs: FuzzKnobs,
    knobs_z: FuzzKnobs,
    dc_in: f32,
    dc_ff: f32,
    dc_out: f32,
    fb_lp: f32,
    howl_lp: f32,
    boat_lp: f32,
    boat_hp: f32,
    env: f32,
    q3: f32,
    was_gated: bool,
    delay: [f32; DELAY],
    delay_i: usize,
}

impl FuzzFactory {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate.max(8_000.0),
            knobs: FuzzKnobs::default(),
            knobs_z: FuzzKnobs::default(),
            dc_in: 0.0,
            dc_ff: 0.0,
            dc_out: 0.0,
            fb_lp: 0.0,
            howl_lp: 0.0,
            boat_lp: 0.0,
            boat_hp: 0.0,
            env: 0.0,
            q3: 0.0,
            was_gated: false,
            delay: [0.0; DELAY],
            delay_i: 0,
        }
    }

    pub fn set_knobs(&mut self, k: FuzzKnobs) {
        self.knobs = k.clamp();
    }

    pub fn knobs(&self) -> FuzzKnobs {
        self.knobs
    }

    pub fn va(&self) -> f32 {
        sag(self.knobs_z.stab)
    }

    pub fn last_iters(&self) -> u32 {
        1
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let a = 0.04;
        self.knobs_z.vol += a * (self.knobs.vol - self.knobs_z.vol);
        self.knobs_z.gate += a * (self.knobs.gate - self.knobs_z.gate);
        self.knobs_z.comp += a * (self.knobs.comp - self.knobs_z.comp);
        self.knobs_z.drive += a * (self.knobs.drive - self.knobs_z.drive);
        self.knobs_z.stab += a * (self.knobs.stab - self.knobs_z.stab);

        let mut acc = 0.0;
        for _ in 0..OS {
            acc += self.tick(x);
        }
        (acc / OS as f32).clamp(-1.4, 1.4)
    }

    fn tick(&mut self, x: f32) -> f32 {
        let k = self.knobs_z;
        let os_sr = self.sr * OS as f32;
        let va = sag(k.stab);
        let stab_open = ((0.90 - k.stab) / 0.90).max(0.0);

        // C1: ~12 Hz input HPF
        let a_in = 1.0 - (-2.0 * std::f32::consts::PI * 12.0 / os_sr).exp();
        self.dc_in += a_in * (x - self.dc_in);
        let ac = x - self.dc_in;

        // Q1 silicon booster (LPB-1): a hair hotter, still not pre-squaring Drive.
        let q1 = fast_tanh(ac * 2.55) * 1.14;

        // C2 into the Face: some bass cut, but keep the grave for velcro.
        let f_ff = 52.0 + k.drive * 95.0;
        let a_ff = 1.0 - (-2.0 * std::f32::consts::PI * f_ff / os_sr).exp();
        self.dc_ff += a_ff * (q1 - self.dc_ff);
        let ff_in = q1 - self.dc_ff;

        // Supply bounce (100µ Stab): loud notes sag the rail, pitch-drops the howl.
        let f_env = 18.0 + k.stab * 70.0;
        let a_env = 1.0 - (-2.0 * std::f32::consts::PI * f_env / os_sr).exp();
        self.env += a_env * (self.q3.abs() - self.env);
        let va_live = va * (1.0 - stab_open * 0.42 * self.env.min(1.15));

        // HF squeal: phase-shift of Q3 through the same clippers.
        // Comp CW thickens/lowers; Drive raises. Sag lengthens the tap (chirp).
        let f_howl = 170.0 + k.drive * 1850.0 + (1.0 - k.comp) * 620.0;
        let n = ((os_sr / f_howl) + stab_open * self.env * 90.0).clamp(16.0, (DELAY - 4) as f32)
            as usize;
        let tap = self.delay[(self.delay_i + DELAY - n) & DELAY_MASK];
        let hp_f = 28.0 + k.stab * 200.0;
        let a_hp = 1.0 - (-2.0 * std::f32::consts::PI * hp_f / os_sr).exp();
        self.howl_lp += a_hp * (tap - self.howl_lp);
        let howl = tap - self.howl_lp;
        let howl_g = stab_open.powf(0.55)
            * (0.38 + k.drive * 2.05)
            * (1.0 - 0.40 * k.gate)
            * (1.0 - 0.28 * k.comp);

        // LF velcro / motorboat: bandpass around the starved supply, not a whistle.
        let f_boat = 48.0 + k.comp * 70.0 + (1.0 - k.drive) * 45.0;
        let a_bl = 1.0 - (-2.0 * std::f32::consts::PI * f_boat / os_sr).exp();
        let a_bh = 1.0 - (-2.0 * std::f32::consts::PI * 22.0 / os_sr).exp();
        self.boat_lp += a_bl * (self.q3 - self.boat_lp);
        self.boat_hp += a_bh * (self.boat_lp - self.boat_hp);
        let boat = self.boat_lp - self.boat_hp;
        let boat_g = stab_open.powf(0.45)
            * (1.05 + k.comp * 1.85)
            * (0.65 + 0.25 * k.gate)
            * (0.65 + (1.0 - k.drive) * 0.50);

        // Iceo: tiny leak so the loop can start when gain > 1 (Stab down).
        let leak = stab_open * (1.0 - 0.85 * k.gate) * 0.0036;

        let nfb = 0.16 * (1.0 - 0.55 * stab_open);
        let mut u = ff_in - nfb * self.fb_lp + howl_g * howl + boat_g * boat + leak;

        // Ge pair is lopsided: negative half starves first (ripping velcro).
        let n_keep = (1.0 - 0.38 * k.gate - 0.24 * k.comp * k.comp).max(0.12);
        if u < 0.0 {
            u *= n_keep;
        }

        // Gate: Q2 collector starve with hysteresis so decay chatters, not fades.
        // Comp pinches Q3 on its own — don't also raise this threshold or the
        // motorboat (grave velcro) dies when Comp is up, which is the ZVex setting.
        let th_on = k.gate * k.gate * 0.62;
        let th_off = th_on * 0.30;
        let au = u.abs();
        let gated = if th_on < 1.0e-4 {
            false
        } else if self.was_gated {
            au < th_on
        } else {
            au < th_off
        };
        if gated {
            let t = (au / th_on.max(1.0e-4)).clamp(0.0, 1.0);
            u *= t * t * t * t;
        } else if self.was_gated {
            u += (0.40 + 0.30 * k.comp) * u.signum();
        }
        self.was_gated = gated;

        // Drive = emitter degeneration of Q2 (less R → more gain).
        let g2 = 0.48 + k.drive * 22.0;
        let head2 = (0.085 + va_live * 0.060) * (1.0 - 0.62 * k.gate);
        let q2 = ge_clip(u * g2, head2.max(0.050));

        // Comp = Q3 emitter: CW pinches (less gain, harder, spatty)
        let g3 = 0.58 + (1.0 - k.comp) * 2.40;
        let pinch = (0.095 + va_live * 0.048) * (1.0 - 0.72 * k.comp);
        let q3 = ge_clip(q2 * g3, pinch.max(0.042));
        self.q3 = q3;
        self.delay[self.delay_i] = q3;
        self.delay_i = (self.delay_i + 1) & DELAY_MASK;

        // 47k-ish LPF around the pair (bias loop)
        let a_fb = 1.0 - (-2.0 * std::f32::consts::PI * 85.0 / os_sr).exp();
        self.fb_lp += a_fb * (q3 - self.fb_lp);

        // Output coupling ~25 Hz, then volume
        let a_out = 1.0 - (-2.0 * std::f32::consts::PI * 25.0 / os_sr).exp();
        self.dc_out += a_out * (q3 - self.dc_out);
        (q3 - self.dc_out) * (0.02 + 1.48 * k.vol)
    }
}

fn sag(stab: f32) -> f32 {
    1.20 + 7.60 * stab.powf(0.78)
}

#[cfg(test)]
fn rms(xs: &[f32]) -> f32 {
    if xs.is_empty() {
        return 0.0;
    }
    let s: f32 = xs.iter().map(|x| x * x).sum();
    (s / xs.len() as f32).sqrt()
}

#[cfg(test)]
fn crest(xs: &[f32]) -> f32 {
    let r = rms(xs);
    if r < 1.0e-6 {
        return 0.0;
    }
    let peak = xs.iter().copied().fold(0.0f32, |a, b| a.max(b.abs()));
    peak / r
}

#[cfg(test)]
fn band_goertzel(xs: &[f32], sr: f32, hz: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * hz / sr;
    let c = 2.0 * w.cos();
    let mut s1 = 0.0f32;
    let mut s2 = 0.0f32;
    for &x in xs {
        let s0 = x + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let re = s1 - s2 * w.cos();
    let im = s2 * w.sin();
    (re * re + im * im).sqrt() / xs.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(ff: &mut FuzzFactory, n: usize, input: impl Fn(usize) -> f32) -> Vec<f32> {
        (0..n).map(|i| ff.process(input(i))).collect()
    }

    fn sine_at(sr: f32, hz: f32, amp: f32) -> impl Fn(usize) -> f32 {
        move |i| (2.0 * std::f32::consts::PI * hz * i as f32 / sr).sin() * amp
    }

    #[test]
    fn sine_gets_fuzzed() {
        let mut ff = FuzzFactory::new(48_000.0);
        ff.set_knobs(FuzzKnobs::smooth());
        let sr = 48_000.0f32;
        let y = render(&mut ff, 8_000, sine_at(sr, 110.0, 0.35));
        let tail = &y[y.len() - 2048..];
        let r = rms(tail);
        assert!(r > 0.08, "fuzz rms {r}");
        let mut c1 = 0.0f32;
        let mut c3 = 0.0f32;
        for (i, &s) in tail.iter().enumerate() {
            let p = 2.0 * std::f32::consts::PI * 110.0 * i as f32 / sr;
            c1 += s * p.sin();
            c3 += s * (3.0 * p).sin();
        }
        assert!(
            c3.abs() > c1.abs() * 0.08,
            "need harmonics h1={} h3={}",
            c1.abs(),
            c3.abs()
        );
    }

    #[test]
    fn stab_up_is_quiet_with_no_input() {
        let mut ff = FuzzFactory::new(48_000.0);
        ff.set_knobs(FuzzKnobs::smooth());
        let y = render(&mut ff, 4_000, |_| 0.0);
        let r = rms(&y[y.len() - 1024..]);
        assert!(r < 0.02, "idle noise {r}");
    }

    #[test]
    fn stab_down_squeals_with_no_input() {
        let mut ff = FuzzFactory::new(48_000.0);
        ff.set_knobs(FuzzKnobs::squeal());
        let y = render(&mut ff, 12_000, |i| {
            if (80..120).contains(&i) {
                0.3
            } else {
                0.0
            }
        });
        let r = rms(&y[y.len() - 4096..]);
        let peak = y[y.len() - 4096..]
            .iter()
            .copied()
            .fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(r > 0.06 && peak > 0.12, "howl rms={r} peak={peak}");
    }

    #[test]
    fn squeal_is_dirty_not_a_sine() {
        let mut ff = FuzzFactory::new(48_000.0);
        ff.set_knobs(FuzzKnobs::squeal());
        let y = render(&mut ff, 16_000, |i| {
            if (80..160).contains(&i) {
                0.25
            } else {
                0.0
            }
        });
        let tail = &y[y.len() - 8192..];
        let cf = crest(tail);
        assert!(cf > 1.25, "squeal too sine-like crest={cf}");
    }

    #[test]
    fn velcro_splats_on_the_decay() {
        let sr = 48_000.0f32;
        let burst = |i: usize| {
            if i < 4_000 {
                (2.0 * std::f32::consts::PI * 98.0 * i as f32 / sr).sin() * 0.55
            } else {
                0.0
            }
        };
        let mut smooth = FuzzFactory::new(sr);
        smooth.set_knobs(FuzzKnobs::smooth());
        let ys = render(&mut smooth, 10_000, burst);
        let mut velcro = FuzzFactory::new(sr);
        velcro.set_knobs(FuzzKnobs::velcro());
        let yv = render(&mut velcro, 10_000, burst);
        let cs = crest(&ys[5_000..]);
        let cv = crest(&yv[5_000..]);
        assert!(cv > cs * 1.08 || cv > 2.0, "velcro crest={cv} smooth={cs}");
    }

    #[test]
    fn grave_velcro_motorboats() {
        let sr = 48_000.0f32;
        let mut ff = FuzzFactory::new(sr);
        ff.set_knobs(FuzzKnobs {
            vol: 0.72,
            gate: 0.10,
            comp: 0.88,
            drive: 0.42,
            stab: 0.34,
        });
        let y = render(&mut ff, 14_000, |i| {
            if (60..180).contains(&i) {
                0.28
            } else {
                0.0
            }
        });
        let tail = &y[y.len() - 6144..];
        let r = rms(tail);
        let low = band_goertzel(tail, sr, 72.0);
        let high = band_goertzel(tail, sr, 1400.0);
        assert!(r > 0.04, "grave velcro should keep oscillating rms={r}");
        assert!(
            low > high * 0.35,
            "want motorboat not whistle low={low} high={high}"
        );
    }

    #[test]
    fn gate_chokes_the_tail() {
        let sr = 48_000.0f32;
        let burst = |i: usize| {
            if i < 3_000 {
                (2.0 * std::f32::consts::PI * 146.8 * i as f32 / sr).sin() * 0.6
            } else {
                0.0
            }
        };
        let mut open = FuzzFactory::new(48_000.0);
        open.set_knobs(FuzzKnobs {
            gate: 0.0,
            stab: 0.97,
            drive: 0.7,
            comp: 0.3,
            vol: 0.7,
        });
        let to = rms(&render(&mut open, 8_000, burst)[5_000..]);
        let mut shut = FuzzFactory::new(48_000.0);
        shut.set_knobs(FuzzKnobs {
            gate: 1.0,
            stab: 0.97,
            drive: 0.7,
            comp: 0.3,
            vol: 0.7,
        });
        let ts = rms(&render(&mut shut, 8_000, burst)[5_000..]);
        assert!(ts < to * 0.55 + 0.004, "gate open={to} shut={ts}");
    }

    #[test]
    fn knobs_move_the_supply() {
        let mut ff = FuzzFactory::new(48_000.0);
        ff.set_knobs(FuzzKnobs {
            stab: 1.0,
            ..FuzzKnobs::default()
        });
        for _ in 0..200 {
            ff.process(0.0);
        }
        let hi = ff.va();
        ff.set_knobs(FuzzKnobs {
            stab: 0.0,
            ..FuzzKnobs::default()
        });
        for _ in 0..200 {
            ff.process(0.0);
        }
        let lo = ff.va();
        assert!(hi > lo + 2.0, "sag hi={hi} lo={lo}");
    }

    #[test]
    fn silent_is_not_nan() {
        let mut ff = FuzzFactory::new(48_000.0);
        for _ in 0..512 {
            let y = ff.process(0.0);
            assert!(y.is_finite());
        }
    }

    #[test]
    fn drive_and_vol_are_audible() {
        let sr = 48_000.0f32;
        let input = sine_at(sr, 220.0, 0.18);

        let mut lo = FuzzFactory::new(sr);
        lo.set_knobs(FuzzKnobs {
            drive: 0.05,
            stab: 0.97,
            gate: 0.0,
            comp: 0.2,
            vol: 0.7,
        });
        let y_lo = render(&mut lo, 6_000, &input);
        let r_lo = rms(&y_lo[2048..]);

        let mut hi = FuzzFactory::new(sr);
        hi.set_knobs(FuzzKnobs {
            drive: 1.0,
            stab: 0.97,
            gate: 0.0,
            comp: 0.2,
            vol: 0.7,
        });
        let y_hi = render(&mut hi, 6_000, &input);
        let r_hi = rms(&y_hi[2048..]);
        let mut diff = 0.0f32;
        for i in 2048..y_lo.len() {
            let d = y_hi[i] - y_lo[i];
            diff += d * d;
        }
        let r_diff = (diff / (y_lo.len() - 2048) as f32).sqrt();
        assert!(
            r_diff > 0.04,
            "drive lo={r_lo} hi={r_hi} diff={r_diff}"
        );

        let mut quiet = FuzzFactory::new(sr);
        quiet.set_knobs(FuzzKnobs {
            vol: 0.05,
            stab: 0.97,
            gate: 0.0,
            drive: 0.7,
            comp: 0.3,
        });
        let r_q = rms(&render(&mut quiet, 4_000, &input)[1024..]);
        let mut loud = FuzzFactory::new(sr);
        loud.set_knobs(FuzzKnobs {
            vol: 1.0,
            stab: 0.97,
            gate: 0.0,
            drive: 0.7,
            comp: 0.3,
        });
        let r_l = rms(&render(&mut loud, 4_000, &input)[1024..]);
        assert!(r_l > r_q * 3.0, "vol quiet={r_q} loud={r_l}");
    }

    #[test]
    fn presets_do_not_sound_the_same() {
        let sr = 48_000.0f32;
        let input = sine_at(sr, 146.8, 0.4);
        let mut buffers = Vec::new();
        for k in [
            FuzzKnobs::smooth(),
            FuzzKnobs::velcro(),
            FuzzKnobs::squeal(),
            FuzzKnobs::gated_hi(),
        ] {
            let mut ff = FuzzFactory::new(sr);
            ff.set_knobs(k);
            buffers.push(render(&mut ff, 8_000, &input));
        }
        let tails: Vec<f32> = buffers.iter().map(|y| rms(&y[4096..])).collect();
        let peak_spread = tails.iter().copied().fold(0.0f32, f32::max)
            - tails.iter().copied().fold(f32::MAX, f32::min);
        assert!(
            peak_spread > 0.03,
            "presets too similar rms={tails:?}"
        );
    }
}
