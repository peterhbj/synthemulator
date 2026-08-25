use super::notes::Waveform;

#[derive(Clone, Copy, Debug)]
pub struct SynthParams {
    pub waveform: Waveform,
    pub cutoff: f32,
    pub resonance: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub volume: f32,
}

impl Default for SynthParams {
    fn default() -> Self {
        Self {
            waveform: Waveform::Sawtooth,
            cutoff: 2200.0,
            resonance: 1.8,
            attack: 0.018,
            decay: 0.16,
            sustain: 0.62,
            release: 0.28,
            volume: 0.72,
        }
    }
}

pub const BEND_RANGES: [u8; 3] = [2, 7, 12];
pub const MAX_VOICES: usize = 12;
pub const VOICE_GAIN: f32 = 0.16;
pub const MIN_ENV: f32 = 0.004;
pub const ANALYSER_SIZE: usize = 2048;
