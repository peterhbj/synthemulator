use crate::synth::Waveform;

#[inline]
pub fn sample(waveform: Waveform, phase: f32) -> f32 {
    match waveform {
        Waveform::Sine => (std::f32::consts::TAU * phase).sin(),
        Waveform::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        // Web Audio: ramp from -1 to 1, then reset.
        Waveform::Sawtooth => 2.0 * phase - 1.0,
        Waveform::Triangle => {
            if phase < 0.25 {
                4.0 * phase
            } else if phase < 0.75 {
                2.0 - 4.0 * phase
            } else {
                4.0 * phase - 4.0
            }
        }
    }
}
