use super::notes::octave_base_midi;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpPattern {
    Up,
    Down,
    UpDown,
    Random,
    Order,
}

impl ArpPattern {
    pub const ALL: [(ArpPattern, &'static str); 5] = [
        (ArpPattern::Up, "Up"),
        (ArpPattern::Down, "Down"),
        (ArpPattern::UpDown, "Up/Dn"),
        (ArpPattern::Random, "Rand"),
        (ArpPattern::Order, "Order"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpDivision {
    Quarter,
    Eighth,
    EighthT,
    Sixteenth,
    SixteenthT,
}

impl ArpDivision {
    pub const ALL: [(ArpDivision, &'static str); 5] = [
        (ArpDivision::Quarter, "1/4"),
        (ArpDivision::Eighth, "1/8"),
        (ArpDivision::EighthT, "1/8T"),
        (ArpDivision::Sixteenth, "1/16"),
        (ArpDivision::SixteenthT, "1/16T"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpPresetId {
    Live,
    Major,
    Minor,
    Maj7,
    Min7,
    Fifths,
    Octaves,
    Sus,
    Cascade,
}

pub struct ArpPreset {
    pub id: ArpPresetId,
    pub label: &'static str,
    pub intervals: &'static [i32],
}

pub const ARP_PRESETS: [ArpPreset; 8] = [
    ArpPreset {
        id: ArpPresetId::Major,
        label: "Major",
        intervals: &[0, 4, 7, 12],
    },
    ArpPreset {
        id: ArpPresetId::Minor,
        label: "Minor",
        intervals: &[0, 3, 7, 12],
    },
    ArpPreset {
        id: ArpPresetId::Maj7,
        label: "Maj7",
        intervals: &[0, 4, 7, 11],
    },
    ArpPreset {
        id: ArpPresetId::Min7,
        label: "Min7",
        intervals: &[0, 3, 7, 10],
    },
    ArpPreset {
        id: ArpPresetId::Fifths,
        label: "Fifths",
        intervals: &[0, 7, 12, 19],
    },
    ArpPreset {
        id: ArpPresetId::Octaves,
        label: "Octaves",
        intervals: &[0, 12, 24, 12],
    },
    ArpPreset {
        id: ArpPresetId::Sus,
        label: "Sus",
        intervals: &[0, 5, 7, 12],
    },
    ArpPreset {
        id: ArpPresetId::Cascade,
        label: "Cascade",
        intervals: &[0, 4, 7, 11, 14, 19],
    },
];

pub fn notes_from_preset(id: ArpPresetId, octave: i32) -> Vec<u8> {
    let Some(preset) = ARP_PRESETS.iter().find(|p| p.id == id) else {
        return Vec::new();
    };
    let root = octave_base_midi(octave) as i32;
    preset
        .intervals
        .iter()
        .map(|interval| (root + interval).clamp(0, 127) as u8)
        .collect()
}

pub fn step_seconds(bpm: f32, division: ArpDivision) -> f32 {
    let beat = 60.0 / bpm.max(40.0);
    match division {
        ArpDivision::Quarter => beat,
        ArpDivision::Eighth => beat / 2.0,
        ArpDivision::EighthT => beat / 3.0,
        ArpDivision::Sixteenth => beat / 4.0,
        ArpDivision::SixteenthT => beat / 6.0,
    }
}

fn expand_octaves(notes: &[u8], octaves: u8) -> Vec<u8> {
    let mut out = notes.to_vec();
    for octave in 1..octaves {
        for &note in notes {
            let shifted = note as i32 + 12 * octave as i32;
            if shifted <= 127 {
                out.push(shifted as u8);
            }
        }
    }
    out
}

pub fn build_arp_sequence(notes: &[u8], pattern: ArpPattern, octaves: u8) -> Vec<u8> {
    if notes.is_empty() {
        return Vec::new();
    }
    if pattern == ArpPattern::Order {
        return expand_octaves(notes, octaves);
    }
    let mut unique = notes.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let expanded = expand_octaves(&unique, octaves);
    match pattern {
        ArpPattern::Down => {
            let mut v = expanded;
            v.reverse();
            v
        }
        ArpPattern::UpDown => {
            if expanded.len() < 2 {
                return expanded;
            }
            let mut v = expanded.clone();
            v.extend(expanded[1..expanded.len() - 1].iter().rev().copied());
            v
        }
        _ => expanded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_from_c3() {
        let notes = notes_from_preset(ArpPresetId::Major, 3);
        assert_eq!(notes, vec![48, 52, 55, 60]);
    }

    #[test]
    fn updown_mirrors() {
        let seq = build_arp_sequence(&[48, 52, 55], ArpPattern::UpDown, 1);
        assert_eq!(seq, vec![48, 52, 55, 52]);
    }

    #[test]
    fn eighth_at_120() {
        let s = step_seconds(120.0, ArpDivision::Eighth);
        assert!((s - 0.25).abs() < 1e-6);
    }
}
