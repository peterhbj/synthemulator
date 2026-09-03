//! One circuit: LPB-1 (Q1) into a germanium Fuzz Face (Q2/Q3).
//!
//! Stab = how stable the wave is. CW the pair sits still (Face). CCW it
//! hears itself; raising Stab then pushes that howl up in pitch until it
//! can't hold and goes quiet.
//! Comp = the treble / pitch of that wave. At 0 the texture is velcro
//! (and with Stab down, silence — Q3 is off). Turning Comp up starts a
//! bass rip and climbs to a high squeal.
//! Gate starves Q2. Drive is Q2's emitter gain. Vol is after Q3.

const OS: usize = 4;
const DELAY: usize = 4096;
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
        // Always-on Face: Stab CW, Gate open.
        Self {
            vol: 0.70,
            gate: 0.04,
            comp: 0.28,
            drive: 0.62,
            stab: 0.94,
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

    /// Stab CW, Comp up — open Face.
    pub fn smooth() -> Self {
        Self {
            vol: 0.72,
            gate: 0.0,
            comp: 0.55,
            drive: 0.58,
            stab: 1.0,
        }
    }

    /// Comp at 0: velcro rip on the note. Stab not fully down (that would mute).
    pub fn velcro() -> Self {
        Self {
            vol: 0.74,
            gate: 0.08,
            comp: 0.04,
            drive: 0.80,
            stab: 0.72,
        }
    }

    /// Stab down, Comp up — high squeal.
    pub fn squeal() -> Self {
        Self {
            vol: 0.70,
            gate: 0.10,
            comp: 0.78,
            drive: 0.82,
            stab: 0.14,
        }
    }

    pub fn gated_hi() -> Self {
        Self {
            vol: 0.74,
            gate: 0.55,
            comp: 0.35,
            drive: 0.85,
            stab: 0.95,
        }
    }

    /// Stab CW, Comp mid-high — singing lead.
    pub fn plug() -> Self {
        Self {
            vol: 0.76,
            gate: 0.12,
            comp: 0.62,
            drive: 0.72,
            stab: 0.96,
        }
    }
}

#[inline]
fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-4.5, 4.5);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

#[inline]
fn ge_clip(x: f32, head: f32) -> f32 {
    let s = head.max(0.07);
    let y = fast_tanh(x / s);
    (y + 0.32 * y * y - 0.07 * y * y * y) * s
}

pub struct FuzzFactory {
    sr: f32,
    knobs: FuzzKnobs,
    knobs_z: FuzzKnobs,
    dc_in: f32,
    dc_ff: f32,
    dc_out: f32,
    q3_lp: f32,
    fb_lp: f32,
    rail_lp: f32,
    env: f32,
    q3: f32,
    f_z: f32,
    rng: u32,
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
            q3_lp: 0.0,
            fb_lp: 0.0,
            rail_lp: 0.0,
            env: 0.0,
            q3: 0.0,
            f_z: 200.0,
            rng: 0xC0FFEE,
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
        let a_pitch = 0.012;
        self.knobs_z.vol += a * (self.knobs.vol - self.knobs_z.vol);
        self.knobs_z.gate += a * (self.knobs.gate - self.knobs_z.gate);
        self.knobs_z.comp += a_pitch * (self.knobs.comp - self.knobs_z.comp);
        self.knobs_z.drive += a * (self.knobs.drive - self.knobs_z.drive);
        self.knobs_z.stab += a_pitch * (self.knobs.stab - self.knobs_z.stab);

        let mut acc = 0.0;
        for _ in 0..OS {
            acc += self.tick(x);
        }
        (acc / OS as f32).clamp(-1.4, 1.4)
    }

    fn tick(&mut self, x: f32) -> f32 {
        let k = self.knobs_z;
        let os_sr = self.sr * OS as f32;
        let pi = std::f32::consts::PI;

        // Stab = rail stiffness. Low = the pair can hear itself.
        let unstable = ((0.84 - k.stab) / 0.84).max(0.0);
        let va = 0.11 + 0.58 * k.stab.powf(0.72);
        let a_env = 1.0 - (-2.0 * pi * (14.0 + k.stab * 50.0) / os_sr).exp();
        self.env += a_env * (self.q3.abs() - self.env);
        let va_live = (va * (1.0 - unstable * 0.50 * self.env.min(1.1))).max(0.055);

        // Stab down + Comp at 0: Q3 is off → silence. Comp is the start switch.
        let live = (k.comp / 0.10).clamp(0.0, 1.0);
        let dead = unstable * (1.0 - live);

        let a_in = 1.0 - (-2.0 * pi * 12.0 / os_sr).exp();
        self.dc_in += a_in * (x - self.dc_in);
        let ac = x - self.dc_in;

        let q1 = fast_tanh(ac * 3.4) * 1.08;

        let f_ff = 48.0 + k.drive * 55.0;
        let a_ff = 1.0 - (-2.0 * pi * f_ff / os_sr).exp();
        self.dc_ff += a_ff * (q1 - self.dc_ff);
        let ff_in = q1 - self.dc_ff;

        // Howl pitch: Comp climbs grave→squeal; Stab also pushes pitch up until
        // the wave is too stable to hold.
        let f_osc = 40.0
            * (3600.0_f32 / 40.0).powf(k.comp.powf(0.88))
            * (1.0 + k.stab * 2.0)
            * (0.92 + 0.16 * k.drive);
        let a_f = 1.0 - (-2.0 * pi * 14.0 / os_sr).exp();
        self.f_z += a_f * (f_osc - self.f_z);
        let n = (os_sr / self.f_z.max(20.0)).clamp(2.0, (DELAY - 3) as f32);
        let tap = self.delay_at(n);
        // Comp is the treble of the wave: low Comp keeps rumble, Comp up strips bass.
        let hp_f = 18.0 + k.comp * (0.55 * f_osc).min(900.0);
        let a_rail = 1.0 - (-2.0 * pi * hp_f / os_sr).exp();
        self.rail_lp += a_rail * (tap - self.rail_lp);
        let tap_ac = tap - self.rail_lp;

        let nfb = 0.03 + 0.18 * k.stab;
        let pfb = unstable.powf(0.55)
            * live.powf(0.40)
            * (1.10 + k.drive * 0.85)
            * (1.0 - 0.28 * k.gate);
        let mut u = ff_in - nfb * self.fb_lp + pfb * tap_ac;
        u *= 1.0 - 0.97 * dead;
        // Iceo: the pair starts itself. No key-tap needed.
        if pfb > 0.2 && self.env < 0.05 {
            self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let nse = (self.rng >> 16) as f32 / 32_768.0 - 1.0;
            u += pfb * 0.045 * nse;
        }

        // Comp at 0 (with Stab still up) = velcro splat on the note.
        let velcro = (1.0 - k.comp).powf(1.35) * k.stab;
        let starve = k.gate * (0.40 + 0.35 * (1.0 - k.comp)) + velcro * 0.28;
        let th = starve * starve * 0.50;
        let au = u.abs();
        if th > 1.0e-4 && au < th {
            let t = au / th;
            u *= t * t;
        }

        let g2 = 1.4 + k.drive * 15.0 + k.comp * 2.2;
        let head2 = (0.09 + va_live * 0.55) * (1.0 - 0.48 * k.gate);
        let q2 = ge_clip(u * g2, head2.max(0.05));

        // Comp up = Q3 conducts more treble. Comp 0 + Stab down = Q3 off.
        let g3 = (0.38 + k.comp * 3.1) * (1.0 - 0.95 * dead);
        let head3 = 0.08 + va_live * 0.48 + k.comp * 0.06;
        let mut q3 = ge_clip(q2 * g3, head3.max(0.045));

        let f_q3 = 650.0 + k.comp * 8200.0 + k.stab * 900.0;
        let a_q3 = 1.0 - (-2.0 * pi * f_q3 / os_sr).exp();
        self.q3_lp += a_q3 * (q3 - self.q3_lp);
        q3 = self.q3_lp;

        self.q3 = q3;
        self.delay[self.delay_i] = q3;
        self.delay_i = (self.delay_i + 1) & DELAY_MASK;
        let a_fb = 1.0 - (-2.0 * pi * (30.0 + k.stab * 80.0) / os_sr).exp();
        self.fb_lp += a_fb * (q3 - self.fb_lp);

        let a_hpf = 1.0 - (-2.0 * pi * 22.0 / os_sr).exp();
        self.dc_out += a_hpf * (q3 - self.dc_out);
        (q3 - self.dc_out) * (0.02 + 1.48 * k.vol)
    }

    fn delay_at(&self, n: f32) -> f32 {
        let n = n.clamp(1.0, (DELAY - 2) as f32);
        let n0 = n.floor() as usize;
        let frac = n - n0 as f32;
        let a = self.delay[(self.delay_i + DELAY - n0) & DELAY_MASK];
        let b = self.delay[(self.delay_i + DELAY - (n0 + 1)) & DELAY_MASK];
        a + frac * (b - a)
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
    fn smooth_stays_on_for_the_whole_note() {
        let sr = 48_000.0f32;
        let mut ff = FuzzFactory::new(sr);
        ff.set_knobs(FuzzKnobs::smooth());
        let y = render(&mut ff, 12_000, sine_at(sr, 146.8, 0.4));
        let mid = rms(&y[3000..5000]);
        let late = rms(&y[10000..12000]);
        assert!(mid > 0.08, "mid {mid}");
        assert!(late > mid * 0.75, "sustain died mid={mid} late={late}");
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
    fn howl_starts_from_absolute_silence() {
        let mut ff = FuzzFactory::new(48_000.0);
        ff.set_knobs(FuzzKnobs::squeal());
        let y = render(&mut ff, 16_000, |_| 0.0);
        let r = rms(&y[y.len() - 4096..]);
        assert!(r > 0.05, "must self-start without a key tap rms={r}");
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
        assert!(r > 0.05 && peak > 0.10, "howl rms={r} peak={peak}");
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
        assert!(cv > cs * 1.05 || cv > 1.8, "velcro crest={cv} smooth={cs}");
    }

    #[test]
    fn grave_velcro_motorboats() {
        let sr = 48_000.0f32;
        let mut ff = FuzzFactory::new(sr);
        ff.set_knobs(FuzzKnobs {
            vol: 0.72,
            gate: 0.08,
            comp: 0.22,
            drive: 0.70,
            stab: 0.16,
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
        assert!(r > 0.035, "grave velcro should keep oscillating rms={r}");
        assert!(
            low > high * 0.25,
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
        let mut diff = 0.0f32;
        for i in 2048..y_lo.len() {
            let d = y_hi[i] - y_lo[i];
            diff += d * d;
        }
        let r_diff = (diff / (y_lo.len() - 2048) as f32).sqrt();
        assert!(r_diff > 0.04, "drive lo={r_lo} hi={} diff={r_diff}", rms(&y_hi[2048..]));

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

    fn zcr(xs: &[f32]) -> f32 {
        if xs.len() < 2 {
            return 0.0;
        }
        let mut n = 0.0;
        for i in 1..xs.len() {
            if xs[i] == 0.0 || xs[i - 1] == 0.0 {
                continue;
            }
            if xs[i].signum() != xs[i - 1].signum() {
                n += 1.0;
            }
        }
        n * 0.5 * 48_000.0 / xs.len() as f32
    }

    fn howl(comp: f32, stab: f32) -> Vec<f32> {
        let mut ff = FuzzFactory::new(48_000.0);
        ff.set_knobs(FuzzKnobs {
            vol: 0.7,
            gate: 0.05,
            drive: 0.8,
            comp,
            stab,
        });
        render(&mut ff, 14_000, |i| {
            if (80..200).contains(&i) {
                0.25
            } else {
                0.0
            }
        })
    }

    #[test]
    fn stab_low_comp_zero_is_silence() {
        let y = howl(0.0, 0.08);
        let r = rms(&y[y.len() - 4096..]);
        assert!(r < 0.03, "should be silent rms={r}");
    }

    #[test]
    fn comp_climbs_from_velcro_to_squeal() {
        let sr = 48_000.0f32;
        let low = howl(0.22, 0.12);
        let high = howl(0.82, 0.12);
        let lt = &low[low.len() - 6144..];
        let ht = &high[high.len() - 6144..];
        assert!(rms(lt) > 0.04, "low Comp should howl");
        assert!(rms(ht) > 0.04, "high Comp should howl");
        let bass_lo = band_goertzel(lt, sr, 80.0);
        let tre_hi = band_goertzel(ht, sr, 1800.0);
        let tre_lo = band_goertzel(lt, sr, 1800.0);
        let bass_hi = band_goertzel(ht, sr, 80.0);
        assert!(bass_lo > tre_lo, "low Comp should be grave bass={bass_lo} tre={tre_lo}");
        assert!(
            tre_hi > tre_lo * 2.0,
            "Comp up should add squeal tre_lo={tre_lo} tre_hi={tre_hi} bass_hi={bass_hi}"
        );
    }

    #[test]
    fn stab_raises_pitch_then_kills_the_howl() {
        let a = howl(0.70, 0.10);
        let b = howl(0.70, 0.45);
        let c = howl(0.70, 0.96);
        let fa = zcr(&a[a.len() - 6144..]);
        let fb = zcr(&b[b.len() - 6144..]);
        assert!(rms(&a[a.len() - 4096..]) > 0.04, "stab down should howl");
        assert!(fb > fa * 1.15, "Stab up should raise pitch a={fa} b={fb}");
        assert!(
            rms(&c[c.len() - 4096..]) < 0.04,
            "Stab CW should kill the squeal rms={}",
            rms(&c[c.len() - 4096..])
        );
    }

    #[test]
    fn comp_adds_treble() {
        let sr = 48_000.0f32;
        let input = sine_at(sr, 220.0, 0.35);
        let base = FuzzKnobs {
            vol: 0.7,
            gate: 0.0,
            drive: 0.65,
            stab: 0.97,
            comp: 0.08,
        };
        let mut dark = FuzzFactory::new(sr);
        dark.set_knobs(base);
        let yo = render(&mut dark, 8_000, &input);
        let mut bright = FuzzFactory::new(sr);
        bright.set_knobs(FuzzKnobs {
            comp: 0.92,
            ..base
        });
        let ys = render(&mut bright, 8_000, &input);
        let hi_o = band_goertzel(&yo[2048..], sr, 1760.0);
        let hi_s = band_goertzel(&ys[2048..], sr, 1760.0);
        assert!(hi_s > hi_o * 1.15, "Comp up should add treble dark={hi_o} bright={hi_s}");
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
            FuzzKnobs::plug(),
        ] {
            let mut ff = FuzzFactory::new(sr);
            ff.set_knobs(k);
            buffers.push(render(&mut ff, 8_000, &input));
        }
        let tails: Vec<f32> = buffers.iter().map(|y| rms(&y[4096..])).collect();
        let peak_spread = tails.iter().copied().fold(0.0f32, f32::max)
            - tails.iter().copied().fold(f32::MAX, f32::min);
        assert!(peak_spread > 0.03, "presets too similar rms={tails:?}");
    }
}
