use std::collections::HashSet;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Ui, Vec2};

use super::theme::{ACCENT, FG, GLOW, KEY, KEY_FG, KEY_SHARP, KEY_SHARP_FG};
use crate::synth::{
    count_white_keys, is_black_key, midi_to_name, octave_base_midi, offset_key_label,
    visible_midis, white_key_index,
};

pub fn piano(
    ui: &mut Ui,
    octave: i32,
    active: &HashSet<u8>,
    pooled: &HashSet<u8>,
    pointer_held: &mut HashSet<u8>,
) -> Vec<(u8, bool)> {
    let mut events = Vec::new();
    let midis = visible_midis(octave);
    let white_count = count_white_keys(&midis).max(1);
    let base = octave_base_midi(octave);
    let height = 176.0;
    let width = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());

    let whites: Vec<u8> = midis
        .iter()
        .copied()
        .filter(|m| !is_black_key(*m))
        .collect();
    let blacks: Vec<u8> = midis.iter().copied().filter(|m| is_black_key(*m)).collect();
    let white_w = rect.width() / white_count as f32;

    for midi in whites {
        let idx = white_key_index(midi, base);
        let key = Rect::from_min_size(
            Pos2::new(rect.left() + idx as f32 * white_w + 1.0, rect.top()),
            Vec2::new(white_w - 2.0, rect.height()),
        );
        let lit = active.contains(&midi);
        let queued = pooled.contains(&midi);
        let fill = if lit { GLOW } else { KEY };
        let rounding = egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 6,
            se: 6,
        };
        ui.painter().rect_filled(key, rounding, fill);
        if queued && !lit {
            ui.painter().rect_stroke(
                key.shrink(1.0),
                rounding,
                egui::Stroke::new(1.0, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let id = ui.id().with(("white", midi));
        let resp = ui.interact(key, id, Sense::click_and_drag());
        handle_key(&resp, midi, pointer_held, &mut events);
        let label = offset_key_label(midi as i32 - base as i32).unwrap_or("");
        ui.painter().text(
            Pos2::new(key.center().x, key.bottom() - 28.0),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(11.0),
            Color32::from_rgba_unmultiplied(KEY_FG.r(), KEY_FG.g(), KEY_FG.b(), 110),
        );
        ui.painter().text(
            Pos2::new(key.center().x, key.bottom() - 12.0),
            egui::Align2::CENTER_CENTER,
            midi_to_name(midi),
            egui::FontId::monospace(11.0),
            Color32::from_rgba_unmultiplied(KEY_FG.r(), KEY_FG.g(), KEY_FG.b(), 140),
        );
    }

    for midi in blacks {
        let after = white_key_index(midi, base);
        let width = white_w * 0.62;
        let left = rect.left() + after as f32 * white_w - width / 2.0;
        let key = Rect::from_min_size(
            Pos2::new(left, rect.top()),
            Vec2::new(width, rect.height() * 0.58),
        );
        let lit = active.contains(&midi);
        let queued = pooled.contains(&midi);
        let fill = if lit { FG } else { KEY_SHARP };
        let rounding = egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 6,
            se: 6,
        };
        ui.painter().rect_filled(key, rounding, fill);
        if queued && !lit {
            ui.painter().rect_stroke(
                key.shrink(1.0),
                rounding,
                egui::Stroke::new(1.0, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let id = ui.id().with(("black", midi));
        let resp = ui.interact(key, id, Sense::click_and_drag());
        handle_key(&resp, midi, pointer_held, &mut events);
        let label = offset_key_label(midi as i32 - base as i32).unwrap_or("");
        let color = if lit { ACCENT } else { KEY_SHARP_FG };
        ui.painter().text(
            Pos2::new(key.center().x, key.bottom() - 12.0),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(11.0),
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 140),
        );
    }

    events
}

fn handle_key(
    resp: &egui::Response,
    midi: u8,
    pointer_held: &mut HashSet<u8>,
    events: &mut Vec<(u8, bool)>,
) {
    if resp.drag_started() && pointer_held.insert(midi) {
        events.push((midi, true));
    }
    if resp.drag_stopped() && pointer_held.remove(&midi) {
        events.push((midi, false));
    }
}
