use std::collections::HashSet;
use std::f32::consts::PI;

use crate::synth::{
    build_arp_sequence, sixteenth, step_seconds, ArpDivision, ArpPattern, Command, SynthParams,
    ANALYSER_SIZE, MAX_VOICES, MIN_ENV, VOICE_GAIN, WHAMMY_SEQUENCE,
};

use super::osc;
use super::pitch::PitchShift;
use super::ring::AudioRing;
use std::sync::Arc;

const FILTER_TC: f32 = 0.03;
const RES_TC: f32 = 0.03;
const VOL_TC: f32 = 0.02;
const BEND_TC: f32 = 0.008;

#[derive(Clone, Copy)]
enum EnvStage {
    Attack,
    Decay,
    Sustain,
    Release,
}

struct Voice {
    midi: u8,
    phase: f32,
    gain: f32,
    mul: f32,
    samples_left: u32,
    stage: EnvStage,
    peak: f32,
    started_at: u64,
    stopping: bool,
}

struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
    cutoff: f32,
    q: f32,
    cutoff_target: f32,
    q_target: f32,
    cutoff_coeff: f32,
    q_coeff: f32,
}

impl Biquad {
    fn new(sr: f32, cutoff: f32, q: f32) -> Self {
        let mut f = Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
            cutoff,
            q,
            cutoff_target: cutoff,
            q_target: q,
            cutoff_coeff: (-1.0 / (FILTER_TC * sr)).exp(),
            q_coeff: (-1.0 / (RES_TC * sr)).exp(),
        };
        f.recompute(sr);
        f
    }

    fn set_cutoff(&mut self, hz: f32) {
        self.cutoff_target = hz.clamp(20.0, 20_000.0);
    }

    fn set_q(&mut self, q: f32) {
        self.q_target = q.clamp(0.05, 24.0);
    }

    fn tick_smooth(&mut self, sr: f32) {
        self.cutoff = self.cutoff_target + (self.cutoff - self.cutoff_target) * self.cutoff_coeff;
        self.q = self.q_target + (self.q - self.q_target) * self.q_coeff;
        self.recompute(sr);
    }

    fn recompute(&mut self, sr: f32) {
        let nyq = sr * 0.49;
        let f0 = self.cutoff.clamp(20.0, nyq);
        let q = self.q.max(0.05);
        let w0 = 2.0 * PI * (f0 / sr);
        let alpha = w0.sin() / (2.0 * q);
        let cosw = w0.cos();
        let b0 = (1.0 - cosw) * 0.5;
        let b1 = 1.0 - cosw;
        let b2 = (1.0 - cosw) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cosw;
        let a2 = 1.0 - alpha;
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

struct Compressor {
    envelope: f32,
    attack_c: f32,
    release_c: f32,
    threshold_db: f32,
    knee: f32,
    ratio: f32,
}

impl Compressor {
    fn new(sr: f32) -> Self {
        Self {
            envelope: 1e-6,
            attack_c: (-1.0 / (0.004 * sr)).exp(),
            release_c: (-1.0 / (0.12 * sr)).exp(),
            threshold_db: -14.0,
            knee: 10.0,
            ratio: 3.5,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let abs = x.abs().max(1e-8);
        if abs > self.envelope {
            self.envelope = abs + (self.envelope - abs) * self.attack_c;
        } else {
            self.envelope = abs + (self.envelope - abs) * self.release_c;
        }
        let level_db = 20.0 * self.envelope.log10();
        let gr = gain_reduction_db(level_db, self.threshold_db, self.knee, self.ratio);
        let gain = 10f32.powf(-gr / 20.0);
        x * gain
    }
}

fn gain_reduction_db(level_db: f32, thresh: f32, knee: f32, ratio: f32) -> f32 {
    let half = knee * 0.5;
    if level_db <= thresh - half {
        0.0
    } else if level_db >= thresh + half {
        let over = level_db - thresh;
        over - over / ratio
    } else {
        let x = level_db - thresh + half;
        (1.0 - 1.0 / ratio) * x * x / (2.0 * knee)
    }
}

struct Rng(u64);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) as u32
    }

    fn gen_index(&mut self, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            (self.next_u32() as usize) % len
        }
    }
}

pub struct Engine {
    sr: f32,
    params: SynthParams,
    voices: Vec<Voice>,
    filter: Biquad,
    compressor: Compressor,
    master: f32,
    master_target: f32,
    master_coeff: f32,
    bend_amount: f32,
    bend_amount_target: f32,
    bend_coeff: f32,
    bend_range: f32,
    octave_shift: f32,
    sample_count: u64,
    analyser: Vec<f32>,
    analyser_i: usize,
    held: HashSet<u8>,
    pedal: bool,
    arp_on: bool,
    arp_latch: bool,
    arp_pattern: ArpPattern,
    arp_rate: ArpDivision,
    arp_tempo: f32,
    arp_octaves: u8,
    arp_gate: f32,
    arp_pool: Vec<u8>,
    arp_samples_until_step: f64,
    arp_samples_until_gate: f64,
    arp_step_index: usize,
    arp_sounding: Option<u8>,
    whammy_on: bool,
    whammy_step: usize,
    whammy_applied: usize,
    whammy_samples_until: f64,
    rng: Rng,
    guitar_ring: Arc<AudioRing>,
    guitar_on: bool,
    guitar_gain: f32,
    guitar_shifter: PitchShift,
    guitar_level: f32,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_guitar(sample_rate, Arc::new(AudioRing::new(1024)))
    }

    pub fn with_guitar(sample_rate: f32, guitar_ring: Arc<AudioRing>) -> Self {
        let params = SynthParams::default();
        let master_coeff = (-1.0 / (VOL_TC * sample_rate)).exp();
        let bend_coeff = (-1.0 / (BEND_TC * sample_rate)).exp();
        Self {
            sr: sample_rate,
            filter: Biquad::new(sample_rate, params.cutoff, params.resonance),
            compressor: Compressor::new(sample_rate),
            master: params.volume,
            master_target: params.volume,
            master_coeff,
            bend_amount: 0.0,
            bend_amount_target: 0.0,
            bend_coeff,
            bend_range: 2.0,
            octave_shift: 0.0,
            sample_count: 0,
            analyser: vec![0.0; ANALYSER_SIZE],
            analyser_i: 0,
            voices: Vec::with_capacity(MAX_VOICES),
            held: HashSet::new(),
            pedal: false,
            arp_on: false,
            arp_latch: true,
            arp_pattern: ArpPattern::Up,
            arp_rate: ArpDivision::Eighth,
            arp_tempo: 125.0,
            arp_octaves: 1,
            arp_gate: 0.62,
            arp_pool: Vec::new(),
            arp_samples_until_step: 0.0,
            arp_samples_until_gate: 0.0,
            arp_step_index: 0,
            arp_sounding: None,
            whammy_on: false,
            whammy_step: 0,
            whammy_applied: 0,
            whammy_samples_until: 0.0,
            rng: Rng(0xC0FFEE_u64.wrapping_mul(sample_rate as u64 + 1)),
            guitar_ring,
            guitar_on: false,
            guitar_gain: 0.85,
            guitar_shifter: PitchShift::new(sample_rate),
            guitar_level: 0.0,
            params,
        }
    }

    pub fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::NoteOn { midi, velocity } => self.note_on_cmd(midi, velocity),
            Command::NoteOff { midi } => self.note_off_cmd(midi),
            Command::SetWaveform(w) => self.params.waveform = w,
            Command::SetCutoff(v) => {
                self.params.cutoff = v;
                self.filter.set_cutoff(v);
            }
            Command::SetResonance(v) => {
                self.params.resonance = v;
                self.filter.set_q(v);
            }
            Command::SetAttack(v) => self.params.attack = v,
            Command::SetDecay(v) => self.params.decay = v,
            Command::SetSustain(v) => self.params.sustain = v,
            Command::SetRelease(v) => self.params.release = v,
            Command::SetVolume(v) => {
                self.params.volume = v;
                self.master_target = v;
            }
            Command::SetBend { amount, range } => {
                self.bend_amount_target = amount.clamp(-1.0, 1.0);
                if range > 0 {
                    self.bend_range = range as f32;
                }
            }
            Command::SetPedal(on) => self.set_pedal(on),
            Command::Panic => self.panic(),
            Command::SetArpOn(on) => self.set_arp_on(on),
            Command::SetArpLatch(on) => {
                self.arp_latch = on;
                if !on {
                    self.arp_pool.retain(|n| self.held.contains(n));
                }
            }
            Command::SetArpPattern(p) => {
                self.arp_pattern = p;
                self.restart_arp();
            }
            Command::SetArpRate(r) => self.arp_rate = r,
            Command::SetArpTempo(t) => self.arp_tempo = t.clamp(70.0, 180.0),
            Command::SetArpOctaves(o) => {
                self.arp_octaves = o.clamp(1, 3);
                self.restart_arp();
            }
            Command::SetArpGate(g) => self.arp_gate = g.clamp(0.12, 0.95),
            Command::SetArpPool(pool) => self.arp_pool = pool,
            Command::RestartArp => self.restart_arp(),
            Command::ClearArp => self.clear_arp(),
            Command::SetWhammyOn(on) => self.set_whammy_on(on),
            Command::SetGuitarOn(on) => self.guitar_on = on,
            Command::SetGuitarGain(g) => self.guitar_gain = g.clamp(0.0, 1.5),
        }
    }

    fn note_on_cmd(&mut self, midi: u8, velocity: f32) {
        self.held.insert(midi);
        if self.arp_on {
            if !self.arp_pool.contains(&midi) {
                self.arp_pool.push(midi);
            }
            if self.arp_samples_until_step <= 0.0 {
                self.arp_samples_until_step = 1.0;
            }
            return;
        }
        self.voice_on(midi, velocity);
    }

    fn note_off_cmd(&mut self, midi: u8) {
        self.held.remove(&midi);
        if self.arp_on {
            if !self.arp_latch {
                self.arp_pool.retain(|n| *n != midi);
            }
            return;
        }
        if self.pedal {
            return;
        }
        self.voice_off(midi, None);
    }

    fn set_pedal(&mut self, on: bool) {
        if on {
            self.pedal = true;
            return;
        }
        if self.arp_on {
            self.pedal = false;
            return;
        }
        let releasing: Vec<u8> = self
            .voices
            .iter()
            .filter(|v| !v.stopping && !self.held.contains(&v.midi))
            .map(|v| v.midi)
            .collect();
        for midi in releasing {
            self.voice_off(midi, None);
        }
        self.pedal = false;
    }

    fn set_arp_on(&mut self, on: bool) {
        if on {
            if self.arp_pool.is_empty() {
                self.arp_pool = self.held.iter().copied().collect();
            }
            self.arp_on = true;
            self.restart_arp();
            return;
        }
        self.arp_on = false;
        self.silence_arp();
        self.all_notes_off(0.04);
    }

    fn clear_arp(&mut self) {
        self.arp_on = false;
        self.arp_pool.clear();
        self.silence_arp();
        self.all_notes_off(0.04);
    }

    fn restart_arp(&mut self) {
        self.arp_step_index = 0;
        self.arp_samples_until_step = 0.0;
    }

    fn silence_arp(&mut self) {
        if let Some(midi) = self.arp_sounding.take() {
            self.voice_off(midi, Some(0.02));
        }
        self.arp_samples_until_gate = 0.0;
        self.arp_samples_until_step = 0.0;
        self.arp_step_index = 0;
    }

    fn set_whammy_on(&mut self, on: bool) {
        if on {
            self.whammy_on = true;
            self.whammy_step = 0;
            self.whammy_applied = 0;
            self.whammy_samples_until = 0.0;
            self.octave_shift = WHAMMY_SEQUENCE[0] as f32;
            return;
        }
        self.whammy_on = false;
        self.whammy_step = 0;
        self.whammy_applied = 0;
        self.whammy_samples_until = 0.0;
        self.octave_shift = 0.0;
    }

    fn panic(&mut self) {
        self.silence_arp();
        self.all_notes_off(0.04);
        self.bend_amount = 0.0;
        self.bend_amount_target = 0.0;
        self.held.clear();
        self.pedal = false;
        if self.whammy_on {
            self.whammy_step = 0;
            self.whammy_applied = 0;
            self.whammy_samples_until = 0.0;
            self.octave_shift = WHAMMY_SEQUENCE[0] as f32;
        }
    }

    fn voice_on(&mut self, midi: u8, velocity: f32) {
        if let Some(idx) = self.voices.iter().position(|v| v.midi == midi) {
            self.start_release_at(idx, 0.012);
        }
        if self.live_voice_count() >= MAX_VOICES {
            self.steal_oldest();
        }
        let vel = velocity.clamp(0.0, 1.0);
        let peak = VOICE_GAIN * (0.2 + 0.8 * vel);
        let atk = (self.params.attack.max(MIN_ENV) * self.sr).max(1.0);
        let mut voice = Voice {
            midi,
            phase: 0.0,
            gain: 0.0001,
            mul: 1.0,
            samples_left: atk as u32,
            stage: EnvStage::Attack,
            peak,
            started_at: self.sample_count,
            stopping: false,
        };
        voice.mul = (peak / voice.gain).powf(1.0 / atk);
        self.voices.push(voice);
    }

    fn voice_off(&mut self, midi: u8, release_override: Option<f32>) {
        if let Some(idx) = self
            .voices
            .iter()
            .position(|v| v.midi == midi && !v.stopping)
        {
            let rel = release_override.unwrap_or(self.params.release);
            self.start_release_at(idx, rel);
        }
    }

    fn start_release_at(&mut self, idx: usize, release_time: f32) {
        let voice = &mut self.voices[idx];
        if voice.stopping {
            return;
        }
        voice.stopping = true;
        voice.stage = EnvStage::Release;
        let release = (release_time.max(MIN_ENV) * self.sr).max(1.0);
        voice.samples_left = release as u32;
        let current = voice.gain.max(0.0001);
        voice.gain = current;
        voice.mul = (0.0001 / current).powf(1.0 / release);
    }

    fn steal_oldest(&mut self) {
        let oldest = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.stopping)
            .min_by_key(|(_, v)| v.started_at)
            .map(|(i, _)| i);
        if let Some(idx) = oldest {
            self.start_release_at(idx, 0.02);
        }
    }

    fn live_voice_count(&self) -> usize {
        self.voices.iter().filter(|v| !v.stopping).count()
    }

    fn all_notes_off(&mut self, release: f32) {
        let n = self.voices.len();
        for i in 0..n {
            if !self.voices[i].stopping {
                self.start_release_at(i, release);
            }
        }
    }

    fn cents(&self) -> f32 {
        self.bend_amount * self.bend_range * 100.0 + self.octave_shift * 100.0
    }

    fn tick_arp(&mut self) {
        if !self.arp_on {
            return;
        }
        if self.arp_sounding.is_some() {
            self.arp_samples_until_gate -= 1.0;
            if self.arp_samples_until_gate <= 0.0 {
                if let Some(midi) = self.arp_sounding.take() {
                    self.voice_off(midi, None);
                }
            }
        }
        self.arp_samples_until_step -= 1.0;
        if self.arp_samples_until_step <= 0.0 {
            self.play_arp_step();
            let step = step_seconds(self.arp_tempo, self.arp_rate) as f64 * self.sr as f64;
            self.arp_samples_until_step += step;
            if self.arp_samples_until_step < 0.0 {
                self.arp_samples_until_step = step;
            }
        }
    }

    fn play_arp_step(&mut self) {
        let sequence = build_arp_sequence(&self.arp_pool, self.arp_pattern, self.arp_octaves);
        if sequence.is_empty() {
            if let Some(midi) = self.arp_sounding.take() {
                self.voice_off(midi, Some(0.02));
            }
            return;
        }
        let midi = if self.arp_pattern == ArpPattern::Random {
            let mut pick = sequence[self.rng.gen_index(sequence.len())];
            if sequence.len() > 1 && Some(pick) == self.arp_sounding {
                let idx = sequence.iter().position(|n| *n == pick).unwrap_or(0);
                pick = sequence[(idx + 1) % sequence.len()];
            }
            pick
        } else {
            let midi = sequence[self.arp_step_index % sequence.len()];
            self.arp_step_index += 1;
            midi
        };

        if let Some(prev) = self.arp_sounding {
            if prev != midi {
                self.voice_off(prev, Some(0.02));
            }
        }
        self.voice_on(midi, 1.0);
        self.arp_sounding = Some(midi);
        let step = step_seconds(self.arp_tempo, self.arp_rate) as f64 * self.sr as f64;
        let gate = (step * self.arp_gate as f64).max(0.02 * self.sr as f64);
        self.arp_samples_until_gate = gate;
    }

    fn tick_whammy(&mut self) {
        if !self.whammy_on {
            return;
        }
        self.whammy_samples_until -= 1.0;
        if self.whammy_samples_until <= 0.0 {
            self.whammy_applied = self.whammy_step;
            let semitones = WHAMMY_SEQUENCE[self.whammy_step];
            self.octave_shift = semitones as f32;
            self.whammy_step = (self.whammy_step + 1) % WHAMMY_SEQUENCE.len();
            let dur = sixteenth(self.arp_tempo) as f64 * self.sr as f64;
            self.whammy_samples_until += dur;
            if self.whammy_samples_until < 0.0 {
                self.whammy_samples_until = dur;
            }
        }
    }

    fn tick_voice(&mut self, idx: usize) -> f32 {
        let cents = self.cents();
        let waveform = self.params.waveform;
        let sr = self.sr;
        let voice = &mut self.voices[idx];
        let freq = crate::synth::midi_to_hz(voice.midi) * 2f32.powf(cents / 1200.0);
        let s = osc::sample(waveform, voice.phase) * voice.gain;
        voice.phase += freq / sr;
        voice.phase -= voice.phase.floor();
        voice.gain *= voice.mul;
        if voice.samples_left > 0 {
            voice.samples_left -= 1;
        }
        if voice.samples_left == 0 {
            match voice.stage {
                EnvStage::Attack => {
                    let dec = (self.params.decay.max(MIN_ENV) * sr).max(1.0);
                    let sus = (voice.peak * self.params.sustain).max(0.0001);
                    voice.stage = EnvStage::Decay;
                    voice.samples_left = dec as u32;
                    voice.gain = voice.gain.max(0.0001);
                    voice.mul = (sus / voice.gain).powf(1.0 / dec);
                }
                EnvStage::Decay => {
                    voice.stage = EnvStage::Sustain;
                    voice.mul = 1.0;
                    voice.gain = (voice.peak * self.params.sustain).max(0.0001);
                    voice.samples_left = u32::MAX;
                }
                EnvStage::Sustain => {}
                EnvStage::Release => {
                    voice.gain = 0.0;
                    voice.mul = 1.0;
                }
            }
        }
        s
    }

    pub fn render(&mut self) -> f32 {
        self.tick_arp();
        self.tick_whammy();
        self.filter.tick_smooth(self.sr);
        self.bend_amount = self.bend_amount_target
            + (self.bend_amount - self.bend_amount_target) * self.bend_coeff;
        self.master = self.master_target + (self.master - self.master_target) * self.master_coeff;

        let n = self.voices.len();
        let mut mix = 0.0;
        for i in 0..n {
            mix += self.tick_voice(i);
        }

        let g_in = self.guitar_ring.pop().unwrap_or(0.0);
        self.guitar_level = self.guitar_level * 0.995 + g_in.abs() * 0.005;
        if self.guitar_on {
            let shifted = if self.whammy_on {
                self.guitar_shifter.set_semitones(self.octave_shift);
                self.guitar_shifter.process(g_in)
            } else {
                g_in
            };
            mix += shifted * self.guitar_gain;
        }
        self.voices
            .retain(|v| !(matches!(v.stage, EnvStage::Release) && v.gain <= 0.00012));

        let filtered = self.filter.process(mix);
        let compressed = self.compressor.process(filtered);
        let out = (compressed * self.master).clamp(-1.0, 1.0);

        self.analyser[self.analyser_i] = out;
        self.analyser_i = (self.analyser_i + 1) % ANALYSER_SIZE;
        self.sample_count += 1;
        out
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut time_domain = vec![0.0; ANALYSER_SIZE];
        for i in 0..ANALYSER_SIZE {
            time_domain[i] = self.analyser[(self.analyser_i + i) % ANALYSER_SIZE];
        }
        let mut active: Vec<u8> = self
            .voices
            .iter()
            .filter(|v| !v.stopping)
            .map(|v| v.midi)
            .collect();
        active.sort_unstable();
        active.dedup();
        Snapshot {
            time_domain,
            active_notes: active,
            arp_pool: self.arp_pool.clone(),
            held_notes: self.held.iter().copied().collect(),
            pedal: self.pedal,
            arp_on: self.arp_on,
            whammy_on: self.whammy_on,
            whammy_step: self.whammy_applied,
            bend: self.bend_amount,
            sample_rate: self.sr,
            guitar_on: self.guitar_on,
            guitar_level: self.guitar_level,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub time_domain: Vec<f32>,
    pub active_notes: Vec<u8>,
    pub arp_pool: Vec<u8>,
    pub held_notes: Vec<u8>,
    pub pedal: bool,
    pub arp_on: bool,
    pub whammy_on: bool,
    pub whammy_step: usize,
    pub bend: f32,
    pub sample_rate: f32,
    pub guitar_on: bool,
    pub guitar_level: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_produces_energy() {
        let mut e = Engine::new(48_000.0);
        e.voice_on(69, 1.0);
        let mut sum = 0.0;
        for _ in 0..4800 {
            let s = e.render();
            sum += s * s;
        }
        assert!(sum > 0.001, "energy {sum}");
    }

    #[test]
    fn silence_without_notes() {
        let mut e = Engine::new(48_000.0);
        let mut sum = 0.0;
        for _ in 0..512 {
            sum += e.render().abs();
        }
        assert!(sum < 1e-4);
    }
}
