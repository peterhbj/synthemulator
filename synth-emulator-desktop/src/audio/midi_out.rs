use midir::{MidiOutput, MidiOutputConnection};

/// Roland DT1 for GT-100 TEMPORARY PATCH FX1 PS1:PITCH.
///
/// Packet: `F0 41 <dev> 00 00 60 12  <addr 4>  <data>  <checksum>  F7`
/// (GT-100 Ver.2 / vguitarforums). Device id `10`. Model `00 00 60`.
///
/// Address `00 00 02 37` is the sourced offset for FX1 PS1:PITCH. The
/// MIDI Implementation PDF was not reachable from here; if pitch does
/// not stick live, try TEMPORARY PATCH base `20 00 00 00` added in,
/// i.e. `20 00 02 37`.
///
/// Data `00`..=`30` hex = -24..=+24 st. Bytes 12 / 24 / 36 (decimal)
/// map to -12 / 0 / +12.
pub const DEVICE_ID: u8 = 0x10;
pub const MODEL_ID: [u8; 3] = [0x00, 0x00, 0x60];
pub const DT1: u8 = 0x12;
pub const PS1_PITCH_ADDR: [u8; 4] = [0x00, 0x00, 0x02, 0x37];

pub fn pitch_data_byte(semitones: i32) -> u8 {
    (semitones + 24).clamp(0, 48) as u8
}

pub fn roland_checksum(addr: &[u8], data: &[u8]) -> u8 {
    let mut sum: u16 = 0;
    for &b in addr.iter().chain(data.iter()) {
        sum = sum.wrapping_add(b as u16);
    }
    ((0x80u16 - (sum & 0x7F)) & 0x7F) as u8
}

pub fn dt1_message(addr: [u8; 4], data: &[u8]) -> Vec<u8> {
    let cs = roland_checksum(&addr, data);
    let mut msg = Vec::with_capacity(8 + addr.len() + data.len() + 2);
    msg.extend_from_slice(&[0xF0, 0x41, DEVICE_ID]);
    msg.extend_from_slice(&MODEL_ID);
    msg.push(DT1);
    msg.extend_from_slice(&addr);
    msg.extend_from_slice(data);
    msg.push(cs);
    msg.push(0xF7);
    msg
}

pub fn pitch_sysex(semitones: i32) -> Vec<u8> {
    dt1_message(PS1_PITCH_ADDR, &[pitch_data_byte(semitones)])
}

pub struct Gt100Out {
    conn: Option<MidiOutputConnection>,
    pub ports: Vec<String>,
    pub connected: Option<String>,
    pub hold_off: bool,
    last_byte: Option<u8>,
}

impl Gt100Out {
    pub fn new() -> Self {
        let mut out = Self {
            conn: None,
            ports: Vec::new(),
            connected: None,
            hold_off: false,
            last_byte: None,
        };
        out.refresh();
        let _ = out.auto_connect();
        out
    }

    pub fn refresh(&mut self) {
        self.ports = scan_out_ports();
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
        self.conn = None;
        self.connected = None;
        self.last_byte = None;
    }

    pub fn connect(&mut self, name: &str) -> Result<(), String> {
        self.hold_off = false;
        self.conn = None;
        self.connected = None;
        self.last_byte = None;
        let midi_out = MidiOutput::new("helix-out").map_err(|e| format!("midi output: {e}"))?;
        let port = midi_out
            .ports()
            .into_iter()
            .find(|p| midi_out.port_name(p).ok().as_deref() == Some(name))
            .ok_or_else(|| format!("porta MIDI out nao encontrada: {name}"))?;
        let conn = midi_out
            .connect(&port, "helix-gt100")
            .map_err(|e| format!("midi out connect: {e}"))?;
        self.connected = Some(name.to_string());
        self.conn = Some(conn);
        Ok(())
    }

    pub fn auto_connect(&mut self) -> Result<(), String> {
        if self.hold_off || self.connected.is_some() {
            return Ok(());
        }
        self.ports = scan_out_ports();
        if let Some(name) = pick_gt100_out(&self.ports) {
            self.connect(&name)
        } else {
            Ok(())
        }
    }

    /// Send only when the data byte changes. Call from the UI thread.
    pub fn send_semitones(&mut self, semitones: i32) -> Result<(), String> {
        if self.connected.is_none() && !self.hold_off {
            self.auto_connect()?;
        }
        let byte = pitch_data_byte(semitones);
        if self.last_byte == Some(byte) {
            return Ok(());
        }
        let Some(conn) = self.conn.as_mut() else {
            return Ok(());
        };
        let msg = pitch_sysex(semitones);
        conn.send(&msg).map_err(|e| format!("midi send: {e}"))?;
        self.last_byte = Some(byte);
        Ok(())
    }
}

fn scan_out_ports() -> Vec<String> {
    let Ok(midi_out) = MidiOutput::new("helix-out-scan") else {
        return Vec::new();
    };
    midi_out
        .ports()
        .iter()
        .filter_map(|p| midi_out.port_name(p).ok())
        .collect()
}

pub fn pick_gt100_out(ports: &[String]) -> Option<String> {
    ports
        .iter()
        .max_by_key(|p| score_gt100_out(p))
        .filter(|p| score_gt100_out(p) >= 40)
        .cloned()
}

pub fn score_gt100_out(name: &str) -> i32 {
    let l = name.to_ascii_lowercase();
    if l.contains("through")
        || l.contains("pipewire")
        || l.contains("helix")
        || l.contains("announce")
        || l.contains("timer")
        || l.contains("system:")
    {
        return -100;
    }
    if l.contains("yamaha") || l.contains("digital keyboard") {
        return -50;
    }
    let mut s = 0;
    if l.contains("gt-100") || l.contains("gt100") {
        s += 80;
    }
    if l.contains("boss") {
        s += 50;
    }
    if l.contains("roland") {
        s += 30;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_bytes_offset_from_minus_24() {
        assert_eq!(pitch_data_byte(-12), 12);
        assert_eq!(pitch_data_byte(0), 24);
        assert_eq!(pitch_data_byte(12), 36);
        assert_eq!(pitch_data_byte(-24), 0);
        assert_eq!(pitch_data_byte(24), 48);
    }

    #[test]
    fn dt1_unison_packet() {
        let msg = pitch_sysex(0);
        assert_eq!(msg[0], 0xF0);
        assert_eq!(msg[1], 0x41);
        assert_eq!(msg[2], 0x10);
        assert_eq!(&msg[3..6], &[0x00, 0x00, 0x60]);
        assert_eq!(msg[6], 0x12);
        assert_eq!(&msg[7..11], &[0x00, 0x00, 0x02, 0x37]);
        assert_eq!(msg[11], 24);
        let cs = roland_checksum(&PS1_PITCH_ADDR, &[24]);
        assert_eq!(msg[12], cs);
        assert_eq!(msg[13], 0xF7);
        assert_eq!(msg.len(), 14);
        assert!(cs < 0x80);
    }

    #[test]
    fn dt1_octaves() {
        let down = pitch_sysex(-12);
        let up = pitch_sysex(12);
        assert_eq!(down[11], 12);
        assert_eq!(up[11], 36);
        assert_eq!(down[12], roland_checksum(&PS1_PITCH_ADDR, &[12]));
        assert_eq!(up[12], roland_checksum(&PS1_PITCH_ADDR, &[36]));
    }

    #[test]
    fn prefers_gt100_not_yamaha() {
        let name = pick_gt100_out(&[
            "Digital Keyboard:Digital Keyboard MIDI 1 20:0".into(),
            "GT-100:GT-100 MIDI 1 24:0".into(),
            "Midi Through:Midi Through Port-0 14:0".into(),
        ]);
        assert!(name.unwrap().contains("GT-100"));
    }

    #[test]
    fn skips_yamaha_alone() {
        assert!(pick_gt100_out(&["Yamaha Digital Keyboard".into()]).is_none());
    }
}
