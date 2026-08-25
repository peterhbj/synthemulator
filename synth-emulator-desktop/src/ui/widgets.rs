use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};

use super::theme::{self, ACCENT, ACCENT_FG, ELEVATED, FG, MUTED, SUBTLE};
use crate::synth::Waveform;

pub fn format_hz(value: f32) -> String {
    if value >= 1000.0 {
        format!("{:.1} kHz", value / 1000.0)
    } else {
        format!("{} Hz", value.round())
    }
}

pub fn format_ms(value: f32) -> String {
    if value < 1.0 {
        format!("{} ms", (value * 1000.0).round())
    } else {
        format!("{value:.2} s")
    }
}

pub fn format_pct(value: f32) -> String {
    format!("{}%", (value * 100.0).round())
}

fn to_norm(value: f32, min: f32, max: f32, log: bool) -> f32 {
    if log {
        let a = min.max(0.0001).ln();
        let b = max.max(0.0001).ln();
        (value.max(0.0001).ln() - a) / (b - a)
    } else {
        (value - min) / (max - min)
    }
}

fn from_norm(t: f32, min: f32, max: f32, log: bool) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if log {
        let a = min.max(0.0001).ln();
        let b = max.max(0.0001).ln();
        (a + t * (b - a)).exp()
    } else {
        min + t * (max - min)
    }
}

pub fn knob(
    ui: &mut Ui,
    id: &'static str,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    step: f32,
    logarithmic: bool,
    format: impl Fn(f32) -> String,
) -> bool {
    let mut changed = false;
    ui.push_id(id, |ui| {
        ui.vertical(|ui| {
            ui.set_width(56.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.label(theme::section_label(label));
                let desired = Vec2::splat(52.0);
                let (rect, response) = ui.allocate_exact_size(desired, Sense::drag());
                let t = to_norm(*value, min, max, logarithmic).clamp(0.0, 1.0);
                if response.dragged() {
                    let dy = -response.drag_delta().y;
                    let next_t = (t + dy / 140.0).clamp(0.0, 1.0);
                    let mut next = from_norm(next_t, min, max, logarithmic);
                    if step > 0.0 {
                        next = (next / step).round() * step;
                    }
                    *value = next.clamp(min, max);
                    changed = true;
                }
                paint_knob(ui, rect, t, response.hovered() || response.dragged());
                ui.label(
                    egui::RichText::new(format(*value))
                        .monospace()
                        .size(11.0)
                        .color(MUTED),
                );
            });
        });
    });
    changed
}

fn paint_knob(ui: &Ui, rect: Rect, t: f32, hot: bool) {
    let painter = ui.painter();
    let center = rect.center();
    let radius = rect.width() * 0.5 - 2.0;
    painter.circle_filled(center, radius, ELEVATED);
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(
            1.0,
            if hot {
                Color32::from_white_alpha(32)
            } else {
                Color32::from_white_alpha(18)
            },
        ),
    );
    let r_arc = radius - 6.0;
    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    let steps = 48;
    let mut bg = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let a = start + sweep * (i as f32 / steps as f32);
        bg.push(Pos2::new(
            center.x + r_arc * a.cos(),
            center.y + r_arc * a.sin(),
        ));
    }
    painter.add(Shape::line(
        bg,
        Stroke::new(1.25, Color32::from_white_alpha(28)),
    ));
    let lit = (t * steps as f32).round() as usize;
    if lit > 0 {
        let mut fg = Vec::with_capacity(lit + 1);
        for i in 0..=lit {
            let a = start + sweep * (i as f32 / steps as f32);
            fg.push(Pos2::new(
                center.x + r_arc * a.cos(),
                center.y + r_arc * a.sin(),
            ));
        }
        painter.add(Shape::line(fg, Stroke::new(1.75, ACCENT)));
    }
    let a = std::f32::consts::PI * 0.75 + t * sweep;
    let inner = Pos2::new(center.x, center.y);
    let outer = Pos2::new(
        center.x + (radius - 10.0) * a.cos(),
        center.y + (radius - 10.0) * a.sin(),
    );
    painter.line_segment([inner, outer], Stroke::new(2.0, FG));
}

pub fn chip(ui: &mut Ui, label: &str, active: bool) -> Response {
    let fill = if active { ACCENT } else { ELEVATED };
    let text = if active { ACCENT_FG } else { MUTED };
    let button = egui::Button::new(
        egui::RichText::new(label.to_uppercase())
            .size(11.0)
            .extra_letter_spacing(0.8)
            .color(text),
    )
    .fill(fill)
    .corner_radius(theme::rounding_sm())
    .min_size(Vec2::new(0.0, 32.0));
    ui.add(button)
}

pub fn icon_button(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(egui::RichText::new(label).size(16.0).color(FG))
            .fill(ELEVATED)
            .corner_radius(20.0)
            .min_size(Vec2::splat(32.0)),
    )
}

pub fn waveform_select(ui: &mut Ui, value: &mut Waveform) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        ui.label(theme::section_label("WAVE"));
        egui::Frame::new()
            .fill(ELEVATED)
            .corner_radius(theme::rounding_md())
            .inner_margin(egui::Margin::same(4))
            .show(ui, |ui| {
                ui.columns(4, |cols| {
                    for (i, wave) in Waveform::ALL.iter().enumerate() {
                        let active = *value == *wave;
                        let fill = if active { ACCENT } else { Color32::TRANSPARENT };
                        let color = if active { ACCENT_FG } else { MUTED };
                        let resp = cols[i].add(
                            egui::Button::new("")
                                .fill(fill)
                                .corner_radius(theme::rounding_sm())
                                .min_size(Vec2::new(0.0, 48.0)),
                        );
                        paint_wave_icon(&mut cols[i], resp.rect, *wave, color, wave.label());
                        if resp.clicked() {
                            *value = *wave;
                            changed = true;
                        }
                    }
                });
            });
    });
    changed
}

fn paint_wave_icon(ui: &mut Ui, rect: Rect, wave: Waveform, color: Color32, label: &str) {
    let painter = ui.painter();
    let icon = Rect::from_center_size(rect.center() - Vec2::new(0.0, 7.0), Vec2::new(28.0, 16.0));
    let d = match wave {
        Waveform::Sine => vec![
            (0.0, 0.5),
            (0.15, 0.5),
            (0.25, 0.1),
            (0.4, 0.1),
            (0.5, 0.9),
            (0.65, 0.9),
            (0.75, 0.5),
            (1.0, 0.5),
        ],
        Waveform::Triangle => vec![(0.0, 0.9), (0.33, 0.1), (0.66, 0.9), (1.0, 0.1)],
        Waveform::Sawtooth => vec![(0.0, 0.9), (0.0, 0.1), (1.0, 0.9)],
        Waveform::Square => vec![
            (0.0, 0.9),
            (0.0, 0.1),
            (0.5, 0.1),
            (0.5, 0.9),
            (1.0, 0.9),
            (1.0, 0.1),
        ],
    };
    let pts: Vec<Pos2> = d
        .into_iter()
        .map(|(x, y)| {
            Pos2::new(
                icon.left() + x * icon.width(),
                icon.top() + y * icon.height(),
            )
        })
        .collect();
    painter.add(Shape::line(pts, Stroke::new(1.75, color)));
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 8.0),
        egui::Align2::CENTER_CENTER,
        label.to_uppercase(),
        egui::FontId::proportional(10.5),
        color,
    );
}

pub fn volume_slider(ui: &mut Ui, volume: &mut f32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("VOL")
                .size(10.5)
                .color(MUTED)
                .extra_letter_spacing(1.0),
        );
        let w = ui.available_width() - 40.0;
        let height = 18.0;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(w.max(80.0), height), Sense::click_and_drag());
        if response.dragged() || response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let t = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                *volume = t;
                changed = true;
            }
        }
        let painter = ui.painter();
        let y = rect.center().y;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(2.0, Color32::from_white_alpha(20)),
        );
        let x = rect.left() + *volume * rect.width();
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(x, y)],
            Stroke::new(2.0, SUBTLE),
        );
        painter.circle_filled(Pos2::new(x, y), 6.0, ACCENT);
        ui.label(
            egui::RichText::new(format_pct(*volume))
                .monospace()
                .size(11.0)
                .color(MUTED),
        );
    });
    changed
}

pub fn badge_pill(ui: &mut Ui, label: &str, on: bool) {
    let fill = if on { ACCENT } else { ELEVATED };
    let color = if on { ACCENT_FG } else { SUBTLE };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label.to_uppercase())
                    .size(10.5)
                    .extra_letter_spacing(1.2)
                    .color(color),
            );
        });
}
