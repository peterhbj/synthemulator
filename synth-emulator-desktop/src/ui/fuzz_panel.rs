use eframe::egui::{self, Color32, Ui};

use super::theme::{self, MUTED};
use super::widgets::{chip, format_pct, knob};
use crate::audio::fuzz_factory::FuzzKnobs;

pub struct FuzzUiState {
    pub on: bool,
    pub knobs: FuzzKnobs,
}

impl Default for FuzzUiState {
    fn default() -> Self {
        Self {
            on: false,
            knobs: FuzzKnobs::default(),
        }
    }
}

pub enum FuzzEvent {
    Toggle,
    Knobs,
    Preset(FuzzKnobs),
    Learn,
}

pub fn fuzz_panel(
    ui: &mut Ui,
    state: &mut FuzzUiState,
    va: f32,
    cc: u8,
    learning: bool,
) -> Vec<FuzzEvent> {
    let mut events = Vec::new();
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(0x1c, 0x14, 0x12, 170))
        .stroke(theme::panel_stroke())
        .corner_radius(theme::rounding_lg())
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::section_label("FUZZ FACTORY"));
                if chip(ui, if state.on { "On" } else { "Off" }, state.on).clicked() {
                    events.push(FuzzEvent::Toggle);
                }
                if chip(
                    ui,
                    if learning {
                        "pisa o CTL…"
                    } else {
                        "Learn"
                    },
                    learning,
                )
                .clicked()
                {
                    events.push(FuzzEvent::Learn);
                }
                ui.label(
                    egui::RichText::new(format!("VA {va:.2} V"))
                        .monospace()
                        .size(11.0)
                        .color(MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(if learning {
                            "pisa o pedal da GT-100 pra gravar".into()
                        } else {
                            format!("CC {cc} tap=on/off · knobs 16–20")
                        })
                        .size(11.0)
                        .color(MUTED),
                    );
                });
            });

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if chip(ui, "Smooth", false).clicked() {
                    events.push(FuzzEvent::Preset(FuzzKnobs::smooth()));
                }
                if chip(ui, "Velcro", false).clicked() {
                    events.push(FuzzEvent::Preset(FuzzKnobs::velcro()));
                }
                if chip(ui, "Squeal", false).clicked() {
                    events.push(FuzzEvent::Preset(FuzzKnobs::squeal()));
                }
                if chip(ui, "Hi Comp", false).clicked() {
                    events.push(FuzzEvent::Preset(FuzzKnobs::gated_hi()));
                }
                if chip(ui, "Plug In", false).clicked() {
                    events.push(FuzzEvent::Preset(FuzzKnobs::plug()));
                }
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if knob(
                    ui,
                    "ff-vol",
                    "Vol",
                    &mut state.knobs.vol,
                    0.0,
                    1.0,
                    0.0,
                    false,
                    format_pct,
                ) {
                    events.push(FuzzEvent::Knobs);
                }
                if knob(
                    ui,
                    "ff-gate",
                    "Gate",
                    &mut state.knobs.gate,
                    0.0,
                    1.0,
                    0.0,
                    false,
                    format_pct,
                ) {
                    events.push(FuzzEvent::Knobs);
                }
                if knob(
                    ui,
                    "ff-comp",
                    "Comp",
                    &mut state.knobs.comp,
                    0.0,
                    1.0,
                    0.0,
                    false,
                    format_pct,
                ) {
                    events.push(FuzzEvent::Knobs);
                }
                if knob(
                    ui,
                    "ff-drive",
                    "Drive",
                    &mut state.knobs.drive,
                    0.0,
                    1.0,
                    0.0,
                    false,
                    format_pct,
                ) {
                    events.push(FuzzEvent::Knobs);
                }
                if knob(
                    ui,
                    "ff-stab",
                    "Stab",
                    &mut state.knobs.stab,
                    0.0,
                    1.0,
                    0.0,
                    false,
                    format_pct,
                ) {
                    events.push(FuzzEvent::Knobs);
                }
            });
        });
    events
}
