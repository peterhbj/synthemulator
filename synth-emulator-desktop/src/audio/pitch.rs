use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// Polyphonic pitch shifter (STFT phase vocoder + peak lock).
///
/// Chords stay in interval: every partial moves by the same ratio. This replaces
/// the delay-line harmonizer, which tore polyphonic guitar into inharmonic junk.
pub struct PitchShift {
    fft_size: usize,
    hop: usize,
    osamp: f32,
    sr: f32,
    window: Vec<f32>,
    hist: Vec<f32>,
    hist_w: usize,
    hop_count: usize,
    out_accum: Vec<f32>,
    out_q: Vec<f32>,
    out_qi: usize,
    last_phase: Vec<f32>,
    sum_phase: Vec<f32>,
    ana_magn: Vec<f32>,
    ana_freq: Vec<f32>,
    syn_magn: Vec<f32>,
    syn_freq: Vec<f32>,
    fft_buf: Vec<Complex<f32>>,
    fft_fwd: Arc<dyn Fft<f32>>,
    fft_inv: Arc<dyn Fft<f32>>,
    ratio: f32,
}

impl PitchShift {
    pub fn new(sample_rate: f32) -> Self {
        // 4096 @ 48 kHz → 11.7 Hz/bin. Needed so −12 on C3 (~65 Hz) doesn't
        // collapse into the next chord tone.
        let fft_size = 4096;
        let osamp = 8usize;
        let hop = fft_size / osamp;
        let bins = fft_size / 2 + 1;
        let mut planner = FftPlanner::<f32>::new();
        let fft_fwd = planner.plan_fft_forward(fft_size);
        let fft_inv = planner.plan_fft_inverse(fft_size);
        let window: Vec<f32> = (0..fft_size)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / fft_size as f32).cos())
            .collect();
        Self {
            fft_size,
            hop,
            osamp: osamp as f32,
            sr: sample_rate,
            window,
            hist: vec![0.0; fft_size],
            hist_w: 0,
            hop_count: 0,
            out_accum: vec![0.0; fft_size * 2],
            out_q: vec![0.0; hop],
            out_qi: hop, // empty until first frame
            last_phase: vec![0.0; bins],
            sum_phase: vec![0.0; bins],
            ana_magn: vec![0.0; bins],
            ana_freq: vec![0.0; bins],
            syn_magn: vec![0.0; bins],
            syn_freq: vec![0.0; bins],
            fft_buf: vec![Complex::new(0.0, 0.0); fft_size],
            fft_fwd,
            fft_inv,
            ratio: 1.0,
        }
    }

    pub fn set_semitones(&mut self, semitones: f32) {
        self.ratio = 2f32.powf(semitones / 12.0).clamp(0.25, 4.0);
    }

    pub fn process(&mut self, input: f32) -> f32 {
        self.hist[self.hist_w] = input;
        self.hist_w = (self.hist_w + 1) % self.fft_size;
        self.hop_count += 1;
        if self.hop_count >= self.hop {
            self.hop_count = 0;
            self.process_frame();
            self.out_qi = 0;
        }
        if self.out_qi < self.hop {
            let y = self.out_q[self.out_qi];
            self.out_qi += 1;
            y.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }

    fn process_frame(&mut self) {
        let n = self.fft_size;
        let hop = self.hop;
        let bins = n / 2;
        let freq_per_bin = self.sr / n as f32;
        let expct = TAU * hop as f32 / n as f32;

        for i in 0..n {
            let idx = (self.hist_w + i) % n;
            self.fft_buf[i] = Complex::new(self.hist[idx] * self.window[i], 0.0);
        }
        self.fft_fwd.process(&mut self.fft_buf);

        for k in 0..=bins {
            let re = self.fft_buf[k].re;
            let im = self.fft_buf[k].im;
            let magn = (re * re + im * im).sqrt() * 2.0;
            let phase = im.atan2(re);
            let mut delta = phase - self.last_phase[k];
            self.last_phase[k] = phase;
            delta -= k as f32 * expct;
            delta = wrap_pi(delta);
            let true_freq = k as f32 * freq_per_bin + delta * freq_per_bin * self.osamp / TAU;
            self.ana_magn[k] = magn;
            self.ana_freq[k] = true_freq;
        }

        self.syn_magn.fill(0.0);
        self.syn_freq.fill(0.0);
        for k in 0..=bins {
            let dest = (k as f32 * self.ratio).round() as usize;
            if dest <= bins {
                let mag = self.ana_magn[k];
                if mag >= self.syn_magn[dest] {
                    self.syn_freq[dest] = self.ana_freq[k] * self.ratio;
                }
                self.syn_magn[dest] += mag;
            }
        }

        for k in 0..=bins {
            let magn = self.syn_magn[k];
            let mut tmp = self.syn_freq[k];
            tmp -= k as f32 * freq_per_bin;
            tmp /= freq_per_bin;
            tmp = TAU * tmp / self.osamp;
            tmp += k as f32 * expct;
            self.sum_phase[k] = wrap_pi(self.sum_phase[k] + tmp);
            let phase = self.sum_phase[k];
            self.fft_buf[k] = Complex::new(magn * phase.cos(), magn * phase.sin());
            if k > 0 && k < bins {
                self.fft_buf[n - k] = self.fft_buf[k].conj();
            }
        }
        self.fft_buf[0].im = 0.0;
        self.fft_buf[bins].im = 0.0;

        self.fft_inv.process(&mut self.fft_buf);
        let norm = 2.0 / (n as f32 * self.osamp);
        for i in 0..n {
            self.out_accum[i] += self.window[i] * self.fft_buf[i].re * norm;
        }
        self.out_q[..hop].copy_from_slice(&self.out_accum[..hop]);
        self.out_accum.copy_within(hop..hop + n, 0);
        for v in &mut self.out_accum[n - hop..n] {
            *v = 0.0;
        }
    }
}

fn wrap_pi(x: f32) -> f32 {
    ((x + PI).rem_euclid(TAU)) - PI
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(sr: f32, hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (TAU * hz * i as f32 / sr).sin() * 0.4)
            .collect()
    }

    fn peak_hz(buf: &[f32], sr: f32) -> Vec<f32> {
        top_peaks(buf, sr, 8).into_iter().map(|(f, _)| f).collect()
    }

    /// Strongest spectral peaks (Hz, power), Hann-windowed DFT.
    fn top_peaks(buf: &[f32], sr: f32, keep: usize) -> Vec<(f32, f32)> {
        let n = 8192.min(buf.len());
        let mut spec = vec![0.0; n / 2];
        for k in 1..n / 2 {
            let mut re = 0.0;
            let mut im = 0.0;
            for (i, &x) in buf.iter().take(n).enumerate() {
                let w = 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos();
                let a = TAU * k as f32 * i as f32 / n as f32;
                re += x * w * a.cos();
                im += x * w * a.sin();
            }
            spec[k] = re * re + im * im;
        }
        let mut peaks = Vec::new();
        for k in 2..n / 2 - 2 {
            if spec[k] > spec[k - 1] && spec[k] > spec[k + 1] {
                peaks.push((k as f32 * sr / n as f32, spec[k]));
            }
        }
        peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        peaks.truncate(keep);
        peaks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        peaks
    }

    fn nearest(peaks: &[(f32, f32)], target: f32) -> Option<f32> {
        peaks
            .iter()
            .min_by(|a, b| {
                (a.0 - target)
                    .abs()
                    .partial_cmp(&(b.0 - target).abs())
                    .unwrap()
            })
            .map(|p| p.0)
    }

    fn run_shift(mix: &[f32], sr: f32, semis: f32) -> Vec<f32> {
        let mut p = PitchShift::new(sr);
        p.set_semitones(semis);
        mix.iter().map(|&x| p.process(x)).collect()
    }

    #[test]
    fn unity_passes_signal() {
        let mut p = PitchShift::new(48_000.0);
        p.set_semitones(0.0);
        let mut y = 0.0;
        for _ in 0..2048 {
            y = p.process(0.5);
        }
        assert!(y.is_finite());
    }

    #[test]
    fn octave_up_keeps_chord_intervals() {
        let sr = 48_000.0;
        let mut p = PitchShift::new(sr);
        p.set_semitones(12.0);
        let a = tone(sr, 220.0, 12_000);
        let b = tone(sr, 330.0, 12_000);
        let mut out = Vec::with_capacity(a.len());
        for i in 0..a.len() {
            out.push(p.process(a[i] + b[i]));
        }
        // skip STFT latency
        let peaks = peak_hz(&out[8192..], sr);
        assert!(
            peaks.iter().any(|f| (f - 440.0).abs() < 25.0),
            "expected ~440 Hz, got {peaks:?}"
        );
        assert!(
            peaks.iter().any(|f| (f - 660.0).abs() < 30.0),
            "expected ~660 Hz, got {peaks:?}"
        );
    }

    #[test]
    fn triad_shifts_both_octaves() {
        let sr = 48_000.0;
        let n = 24_000;
        // C3–E3–G3 (guitar-ish closed triad)
        let freqs = [130.81f32, 164.81, 196.00];
        let mix: Vec<f32> = (0..n)
            .map(|i| {
                freqs
                    .iter()
                    .map(|hz| (TAU * hz * i as f32 / sr).sin() * 0.25)
                    .sum()
            })
            .collect();

        let up = run_shift(&mix, sr, 12.0);
        let down = run_shift(&mix, sr, -12.0);
        let skip = 8192;
        let up_p = top_peaks(&up[skip..], sr, 6);
        let down_p = top_peaks(&down[skip..], sr, 6);

        eprintln!(
            "+12 peaks (Hz): {:?}",
            up_p.iter().map(|p| p.0).collect::<Vec<_>>()
        );
        eprintln!(
            "-12 peaks (Hz): {:?}",
            down_p.iter().map(|p| p.0).collect::<Vec<_>>()
        );

        for hz in freqs {
            let hi = nearest(&up_p, hz * 2.0).expect("peak");
            assert!(
                (hi - hz * 2.0).abs() < 12.0,
                "+12: wanted {:.1}, nearest {hi:.1} in {up_p:?}",
                hz * 2.0
            );
            let lo = nearest(&down_p, hz / 2.0).expect("peak");
            assert!(
                (lo - hz / 2.0).abs() < 10.0,
                "-12: wanted {:.1}, nearest {lo:.1} in {down_p:?}",
                hz / 2.0
            );
        }
    }
}
