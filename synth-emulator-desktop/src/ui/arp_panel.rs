use eframe::egui::{self, Color32, Pos2, Rect, Sense, Ui, Vec2};

use super::theme::{self, ACCENT, ACCENT_FG, MUTED, SURFACE};
use super::widgets::{chip, format_pct, knob};
use crate::synth::{ArpDivision, ArpPattern, ArpPresetId, ARP_PRESETS, WHAMMY_SEQUENCE};

pub struct ArpUiState {
    pub arp_on: bool,
    pub arp_latch: bool,
    pub arp_pattern: ArpPattern,
    pub arp_rate: ArpDivision,
    pub arp_tempo: f32,
    pub arp_octaves: f32,
    pub arp_gate: f32,
    pub arp_preset: Option<ArpPresetId>,
    pub whammy_on: bool,
    pub whammy_step: usize,
    pub guitar_on: bool,
    pub guitar_gain: f32,
}

pub enum ArpEvent {
    ToggleArp,
    ToggleLatch,
    Clear,
    ToggleWhammy,
    ToggleGuitar,
    GuitarGain,
    Pattern(ArpPattern),
    Rate(ArpDivision),
    Tempo,
    Octaves,
    Gate,
    Preset(ArpPresetId),
}

pub fn arp_panel(ui: &mut Ui, state: &mut ArpUiState) -> Vec<ArpEvent> {
    let mut events = Vec::new();
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(0x19, 0x19, 0x1d, 153))
        .stroke(theme::panel_stroke())
        .corner_radius(theme::rounding_lg())
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::section_label("ARP"));
                if chip(ui, if state.arp_on { "On" } else { "Off" }, state.arp_on).clicked() {
                    events.push(ArpEvent::ToggleArp);
                }
                if chip(ui, "Latch", state.arp_latch).clicked() {
                    events.push(ArpEvent::ToggleLatch);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let enabled = state.arp_on || state.arp_preset.is_some();
                    if ui
                        .add_enabled(
                            enabled,
                            egui::Button::new(egui::RichText::new("Clear").size(12.0).color(MUTED))
                                .fill(Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        events.push(ArpEvent::Clear);
                    }
                });
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(theme::section_label("WHAMMY"));
                if chip(
                    ui,
                    if state.whammy_on { "On" } else { "Off" },
                    state.whammy_on,
                )
                .clicked()
                {
                    events.push(ArpEvent::ToggleWhammy);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("−1 / 0 / +1")
                            .monospace()
                            .size(11.0)
                            .color(MUTED),
                    );
                });
            });
            whammy_strip(ui, state.whammy_on, state.whammy_step);

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(theme::section_label("GUITAR"));
                if chip(
                    ui,
                    if state.guitar_on { "On" } else { "Off" },
                    state.guitar_on,
                )
                .clicked()
                {
                    events.push(ArpEvent::ToggleGuitar);
                }
                ui.label(
                    egui::RichText::new("poly FFT · acorde inteiro −1 / 0 / +1")
                        .size(11.0)
                        .color(MUTED),
                );
            });
            ui.horizontal(|ui| {
                if knob(
                    ui,
                    "gtr-gain",
                    "Gain",
                    &mut state.guitar_gain,
                    0.0,
                    1.5,
                    0.01,
                    false,
                    format_pct,
                ) {
                    events.push(ArpEvent::GuitarGain);
                }
            });

            ui.add_space(6.0);
            ui.label(theme::section_label("MOTION"));
            ui.horizontal_wrapped(|ui| {
                for (pat, label) in ArpPattern::ALL {
                    if chip(ui, label, state.arp_pattern == pat).clicked() {
                        events.push(ArpEvent::Pattern(pat));
                    }
                }
            });

            ui.label(theme::section_label("RATE"));
            ui.horizontal_wrapped(|ui| {
                for (rate, label) in ArpDivision::ALL {
                    if chip(ui, label, state.arp_rate == rate).clicked() {
                        events.push(ArpEvent::Rate(rate));
                    }
                }
            });

            ui.vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 28.0;
                    if knob(
                        ui,
                        "tempo",
                        "Tempo",
                        &mut state.arp_tempo,
                        70.0,
                        180.0,
                        1.0,
                        false,
                        |v| format!("{}", v.round()),
                    ) {
                        events.push(ArpEvent::Tempo);
                    }
                    if knob(
                        ui,
                        "oct",
                        "Oct",
                        &mut state.arp_octaves,
                        1.0,
                        3.0,
                        1.0,
                        false,
                        |v| format!("{}", v.round()),
                    ) {
                        events.push(ArpEvent::Octaves);
                    }
                    if knob(
                        ui,
                        "gate",
                        "Gate",
                        &mut state.arp_gate,
                        0.15,
                        0.95,
                        0.01,
                        false,
                        format_pct,
                    ) {
                        events.push(ArpEvent::Gate);
                    }
                });
            });

            ui.label(theme::section_label("LOOPS"));
            ui.horizontal_wrapped(|ui| {
                for preset in ARP_PRESETS {
                    let active = state.arp_preset == Some(preset.id);
                    if chip(ui, preset.label, active).clicked() {
                        events.push(ArpEvent::Preset(preset.id));
                    }
                }
            });
        });
    events
}

fn whammy_strip(ui: &mut Ui, on: bool, step: usize) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
    let n = WHAMMY_SEQUENCE.len();
    let gap = 2.0;
    let w = (rect.width() - gap * (n as f32 - 1.0)) / n as f32;
    for (i, semitones) in WHAMMY_SEQUENCE.iter().enumerate() {
        let x = rect.left() + i as f32 * (w + gap);
        let cell = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, rect.height()));
        let current = on && i == step;
        ui.painter()
            .rect_filled(cell, 3.0, if current { ACCENT } else { SURFACE });
        let dot_y = if *semitones > 0 {
            cell.top() + 4.0
        } else if *semitones == 0 {
            cell.center().y
        } else {
            cell.bottom() - 4.0
        };
        ui.painter().circle_filled(
            Pos2::new(cell.center().x, dot_y),
            2.0,
            if current { ACCENT_FG } else { MUTED },
        );
    }
}
