#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

impl Waveform {
    pub const ALL: [Waveform; 4] = [
        Waveform::Sine,
        Waveform::Triangle,
        Waveform::Sawtooth,
        Waveform::Square,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Waveform::Sine => "Sine",
            Waveform::Triangle => "Tri",
            Waveform::Sawtooth => "Saw",
            Waveform::Square => "Square",
        }
    }
}

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

pub const VISIBLE_SEMITONES: i32 = 24;
pub const MIN_OCTAVE: i32 = 1;
pub const MAX_OCTAVE: i32 = 6;

/// Computer-key → semitone offset from the current octave's C.
pub fn midi_to_hz(midi: u8) -> f32 {
    440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0)
}

pub fn midi_to_name(midi: u8) -> String {
    let pc = (midi as i32).rem_euclid(12) as usize;
    let octave = (midi as i32) / 12 - 1;
    format!("{}{}", NOTE_NAMES[pc], octave)
}

pub fn is_black_key(midi: u8) -> bool {
    matches!((midi as i32).rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

pub fn octave_base_midi(octave: i32) -> u8 {
    ((octave + 1) * 12) as u8
}

pub fn visible_midis(octave: i32) -> Vec<u8> {
    let base = octave_base_midi(octave);
    (0..=VISIBLE_SEMITONES as u8).map(|i| base + i).collect()
}

pub fn white_key_index(midi: u8, base_midi: u8) -> usize {
    let mut index = 0;
    for m in base_midi..midi {
        if !is_black_key(m) {
            index += 1;
        }
    }
    index
}

pub fn count_white_keys(midis: &[u8]) -> usize {
    midis.iter().filter(|m| !is_black_key(**m)).count()
}

pub fn offset_key_label(offset: i32) -> Option<&'static str> {
    Some(match offset {
        0 => "Z",
        1 => "S",
        2 => "X",
        3 => "D",
        4 => "C",
        5 => "V",
        6 => "G",
        7 => "B",
        8 => "H",
        9 => "N",
        10 => "J",
        11 => "M",
        12 => "Q",
        13 => "2",
        14 => "W",
        15 => "3",
        16 => "E",
        17 => "R",
        18 => "5",
        19 => "T",
        20 => "6",
        21 => "Y",
        22 => "7",
        23 => "U",
        24 => "I",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_is_440() {
        let hz = midi_to_hz(69);
        assert!((hz - 440.0).abs() < 1e-4);
    }

    #[test]
    fn names() {
        assert_eq!(midi_to_name(60), "C4");
        assert_eq!(midi_to_name(69), "A4");
        assert!(is_black_key(61));
        assert!(!is_black_key(60));
    }

    #[test]
    fn octave_c3() {
        assert_eq!(octave_base_midi(3), 48);
        assert_eq!(midi_to_name(48), "C3");
    }
}
