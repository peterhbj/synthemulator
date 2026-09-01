use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Color32, Key, Margin};

use super::arp_panel::{self, ArpEvent, ArpUiState};
use super::keyboard;
use super::oscilloscope;
use super::pitch_wheel;
use super::theme::{self, BG, FG, MUTED, SUBTLE, SURFACE};
use super::widgets::{
    badge_pill, format_hz, format_ms, format_pct, icon_button, knob, volume_slider, waveform_select,
};
use crate::audio::{AudioHost, Gt100In, Gt100Out, GuitarInput, MidiHub};
use crate::synth::{
    midi_to_name, notes_from_preset, octave_base_midi, ArpDivision, ArpPattern, ArpPresetId,
    Command, SynthParams, MAX_OCTAVE, MIN_OCTAVE, VISIBLE_SEMITONES, WHAMMY_SEQUENCE,
};

pub struct HelixApp {
    audio: Option<AudioHost>,
    audio_err: Option<String>,
    midi: Option<MidiHub>,
    midi_err: Option<String>,
    guitar: Option<GuitarInput>,
    guitar_err: Option<String>,
    gt100: Option<Gt100Out>,
    gt100_err: Option<String>,
    gt100_in: Option<Gt100In>,
    gt100_in_err: Option<String>,
    params: SynthParams,
    octave: i32,
    bend: f32,
    bend_range: u8,
    arp: ArpUiState,
    pointer_held: HashSet<u8>,
    key_held: HashMap<Key, u8>,
    arrow_up: bool,
    arrow_down: bool,
    dragging_bend: bool,
    spring_from: Option<(f32, f64)>,
    peak: f32,
    was_dragging_bend: bool,
    midi_poll_at: f64,
    /// UI just sent SetWhammyOn; ignore stale snapshot until engine matches.
    whammy_cmd_pending: bool,
}

impl HelixApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::install_style(&cc.egui_ctx);
        cc.egui_ctx.set_zoom_factor(1.0);
        cc.egui_ctx.options_mut(|o| {
            o.zoom_with_keyboard = false;
        });

        let (audio, audio_err, midi, guitar, gt100, gt100_in) = match AudioHost::start() {
            Ok(host) => {
                let midi = MidiHub::new(host.sender());
                let guitar = GuitarInput::new(
                    host.guitar_ring.clone(),
                    host.guitar_peak.clone(),
                    host.sample_rate,
                );
                let gt100 = Gt100Out::new();
                let gt100_in = Gt100In::new(host.sender());
                (
                    Some(host),
                    None,
                    Some(midi),
                    Some(guitar),
                    Some(gt100),
                    Some(gt100_in),
                )
            }
            Err(e) => (None, Some(e), None, None, None, None),
        };

        Self {
            audio,
            audio_err,
            midi,
            midi_err: None,
            guitar,
            guitar_err: None,
            gt100,
            gt100_err: None,
            gt100_in,
            gt100_in_err: None,
            params: SynthParams::default(),
            octave: 3,
            bend: 0.0,
            bend_range: 2,
            arp: ArpUiState {
                arp_on: false,
                arp_latch: true,
                arp_pattern: ArpPattern::Up,
                arp_rate: ArpDivision::Eighth,
                arp_tempo: 125.0,
                arp_octaves: 1.0,
                arp_gate: 0.62,
                arp_preset: None,
                whammy_on: false,
                whammy_step: 0,
                guitar_on: false,
                guitar_gain: 0.85,
            },
            pointer_held: HashSet::new(),
            key_held: HashMap::new(),
            arrow_up: false,
            arrow_down: false,
            dragging_bend: false,
            spring_from: None,
            peak: 0.0,
            was_dragging_bend: false,
            midi_poll_at: 0.0,
            whammy_cmd_pending: false,
        }
    }

    fn send(&self, cmd: Command) {
        if let Some(audio) = &self.audio {
            audio.send(cmd);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let typing = ctx.wants_keyboard_input();
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } = event
                {
                    if modifiers.ctrl || modifiers.mac_cmd || modifiers.alt {
                        continue;
                    }
                    if *repeat {
                        continue;
                    }
                    if *key == Key::Escape {
                        self.send(Command::Panic);
                        self.pointer_held.clear();
                        self.key_held.clear();
                        self.bend = 0.0;
                        continue;
                    }
                    if typing {
                        continue;
                    }
                    if *key == Key::Space {
                        self.send(Command::SetPedal(*pressed));
                        continue;
                    }
                    if *pressed && *key == Key::OpenBracket {
                        self.shift_octave(-1);
                        continue;
                    }
                    if *pressed && *key == Key::CloseBracket {
                        self.shift_octave(1);
                        continue;
                    }
                    if *key == Key::ArrowUp || *key == Key::ArrowDown {
                        if *key == Key::ArrowUp {
                            self.arrow_up = *pressed;
                        } else {
                            self.arrow_down = *pressed;
                        }
                        self.spring_from = None;
                        let dir = match (self.arrow_up, self.arrow_down) {
                            (true, false) => 1.0,
                            (false, true) => -1.0,
                            _ => 0.0,
                        };
                        if dir == 0.0 && !*pressed {
                            self.spring_from = Some((self.bend, i.time));
                        } else {
                            self.bend = dir;
                            self.send(Command::SetBend {
                                amount: self.bend,
                                range: self.bend_range,
                            });
                        }
                        continue;
                    }
                    if let Some(offset) = computer_key_offset(*key) {
                        if *pressed {
                            if self.key_held.contains_key(key) {
                                continue;
                            }
                            let midi = octave_base_midi(self.octave).saturating_add(offset);
                            self.key_held.insert(*key, midi);
                            if self.arp.arp_on {
                                self.arp.arp_preset = Some(ArpPresetId::Live);
                            }
                            self.send(Command::NoteOn {
                                midi,
                                velocity: 1.0,
                            });
                        } else if let Some(midi) = self.key_held.remove(key) {
                            self.send(Command::NoteOff { midi });
                        }
                    }
                }
            }
        });

        if !self.dragging_bend && !self.arrow_up && !self.arrow_down {
            if let Some((from, t0)) = self.spring_from {
                let t = ((ctx.input(|i| i.time) - t0) / 0.18).clamp(0.0, 1.0) as f32;
                let eased = 1.0 - (1.0 - t).powi(3);
                self.bend = from * (1.0 - eased);
                self.send(Command::SetBend {
                    amount: self.bend,
                    range: self.bend_range,
                });
                if t >= 1.0 {
                    self.bend = 0.0;
                    self.spring_from = None;
                }
            }
        }
    }

    fn shift_octave(&mut self, delta: i32) {
        let next = (self.octave + delta).clamp(MIN_OCTAVE, MAX_OCTAVE);
        self.octave = next;
        if let Some(id) = self.arp.arp_preset {
            if id != ArpPresetId::Live {
                let pool = notes_from_preset(id, self.octave);
                self.send(Command::SetArpPool(pool));
                self.send(Command::RestartArp);
            }
        }
    }

    /// Keep the badge / GT-100 out poll in sync with the engine (UI or CTL).
    fn set_whammy_ui(&mut self, on: bool) {
        self.arp.whammy_on = on;
        if on {
            self.connect_gt100_out();
        } else if let Some(out) = self.gt100.as_mut() {
            let _ = out.send_semitones(0);
        }
    }

    fn connect_gt100_out(&mut self) {
        if let Some(out) = self.gt100.as_mut() {
            out.hold_off = false;
            match out.auto_connect() {
                Ok(()) => self.gt100_err = None,
                Err(e) => self.gt100_err = Some(e),
            }
            if out.connected.is_none() {
                self.gt100_err = Some(
                    "Plugue a GT-100 USB MIDI out (nao o Yamaha) e ligue Whammy.".into(),
                );
            }
        }
    }
}

impl eframe::App for HelixApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        self.handle_keys(ctx);

        let snap_whammy = self
            .audio
            .as_ref()
            .map(|a| a.snapshot().whammy_on)
            .unwrap_or(false);
        if snap_whammy == self.arp.whammy_on {
            self.whammy_cmd_pending = false;
        } else if !self.whammy_cmd_pending {
            // CTL (or any engine-side change) — badge + Gt100Out follow the engine.
            self.set_whammy_ui(snap_whammy);
        }

        let now = ctx.input(|i| i.time);
        if now - self.midi_poll_at > 0.8 {
            self.midi_poll_at = now;
            if let Some(midi) = self.midi.as_mut() {
                midi.refresh();
                if let Err(e) = midi.auto_connect() {
                    self.midi_err = Some(e);
                }
            }
            if let Some(ctrl) = self.gt100_in.as_mut() {
                ctrl.refresh();
                match ctrl.auto_connect() {
                    Ok(()) => self.gt100_in_err = None,
                    Err(e) => self.gt100_in_err = Some(e),
                }
            }
            if self.arp.guitar_on {
                if let Some(g) = self.guitar.as_mut() {
                    g.refresh();
                    if let Err(e) = g.auto_connect() {
                        self.guitar_err = Some(e);
                    }
                }
            }
            if self.arp.whammy_on {
                if let Some(out) = self.gt100.as_mut() {
                    out.refresh();
                    if let Err(e) = out.auto_connect() {
                        self.gt100_err = Some(e);
                    }
                }
            }
        }

        if self.was_dragging_bend && !self.dragging_bend {
            self.spring_from = Some((self.bend, ctx.input(|i| i.time)));
        }
        self.was_dragging_bend = self.dragging_bend;

        if !self.dragging_bend && self.spring_from.is_none() && !self.arrow_up && !self.arrow_down {
            if let Some(audio) = &self.audio {
                let snap = audio.snapshot();
                self.bend = snap.bend;
                self.arp.whammy_step = snap.whammy_step;
            }
        } else if let Some(audio) = &self.audio {
            self.arp.whammy_step = audio.snapshot().whammy_step;
        }

        let snap = self
            .audio
            .as_ref()
            .map(|a| a.snapshot())
            .unwrap_or_default();
        if !snap.time_domain.is_empty() {
            let mut sum = 0.0;
            for v in &snap.time_domain {
                sum += *v * *v;
            }
            let rms = (sum / snap.time_domain.len() as f32).sqrt();
            self.peak = rms.max(self.peak * 0.92);
        } else {
            self.peak *= 0.92;
        }
        let active: HashSet<u8> = snap.active_notes.iter().copied().collect();
        let pooled: HashSet<u8> = snap.arp_pool.iter().copied().collect();

        if snap.whammy_on {
            if let Some(out) = self.gt100.as_mut() {
                let i = snap.whammy_step.min(WHAMMY_SEQUENCE.len() - 1);
                if let Err(e) = out.send_semitones(WHAMMY_SEQUENCE[i]) {
                    self.gt100_err = Some(e);
                }
            }
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(20, 12)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.paint_ui(ui, &snap, &active, &pooled);
                    });
            });
    }
}

impl HelixApp {
    fn paint_ui(
        &mut self,
        ui: &mut egui::Ui,
        snap: &crate::audio::Snapshot,
        active: &HashSet<u8>,
        pooled: &HashSet<u8>,
    ) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("ANALOG KEYBOARD")
                        .size(10.5)
                        .color(SUBTLE)
                        .extra_letter_spacing(2.4),
                );
                ui.label(egui::RichText::new("Helix").heading().color(FG));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                midi_picker(ui, self);
            });
        });
        ui.add_space(10.0);

        if let Some(err) = &self.audio_err {
            ui.colored_label(
                egui::Color32::from_rgb(220, 80, 80),
                format!("Audio: {err}"),
            );
        }

        egui::Frame::new()
            .fill(SURFACE)
            .stroke(theme::panel_stroke())
            .corner_radius(theme::rounding_xl())
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

                let full_w = ui.available_width();
                let left_w = (full_w * 0.40).clamp(220.0, 460.0);
                let right_w = (full_w - left_w - 16.0).max(280.0);
                let top_h = 252.0;

                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left_w, top_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            oscilloscope::oscilloscope(
                                ui,
                                &snap.time_domain,
                                (self.peak * 2.2).min(1.0),
                            );
                        },
                    );
                    ui.add_space(12.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(right_w, top_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            if waveform_select(ui, &mut self.params.waveform) {
                                self.send(Command::SetWaveform(self.params.waveform));
                            }
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(theme::section_label("FILTER"));
                                    ui.horizontal(|ui| {
                                        if knob(
                                            ui,
                                            "cutoff",
                                            "Cutoff",
                                            &mut self.params.cutoff,
                                            80.0,
                                            12_000.0,
                                            0.0,
                                            true,
                                            format_hz,
                                        ) {
                                            self.send(Command::SetCutoff(self.params.cutoff));
                                        }
                                        if knob(
                                            ui,
                                            "reso",
                                            "Reso",
                                            &mut self.params.resonance,
                                            0.1,
                                            18.0,
                                            0.1,
                                            false,
                                            |v| format!("{v:.1}"),
                                        ) {
                                            self.send(Command::SetResonance(self.params.resonance));
                                        }
                                    });
                                });
                                ui.add_space(12.0);
                                ui.vertical(|ui| {
                                    ui.label(theme::section_label("ENVELOPE"));
                                    ui.horizontal_wrapped(|ui| {
                                        if knob(
                                            ui,
                                            "a",
                                            "A",
                                            &mut self.params.attack,
                                            0.005,
                                            2.0,
                                            0.0,
                                            true,
                                            format_ms,
                                        ) {
                                            self.send(Command::SetAttack(self.params.attack));
                                        }
                                        if knob(
                                            ui,
                                            "d",
                                            "D",
                                            &mut self.params.decay,
                                            0.01,
                                            2.0,
                                            0.0,
                                            true,
                                            format_ms,
                                        ) {
                                            self.send(Command::SetDecay(self.params.decay));
                                        }
                                        if knob(
                                            ui,
                                            "s",
                                            "S",
                                            &mut self.params.sustain,
                                            0.0,
                                            1.0,
                                            0.01,
                                            false,
                                            format_pct,
                                        ) {
                                            self.send(Command::SetSustain(self.params.sustain));
                                        }
                                        if knob(
                                            ui,
                                            "r",
                                            "R",
                                            &mut self.params.release,
                                            0.02,
                                            3.0,
                                            0.0,
                                            true,
                                            format_ms,
                                        ) {
                                            self.send(Command::SetRelease(self.params.release));
                                        }
                                    });
                                });
                            });
                        },
                    );
                });

                ui.add_space(10.0);
                let events = arp_panel::arp_panel(ui, &mut self.arp);
                for ev in events {
                    self.handle_arp_event(ev);
                }
                guitar_in_picker(ui, self);
                if let Some(err) = &self.guitar_err {
                    ui.colored_label(Color32::from_rgb(220, 80, 80), err);
                }
                gt100_out_picker(ui, self);
                if let Some(err) = &self.gt100_err {
                    ui.colored_label(Color32::from_rgb(220, 80, 80), err);
                }
                if let Some(err) = &self.gt100_in_err {
                    ui.colored_label(Color32::from_rgb(220, 80, 80), err);
                }

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if icon_button(ui, "−", self.octave > MIN_OCTAVE).clicked() {
                        self.shift_octave(-1);
                    }
                    ui.vertical(|ui| {
                        ui.label(theme::section_label("OCTAVE"));
                        let low = midi_to_name(octave_base_midi(self.octave));
                        let high =
                            midi_to_name(octave_base_midi(self.octave) + VISIBLE_SEMITONES as u8);
                        ui.label(
                            egui::RichText::new(format!("{low}–{high}"))
                                .monospace()
                                .color(FG),
                        );
                    });
                    if icon_button(ui, "+", self.octave < MAX_OCTAVE).clicked() {
                        self.shift_octave(1);
                    }
                    badge_pill(ui, "Sustain", snap.pedal);
                    badge_pill(ui, "Arp", self.arp.arp_on);
                    badge_pill(ui, "Whammy", self.arp.whammy_on);
                    badge_pill(ui, "Guitar", self.arp.guitar_on);
                    ui.allocate_ui(
                        egui::vec2(ui.available_width().clamp(160.0, 280.0), 28.0),
                        |ui| {
                            if volume_slider(ui, &mut self.params.volume) {
                                self.send(Command::SetVolume(self.params.volume));
                            }
                        },
                    );
                });

                ui.add_space(6.0);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    let mut bend = self.bend;
                    let mut range = self.bend_range;
                    if pitch_wheel::pitch_wheel(ui, &mut bend, &mut range, &mut self.dragging_bend)
                    {
                        self.bend = bend;
                        self.bend_range = range;
                        self.spring_from = None;
                        self.send(Command::SetBend {
                            amount: self.bend,
                            range: self.bend_range,
                        });
                    } else {
                        self.bend = bend;
                    }
                    ui.add_space(8.0);
                    let kb_w = ui.available_width();
                    ui.allocate_ui(egui::vec2(kb_w, 176.0), |ui| {
                        let events = keyboard::piano(
                            ui,
                            self.octave,
                            active,
                            pooled,
                            &mut self.pointer_held,
                        );
                        for (midi, on) in events {
                            if on {
                                if self.arp.arp_on {
                                    self.arp.arp_preset = Some(ArpPresetId::Live);
                                }
                                self.send(Command::NoteOn {
                                    midi,
                                    velocity: 1.0,
                                });
                            } else {
                                self.send(Command::NoteOff { midi });
                            }
                        }
                    });
                });
            });

        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(
                "Whammy drives the GT-100 Pitch Shifter over SysEx DT1 (octaves -12 / 0 / +12 on 16ths). Helix passes USB guitar dry. Pair it with a loop and twist the filter. Latch holds your chord. Drag the bend wheel or hold up/down arrows. Z and Q rows play notes, Space sustains, [ ] shifts octave, Esc silences all. MIDI USB in: notes, sustain, pitch bend. CTL1 / CC#80 on the GT-100 USB MIDI port toggles Whammy.",
            )
            .size(12.0)
            .color(MUTED),
        );
        if let Some(err) = &self.midi_err {
            ui.colored_label(Color32::from_rgb(220, 80, 80), err);
        }
    }
}

impl HelixApp {
    fn handle_arp_event(&mut self, ev: ArpEvent) {
        match ev {
            ArpEvent::ToggleArp => {
                self.arp.arp_on = !self.arp.arp_on;
                self.send(Command::SetArpOn(self.arp.arp_on));
            }
            ArpEvent::ToggleLatch => {
                self.arp.arp_latch = !self.arp.arp_latch;
                self.send(Command::SetArpLatch(self.arp.arp_latch));
            }
            ArpEvent::Clear => {
                self.arp.arp_on = false;
                self.arp.arp_preset = None;
                self.send(Command::ClearArp);
            }
            ArpEvent::ToggleWhammy => {
                let on = !self.arp.whammy_on;
                self.send(Command::SetWhammyOn(on));
                self.set_whammy_ui(on);
                self.whammy_cmd_pending = true;
            }
            ArpEvent::ToggleGuitar => {
                self.arp.guitar_on = !self.arp.guitar_on;
                self.send(Command::SetGuitarOn(self.arp.guitar_on));
                if self.arp.guitar_on {
                    if let Some(g) = self.guitar.as_mut() {
                        g.hold_off = false;
                        match g.auto_connect() {
                            Ok(()) => self.guitar_err = None,
                            Err(e) => self.guitar_err = Some(e),
                        }
                        if g.connected.is_none() {
                            self.guitar_err = Some(
                                "Plugue a GT-100 via USB e ligue Guitar On (entrada, não MIDI)."
                                    .into(),
                            );
                        }
                    }
                }
            }
            ArpEvent::GuitarGain => {
                self.send(Command::SetGuitarGain(self.arp.guitar_gain));
            }
            ArpEvent::Pattern(p) => {
                self.arp.arp_pattern = p;
                self.send(Command::SetArpPattern(p));
            }
            ArpEvent::Rate(r) => {
                self.arp.arp_rate = r;
                self.send(Command::SetArpRate(r));
            }
            ArpEvent::Tempo => self.send(Command::SetArpTempo(self.arp.arp_tempo)),
            ArpEvent::Octaves => {
                self.send(Command::SetArpOctaves(self.arp.arp_octaves.round() as u8));
            }
            ArpEvent::Gate => self.send(Command::SetArpGate(self.arp.arp_gate)),
            ArpEvent::Preset(id) => {
                self.arp.arp_preset = Some(id);
                self.arp.arp_on = true;
                self.arp.arp_latch = true;
                let pool = notes_from_preset(id, self.octave);
                self.send(Command::SetArpLatch(true));
                self.send(Command::SetArpPool(pool));
                self.send(Command::SetArpOn(true));
                self.send(Command::RestartArp);
            }
        }
    }
}

fn short_midi_name(name: &str) -> String {
    name.split(':').next().unwrap_or(name).trim().to_string()
}

fn guitar_in_picker(ui: &mut egui::Ui, app: &mut HelixApp) {
    let Some(gtr) = app.guitar.as_mut() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(theme::section_label("INPUT"));
        let peak = gtr.peak();
        let (meter, _) = ui.allocate_exact_size(egui::vec2(72.0, 10.0), egui::Sense::hover());
        ui.painter().rect_filled(meter, 4.0, theme::ELEVATED);
        let w = (peak.clamp(0.0, 1.0)) * meter.width();
        ui.painter().rect_filled(
            egui::Rect::from_min_size(meter.min, egui::vec2(w, meter.height())),
            4.0,
            theme::ACCENT,
        );
        let label = gtr
            .connected
            .as_deref()
            .map(short_midi_name)
            .unwrap_or_else(|| "Guitar in".to_string());
        egui::ComboBox::from_id_salt("guitar-in")
            .selected_text(egui::RichText::new(label).size(12.0).color(FG))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(gtr.connected.is_none(), "None")
                    .clicked()
                {
                    gtr.disconnect();
                }
                let ports = gtr.devices.clone();
                if ports.is_empty() {
                    ui.label(
                        egui::RichText::new("Nenhuma entrada. Plugue a GT-100 USB.")
                            .size(11.0)
                            .color(MUTED),
                    );
                }
                for name in ports {
                    let selected = gtr.connected.as_deref() == Some(name.as_str());
                    if ui
                        .selectable_label(selected, short_midi_name(&name))
                        .clicked()
                    {
                        match gtr.connect(&name) {
                            Ok(()) => app.guitar_err = None,
                            Err(e) => app.guitar_err = Some(e),
                        }
                    }
                }
            });
    });
}

fn gt100_out_picker(ui: &mut egui::Ui, app: &mut HelixApp) {
    let Some(out) = app.gt100.as_mut() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(theme::section_label("GT-100 MIDI OUT"));
        let label = out
            .connected
            .as_deref()
            .map(short_midi_name)
            .unwrap_or_else(|| "GT-100 out".to_string());
        egui::ComboBox::from_id_salt("gt100-out")
            .selected_text(egui::RichText::new(label).size(12.0).color(FG))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(out.connected.is_none(), "None")
                    .clicked()
                {
                    out.disconnect();
                }
                let ports = out.ports.clone();
                if ports.is_empty() {
                    ui.label(
                        egui::RichText::new("Nenhuma porta MIDI out. Plugue a GT-100 USB.")
                            .size(11.0)
                            .color(MUTED),
                    );
                }
                for name in ports {
                    let selected = out.connected.as_deref() == Some(name.as_str());
                    if ui
                        .selectable_label(selected, short_midi_name(&name))
                        .clicked()
                    {
                        match out.connect(&name) {
                            Ok(()) => app.gt100_err = None,
                            Err(e) => app.gt100_err = Some(e),
                        }
                    }
                }
            });
    });
}

fn midi_picker(ui: &mut egui::Ui, app: &mut HelixApp) {

    let Some(midi) = app.midi.as_mut() else {
        ui.label(egui::RichText::new("No MIDI").color(SUBTLE));
        return;
    };
    ui.horizontal(|ui| {
        if ui
            .add(
                egui::Button::new(egui::RichText::new("Refresh").size(12.0).color(MUTED))
                    .fill(Color32::TRANSPARENT),
            )
            .clicked()
        {
            midi.refresh();
            midi.hold_off = false;
            match midi.auto_connect() {
                Ok(()) => app.midi_err = None,
                Err(e) => app.midi_err = Some(e),
            }
        }
        let live = midi.live();
        let label = midi
            .connected
            .as_deref()
            .map(short_midi_name)
            .unwrap_or_else(|| {
                if midi.ports.iter().any(|p| {
                    let l = p.to_ascii_lowercase();
                    l.contains("keyboard") || l.contains("yamaha")
                }) {
                    "Yamaha (tap Refresh)".to_string()
                } else {
                    "MIDI in".to_string()
                }
            });
        let label_color = if live { theme::ACCENT_FG } else { FG };
        egui::Frame::new()
            .fill(if live { theme::ACCENT } else { theme::ELEVATED })
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(4, 2))
            .show(ui, |ui| {
                egui::ComboBox::from_id_salt("midi-in")
                    .selected_text(egui::RichText::new(label).size(12.0).color(label_color))
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(midi.connected.is_none(), "None")
                            .clicked()
                        {
                            midi.disconnect();
                        }
                        let ports = midi.ports.clone();
                        if ports.is_empty() {
                            ui.label(
                                egui::RichText::new("Nenhum dispositivo. Plugue o USB.")
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        }
                        for name in ports {
                            let selected = midi.connected.as_deref() == Some(name.as_str());
                            if ui
                                .selectable_label(selected, short_midi_name(&name))
                                .clicked()
                            {
                                match midi.connect(&name) {
                                    Ok(()) => app.midi_err = None,
                                    Err(e) => app.midi_err = Some(e),
                                }
                            }
                        }
                    });
            });
    });
}

fn computer_key_offset(key: Key) -> Option<u8> {
    Some(match key {
        Key::Z => 0,
        Key::S => 1,
        Key::X => 2,
        Key::D => 3,
        Key::C => 4,
        Key::V => 5,
        Key::G => 6,
        Key::B => 7,
        Key::H => 8,
        Key::N => 9,
        Key::J => 10,
        Key::M => 11,
        Key::Comma | Key::Q => 12,
        Key::L | Key::Num2 => 13,
        Key::Period | Key::W => 14,
        Key::Semicolon | Key::Num3 => 15,
        Key::Slash | Key::E => 16,
        Key::R => 17,
        Key::Num5 => 18,
        Key::T => 19,
        Key::Num6 => 20,
        Key::Y => 21,
        Key::Num7 => 22,
        Key::U => 23,
        Key::I => 24,
        Key::Num9 => 25,
        Key::O => 26,
        Key::Num0 => 27,
        Key::P => 28,
        _ => return None,
    })
}

pub fn run() -> eframe::Result<()> {
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../../assets/helix-synth.png")).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 860.0])
        .with_min_inner_size([880.0, 640.0])
        .with_title("Helix")
        .with_app_id("helix-synth");
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "helix-synth",
        options,
        Box::new(|cc| Ok(Box::new(HelixApp::new(cc)))),
    )
}
