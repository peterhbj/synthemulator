use super::{ArpDivision, ArpPattern, Waveform};

#[derive(Clone, Debug)]
pub enum Command {
    NoteOn { midi: u8, velocity: f32 },
    NoteOff { midi: u8 },
    SetWaveform(Waveform),
    SetCutoff(f32),
    SetResonance(f32),
    SetAttack(f32),
    SetDecay(f32),
    SetSustain(f32),
    SetRelease(f32),
    SetVolume(f32),
    SetBend { amount: f32, range: u8 },
    SetPedal(bool),
    Panic,
    SetArpOn(bool),
    SetArpLatch(bool),
    SetArpPattern(ArpPattern),
    SetArpRate(ArpDivision),
    SetArpTempo(f32),
    SetArpOctaves(u8),
    SetArpGate(f32),
    SetArpPool(Vec<u8>),
    RestartArp,
    ClearArp,
    SetWhammyOn(bool),
    SetGuitarOn(bool),
    SetGuitarGain(f32),
    SetFuzzOn(bool),
    SetFuzzVol(f32),
    SetFuzzGate(f32),
    SetFuzzComp(f32),
    SetFuzzDrive(f32),
    SetFuzzStab(f32),
    /// Raw CC from keyboard or GT-100. Engine maps footswitches (toggle on press).
    MidiCc { cc: u8, val: u8 },
    LearnFoot(Option<FootLearn>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FootLearn {
    Whammy,
    Fuzz,
}

impl Command {
    /// Continuous / follow CCs. Footswitches (80/81 by default) are handled in the engine.
    pub fn from_cc(cc: u8, val: u8) -> Option<Self> {
        let on = val >= 64;
        let unipolar = val as f32 / 127.0;
        match cc {
            16 => Some(Self::SetFuzzGate(unipolar)),
            17 => Some(Self::SetFuzzComp(unipolar)),
            18 => Some(Self::SetFuzzStab(unipolar)),
            19 => Some(Self::SetFuzzDrive(unipolar)),
            20 => Some(Self::SetFuzzVol(unipolar)),
            21 => Some(Self::SetFuzzOn(on)),
            64 => Some(Self::SetPedal(on)),
            120 | 123 => Some(Self::Panic),
            _ => None,
        }
    }
}
