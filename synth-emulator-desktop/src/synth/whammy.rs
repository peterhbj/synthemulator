/// Map of the Problematique octave sequence (16ths, no portamento).
pub const WHAMMY_SEQUENCE: [i32; 16] = [-12, 0, 12, 0, 12, 0, -12, 0, 12, 0, 12, 0, -12, 0, 12, 0];

pub const WHAMMY_BPM: f32 = 125.0;

pub fn sixteenth(bpm: f32) -> f32 {
    60.0 / bpm.max(40.0) / 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_length() {
        assert_eq!(WHAMMY_SEQUENCE.len(), 16);
    }
}
