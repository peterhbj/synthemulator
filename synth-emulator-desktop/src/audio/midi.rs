use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;
use midir::{Ignore, MidiInput, MidiInputConnection};

use crate::synth::Command;

use super::midi_out::{pick_gt100_out, score_gt100_out};

pub struct MidiHub {
    tx: Sender<Command>,
    pub ports: Vec<String>,
    pub connected: Option<String>,
    /// User picked "None" — don't auto-grab the Yamaha until they choose a port.
    pub hold_off: bool,
    activity_ms: Arc<AtomicU64>,
    _conn: Option<MidiInputConnection<MidiCallback>>,
}

struct MidiCallback {
    tx: Sender<Command>,
    parser: MidiParser,
    activity_ms: Arc<AtomicU64>,
}

impl MidiHub {
    pub fn new(tx: Sender<Command>) -> Self {
        let mut hub = Self {
            tx,
            ports: Vec::new(),
            connected: None,
            hold_off: false,
            activity_ms: Arc::new(AtomicU64::new(0)),
            _conn: None,
        };
        hub.refresh();
        let _ = hub.auto_connect();
        hub
    }

    pub fn live(&self) -> bool {
        let last = self.activity_ms.load(Ordering::Relaxed);
        if last == 0 {
            return false;
        }
        now_ms().saturating_sub(last) < 350
    }

    pub fn refresh(&mut self) {
        self.ports = scan_ports();
        if let Some(name) = &self.connected {
            if !self.ports.iter().any(|p| p == name) {
                self.disconnect_silent();
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.hold_off = true;
        self.disconnect_silent();
    }

    fn disconnect_silent(&mut self) {
        self._conn = None;
        self.connected = None;
        let _ = self.tx.try_send(Command::Panic);
    }

    pub fn connect(&mut self, name: &str) -> Result<(), String> {
        self.hold_off = false;
        self._conn = None;
        self.connected = None;
        let mut midi_in = MidiInput::new("helix").map_err(|e| format!("midi input: {e}"))?;
        midi_in.ignore(Ignore::None);
        let port = midi_in
            .ports()
            .into_iter()
            .find(|p| midi_in.port_name(p).ok().as_deref() == Some(name))
            .ok_or_else(|| format!("porta MIDI não encontrada: {name}"))?;
        let tx = self.tx.clone();
        let activity_ms = self.activity_ms.clone();
        let conn = midi_in
            .connect(
                &port,
                "helix-in",
                move |_stamp, message, cb: &mut MidiCallback| {
                    if cb.parser.feed(message, &cb.tx) {
                        cb.activity_ms.store(now_ms(), Ordering::Relaxed);
                    }
                },
                MidiCallback {
                    tx,
                    parser: MidiParser::new(),
                    activity_ms,
                },
            )
            .map_err(|e| format!("midi connect: {e}"))?;
        self.connected = Some(name.to_string());
        self._conn = Some(conn);
        Ok(())
    }

    pub fn auto_connect(&mut self) -> Result<(), String> {
        if self.hold_off || self.connected.is_some() {
            return Ok(());
        }
        self.ports = scan_ports();
        if let Some(name) = pick_hardware_port(&self.ports) {
            self.connect(&name)
        } else {
            Ok(())
        }
    }
}

/// Second MIDI input: GT-100 USB port (same pick as `Gt100Out`).
/// Yamaha stays on `MidiHub`. CC#80 (CTL1) → Whammy on/off.
pub struct Gt100In {
    tx: Sender<Command>,
    pub ports: Vec<String>,
    pub connected: Option<String>,
    /// User picked "None" — don't auto-grab until they choose a port.
    pub hold_off: bool,
    _conn: Option<MidiInputConnection<Gt100Callback>>,
}

struct Gt100Callback {
    tx: Sender<Command>,
    parser: MidiParser,
}

impl Gt100In {
    pub fn new(tx: Sender<Command>) -> Self {
        let mut hub = Self {
            tx,
            ports: Vec::new(),
            connected: None,
            hold_off: false,
            _conn: None,
        };
        hub.refresh();
        let _ = hub.auto_connect();
        hub
    }

    pub fn refresh(&mut self) {
        self.ports = scan_ports();
        if let Some(name) = &self.connected {
            if !self.ports.iter().any(|p| p == name) {
                self.disconnect_silent();
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.hold_off = true;
        self.disconnect_silent();
    }

    fn disconnect_silent(&mut self) {
        // Do not Panic: this port is only CTL, not the keyboard.
        self._conn = None;
        self.connected = None;
    }

    pub fn connect(&mut self, name: &str) -> Result<(), String> {
        self.hold_off = false;
        self._conn = None;
        self.connected = None;
        let mut midi_in =
            MidiInput::new("helix-gt100-in").map_err(|e| format!("midi input: {e}"))?;
        midi_in.ignore(Ignore::None);
        let port = midi_in
            .ports()
            .into_iter()
            .find(|p| midi_in.port_name(p).ok().as_deref() == Some(name))
            .ok_or_else(|| format!("porta MIDI não encontrada: {name}"))?;
        let tx = self.tx.clone();
        let conn = midi_in
            .connect(
                &port,
                "helix-gt100-ctrl",
                move |_stamp, message, cb: &mut Gt100Callback| {
                    cb.parser.feed(message, &cb.tx);
                },
                Gt100Callback {
                    tx,
                    parser: MidiParser::gt100_ctrl(),
                },
            )
            .map_err(|e| format!("midi connect: {e}"))?;
        self.connected = Some(name.to_string());
        self._conn = Some(conn);
        Ok(())
    }

    pub fn auto_connect(&mut self) -> Result<(), String> {
        if self.hold_off || self.connected.is_some() {
            return Ok(());
        }
        self.ports = scan_ports();
        if let Some(name) = pick_gt100_out(&self.ports) {
            self.connect(&name)
        } else {
            Ok(())
        }
    }
}

fn scan_ports() -> Vec<String> {
    let Ok(mut midi_in) = MidiInput::new("helix-scan") else {
        return Vec::new();
    };
    midi_in.ignore(Ignore::None);
    midi_in
        .ports()
        .iter()
        .filter_map(|p| midi_in.port_name(p).ok())
        .collect()
}

fn pick_hardware_port(ports: &[String]) -> Option<String> {
    let scored = |name: &str| -> i32 {
        let l = name.to_ascii_lowercase();
        if is_virtual_port(&l) {
            return -100;
        }
        // GT-100 USB is reserved for Gt100In (CTL) / Gt100Out (SysEx).
        if score_gt100_out(name) >= 40 {
            return -100;
        }
        let mut score = 10;
        if l.contains("yamaha") {
            score += 50;
        }
        if l.contains("digital keyboard") {
            score += 40;
        }
        if l.contains("keyboard") {
            score += 20;
        }
        if l.contains("usb") {
            score += 5;
        }
        score
    };
    ports
        .iter()
        .max_by_key(|p| scored(p))
        .filter(|p| scored(p) > 0)
        .cloned()
}

fn is_virtual_port(lower: &str) -> bool {
    lower.contains("through")
        || lower.contains("pipewire")
        || lower.contains("helix")
        || lower.contains("announce")
        || lower.contains("timer")
        || lower.contains("system:")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct MidiParser {
    status: u8,
    data: [u8; 2],
    got: usize,
    in_sysex: bool,
    decode: fn(u8, [u8; 2]) -> Option<Command>,
}

impl MidiParser {
    fn new() -> Self {
        Self {
            status: 0,
            data: [0; 2],
            got: 0,
            in_sysex: false,
            decode,
        }
    }

    fn gt100_ctrl() -> Self {
        Self {
            status: 0,
            data: [0; 2],
            got: 0,
            in_sysex: false,
            decode: decode_gt100_ctrl,
        }
    }

    /// Returns true if at least one playable command was emitted.
    fn feed(&mut self, bytes: &[u8], tx: &Sender<Command>) -> bool {
        let mut any = false;
        for &b in bytes {
            if self.push_byte(b, tx) {
                any = true;
            }
        }
        any
    }

    fn push_byte(&mut self, b: u8, tx: &Sender<Command>) -> bool {
        // Real-time messages can interrupt anything else.
        if b >= 0xF8 {
            return false;
        }
        if b == 0xF7 {
            self.in_sysex = false;
            return false;
        }
        if b == 0xF0 {
            self.in_sysex = true;
            self.got = 0;
            return false;
        }
        if self.in_sysex {
            return false;
        }

        if b & 0x80 != 0 {
            if (0xF1..=0xF6).contains(&b) {
                self.status = 0;
                self.got = 0;
                return false;
            }
            self.status = b;
            self.got = 0;
            if data_bytes(self.status) == 0 {
                return false;
            }
            return false;
        }

        // Data byte — running status if we already have a voice status.
        if self.status == 0 || data_bytes(self.status) == 0 {
            return false;
        }
        if self.got >= 2 {
            self.got = 0;
        }
        self.data[self.got] = b;
        self.got += 1;
        if self.got < data_bytes(self.status) {
            return false;
        }
        self.got = 0;
        if let Some(cmd) = (self.decode)(self.status, self.data) {
            let _ = tx.try_send(cmd);
            return true;
        }
        false
    }
}

fn data_bytes(status: u8) -> usize {
    match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 2,
        0xC0 | 0xD0 => 1,
        _ => 0,
    }
}

fn decode(status: u8, data: [u8; 2]) -> Option<Command> {
    match status & 0xF0 {
        0x90 => {
            let midi = data[0];
            let vel = data[1];
            if vel == 0 {
                Some(Command::NoteOff { midi })
            } else {
                Some(Command::NoteOn {
                    midi,
                    velocity: vel as f32 / 127.0,
                })
            }
        }
        0x80 => Some(Command::NoteOff { midi: data[0] }),
        0xB0 => match data[0] {
            64 => Some(Command::SetPedal(data[1] >= 64)),
            120 | 123 => Some(Command::Panic),
            // CC#80 (GT-100 CTL1) is decoded only on Gt100In.
            _ => None,
        },
        0xE0 => {
            let value = data[0] as u16 | ((data[1] as u16) << 7);
            let amount = (value as f32 - 8192.0) / 8192.0;
            Some(Command::SetBend {
                amount: amount.clamp(-1.0, 1.0),
                range: 0,
            })
        }
        _ => None,
    }
}

/// GT-100 CTL1 is typically assigned to CC#80. Momentary: hold ≥64 = on.
fn decode_gt100_ctrl(status: u8, data: [u8; 2]) -> Option<Command> {
    match status & 0xF0 {
        0xB0 if data[0] == 80 => Some(Command::SetWhammyOn(data[1] >= 64)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;

    fn collect_with(mut p: MidiParser, bytes: &[u8]) -> Vec<Command> {
        let (tx, rx) = bounded(32);
        p.feed(bytes, &tx);
        let mut out = Vec::new();
        while let Ok(c) = rx.try_recv() {
            out.push(c);
        }
        out
    }

    fn collect(bytes: &[u8]) -> Vec<Command> {
        collect_with(MidiParser::new(), bytes)
    }

    fn collect_gt100(bytes: &[u8]) -> Vec<Command> {
        collect_with(MidiParser::gt100_ctrl(), bytes)
    }

    #[test]
    fn note_on_off() {
        let cmds = collect(&[0x90, 60, 100, 0x80, 60, 0]);
        assert!(matches!(cmds[0], Command::NoteOn { midi: 60, .. }));
        assert!(matches!(cmds[1], Command::NoteOff { midi: 60 }));
    }

    #[test]
    fn yamaha_running_status() {
        // Two notes with one status byte — typical Yamaha USB.
        let cmds = collect(&[0x90, 60, 80, 64, 90]);
        assert_eq!(cmds.len(), 2);
        assert!(matches!(cmds[0], Command::NoteOn { midi: 60, .. }));
        assert!(matches!(cmds[1], Command::NoteOn { midi: 64, .. }));
    }

    #[test]
    fn note_on_vel_zero_is_off() {
        let cmds = collect(&[0x90, 60, 0]);
        assert!(matches!(cmds[0], Command::NoteOff { midi: 60 }));
    }

    #[test]
    fn sustain_and_bend() {
        let cmds = collect(&[0xB0, 64, 127, 0xE0, 0x00, 0x40]);
        assert!(matches!(cmds[0], Command::SetPedal(true)));
        assert!(matches!(cmds[1], Command::SetBend { range: 0, .. }));
    }

    #[test]
    fn skips_through() {
        assert!(pick_hardware_port(&[
            "Midi Through:Midi Through Port-0 14:0".into(),
            "Digital Keyboard:Digital Keyboard MIDI 1 20:0".into(),
        ])
        .unwrap()
        .contains("Digital Keyboard"));
    }

    #[test]
    fn yamaha_hub_skips_gt100() {
        assert!(pick_hardware_port(&[
            "GT-100:GT-100 MIDI 1 24:0".into(),
            "Digital Keyboard:Digital Keyboard MIDI 1 20:0".into(),
        ])
        .unwrap()
        .contains("Digital Keyboard"));
        assert!(pick_hardware_port(&["GT-100:GT-100 MIDI 1 24:0".into()]).is_none());
    }

    #[test]
    fn yamaha_cc64_is_pedal_cc80_ignored() {
        let cmds = collect(&[0xB0, 64, 127, 0xB0, 80, 127]);
        assert_eq!(cmds.len(), 1);
        assert!(matches!(cmds[0], Command::SetPedal(true)));
    }

    #[test]
    fn gt100_cc80_whammy_on() {
        let cmds = collect_gt100(&[0xB0, 80, 127]);
        assert!(matches!(cmds[0], Command::SetWhammyOn(true)));
    }

    #[test]
    fn gt100_cc80_whammy_off() {
        let cmds = collect_gt100(&[0xB0, 80, 0]);
        assert!(matches!(cmds[0], Command::SetWhammyOn(false)));
    }

    #[test]
    fn gt100_cc80_threshold_and_running_status() {
        let cmds = collect_gt100(&[0xB0, 80, 64, 80, 63]);
        assert!(matches!(cmds[0], Command::SetWhammyOn(true)));
        assert!(matches!(cmds[1], Command::SetWhammyOn(false)));
    }

    #[test]
    fn gt100_ignores_sustain_and_notes() {
        let cmds = collect_gt100(&[0xB0, 64, 127, 0x90, 60, 100]);
        assert!(cmds.is_empty());
    }
}
