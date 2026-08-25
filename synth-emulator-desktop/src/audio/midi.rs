use crossbeam_channel::Sender;
use midir::{Ignore, MidiInput, MidiInputConnection};

use crate::synth::Command;

pub struct MidiHub {
    tx: Sender<Command>,
    pub ports: Vec<String>,
    pub connected: Option<String>,
    _conn: Option<MidiInputConnection<()>>,
}

impl MidiHub {
    pub fn new(tx: Sender<Command>) -> Self {
        let mut hub = Self {
            tx,
            ports: Vec::new(),
            connected: None,
            _conn: None,
        };
        hub.refresh();
        hub
    }

    pub fn refresh(&mut self) {
        self._conn = None;
        self.connected = None;
        self.ports.clear();
        let Ok(mut midi_in) = MidiInput::new("helix-scan") else {
            return;
        };
        midi_in.ignore(Ignore::None);
        for port in midi_in.ports() {
            if let Ok(name) = midi_in.port_name(&port) {
                self.ports.push(name);
            }
        }
    }

    pub fn disconnect(&mut self) {
        self._conn = None;
        self.connected = None;
    }

    pub fn connect(&mut self, name: &str) -> Result<(), String> {
        self._conn = None;
        self.connected = None;
        let mut midi_in = MidiInput::new("helix").map_err(|e| format!("midi input: {e}"))?;
        midi_in.ignore(Ignore::None);
        let port = midi_in
            .ports()
            .into_iter()
            .find(|p| midi_in.port_name(p).ok().as_deref() == Some(name))
            .ok_or_else(|| format!("port not found: {name}"))?;
        let tx = self.tx.clone();
        let conn = midi_in
            .connect(
                &port,
                "helix-in",
                move |_stamp, message, _| {
                    if let Some(cmd) = parse_midi(message) {
                        let _ = tx.try_send(cmd);
                    }
                },
                (),
            )
            .map_err(|e| format!("midi connect: {e}"))?;
        self.connected = Some(name.to_string());
        self._conn = Some(conn);
        Ok(())
    }
}

fn parse_midi(msg: &[u8]) -> Option<Command> {
    if msg.is_empty() {
        return None;
    }
    let status = msg[0] & 0xF0;
    match status {
        0x90 => {
            if msg.len() < 3 {
                return None;
            }
            let midi = msg[1];
            let vel = msg[2];
            if vel == 0 {
                Some(Command::NoteOff { midi })
            } else {
                Some(Command::NoteOn {
                    midi,
                    velocity: vel as f32 / 127.0,
                })
            }
        }
        0x80 => {
            if msg.len() < 2 {
                return None;
            }
            Some(Command::NoteOff { midi: msg[1] })
        }
        0xB0 => {
            if msg.len() < 3 {
                return None;
            }
            match msg[1] {
                64 => Some(Command::SetPedal(msg[2] >= 64)),
                120 | 123 => Some(Command::Panic),
                _ => None,
            }
        }
        0xE0 => {
            if msg.len() < 3 {
                return None;
            }
            let value = msg[1] as u16 | ((msg[2] as u16) << 7);
            let amount = (value as f32 - 8192.0) / 8192.0;
            Some(Command::SetBend {
                amount: amount.clamp(-1.0, 1.0),
                range: 0, // host should ignore range 0 = keep
            })
        }
        _ => None,
    }
}
