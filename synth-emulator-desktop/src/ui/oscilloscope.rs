use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use super::theme::{self, ACCENT, ELEVATED, GLOW, SUBTLE};

pub fn oscilloscope(ui: &mut Ui, time_domain: &[f32], peak: f32) {
    ui.horizontal(|ui| {
        let height = ui.available_height().clamp(140.0, 280.0);
        let meter_w = 10.0;
        let gap = 8.0;
        let width = (ui.available_width() - meter_w - gap).max(80.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        paint_scope(ui, rect, time_domain);
        let (meter, _) = ui.allocate_exact_size(Vec2::new(meter_w, height), Sense::hover());
        ui.painter().rect_filled(meter, 8.0, ELEVATED);
        let h = (peak.clamp(0.0, 1.0)) * meter.height();
        let bar = Rect::from_min_max(
            Pos2::new(meter.left(), meter.bottom() - h),
            Pos2::new(meter.right(), meter.bottom()),
        );
        ui.painter().rect_filled(bar, 8.0, ACCENT);
    });
}

fn paint_scope(ui: &Ui, rect: Rect, time_domain: &[f32]) {
    let painter = ui.painter();
    painter.rect_filled(rect, theme::rounding_md(), ELEVATED);
    painter.rect_stroke(
        rect,
        theme::rounding_md(),
        Stroke::new(1.0, Color32::from_white_alpha(18)),
        egui::StrokeKind::Outside,
    );
    painter.line_segment(
        [
            Pos2::new(rect.left(), rect.center().y),
            Pos2::new(rect.right(), rect.center().y),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(16)),
    );
    if time_domain.len() < 2 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "SCOPE",
            egui::FontId::proportional(12.0),
            SUBTLE,
        );
        return;
    }
    let n = time_domain.len().min(1024);
    let step = time_domain.len() / n;
    let mut pts = Vec::with_capacity(n);
    for i in 0..n {
        let v = time_domain[i * step];
        let x = rect.left() + (i as f32 / (n - 1) as f32) * rect.width();
        let y = rect.center().y - v * 0.42 * rect.height();
        pts.push(Pos2::new(x, y.clamp(rect.top() + 2.0, rect.bottom() - 2.0)));
    }
    painter.add(Shape::line(pts, Stroke::new(1.5, GLOW)));
}
