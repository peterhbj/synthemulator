use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Color32, Key, Margin};

use super::arp_panel::{self, ArpEvent, ArpUiState};
use super::fuzz_panel::{self, FuzzEvent, FuzzUiState};
use super::keyboard;
use super::oscilloscope;
use super::pitch_wheel;
use super::theme::{self, BG, FG, MUTED, SUBTLE, SURFACE};
use super::widgets::{
    badge_pill, format_hz, format_ms, format_pct, icon_button, knob, volume_slider, waveform_select,
};
use super::remix_panel::RemixUi;
use super::widgets::chip;
use super::xy_panel::{XyUi, XyWrite};
use crate::audio::{AudioHost, Gt100Hub, GuitarInput, MidiHub};
use crate::settings::Settings;
use crate::xy::XyTarget;
use remix::{Band, CrossfaderCurve, DeckCmd, RemixCmd, RemixState, DECKS};
use crate::synth::{
    midi_to_name, notes_from_preset, octave_base_midi, ArpDivision, ArpPattern, ArpPresetId,
    Command, FootLearn, SynthParams, MAX_OCTAVE, MIN_OCTAVE, VISIBLE_SEMITONES,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Synth,
    Remix,
}

/// One key event: (physical key, logical key, pressed, repeat, modifiers).
type KeyEvent = (Key, Key, bool, bool, egui::Modifiers);

/// Values the XY pad can read, copied out so the pad can borrow them while
/// the app is mutably borrowed.
#[derive(Clone, Copy)]
struct XyValues {
    cutoff: f32,
    resonance: f32,
    bend: f32,
    mix: super::remix_panel::MixUi,
}

impl XyValues {
    fn get(&self, t: XyTarget) -> f32 {
        let m = &self.mix;
        match t {
            XyTarget::None => 0.0,
            XyTarget::MasterFilter => m.master_filter,
            XyTarget::Crossfader => m.crossfader,
            XyTarget::EchoMix => m.echo_mix,
            XyTarget::EchoFeedback => m.echo_feedback,
            XyTarget::EchoTime => m.echo_beats,
            XyTarget::FilterA => m.filter[0],
            XyTarget::FilterB => m.filter[1],
            XyTarget::LowA => m.eq[0][Band::Low.index()],
            XyTarget::LowB => m.eq[1][Band::Low.index()],
            XyTarget::HighA => m.eq[0][Band::High.index()],
            XyTarget::HighB => m.eq[1][Band::High.index()],
            XyTarget::SynthCutoff => self.cutoff,
            XyTarget::SynthResonance => self.resonance,
            XyTarget::SynthBend => self.bend,
        }
    }
}

pub struct HelixApp {
    tab: Tab,
    settings: Settings,
    settings_dirty_at: Option<f64>,
    remix: RemixUi,
    remix_state: RemixState,
    xy: XyUi,
    audio: Option<AudioHost>,
    audio_err: Option<String>,
    midi: Option<MidiHub>,
    midi_err: Option<String>,
    guitar: Option<GuitarInput>,
    guitar_err: Option<String>,
    gt100: Option<Gt100Hub>,
    params: SynthParams,
    octave: i32,
    bend: f32,
    bend_range: u8,
    arp: ArpUiState,
    fuzz: FuzzUiState,
    pointer_held: HashSet<u8>,
    key_held: HashMap<Key, u8>,
    arrow_up: bool,
    arrow_down: bool,
    dragging_bend: bool,
    spring_from: Option<(f32, f64)>,
    peak: f32,
    was_dragging_bend: bool,
    midi_poll_at: f64,
    /// The sustain pedal is down because Space set it (not a MIDI pedal).
    space_pedal: bool,
}

impl HelixApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::install_style(&cc.egui_ctx);
        cc.egui_ctx.set_zoom_factor(1.0);
        cc.egui_ctx.options_mut(|o| {
            o.zoom_with_keyboard = false;
        });

        let (audio, audio_err, midi, guitar, gt100) = match AudioHost::start() {
            Ok(mut host) => {
                let sender = host.sender();
                let midi = MidiHub::new(sender.clone());
                let guitar = GuitarInput::new(
                    host.guitar_ring.clone(),
                    host.guitar_peak.clone(),
                    host.sample_rate,
                );
                let gt100 = host.take_gt_rx().map(|rx| Gt100Hub::new(rx, sender.clone()));
                (Some(host), None, Some(midi), Some(guitar), gt100)
            }
            Err(e) => (None, Some(e), None, None, None),
        };

        let settings = Settings::load();
        let sample_rate = audio.as_ref().map(|a| a.sample_rate as u32).unwrap_or(48_000);
        let remix = RemixUi::new(sample_rate, &settings);
        if let Some(a) = &audio {
            if settings.sharp_crossfader {
                a.send_remix(RemixCmd::SetCrossfaderCurve(CrossfaderCurve::Sharp));
            }
            remix.send_echo(&|c| a.send_remix(c));
        }

        Self::from_parts(settings, remix, audio, audio_err, midi, guitar, gt100)
    }

    /// The app around its hardware. `new` supplies the real devices; tests pass none.
    fn from_parts(
        settings: Settings,
        remix: RemixUi,
        audio: Option<AudioHost>,
        audio_err: Option<String>,
        midi: Option<MidiHub>,
        guitar: Option<GuitarInput>,
        gt100: Option<Gt100Hub>,
    ) -> Self {
        Self {
            tab: Tab::Synth,
            settings,
            settings_dirty_at: None,
            remix,
            remix_state: RemixState::default(),
            xy: XyUi::new(),
            audio,
            audio_err,
            midi,
            midi_err: None,
            guitar,
            guitar_err: None,
            gt100,
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
            fuzz: FuzzUiState::default(),
            pointer_held: HashSet::new(),
            key_held: HashMap::new(),
            arrow_up: false,
            arrow_down: false,
            dragging_bend: false,
            spring_from: None,
            peak: 0.0,
            was_dragging_bend: false,
            midi_poll_at: 0.0,
            space_pedal: false,
        }
    }

    fn send(&self, cmd: Command) {
        if let Some(audio) = &self.audio {
            audio.send(cmd);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let typing = ctx.wants_keyboard_input();
        // One entry per key event as (physical, logical, pressed, repeat,
        // modifiers); `None` is the window losing focus. Physical keys place the
        // notes and the Remix map, so they stay put on ABNT2 and other layouts.
        // The logical key carries the octave keys, which follow the printed `[` `]`.
        let inputs: Vec<Option<KeyEvent>> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        physical_key,
                        pressed,
                        repeat,
                        modifiers,
                    } => Some(Some((physical_key.unwrap_or(*key), *key, *pressed, *repeat, *modifiers))),
                    egui::Event::WindowFocused(false) => Some(None),
                    _ => None,
                })
                .collect()
        });
        let time = ctx.input(|i| i.time);
        for input in inputs {
            let Some((key, logical, pressed, repeat, modifiers)) = input else {
                // Focus lost: the releases will not arrive, so let go of everything.
                self.release_synth_keys(time);
                self.release_remix_keys();
                continue;
            };
            // Only new presses are filtered. A release always gets through, so
            // a key held across Ctrl, Alt or Cmd cannot stick.
            if pressed && (modifiers.ctrl || modifiers.mac_cmd || modifiers.alt || repeat) {
                continue;
            }
            if matches!(key, Key::F1 | Key::F2 | Key::F10 | Key::Escape) {
                if !pressed {
                    continue;
                }
                match key {
                    Key::F1 => self.switch_tab(ctx, Tab::Synth),
                    Key::F2 => self.switch_tab(ctx, Tab::Remix),
                    Key::F10 => self.toggle_xy(ctx),
                    _ => {
                        if self.xy.active {
                            self.exit_xy(ctx);
                        } else if modifiers.shift && self.tab == Tab::Remix {
                            for d in 0..DECKS {
                                self.send_remix(RemixCmd::Deck(d, DeckCmd::Pause));
                            }
                        } else {
                            self.panic();
                        }
                    }
                }
                continue;
            }
            // Typing in a text field holds back new presses; releases still count.
            if typing && pressed {
                continue;
            }
            match self.tab {
                Tab::Synth => self.synth_key(key, logical, pressed, time),
                Tab::Remix => {
                    let audio = &self.audio;
                    let sink = |c: RemixCmd| {
                        if let Some(a) = audio {
                            a.send_remix(c);
                        }
                    };
                    if self.remix.handle_key(
                        key,
                        pressed,
                        modifiers.shift,
                        time,
                        &self.remix_state,
                        &mut self.settings,
                        &sink,
                    ) {
                        self.settings_dirty_at.get_or_insert(time);
                    }
                }
            }
        }

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

    fn panic(&mut self) {
        self.send(Command::Panic);
        self.pointer_held.clear();
        self.key_held.clear();
        self.bend = 0.0;
    }

    /// `key` is the physical key (notes, bend, sustain); `logical` is the key as
    /// printed, which the octave keys use.
    fn synth_key(&mut self, key: Key, logical: Key, pressed: bool, time: f64) {
        if key == Key::Space {
            self.space_pedal = pressed;
            self.send(Command::SetPedal(pressed));
            return;
        }
        if let Some(step) = octave_step(logical, pressed) {
            self.shift_octave(step);
            return;
        }
        if key == Key::ArrowUp || key == Key::ArrowDown {
            if key == Key::ArrowUp {
                self.arrow_up = pressed;
            } else {
                self.arrow_down = pressed;
            }
            self.spring_from = None;
            let dir = match (self.arrow_up, self.arrow_down) {
                (true, false) => 1.0,
                (false, true) => -1.0,
                _ => 0.0,
            };
            if dir == 0.0 && !pressed {
                self.spring_from = Some((self.bend, time));
            } else {
                self.bend = dir;
                self.send(Command::SetBend {
                    amount: self.bend,
                    range: self.bend_range,
                });
            }
            return;
        }
        if let Some(offset) = computer_key_offset(key) {
            if pressed {
                if self.key_held.contains_key(&key) {
                    return;
                }
                let midi = octave_base_midi(self.octave).saturating_add(offset);
                self.key_held.insert(key, midi);
                if self.arp.arp_on {
                    self.arp.arp_preset = Some(ArpPresetId::Live);
                }
                self.send(Command::NoteOn {
                    midi,
                    velocity: 1.0,
                });
            } else if let Some(midi) = self.key_held.remove(&key) {
                self.send(Command::NoteOff { midi });
            }
        }
    }

    fn send_remix(&self, cmd: RemixCmd) {
        if let Some(audio) = &self.audio {
            audio.send_remix(cmd);
        }
    }

    fn switch_tab(&mut self, ctx: &egui::Context, tab: Tab) {
        if self.tab == tab {
            return;
        }
        if self.xy.active {
            self.exit_xy(ctx);
        }
        match self.tab {
            Tab::Synth => self.release_synth_keys(ctx.input(|i| i.time)),
            Tab::Remix => self.release_remix_keys(),
        }
        self.tab = tab;
    }

    /// Let go of the synth's held input: keys, the Space pedal and the arrow bend.
    fn release_synth_keys(&mut self, time: f64) {
        let held: Vec<u8> = self.key_held.drain().map(|(_, m)| m).collect();
        for midi in held {
            self.send(Command::NoteOff { midi });
        }
        if self.arrow_up || self.arrow_down {
            self.arrow_up = false;
            self.arrow_down = false;
            self.spring_from = Some((self.bend, time));
        }
        // Only the pedal that Space set: CC64 from a MIDI pedal is the same flag.
        if self.space_pedal {
            self.space_pedal = false;
            self.send(Command::SetPedal(false));
        }
    }

    /// Let go of the Remix keys: the filter sweeps and the nudges.
    fn release_remix_keys(&mut self) {
        let audio = &self.audio;
        self.remix.release_keys(&|c: RemixCmd| {
            if let Some(a) = audio {
                a.send_remix(c);
            }
        });
    }

    fn xy_targets(&self) -> [XyTarget; 2] {
        match self.tab {
            Tab::Synth => self.settings.xy.synth,
            Tab::Remix => self.settings.xy.remix,
        }
    }

    fn xy_values(&self) -> XyValues {
        XyValues {
            cutoff: self.params.cutoff,
            resonance: self.params.resonance,
            bend: self.bend,
            mix: self.remix.mix,
        }
    }

    fn toggle_xy(&mut self, ctx: &egui::Context) {
        let targets = self.xy_targets();
        let vals = self.xy_values();
        let writes = self
            .xy
            .toggle(ctx, targets, self.settings.xy.spring_back, &|t| vals.get(t));
        for w in writes {
            self.apply_xy(w);
        }
    }

    fn exit_xy(&mut self, ctx: &egui::Context) {
        let writes = self.xy.exit(ctx, self.settings.xy.spring_back);
        for w in writes {
            self.apply_xy(w);
        }
    }

    fn apply_xy(&mut self, w: XyWrite) {
        let v = w.value;
        let m = &mut self.remix.mix;
        let cmd = match w.target {
            XyTarget::None => None,
            XyTarget::SynthCutoff => {
                self.params.cutoff = v;
                self.send(Command::SetCutoff(v));
                None
            }
            XyTarget::SynthResonance => {
                self.params.resonance = v;
                self.send(Command::SetResonance(v));
                None
            }
            XyTarget::SynthBend => {
                self.bend = v;
                self.spring_from = None;
                self.send(Command::SetBend {
                    amount: v,
                    range: self.bend_range,
                });
                None
            }
            XyTarget::MasterFilter => {
                m.master_filter = v;
                Some(RemixCmd::SetMasterFilter(v))
            }
            XyTarget::Crossfader => {
                m.crossfader = v;
                Some(RemixCmd::SetCrossfader(v))
            }
            XyTarget::EchoMix | XyTarget::EchoFeedback | XyTarget::EchoTime => {
                match w.target {
                    XyTarget::EchoMix => m.echo_mix = v,
                    XyTarget::EchoFeedback => m.echo_feedback = v,
                    _ => m.echo_beats = v,
                }
                Some(RemixCmd::SetEcho {
                    mix: m.echo_mix,
                    feedback: m.echo_feedback,
                    beats: m.echo_beats,
                })
            }
            XyTarget::FilterA | XyTarget::FilterB => {
                let deck = (w.target == XyTarget::FilterB) as usize;
                m.filter[deck] = v;
                Some(RemixCmd::SetFilter { deck, pos: v })
            }
            XyTarget::LowA | XyTarget::LowB | XyTarget::HighA | XyTarget::HighB => {
                let deck = matches!(w.target, XyTarget::LowB | XyTarget::HighB) as usize;
                let band = if matches!(w.target, XyTarget::LowA | XyTarget::LowB) {
                    Band::Low
                } else {
                    Band::High
                };
                m.eq[deck][band.index()] = v;
                Some(RemixCmd::SetEq { deck, band, gain: v })
            }
        };
        if let Some(c) = cmd {
            self.send_remix(c);
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
}

impl eframe::App for HelixApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        if let Some(a) = &self.audio {
            a.drain_trash();
            self.remix_state = a.remix_state();
        }
        self.handle_keys(ctx);

        let now = ctx.input(|i| i.time);
        {
            let audio = &self.audio;
            let sink = |c: RemixCmd| {
                if let Some(a) = audio {
                    a.send_remix(c);
                }
            };
            if self.remix.tick(now, &self.remix_state, &mut self.settings, &sink) {
                self.settings_dirty_at.get_or_insert(now);
            }
        }
        self.remix.handle_dropped_files(ctx, &self.remix_state, now);
        if self.tab == Tab::Synth && ctx.input(|i| !i.raw.dropped_files.is_empty()) {
            self.switch_tab(ctx, Tab::Remix);
        }

        let targets = self.xy_targets();
        let vals = self.xy_values();
        let writes = self.xy.update(
            ctx,
            targets,
            self.settings.xy.spring_back,
            self.settings.xy.sensitivity,
            &|t| vals.get(t),
        );
        for w in writes {
            self.apply_xy(w);
        }

        if let Some(t) = self.settings_dirty_at {
            if now - t > 1.5 {
                if let Err(e) = self.settings.save() {
                    crate::log::warn(&format!("could not save settings: {e}"));
                }
                self.settings_dirty_at = None;
            }
        }
        if now - self.midi_poll_at > 0.8 {
            self.midi_poll_at = now;
            if let Some(midi) = self.midi.as_mut() {
                midi.refresh();
                if let Err(e) = midi.auto_connect() {
                    self.midi_err = Some(e);
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
            if let Some(gt) = self.gt100.as_mut() {
                gt.refresh();
                if !gt.hold_off {
                    let _ = gt.auto_connect();
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
                        self.paint_header(ui, now);
                        match self.tab {
                            Tab::Synth => self.paint_ui(ui, &snap, &active, &pooled),
                            Tab::Remix => {
                                let audio = &self.audio;
                                let sink = |c: RemixCmd| {
                                    if let Some(a) = audio {
                                        a.send_remix(c);
                                    }
                                };
                                if self.remix.show(ui, &self.remix_state, &mut self.settings, &sink, now) {
                                    self.settings_dirty_at.get_or_insert(now);
                                }
                            }
                        }
                    });
            });

        let vals = self.xy_values();
        self.xy.overlay(ctx, self.xy_targets(), &|t| vals.get(t));
    }
}

impl Drop for HelixApp {
    fn drop(&mut self) {
        // Test apps have no window: they must not overwrite the user's settings file.
        #[cfg(not(test))]
        {
            if let Err(e) = self.settings.save() {
                crate::log::warn(&format!("could not save settings: {e}"));
            }
        }
    }
}

impl HelixApp {
    fn paint_header(&mut self, ui: &mut egui::Ui, now: f64) {
        let mut switch_to = None;
        let mut toggle_xy = false;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("SYNTH · REMIX")
                        .size(10.5)
                        .color(SUBTLE)
                        .extra_letter_spacing(2.4),
                );
                ui.label(egui::RichText::new("Helix").heading().color(FG));
            });
            ui.add_space(18.0);
            for (tab, label) in [(Tab::Synth, "Synth · F1"), (Tab::Remix, "Remix · F2")] {
                if chip(ui, label, self.tab == tab).clicked() {
                    switch_to = Some(tab);
                }
            }
            ui.add_space(8.0);
            if chip(ui, "XY · F10", self.xy.active).clicked() {
                toggle_xy = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                midi_picker(ui, self);
            });
        });
        let ctx = ui.ctx().clone();
        if let Some(tab) = switch_to {
            self.switch_tab(&ctx, tab);
        }
        if toggle_xy {
            self.toggle_xy(&ctx);
        }
        ui.add_space(4.0);
        let xy = &mut self.settings.xy;
        let targets = match self.tab {
            Tab::Synth => &mut xy.synth,
            Tab::Remix => &mut xy.remix,
        };
        if self
            .xy
            .settings_row(ui, targets, &mut xy.spring_back, &mut xy.sensitivity)
        {
            self.settings_dirty_at.get_or_insert(now);
        }
        ui.add_space(8.0);

        if let Some(err) = &self.audio_err {
            ui.colored_label(
                egui::Color32::from_rgb(220, 80, 80),
                format!("Audio: {err}"),
            );
        }
    }

    fn paint_ui(
        &mut self,
        ui: &mut egui::Ui,
        snap: &crate::audio::Snapshot,
        active: &HashSet<u8>,
        pooled: &HashSet<u8>,
    ) {
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
                let fuzz_events = fuzz_panel::fuzz_panel(
                    ui,
                    &mut self.fuzz,
                    snap.fuzz_va,
                    snap.foot_fuzz_cc,
                    snap.foot_learn == Some(FootLearn::Fuzz),
                );
                let fuzz_touched = !fuzz_events.is_empty();
                for ev in fuzz_events {
                    self.handle_fuzz_event(ev);
                }
                if !fuzz_touched {
                    self.fuzz.on = snap.fuzz_on;
                    self.fuzz.knobs = snap.fuzz_knobs;
                }
                ui.add_space(8.0);
                let events = arp_panel::arp_panel(
                    ui,
                    &mut self.arp,
                    snap.foot_whammy_cc,
                    snap.foot_learn == Some(FootLearn::Whammy),
                );
                let arp_touched = events.iter().any(|e| {
                    matches!(
                        e,
                        ArpEvent::ToggleWhammy | ArpEvent::ToggleGuitar | ArpEvent::LearnWhammy
                    )
                });
                for ev in events {
                    self.handle_arp_event(ev);
                }
                if !arp_touched {
                    self.arp.whammy_on = snap.whammy_on;
                    self.arp.guitar_on = snap.guitar_on;
                }
                guitar_in_picker(ui, self);
                gt100_picker(ui, self);
                if let Some(err) = &self.guitar_err {
                    ui.colored_label(Color32::from_rgb(220, 80, 80), err);
                }
                if let Some(gt) = &self.gt100 {
                    let st = gt.status();
                    if let Some(err) = st.err {
                        ui.colored_label(Color32::from_rgb(220, 80, 80), err);
                    }
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
                    badge_pill(ui, "Fuzz", self.fuzz.on);
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
                "Fuzz Factory sits on the output (keys + guitar). On, then play Z/Q or guitar. Stab down = squeal (no input needed). Comp is the pitch/treble of that wave. Pedais: toque (não segure). Learn + pisa o CTL. CC 80 Whammy, 81 Fuzz. Whammy drives GT-100 P.SHIFT (−12 / 0 / +12). Space sustains, [ ] octave, Esc panic. F1 Synth · F2 Remix · F10 XY pad.",
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
                self.arp.whammy_on = !self.arp.whammy_on;
                self.send(Command::SetWhammyOn(self.arp.whammy_on));
            }
            ArpEvent::LearnWhammy => {
                self.send(Command::LearnFoot(Some(FootLearn::Whammy)));
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
                                "Plugue a GT-100 via USB (áudio + MIDI 1). Guitar On monitora o USB; Whammy manda o pitch pra pedaleira."
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

    fn handle_fuzz_event(&mut self, ev: FuzzEvent) {
        match ev {
            FuzzEvent::Toggle => {
                self.fuzz.on = !self.fuzz.on;
                self.send(Command::SetFuzzOn(self.fuzz.on));
                if self.fuzz.on {
                    self.arm_fuzz_input();
                }
            }
            FuzzEvent::Knobs => {
                if !self.fuzz.on {
                    self.fuzz.on = true;
                    self.send(Command::SetFuzzOn(true));
                    self.arm_fuzz_input();
                }
                self.send_fuzz_knobs();
            }
            FuzzEvent::Preset(k) => {
                self.fuzz.knobs = k;
                if !self.fuzz.on {
                    self.fuzz.on = true;
                    self.send(Command::SetFuzzOn(true));
                }
                self.arm_fuzz_input();
                self.send_fuzz_knobs();
            }
            FuzzEvent::Learn => {
                self.send(Command::LearnFoot(Some(FootLearn::Fuzz)));
            }
        }
    }

    fn arm_fuzz_input(&mut self) {
        if !self.arp.guitar_on {
            self.arp.guitar_on = true;
            self.send(Command::SetGuitarOn(true));
        }
        if let Some(g) = self.guitar.as_mut() {
            g.hold_off = false;
            let _ = g.auto_connect();
        }
    }

    fn send_fuzz_knobs(&self) {
        let k = self.fuzz.knobs;
        self.send(Command::SetFuzzVol(k.vol));
        self.send(Command::SetFuzzGate(k.gate));
        self.send(Command::SetFuzzComp(k.comp));
        self.send(Command::SetFuzzDrive(k.drive));
        self.send(Command::SetFuzzStab(k.stab));
    }
}

fn short_midi_name(name: &str) -> String {
    name.split(':').next().unwrap_or(name).trim().to_string()
}

fn short_port_name(name: &str) -> String {
    let l = name.to_ascii_lowercase();
    if l.contains("gt-100") || l.contains("gt100") {
        if l.contains("midi 2") || l.contains("midi2") {
            return "GT-100 MIDI 2".into();
        }
        return "GT-100 MIDI 1".into();
    }
    short_midi_name(name)
}

fn gt100_picker(ui: &mut egui::Ui, app: &mut HelixApp) {
    let Some(gt) = app.gt100.as_mut() else {
        return;
    };
    let st = gt.status();
    ui.horizontal(|ui| {
        ui.label(theme::section_label("GT-100"));
        let live = st.live || st.taken_over;
        let label = if st.taken_over {
            st.line()
        } else {
            st.connected
                .as_deref()
                .map(short_port_name)
                .unwrap_or_else(|| "MIDI out".to_string())
        };
        let label_color = if live { theme::ACCENT_FG } else { FG };
        egui::Frame::new()
            .fill(if live { theme::ACCENT } else { theme::ELEVATED })
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(4, 2))
            .show(ui, |ui| {
                egui::ComboBox::from_id_salt("gt100-out")
                    .selected_text(egui::RichText::new(label).size(12.0).color(label_color))
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(st.connected.is_none(), "None")
                            .clicked()
                        {
                            gt.disconnect();
                        }
                        let ports = gt.ports.clone();
                        if ports.is_empty() {
                            ui.label(
                                egui::RichText::new("Nenhuma MIDI out. Plugue a GT-100 USB.")
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        }
                        for name in ports {
                            let selected = st.connected.as_deref() == Some(name.as_str());
                            if ui
                                .selectable_label(selected, short_port_name(&name))
                                .clicked()
                            {
                                let _ = gt.connect(&name);
                            }
                        }
                    });
            });
    });
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

/// Octave step for a press of the printed `[` (down) or `]` (up). It reads the
/// logical key: on ABNT2 the key printed `[` sits where US has `]`.
fn octave_step(logical: Key, pressed: bool) -> Option<i32> {
    match (pressed, logical) {
        (true, Key::OpenBracket) => Some(-1),
        (true, Key::CloseBracket) => Some(1),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, Modifiers, Pos2, RawInput, Rect, Vec2};
    use eframe::App as _;
    use std::path::PathBuf;

    /// The app with no audio, MIDI or guitar: keys reach its state and nothing else.
    fn app() -> HelixApp {
        let settings = Settings::default();
        let remix = RemixUi::new(48_000, &settings);
        HelixApp::from_parts(settings, remix, None, None, None, None, None)
    }

    fn remix_app() -> HelixApp {
        let mut app = app();
        app.tab = Tab::Remix;
        app
    }

    fn raw_input(time: f64, events: Vec<Event>) -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 860.0))),
            focused: true,
            time: Some(time),
            events,
            ..Default::default()
        }
    }

    fn key(key: Key, physical: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: Some(physical),
            pressed,
            repeat: false,
            modifiers,
        }
    }

    /// Routes one frame of key events through the app, as `update` does.
    fn send_keys(app: &mut HelixApp, ctx: &egui::Context, events: Vec<Event>) {
        let _ = ctx.run(raw_input(0.0, events), |ctx| app.handle_keys(ctx));
    }

    #[test]
    fn abnt2_open_bracket_lowers_the_octave() {
        // ABNT2: the key printed '[' is the physical US ']' (CloseBracket).
        let ctx = egui::Context::default();
        let mut app = app();
        assert_eq!(app.octave, 3);
        send_keys(&mut app, &ctx, vec![key(Key::OpenBracket, Key::CloseBracket, true, Modifiers::NONE)]);
        assert_eq!(app.octave, 2);
    }

    #[test]
    fn abnt2_close_bracket_raises_the_octave() {
        // ABNT2: the key printed ']' is the physical Backslash.
        let ctx = egui::Context::default();
        let mut app = app();
        send_keys(&mut app, &ctx, vec![key(Key::CloseBracket, Key::Backslash, true, Modifiers::NONE)]);
        assert_eq!(app.octave, 4);
    }

    #[test]
    fn us_brackets_move_the_octave_down_and_up() {
        let ctx = egui::Context::default();
        let mut app = app();
        send_keys(&mut app, &ctx, vec![key(Key::OpenBracket, Key::OpenBracket, true, Modifiers::NONE)]);
        assert_eq!(app.octave, 2);
        send_keys(&mut app, &ctx, vec![key(Key::CloseBracket, Key::CloseBracket, true, Modifiers::NONE)]);
        assert_eq!(app.octave, 3);
    }

    #[test]
    fn octave_step_reads_the_printed_key_and_only_presses() {
        assert_eq!(octave_step(Key::OpenBracket, true), Some(-1));
        assert_eq!(octave_step(Key::CloseBracket, true), Some(1));
        assert_eq!(octave_step(Key::OpenBracket, false), None);
        assert_eq!(octave_step(Key::Z, true), None);
    }

    /// Runs the Remix ticks from `from` for `secs`, in 0.1 s steps.
    fn tick_remix(app: &mut HelixApp, from: f64, secs: f64) {
        let state = app.remix_state;
        let sink = |_: RemixCmd| {};
        let steps = (secs / 0.1).round() as usize;
        for i in 1..=steps {
            app.remix.tick(from + i as f64 * 0.1, &state, &mut app.settings, &sink);
        }
    }

    /// Holds E on the Remix tab, then sends `after`; returns the filter knob
    /// three seconds later.
    fn filter_after_e_held_then(after: Vec<Event>) -> f32 {
        let ctx = egui::Context::default();
        let mut app = remix_app();
        send_keys(&mut app, &ctx, vec![key(Key::E, Key::E, true, Modifiers::NONE)]);
        send_keys(&mut app, &ctx, after);
        tick_remix(&mut app, 0.0, 3.0);
        app.remix.mix.filter[0]
    }

    #[test]
    fn releasing_e_with_ctrl_down_stops_the_filter_sweep() {
        let f = filter_after_e_held_then(vec![key(Key::E, Key::E, false, Modifiers::CTRL)]);
        assert!(f.abs() < 1e-6, "E released, filter still swept to {f}");
    }

    #[test]
    fn alt_tab_releases_stop_the_filter_sweep() {
        // Windows reports Alt+Tab as the Tab press, then the releases, with Alt down,
        // then the focus loss.
        let f = filter_after_e_held_then(vec![
            key(Key::Tab, Key::Tab, true, Modifiers::ALT),
            key(Key::E, Key::E, false, Modifiers::ALT),
            Event::WindowFocused(false),
        ]);
        assert!(f.abs() < 1e-6, "Alt+Tab left the filter swept to {f}");
    }

    #[test]
    fn focus_loss_alone_stops_the_filter_sweep() {
        // Wayland sends no key release when the window loses focus.
        let f = filter_after_e_held_then(vec![Event::WindowFocused(false)]);
        assert!(f.abs() < 1e-6, "focus loss left the filter swept to {f}");
    }

    #[test]
    fn a_knob_set_back_to_zero_stays_there() {
        let ctx = egui::Context::default();
        let mut app = remix_app();
        send_keys(&mut app, &ctx, vec![key(Key::E, Key::E, true, Modifiers::NONE)]);
        send_keys(&mut app, &ctx, vec![key(Key::E, Key::E, false, Modifiers::CTRL)]);
        app.remix.mix.filter[0] = 0.0;
        tick_remix(&mut app, 0.0, 3.0);
        assert_eq!(app.remix.mix.filter[0], 0.0);
    }

    #[test]
    fn focus_loss_releases_held_synth_keys_and_the_space_pedal() {
        let ctx = egui::Context::default();
        let mut app = app();
        send_keys(
            &mut app,
            &ctx,
            vec![
                key(Key::Z, Key::Z, true, Modifiers::NONE),
                key(Key::Space, Key::Space, true, Modifiers::NONE),
            ],
        );
        assert_eq!(app.key_held.len(), 1);
        assert!(app.space_pedal);
        send_keys(&mut app, &ctx, vec![Event::WindowFocused(false)]);
        assert!(app.key_held.is_empty());
        assert!(!app.space_pedal);
    }

    #[test]
    fn remix_key_presses_do_not_mark_the_settings_dirty() {
        let ctx = egui::Context::default();
        let mut app = remix_app();
        send_keys(
            &mut app,
            &ctx,
            vec![
                key(Key::Q, Key::Q, true, Modifiers::NONE),
                key(Key::Q, Key::Q, false, Modifiers::NONE),
            ],
        );
        assert!(app.settings_dirty_at.is_none());
    }

    #[test]
    fn dropping_a_file_on_the_synth_tab_lets_go_of_held_notes() {
        let ctx = egui::Context::default();
        let mut app = app();
        send_keys(&mut app, &ctx, vec![key(Key::Z, Key::Z, true, Modifiers::NONE)]);
        assert_eq!(app.key_held.len(), 1);
        let dropped = egui::DroppedFile {
            path: Some(PathBuf::from("notas.txt")),
            ..Default::default()
        };
        let mut input = raw_input(0.0, Vec::new());
        input.dropped_files = vec![dropped];
        let mut eframe_frame = eframe::Frame::_new_kittest();
        let _ = ctx.run(input, |ctx| app.update(ctx, &mut eframe_frame));
        assert_eq!(app.tab, Tab::Remix);
        assert!(app.key_held.is_empty());
    }
}
