//! XY pad targets: which parameter each axis drives and how the 0..1 pad
//! position maps onto that parameter's own range.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum XyTarget {
    #[default]
    None,
    MasterFilter,
    Crossfader,
    EchoMix,
    EchoFeedback,
    EchoTime,
    FilterA,
    FilterB,
    LowA,
    LowB,
    HighA,
    HighB,
    SynthCutoff,
    SynthResonance,
    SynthBend,
}

/// Echo times the pad steps through, in beats.
pub const ECHO_BEATS: [f32; 7] = [0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0];

impl XyTarget {
    pub const ALL: [XyTarget; 15] = [
        XyTarget::None,
        XyTarget::MasterFilter,
        XyTarget::Crossfader,
        XyTarget::EchoMix,
        XyTarget::EchoFeedback,
        XyTarget::EchoTime,
        XyTarget::FilterA,
        XyTarget::FilterB,
        XyTarget::LowA,
        XyTarget::LowB,
        XyTarget::HighA,
        XyTarget::HighB,
        XyTarget::SynthCutoff,
        XyTarget::SynthResonance,
        XyTarget::SynthBend,
    ];

    pub fn label(self) -> &'static str {
        match self {
            XyTarget::None => "Nada",
            XyTarget::MasterFilter => "Filtro master",
            XyTarget::Crossfader => "Crossfader",
            XyTarget::EchoMix => "Echo: quantidade",
            XyTarget::EchoFeedback => "Echo: repetições",
            XyTarget::EchoTime => "Echo: tempo",
            XyTarget::FilterA => "Filtro deck A",
            XyTarget::FilterB => "Filtro deck B",
            XyTarget::LowA => "Graves deck A",
            XyTarget::LowB => "Graves deck B",
            XyTarget::HighA => "Agudos deck A",
            XyTarget::HighB => "Agudos deck B",
            XyTarget::SynthCutoff => "Synth: cutoff",
            XyTarget::SynthResonance => "Synth: resonance",
            XyTarget::SynthBend => "Synth: pitch bend",
        }
    }

    /// Pad position 0..=1 to the parameter's value.
    pub fn from_norm(self, v: f32) -> f32 {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.5 };
        match self {
            XyTarget::None => 0.0,
            XyTarget::MasterFilter
            | XyTarget::FilterA
            | XyTarget::FilterB
            | XyTarget::SynthBend => v * 2.0 - 1.0,
            XyTarget::Crossfader | XyTarget::EchoMix => v,
            XyTarget::EchoFeedback => v * 0.95,
            XyTarget::EchoTime => {
                let i = ((v * ECHO_BEATS.len() as f32) as usize).min(ECHO_BEATS.len() - 1);
                ECHO_BEATS[i]
            }
            XyTarget::LowA | XyTarget::LowB | XyTarget::HighA | XyTarget::HighB => {
                if v < 0.5 {
                    v * 2.0
                } else {
                    1.0 + (v - 0.5) * 2.0
                }
            }
            XyTarget::SynthCutoff => {
                let (a, b) = (80.0f32.ln(), 12_000.0f32.ln());
                (a + v * (b - a)).exp()
            }
            XyTarget::SynthResonance => 0.1 + v * (18.0 - 0.1),
        }
    }

    /// Parameter value back to a pad position (for placing the puck).
    pub fn to_norm(self, value: f32) -> f32 {
        if !value.is_finite() {
            return 0.5;
        }
        let n = match self {
            XyTarget::None => 0.5,
            XyTarget::MasterFilter
            | XyTarget::FilterA
            | XyTarget::FilterB
            | XyTarget::SynthBend => (value + 1.0) * 0.5,
            XyTarget::Crossfader | XyTarget::EchoMix => value,
            XyTarget::EchoFeedback => value / 0.95,
            XyTarget::EchoTime => {
                let i = ECHO_BEATS
                    .iter()
                    .position(|b| (*b - value).abs() < 1e-4)
                    .unwrap_or(3);
                (i as f32 + 0.5) / ECHO_BEATS.len() as f32
            }
            XyTarget::LowA | XyTarget::LowB | XyTarget::HighA | XyTarget::HighB => {
                if value < 1.0 {
                    value * 0.5
                } else {
                    0.5 + (value - 1.0) * 0.5
                }
            }
            XyTarget::SynthCutoff => {
                let (a, b) = (80.0f32.ln(), 12_000.0f32.ln());
                (value.max(1.0).ln() - a) / (b - a)
            }
            XyTarget::SynthResonance => (value - 0.1) / (18.0 - 0.1),
        };
        n.clamp(0.0, 1.0)
    }

    /// Short readout for the overlay.
    pub fn format(self, value: f32) -> String {
        match self {
            XyTarget::None => String::new(),
            XyTarget::SynthCutoff => {
                if value >= 1000.0 {
                    format!("{:.1} kHz", value / 1000.0)
                } else {
                    format!("{} Hz", value.round())
                }
            }
            XyTarget::EchoTime => {
                let frac = match value {
                    v if (v - 0.125).abs() < 1e-4 => "1/8",
                    v if (v - 0.25).abs() < 1e-4 => "1/4",
                    v if (v - 0.375).abs() < 1e-4 => "3/8",
                    v if (v - 0.5).abs() < 1e-4 => "1/2",
                    v if (v - 0.75).abs() < 1e-4 => "3/4",
                    v if (v - 1.0).abs() < 1e-4 => "1",
                    _ => "2",
                };
                format!("{frac} beat")
            }
            XyTarget::SynthResonance => format!("{value:.1}"),
            _ => format!("{value:+.2}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_pad() {
        for t in XyTarget::ALL {
            if matches!(t, XyTarget::None | XyTarget::EchoTime) {
                continue;
            }
            for i in 0..=20 {
                let v = i as f32 / 20.0;
                let back = t.to_norm(t.from_norm(v));
                assert!((back - v).abs() < 1e-3, "{t:?} {v} -> {back}");
            }
        }
    }

    #[test]
    fn centre_is_neutral() {
        assert_eq!(XyTarget::MasterFilter.from_norm(0.5), 0.0);
        assert_eq!(XyTarget::SynthBend.from_norm(0.5), 0.0);
        assert_eq!(XyTarget::LowA.from_norm(0.5), 1.0);
    }

    #[test]
    fn echo_time_steps() {
        assert_eq!(XyTarget::EchoTime.from_norm(0.0), 0.125);
        assert_eq!(XyTarget::EchoTime.from_norm(1.0), 2.0);
        let v = XyTarget::EchoTime.to_norm(0.75);
        assert_eq!(XyTarget::EchoTime.from_norm(v), 0.75);
    }

    #[test]
    fn junk_input_is_safe() {
        for t in XyTarget::ALL {
            assert!(t.from_norm(f32::NAN).is_finite());
            assert!(t.to_norm(f32::INFINITY).is_finite());
        }
    }
}
