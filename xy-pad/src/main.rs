use std::io;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use evdev::{AbsoluteAxisType, Device, InputEventKind, Key};
use midir::{MidiOutput, MidiOutputConnection};

use xy_pad::{cc_msg, map_axis, score_pad, CC_X, CC_Y};

struct Shared {
    armed: AtomicBool,
    x: AtomicU8,
    y: AtomicU8,
    touching: AtomicBool,
    midi_ok: AtomicBool,
    pad_name: Mutex<String>,
    status: Mutex<String>,
}

impl Shared {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            x: AtomicU8::new(64),
            y: AtomicU8::new(64),
            touching: AtomicBool::new(false),
            midi_ok: AtomicBool::new(false),
            pad_name: Mutex::new(String::from("(nenhum)")),
            status: Mutex::new(String::from("abrindo…")),
        }
    }

    fn set_status(&self, s: impl Into<String>) {
        if let Ok(mut g) = self.status.lock() {
            *g = s.into();
        }
    }
}

fn open_midi() -> Result<MidiOutputConnection, String> {
    let out = MidiOutput::new("xy-pad").map_err(|e| e.to_string())?;
    out.create_virtual("XY Pad")
        .map_err(|e| format!("porta virtual MIDI: {e}"))
}

fn pick_touchpad() -> io::Result<(std::path::PathBuf, Device)> {
    let mut best: Option<(i32, std::path::PathBuf, Device)> = None;
    for (path, dev) in evdev::enumerate() {
        let Some(axes) = dev.supported_absolute_axes() else {
            continue;
        };
        if !axes.contains(AbsoluteAxisType::ABS_X) || !axes.contains(AbsoluteAxisType::ABS_Y) {
            continue;
        }
        let name = dev.name().unwrap_or("").to_string();
        let score = score_pad(&name);
        if score < 40 {
            continue;
        }
        match &best {
            Some((s, _, _)) if score <= *s => {}
            _ => best = Some((score, path, dev)),
        }
    }
    best.map(|(_, p, d)| (p, d))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "touchpad com ABS_X/Y não encontrado"))
}

fn axis_range(dev: &Device, axis: AbsoluteAxisType) -> (i32, i32) {
    match dev.get_abs_state() {
        Ok(st) => {
            let info = st[axis.0 as usize];
            if info.maximum > info.minimum {
                (info.minimum, info.maximum)
            } else {
                (0, 1)
            }
        }
        Err(_) => (0, 1),
    }
}

fn poll_in(fd: i32, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut pfd, 1, timeout_ms) > 0 }
}

fn pad_thread(shared: Arc<Shared>) {
    let (path, mut dev) = match pick_touchpad() {
        Ok(v) => v,
        Err(e) => {
            shared.set_status(format!("pad: {e}"));
            return;
        }
    };
    let name = dev.name().unwrap_or("touchpad").to_string();
    if let Ok(mut g) = shared.pad_name.lock() {
        *g = format!("{name} ({})", path.display());
    }

    let (xmin, xmax) = axis_range(&dev, AbsoluteAxisType::ABS_X);
    let (ymin, ymax) = axis_range(&dev, AbsoluteAxisType::ABS_Y);

    let mut midi = match open_midi() {
        Ok(c) => {
            shared.midi_ok.store(true, Ordering::Relaxed);
            Some(c)
        }
        Err(e) => {
            shared.set_status(format!("MIDI: {e}"));
            None
        }
    };

    let mut grabbed = false;
    let mut last_x = 255u8;
    let mut last_y = 255u8;
    let mut raw_x = (xmin + xmax) / 2;
    let mut raw_y = (ymin + ymax) / 2;

    loop {
        let want = shared.armed.load(Ordering::Relaxed);
        if want && !grabbed {
            match dev.grab() {
                Ok(()) => {
                    grabbed = true;
                    shared.set_status("pad armado (F8 desliga, Esc solta)");
                }
                Err(e) => shared.set_status(format!("grab falhou: {e}")),
            }
        } else if !want && grabbed {
            let _ = dev.ungrab();
            grabbed = false;
            shared.set_status("mouse de volta");
        }

        if poll_in(dev.as_raw_fd(), 8) {
        match dev.fetch_events() {
            Ok(events) => {
                let mut moved = false;
                for ev in events {
                    match ev.kind() {
                        InputEventKind::AbsAxis(AbsoluteAxisType::ABS_X)
                        | InputEventKind::AbsAxis(AbsoluteAxisType::ABS_MT_POSITION_X) => {
                            raw_x = ev.value();
                            moved = true;
                        }
                        InputEventKind::AbsAxis(AbsoluteAxisType::ABS_Y)
                        | InputEventKind::AbsAxis(AbsoluteAxisType::ABS_MT_POSITION_Y) => {
                            raw_y = ev.value();
                            moved = true;
                        }
                        InputEventKind::Key(Key::BTN_TOUCH) | InputEventKind::Key(Key::BTN_TOOL_FINGER)
                            => {
                            shared
                                .touching
                                .store(ev.value() != 0, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
                if moved {
                    let x = map_axis(raw_x, xmin, xmax, false);
                    let y = map_axis(raw_y, ymin, ymax, true);
                    shared.x.store(x, Ordering::Relaxed);
                    shared.y.store(y, Ordering::Relaxed);
                    if grabbed && (x != last_x || y != last_y) {
                        if let Some(conn) = midi.as_mut() {
                            let _ = conn.send(&cc_msg(CC_X, x));
                            let _ = conn.send(&cc_msg(CC_Y, y));
                        }
                        last_x = x;
                        last_y = y;
                    }
                }
            }
            Err(e) => {
                shared.set_status(format!("pad read: {e}"));
                thread::sleep(Duration::from_millis(50));
            }
        }
        }
    }
}

fn is_hotkey_keyboard(dev: &Device) -> bool {
    let name = dev.name().unwrap_or("").to_ascii_lowercase();
    if name.contains("ydotoold") || name.contains("virtual") {
        return false;
    }
    let Some(keys) = dev.supported_keys() else {
        return false;
    };
    keys.contains(Key::KEY_F8) && keys.contains(Key::KEY_ESC)
}

fn hotkey_thread(shared: Arc<Shared>) {
    let mut devices: Vec<Device> = evdev::enumerate()
        .filter_map(|(_, dev)| {
            if is_hotkey_keyboard(&dev) {
                Some(dev)
            } else {
                None
            }
        })
        .collect();
    if devices.is_empty() {
        shared.set_status("teclado evdev não encontrado (F8/Esc)");
        return;
    }
    loop {
        for d in &mut devices {
            if !poll_in(d.as_raw_fd(), 0) {
                continue;
            }
            match d.fetch_events() {
                Ok(events) => {
                    for ev in events {
                        if let InputEventKind::Key(key) = ev.kind() {
                            if ev.value() != 1 {
                                continue;
                            }
                            match key {
                                Key::KEY_F8 => {
                                    let next = !shared.armed.load(Ordering::Relaxed);
                                    shared.armed.store(next, Ordering::Relaxed);
                                }
                                Key::KEY_ESC => {
                                    shared.armed.store(false, Ordering::Relaxed);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Err(_) => {}
            }
        }
        thread::sleep(Duration::from_millis(8));
    }
}

struct Ui {
    shared: Arc<Shared>,
}

impl eframe::App for Ui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(33));
        let armed = self.shared.armed.load(Ordering::Relaxed);
        let x = self.shared.x.load(Ordering::Relaxed);
        let y = self.shared.y.load(Ordering::Relaxed);
        let touching = self.shared.touching.load(Ordering::Relaxed);
        let midi_ok = self.shared.midi_ok.load(Ordering::Relaxed);
        let pad = self
            .shared
            .pad_name
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let status = self
            .shared
            .status
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("XY Pad");
            ui.label("F8 liga/desliga · Esc solta sempre");
            ui.label("MIDI: porta virtual “XY Pad” · CC16 X · CC17 Y");
            ui.separator();
            ui.label(format!("pad: {pad}"));
            ui.label(format!(
                "MIDI: {}",
                if midi_ok { "ok" } else { "falhou" }
            ));
            ui.label(status);
            ui.separator();
            let state = if armed { "ARMADO" } else { "off (mouse)" };
            ui.colored_label(
                if armed {
                    egui::Color32::from_rgb(80, 220, 120)
                } else {
                    egui::Color32::GRAY
                },
                state,
            );
            if ui.button(if armed { "desarmar" } else { "armar" }).clicked() {
                self.shared.armed.store(!armed, Ordering::Relaxed);
            }
            ui.label(format!(
                "X {x:3}   Y {y:3}   {}",
                if touching { "dedo" } else { "" }
            ));
            let (resp, painter) =
                ui.allocate_painter(egui::vec2(260.0, 160.0), egui::Sense::hover());
            let rect = resp.rect;
            painter.rect_filled(rect, 4.0, egui::Color32::from_gray(24));
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                egui::epaint::StrokeKind::Inside,
            );
            let px = rect.left() + rect.width() * (x as f32 / 127.0);
            let py = rect.bottom() - rect.height() * (y as f32 / 127.0);
            painter.circle_filled(
                egui::pos2(px, py),
                7.0,
                if touching {
                    egui::Color32::from_rgb(90, 200, 255)
                } else {
                    egui::Color32::from_gray(140)
                },
            );
        });
    }
}

fn main() -> eframe::Result<()> {
    let shared = Arc::new(Shared::new());
    {
        let s = shared.clone();
        thread::Builder::new()
            .name("xy-pad".into())
            .spawn(move || pad_thread(s))
            .expect("pad thread");
    }
    {
        let s = shared.clone();
        thread::Builder::new()
            .name("xy-hotkey".into())
            .spawn(move || hotkey_thread(s))
            .expect("hotkey thread");
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([320.0, 420.0])
            .with_title("XY Pad"),
        ..Default::default()
    };
    eframe::run_native(
        "XY Pad",
        options,
        Box::new(|_| Ok(Box::new(Ui { shared }))),
    )
}
