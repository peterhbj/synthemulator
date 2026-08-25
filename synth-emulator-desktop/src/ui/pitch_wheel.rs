use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use super::theme::{self, ACCENT, ACCENT_FG, BORDER, ELEVATED, FG, MUTED};
use crate::synth::BEND_RANGES;

pub fn pitch_wheel(ui: &mut Ui, bend: &mut f32, bend_range: &mut u8, dragging: &mut bool) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        ui.set_width(64.0);
        ui.set_max_width(64.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.label(theme::section_label("BEND"));
            let height = 148.0;
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(40.0, height), Sense::click_and_drag());
            if response.drag_started() {
                *dragging = true;
            }
            if response.dragged() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let mid = rect.center().y;
                    let t = (mid - pos.y) / (rect.height() / 2.0 - 8.0);
                    *bend = t.clamp(-1.0, 1.0);
                    changed = true;
                }
            }
            if response.drag_stopped() {
                *dragging = false;
            }
            paint_wheel(ui, rect, *bend);

            let semitones = *bend * *bend_range as f32;
            let label = if semitones.abs() < 0.05 {
                "0.0".to_string()
            } else {
                format!("{semitones:+.1}")
            };
            ui.label(
                egui::RichText::new(label)
                    .monospace()
                    .size(11.0)
                    .color(MUTED),
            );

            egui::Frame::new()
                .fill(ELEVATED)
                .corner_radius(theme::rounding_sm())
                .inner_margin(egui::Margin::symmetric(2, 2))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    ui.horizontal(|ui| {
                        for range in BEND_RANGES {
                            let active = *bend_range == range;
                            let fill = if active { ACCENT } else { Color32::TRANSPARENT };
                            let color = if active { ACCENT_FG } else { MUTED };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(range.to_string())
                                            .monospace()
                                            .size(10.0)
                                            .color(color),
                                    )
                                    .fill(fill)
                                    .min_size(Vec2::new(18.0, 22.0))
                                    .corner_radius(4.0),
                                )
                                .clicked()
                            {
                                *bend_range = range;
                                changed = true;
                            }
                        }
                    });
                });
        });
    });
    changed
}

fn paint_wheel(ui: &Ui, rect: Rect, bend: f32) {
    let painter = ui.painter();
    painter.rect_filled(rect, 20.0, ELEVATED);
    painter.rect_stroke(
        rect,
        20.0,
        Stroke::new(1.0, Color32::from_white_alpha(18)),
        egui::StrokeKind::Outside,
    );
    let mid_y = rect.center().y;
    painter.line_segment(
        [
            Pos2::new(rect.left() + 4.0, mid_y),
            Pos2::new(rect.right() - 4.0, mid_y),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(40)),
    );
    let groove_top = mid_y - 32.0 - bend * 28.0;
    for i in 0..12 {
        let y = groove_top + i as f32 * 5.0;
        if y > rect.top() + 4.0 && y < rect.bottom() - 4.0 {
            painter.line_segment(
                [
                    Pos2::new(rect.left() + 2.0, y),
                    Pos2::new(rect.right() - 2.0, y),
                ],
                Stroke::new(2.0, BORDER),
            );
        }
    }
    let knob_y = rect.center().y - bend * rect.height() * 0.38;
    let knob = Pos2::new(rect.center().x, knob_y);
    let fill = if bend.abs() > 0.04 { ACCENT } else { FG };
    painter.circle_filled(knob, 14.0, fill);
}
