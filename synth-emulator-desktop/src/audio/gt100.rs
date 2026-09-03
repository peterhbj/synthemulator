//! Boss GT-100 SysEx controller: Map of the Problematique octaves on FX1 P.SHIFT.
//!
//! Temporary patch only (`60 00 00 00`). Never writes USER memory.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::synth::Command;
use crossbeam_channel::{bounded, select, Receiver, Sender};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

const MANU: u8 = 0x41;
const MODEL: [u8; 3] = [0x00, 0x00, 0x60];
const DEV_BROADCAST: u8 = 0x7F;

const FX1_ON: [u8; 4] = [0x60, 0x00, 0x01, 0x40];
const FX1_TYPE: [u8; 4] = [0x60, 0x00, 0x01, 0x41];
const PS_VOICE: [u8; 4] = [0x60, 0x00, 0x02, 0x35];
const PS1_MODE: [u8; 4] = [0x60, 0x00, 0x02, 0x36];
const PS1_PITCH: [u8; 4] = [0x60, 0x00, 0x02, 0x37];
const PS1_FINE: [u8; 4] = [0x60, 0x00, 0x02, 0x38];
const PS1_PREDLY: [u8; 4] = [0x60, 0x00, 0x02, 0x39];
const PS1_LEVEL: [u8; 4] = [0x60, 0x00, 0x02, 0x3B];
const PS2_LEVEL: [u8; 4] = [0x60, 0x00, 0x02, 0x41];
const PS_DIRECT: [u8; 4] = [0x60, 0x00, 0x02, 0x43];
/// System USB primary dry mix (0–100 = 0–200%). Leaks unshifted notes into Helix.
const USB_PRI_DRY: [u8; 4] = [0x00, 0x00, 0x00, 0x53];

const TYPE_P_SHIFT: u8 = 0x0F;
const MODE_MEDIUM: u8 = 0x01;
const VOICE_1: u8 = 0x00;
/// FINE 0–100 maps −50…+50 cents; 50 is in tune.
const FINE_CENTER: u8 = 50;

const RQ1_TIMEOUT: Duration = Duration::from_millis(280);
const IDENTITY_TIMEOUT: Duration = Duration::from_millis(400);
const ENABLE_RETRY: Duration = Duration::from_millis(900);

/// Commands from the audio thread (never block the callback).
#[derive(Clone, Copy, Debug)]
pub enum GtCmd {
    Enable { semitones: i32 },
    Pitch(i32),
    Disable,
}

#[derive(Clone, Debug, Default)]
pub struct GtStatus {
    pub connected: Option<String>,
    pub live: bool,
    pub taken_over: bool,
    pub semitones: Option<i32>,
    pub err: Option<String>,
}

impl GtStatus {
    pub fn line(&self) -> String {
        if let Some(err) = &self.err {
            return err.clone();
        }
        if !self.live {
            return "GT-100 MIDI…".into();
        }
        if self.taken_over {
            let st = self.semitones.unwrap_or(0);
            let sign = if st > 0 { "+" } else { "" };
            return format!("GT-100 P.SHIFT {sign}{st}");
        }
        "GT-100 MIDI 1".into()
    }
}

enum UiCmd {
    Connect(String),
    Disconnect,
    AutoConnect,
    Refresh,
}

pub struct Gt100Hub {
    pub ports: Vec<String>,
    pub hold_off: bool,
    ui_tx: Sender<UiCmd>,
    status: Arc<Mutex<GtStatus>>,
}

impl Gt100Hub {
    pub fn new(engine_rx: Receiver<GtCmd>, cmd_tx: Sender<Command>) -> Self {
        let (ui_tx, ui_rx) = bounded::<UiCmd>(16);
        let status = Arc::new(Mutex::new(GtStatus::default()));
        let status_w = status.clone();
        let _ = thread::Builder::new()
            .name("helix-gt100".into())
            .spawn(move || worker_loop(engine_rx, ui_rx, status_w, cmd_tx));
        let mut hub = Self {
            ports: scan_out_ports(),
            hold_off: false,
            ui_tx,
            status,
        };
        let _ = hub.auto_connect();
        hub
    }

    pub fn status(&self) -> GtStatus {
        self.status.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn refresh(&mut self) {
        self.ports = scan_out_ports();
        let _ = self.ui_tx.try_send(UiCmd::Refresh);
    }

    pub fn auto_connect(&mut self) -> Result<(), String> {
        if self.hold_off {
            return Ok(());
        }
        self.ports = scan_out_ports();
        let _ = self.ui_tx.try_send(UiCmd::AutoConnect);
        Ok(())
    }

    pub fn connect(&mut self, name: &str) -> Result<(), String> {
        self.hold_off = false;
        self.ui_tx
            .try_send(UiCmd::Connect(name.to_string()))
            .map_err(|e| format!("gt100: {e}"))
    }

    pub fn disconnect(&mut self) {
        self.hold_off = true;
        let _ = self.ui_tx.try_send(UiCmd::Disconnect);
    }
}

fn worker_loop(
    engine_rx: Receiver<GtCmd>,
    ui_rx: Receiver<UiCmd>,
    status: Arc<Mutex<GtStatus>>,
    cmd_tx: Sender<Command>,
) {
    let mut ctx = Worker::new(status, cmd_tx);
    ctx.auto_connect();
    loop {
        select! {
            recv(engine_rx) -> msg => match msg {
                Ok(cmd) => ctx.handle_engine(cmd),
                Err(_) => {
                    ctx.disable();
                    return;
                }
            },
            recv(ui_rx) -> msg => match msg {
                Ok(UiCmd::Connect(name)) => ctx.connect(&name),
                Ok(UiCmd::Disconnect) => ctx.disconnect(),
                Ok(UiCmd::AutoConnect) => ctx.auto_connect(),
                Ok(UiCmd::Refresh) => ctx.refresh_ports(),
                Err(_) => {
                    ctx.disable();
                    return;
                }
            },
        }
    }
}

struct PatchSnap {
    fx1_on: u8,
    fx1_type: u8,
    voice: u8,
    mode: u8,
    pitch: u8,
    fine: u8,
    level: u8,
    ps2_level: u8,
    direct: u8,
    usb_dry: Option<u8>,
}

struct Worker {
    out: Option<MidiOutputConnection>,
    _in: Option<MidiInputConnection<GtInCb>>,
    out_name: Option<String>,
    sysex_rx: Receiver<Vec<u8>>,
    sysex_tx: Sender<Vec<u8>>,
    cmd_tx: Sender<Command>,
    snapshot: Option<PatchSnap>,
    taken: bool,
    last_enable: Option<Instant>,
    status: Arc<Mutex<GtStatus>>,
}

impl Worker {
    fn new(status: Arc<Mutex<GtStatus>>, cmd_tx: Sender<Command>) -> Self {
        let (sysex_tx, sysex_rx) = bounded(64);
        Self {
            out: None,
            _in: None,
            out_name: None,
            sysex_rx,
            sysex_tx,
            cmd_tx,
            snapshot: None,
            taken: false,
            last_enable: None,
            status,
        }
    }

    fn handle_engine(&mut self, cmd: GtCmd) {
        match cmd {
            GtCmd::Enable { semitones } => self.enable(semitones),
            GtCmd::Pitch(st) => self.pitch(st),
            GtCmd::Disable => self.disable(),
        }
    }

    fn refresh_ports(&mut self) {
        if self.out_name.is_some() {
            let ports = scan_out_ports();
            if let Some(name) = &self.out_name {
                if !ports.iter().any(|p| p == name) {
                    self.drop_io();
                    self.taken = false;
                    self.set_err(Some("GT-100 desconectada".into()));
                }
            }
        }
    }

    fn auto_connect(&mut self) {
        if self.out.is_some() {
            return;
        }
        if let Some(name) = pick_gt100_port(&scan_out_ports()) {
            self.connect(&name);
        } else {
            self.set_status(GtStatus {
                connected: None,
                live: false,
                taken_over: false,
                semitones: None,
                err: None,
            });
        }
    }

    fn connect(&mut self, name: &str) {
        self.drop_io();
        match open_out(name) {
            Ok(conn) => {
                self.out = Some(conn);
                self.out_name = Some(name.to_string());
            }
            Err(e) => {
                self.set_err(Some(e));
                return;
            }
        }
        self._in = open_in(name, self.sysex_tx.clone(), self.cmd_tx.clone()).ok();
        self.set_status(GtStatus {
            connected: Some(name.to_string()),
            live: true,
            taken_over: self.taken,
            semitones: None,
            err: None,
        });
    }

    fn disconnect(&mut self) {
        self.disable();
        self.drop_io();
        self.set_status(GtStatus::default());
    }

    fn drop_io(&mut self) {
        self.out = None;
        self._in = None;
        self.out_name = None;
        while self.sysex_rx.try_recv().is_ok() {}
    }

    fn enable(&mut self, semitones: i32) {
        self.last_enable = Some(Instant::now());
        if self.out.is_none() {
            self.auto_connect();
        }
        if self.out.is_none() {
            self.set_err(Some(
                "Plugue a GT-100 USB (MIDI 1) para o whammy da guitarra.".into(),
            ));
            return;
        }
        while self.sysex_rx.try_recv().is_ok() {}
        let _ = self.send(&identity_request());
        let _ = self.wait_identity();
        self.snapshot = self.read_snapshot();
        if let Err(e) = self.write_takeover(semitones) {
            self.set_err(Some(e));
            self.taken = false;
            return;
        }
        self.taken = true;
        self.set_status(GtStatus {
            connected: self.out_name.clone(),
            live: true,
            taken_over: true,
            semitones: Some(semitones),
            err: None,
        });
    }

    fn pitch(&mut self, semitones: i32) {
        if !self.taken {
            let retry = self
                .last_enable
                .map(|t| t.elapsed() >= ENABLE_RETRY)
                .unwrap_or(true);
            if retry {
                self.enable(semitones);
            }
            return;
        }
        if let Err(e) = self.send(&dt1(PS1_PITCH, &[pitch_value(semitones)])) {
            self.set_err(Some(e));
            return;
        }
        if let Ok(mut g) = self.status.lock() {
            g.semitones = Some(semitones);
            g.taken_over = true;
            g.live = true;
            g.err = None;
        }
    }

    fn disable(&mut self) {
        if !self.taken {
            return;
        }
        if let Some(snap) = self.snapshot.take() {
            let _ = self.write_restore(&snap);
        } else {
            let _ = self.send(&dt1(PS1_PITCH, &[pitch_value(0)]));
            let _ = self.send(&dt1(FX1_ON, &[0]));
        }
        self.taken = false;
        self.set_status(GtStatus {
            connected: self.out_name.clone(),
            live: self.out.is_some(),
            taken_over: false,
            semitones: None,
            err: None,
        });
    }

    fn read_snapshot(&mut self) -> Option<PatchSnap> {
        Some(PatchSnap {
            fx1_on: self.rq1(FX1_ON)?,
            fx1_type: self.rq1(FX1_TYPE)?,
            voice: self.rq1(PS_VOICE)?,
            mode: self.rq1(PS1_MODE)?,
            pitch: self.rq1(PS1_PITCH)?,
            fine: self.rq1(PS1_FINE).unwrap_or(FINE_CENTER),
            level: self.rq1(PS1_LEVEL)?,
            ps2_level: self.rq1(PS2_LEVEL).unwrap_or(0),
            direct: self.rq1(PS_DIRECT)?,
            usb_dry: self.rq1(USB_PRI_DRY),
        })
    }

    fn write_takeover(&mut self, semitones: i32) -> Result<(), String> {
        self.send(&dt1(FX1_TYPE, &[TYPE_P_SHIFT]))?;
        self.send(&dt1(PS_VOICE, &[VOICE_1]))?;
        // MEDIUM: FAST warbles chord tones; SLOW smears 16ths. Boss: slower = less modulation.
        self.send(&dt1(PS1_MODE, &[MODE_MEDIUM]))?;
        self.send(&dt1(PS1_FINE, &[FINE_CENTER]))?;
        self.send(&dt1(PS1_PREDLY, &[0, 0]))?;
        self.send(&dt1(PS1_LEVEL, &[100]))?;
        self.send(&dt1(PS_DIRECT, &[0]))?;
        self.send(&dt1(PS2_LEVEL, &[0]))?;
        self.send(&dt1(PS1_PITCH, &[pitch_value(semitones)]))?;
        self.send(&dt1(FX1_ON, &[1]))?;
        // Only mute USB dry if we snapshotted it (system param — must restore).
        if self.snapshot.as_ref().and_then(|s| s.usb_dry).is_some() {
            self.send(&dt1(USB_PRI_DRY, &[0]))?;
        }
        Ok(())
    }

    fn write_restore(&mut self, snap: &PatchSnap) -> Result<(), String> {
        self.send(&dt1(FX1_ON, &[0]))?;
        self.send(&dt1(FX1_TYPE, &[snap.fx1_type]))?;
        self.send(&dt1(PS_VOICE, &[snap.voice]))?;
        self.send(&dt1(PS1_MODE, &[snap.mode]))?;
        self.send(&dt1(PS1_FINE, &[snap.fine]))?;
        self.send(&dt1(PS1_PITCH, &[snap.pitch]))?;
        self.send(&dt1(PS1_LEVEL, &[snap.level]))?;
        self.send(&dt1(PS2_LEVEL, &[snap.ps2_level]))?;
        self.send(&dt1(PS_DIRECT, &[snap.direct]))?;
        self.send(&dt1(FX1_ON, &[snap.fx1_on]))?;
        if let Some(dry) = snap.usb_dry {
            self.send(&dt1(USB_PRI_DRY, &[dry]))?;
        }
        Ok(())
    }

    fn rq1(&mut self, addr: [u8; 4]) -> Option<u8> {
        self.send(&request(addr)).ok()?;
        self.wait_dt1(addr)
    }

    fn wait_dt1(&mut self, addr: [u8; 4]) -> Option<u8> {
        let deadline = Instant::now() + RQ1_TIMEOUT;
        while Instant::now() < deadline {
            let remain = deadline.saturating_duration_since(Instant::now());
            match self.sysex_rx.recv_timeout(remain) {
                Ok(msg) => {
                    if let Some(v) = parse_dt1_u8(&msg, &addr) {
                        return Some(v);
                    }
                }
                Err(_) => break,
            }
        }
        None
    }

    fn wait_identity(&mut self) -> bool {
        let deadline = Instant::now() + IDENTITY_TIMEOUT;
        while Instant::now() < deadline {
            let remain = deadline.saturating_duration_since(Instant::now());
            match self.sysex_rx.recv_timeout(remain) {
                Ok(msg) if is_gt100_identity(&msg) => return true,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        false
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        let out = self
            .out
            .as_mut()
            .ok_or_else(|| "GT-100 MIDI out fechada".to_string())?;
        out.send(bytes).map_err(|e| format!("midi out: {e}"))
    }

    fn set_err(&self, err: Option<String>) {
        if let Ok(mut g) = self.status.lock() {
            g.err = err;
            g.live = self.out.is_some();
            g.connected = self.out_name.clone();
        }
    }

    fn set_status(&self, s: GtStatus) {
        if let Ok(mut g) = self.status.lock() {
            *g = s;
        }
    }
}

pub fn checksum(data: &[u8]) -> u8 {
    let sum: u32 = data.iter().map(|&b| b as u32).sum();
    ((0x80 - (sum & 0x7F)) & 0x7F) as u8
}

pub fn pitch_value(semitones: i32) -> u8 {
    (semitones + 24).clamp(0, 48) as u8
}

pub fn identity_request() -> Vec<u8> {
    vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]
}

pub fn dt1(addr: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4 + data.len());
    payload.extend_from_slice(&addr);
    payload.extend_from_slice(data);
    let sum = checksum(&payload);
    let mut msg = Vec::with_capacity(10 + data.len());
    msg.extend_from_slice(&[0xF0, MANU, DEV_BROADCAST, MODEL[0], MODEL[1], MODEL[2], 0x12]);
    msg.extend_from_slice(&payload);
    msg.push(sum);
    msg.push(0xF7);
    msg
}

pub fn request(addr: [u8; 4]) -> Vec<u8> {
    let mut payload = [0u8; 8];
    payload[..4].copy_from_slice(&addr);
    payload[4..].copy_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    let sum = checksum(&payload);
    let mut msg = Vec::with_capacity(17);
    msg.extend_from_slice(&[0xF0, MANU, DEV_BROADCAST, MODEL[0], MODEL[1], MODEL[2], 0x11]);
    msg.extend_from_slice(&payload);
    msg.push(sum);
    msg.push(0xF7);
    msg
}

pub fn parse_dt1_u8(msg: &[u8], addr: &[u8; 4]) -> Option<u8> {
    // F0 41 dev 00 00 60 12 a b c d data sum F7
    if msg.len() < 14 || msg[0] != 0xF0 || msg[1] != MANU || msg[6] != 0x12 {
        return None;
    }
    if msg[3..6] != MODEL {
        return None;
    }
    if msg[7..11] != *addr {
        return None;
    }
    if *msg.last()? != 0xF7 {
        return None;
    }
    Some(msg[11])
}

pub fn is_gt100_identity(msg: &[u8]) -> bool {
    // F0 7E dev 06 02 41 60 02 …
    msg.len() >= 9
        && msg[0] == 0xF0
        && msg[1] == 0x7E
        && msg[3] == 0x06
        && msg[4] == 0x02
        && msg[5] == MANU
        && msg[6] == 0x60
        && msg[7] == 0x02
}

pub fn is_gt100_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.contains("gt-100") || l.contains("gt100")
}

pub fn pick_gt100_port(ports: &[String]) -> Option<String> {
    ports
        .iter()
        .max_by_key(|p| score_gt_port(p))
        .filter(|p| score_gt_port(p) >= 50)
        .cloned()
}

fn score_gt_port(name: &str) -> i32 {
    let l = name.to_ascii_lowercase();
    if l.contains("through") || l.contains("pipewire") || l.contains("announce") {
        return -100;
    }
    if !is_gt100_name(name) {
        return 0;
    }
    let mut s = 60;
    if l.contains("midi 1") || l.contains("midi1") {
        s += 30;
    }
    if l.contains("midi 2") || l.contains("midi2") {
        s -= 25;
    }
    s
}

fn scan_out_ports() -> Vec<String> {
    let Ok(midi_out) = MidiOutput::new("helix-gt-scan") else {
        return Vec::new();
    };
    midi_out
        .ports()
        .iter()
        .filter_map(|p| midi_out.port_name(p).ok())
        .collect()
}

fn open_out(name: &str) -> Result<MidiOutputConnection, String> {
    let midi_out = MidiOutput::new("helix-gt-out").map_err(|e| format!("midi out: {e}"))?;
    let port = midi_out
        .ports()
        .into_iter()
        .find(|p| midi_out.port_name(p).ok().as_deref() == Some(name))
        .ok_or_else(|| format!("porta MIDI out não encontrada: {name}"))?;
    midi_out
        .connect(&port, "helix-gt")
        .map_err(|e| format!("midi out connect: {e}"))
}

struct GtInCb {
    sysex_tx: Sender<Vec<u8>>,
    cmd_tx: Sender<Command>,
    running: u8,
}

fn open_in(
    name: &str,
    sysex_tx: Sender<Vec<u8>>,
    cmd_tx: Sender<Command>,
) -> Result<MidiInputConnection<GtInCb>, String> {
    let mut midi_in = MidiInput::new("helix-gt-in").map_err(|e| format!("midi in: {e}"))?;
    midi_in.ignore(Ignore::None);
    let port = midi_in
        .ports()
        .into_iter()
        .find(|p| midi_in.port_name(p).ok().as_deref() == Some(name))
        .ok_or_else(|| format!("porta MIDI in não encontrada: {name}"))?;
    midi_in
        .connect(
            &port,
            "helix-gt-in",
            move |_stamp, message, cb: &mut GtInCb| {
                ingest_gt_midi(message, cb);
            },
            GtInCb {
                sysex_tx,
                cmd_tx,
                running: 0,
            },
        )
        .map_err(|e| format!("midi in connect: {e}"))
}

fn ingest_gt_midi(message: &[u8], cb: &mut GtInCb) {
    if message.first() == Some(&0xF0) {
        let _ = cb.sysex_tx.try_send(message.to_vec());
        return;
    }
    if message.is_empty() {
        return;
    }
    if message[0] & 0x80 != 0 && message[0] < 0xF8 {
        cb.running = message[0];
    }
    let (status, data) = if message[0] & 0x80 != 0 {
        (message[0], &message[1..])
    } else {
        (cb.running, message)
    };
    if status & 0xF0 == 0xB0 && data.len() >= 2 {
        let _ = cb.cmd_tx.try_send(Command::MidiCc {
            cc: data[0],
            val: data[1],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_map() {
        assert_eq!(pitch_value(-12), 0x0C);
        assert_eq!(pitch_value(0), 0x18);
        assert_eq!(pitch_value(12), 0x24);
        assert_eq!(pitch_value(-24), 0);
        assert_eq!(pitch_value(24), 48);
    }

    #[test]
    fn checksum_matches_live_rq1() {
        // Captured against the GT-100: RQ1 FX1 ON.
        let msg = request(FX1_ON);
        assert_eq!(
            msg,
            vec![
                0xF0, 0x41, 0x7F, 0x00, 0x00, 0x60, 0x11, 0x60, 0x00, 0x01, 0x40, 0x00, 0x00,
                0x00, 0x01, 0x5E, 0xF7
            ]
        );
    }

    #[test]
    fn checksum_matches_live_pitch_rq1() {
        let msg = request(PS1_PITCH);
        assert_eq!(msg[msg.len() - 2], 0x66);
    }

    #[test]
    fn dt1_pitch_minus_12() {
        let msg = dt1(PS1_PITCH, &[pitch_value(-12)]);
        assert_eq!(msg[0], 0xF0);
        assert_eq!(msg[6], 0x12);
        assert_eq!(&msg[7..11], &PS1_PITCH);
        assert_eq!(msg[11], 0x0C);
        assert_eq!(*msg.last().unwrap(), 0xF7);
        let sum = checksum(&msg[7..12]);
        assert_eq!(msg[12], sum);
    }

    #[test]
    fn parse_live_dt1_on_off() {
        let msg = [
            0xF0, 0x41, 0x00, 0x00, 0x00, 0x60, 0x12, 0x60, 0x00, 0x01, 0x40, 0x00, 0x5F, 0xF7,
        ];
        assert_eq!(parse_dt1_u8(&msg, &FX1_ON), Some(0));
        let pitch = [
            0xF0, 0x41, 0x00, 0x00, 0x00, 0x60, 0x12, 0x60, 0x00, 0x02, 0x37, 0x18, 0x4F, 0xF7,
        ];
        assert_eq!(parse_dt1_u8(&pitch, &PS1_PITCH), Some(0x18));
    }

    #[test]
    fn identity_bytes_and_parse() {
        assert_eq!(identity_request(), vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]);
        let reply = [
            0xF0, 0x7E, 0x00, 0x06, 0x02, 0x41, 0x60, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xF7,
        ];
        assert!(is_gt100_identity(&reply));
        assert!(!is_gt100_identity(&[0xF0, 0x7E, 0x00, 0x06, 0x02, 0x41, 0x60, 0x03]));
    }

    #[test]
    fn prefers_midi_1() {
        let ports = [
            "Midi Through:Midi Through Port-0 14:0".into(),
            "GT-100:GT-100 MIDI 2 20:1".into(),
            "GT-100:GT-100 MIDI 1 20:0".into(),
        ];
        assert!(pick_gt100_port(&ports).unwrap().contains("MIDI 1"));
    }

    #[test]
    fn takeover_bytes_medium_fine_and_usb_dry() {
        let mode = dt1(PS1_MODE, &[MODE_MEDIUM]);
        assert_eq!(mode[11], 0x01);
        let fine = dt1(PS1_FINE, &[FINE_CENTER]);
        assert_eq!(fine[11], 50);
        let predly = dt1(PS1_PREDLY, &[0, 0]);
        assert_eq!(&predly[11..13], &[0, 0]);
        let dry = dt1(USB_PRI_DRY, &[0]);
        assert_eq!(&dry[7..11], &USB_PRI_DRY);
        assert_eq!(dry[11], 0);
        // Live-style RQ1 USB dry checksum: addr 00 00 00 53 + size 1.
        let req = request(USB_PRI_DRY);
        assert_eq!(req[req.len() - 2], 0x2C);
    }

    #[test]
    fn ctl_cc80_81_on_gt_midi() {
        let (syx, _) = bounded::<Vec<u8>>(8);
        let (cmd, rx) = bounded(8);
        let mut cb = GtInCb {
            sysex_tx: syx,
            cmd_tx: cmd,
            running: 0,
        };
        ingest_gt_midi(&[0xB0, 80, 127], &mut cb);
        ingest_gt_midi(&[81, 0], &mut cb);
        assert!(matches!(
            rx.try_recv().unwrap(),
            Command::MidiCc { cc: 80, val: 127 }
        ));
        assert!(matches!(
            rx.try_recv().unwrap(),
            Command::MidiCc { cc: 81, val: 0 }
        ));
    }
}
