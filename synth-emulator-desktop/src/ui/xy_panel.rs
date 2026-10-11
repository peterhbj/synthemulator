//! F10 XY pad: grabs the touchpad (or locks the cursor) and turns finger
//! position into two assigned parameters.

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use xypad::{RelativePad, Touchpad};

use super::theme::{self, ACCENT, ELEVATED, FG, MUTED, SUBTLE, SURFACE};
use crate::xy::XyTarget;

/// One parameter change requested by the pad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XyWrite {
    pub target: XyTarget,
    pub value: f32,
}

pub struct XyUi {
    pub active: bool,
    pad: Option<Box<dyn Touchpad>>,
    pad_err: Option<String>,
    relative: RelativePad,
    pos: [f32; 2],
    touching: bool,
    /// Parameter values from before the finger landed (spring back).
    saved: Option<[(XyTarget, f32); 2]>,
    last_sent: [Option<f32>; 2],
}

impl Default for XyUi {
    fn default() -> Self {
        Self::new()
    }
}

impl XyUi {
    pub fn new() -> Self {
        Self {
            active: false,
            pad: None,
            pad_err: None,
            relative: RelativePad::new(),
            pos: [0.5, 0.5],
            touching: false,
            saved: None,
            last_sent: [None, None],
        }
    }

    pub fn absolute(&self) -> bool {
        self.pad.is_some()
    }

    pub fn status(&self) -> String {
        match (&self.pad, &self.pad_err) {
            (Some(p), _) => format!("touchpad: {}", p.name()),
            (None, Some(e)) => format!("modo relativo ({e})"),
            (None, None) => "aperte F10 para ativar".into(),
        }
    }

    /// Enter or leave XY mode. Leaving may return writes (spring back).
    pub fn toggle(
        &mut self,
        ctx: &egui::Context,
        targets: [XyTarget; 2],
        spring_back: bool,
        current: &dyn Fn(XyTarget) -> f32,
    ) -> Vec<XyWrite> {
        if self.active {
            self.exit(ctx, spring_back)
        } else {
            self.enter(ctx, targets, current);
            Vec::new()
        }
    }

    fn enter(&mut self, ctx: &egui::Context, targets: [XyTarget; 2], current: &dyn Fn(XyTarget) -> f32) {
        if self.pad.is_none() {
            match xypad::open_touchpad() {
                Ok(p) => {
                    self.pad = Some(p);
                    self.pad_err = None;
                }
                Err(e) => self.pad_err = Some(e),
            }
        }
        if let Some(pad) = self.pad.as_mut() {
            if let Err(e) = pad.set_grab(true) {
                self.pad_err = Some(e);
                self.pad = None;
            }
        }
        if self.pad.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::Confined));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
        }
        self.pos = [targets[0].to_norm(current(targets[0])), targets[1].to_norm(current(targets[1]))];
        self.relative.set(self.pos[0], self.pos[1]);
        self.touching = false;
        self.saved = None;
        self.last_sent = [None, None];
        self.active = true;
    }

    /// Leave XY mode and give the touchpad / cursor back.
    pub fn exit(&mut self, ctx: &egui::Context, spring_back: bool) -> Vec<XyWrite> {
        if !self.active {
            return Vec::new();
        }
        self.active = false;
        if let Some(pad) = self.pad.as_mut() {
            let _ = pad.set_grab(false);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        }
        self.touching = false;
        let restore = self.saved.take();
        match restore {
            Some(saved) if spring_back => saved
                .iter()
                .filter(|(t, _)| *t != XyTarget::None)
                .map(|(t, v)| XyWrite { target: *t, value: *v })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Read the pad and return the parameter changes for this frame.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        targets: [XyTarget; 2],
        spring_back: bool,
        sensitivity: f32,
        current: &dyn Fn(XyTarget) -> f32,
    ) -> Vec<XyWrite> {
        let mut writes = Vec::new();
        if !self.active {
            return writes;
        }
        // Never keep the touchpad grabbed while another window has focus.
        if !ctx.input(|i| i.focused) {
            return self.exit(ctx, spring_back);
        }
        let snapshot = |targets: [XyTarget; 2]| [(targets[0], current(targets[0])), (targets[1], current(targets[1]))];
        if let Some(pad) = &self.pad {
            let t = pad.latest();
            if t.touching {
                if !self.touching {
                    self.touching = true;
                    self.saved = Some(snapshot(targets));
                }
                self.pos = [t.x.clamp(0.0, 1.0), t.y.clamp(0.0, 1.0)];
            } else if self.touching {
                self.touching = false;
                if let Some(saved) = self.saved.take() {
                    if spring_back {
                        for (target, value) in saved {
                            if target != XyTarget::None {
                                writes.push(XyWrite { target, value });
                            }
                        }
                        self.pos = [targets[0].to_norm(saved[0].1), targets[1].to_norm(saved[1].1)];
                    }
                }
                self.last_sent = [None, None];
                return writes;
            }
        } else {
            let (mut dx, mut dy, mut raw) = (0.0f32, 0.0f32, false);
            ctx.input(|i| {
                for e in &i.events {
                    if let egui::Event::MouseMoved(d) = e {
                        dx += d.x;
                        dy += d.y;
                        raw = true;
                    }
                }
                if !raw {
                    let d = i.pointer.delta();
                    dx = d.x;
                    dy = d.y;
                }
            });
            self.relative.sensitivity = sensitivity;
            if self.saved.is_none() {
                self.saved = Some(snapshot(targets));
            }
            self.touching = true;
            let t = self.relative.apply_delta(dx, dy);
            self.pos = [t.x, t.y];
        }
        if self.touching {
            for axis in 0..2 {
                let target = targets[axis];
                if target == XyTarget::None {
                    continue;
                }
                let value = target.from_norm(self.pos[axis]);
                if self.last_sent[axis].is_none_or(|v| (v - value).abs() > 1e-4) {
                    self.last_sent[axis] = Some(value);
                    writes.push(XyWrite { target, value });
                }
            }
        }
        writes
    }

    /// Big translucent pad in the middle of the window while active.
    pub fn overlay(&self, ctx: &egui::Context, targets: [XyTarget; 2], current: &dyn Fn(XyTarget) -> f32) {
        if !self.active {
            return;
        }
        let screen = ctx.screen_rect();
        let side = (screen.width().min(screen.height()) * 0.5).clamp(220.0, 420.0);
        egui::Area::new(egui::Id::new("xy-overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(Pos2::new(screen.center().x - side * 0.5 - 16.0, screen.center().y - side * 0.5 - 52.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(SURFACE.gamma_multiply(0.96))
                    .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.5)))
                    .corner_radius(theme::rounding_lg())
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("XY PAD").size(13.0).color(FG).strong());
                            ui.label(RichText::new(self.status()).size(11.0).color(MUTED));
                        });
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
                        let p = ui.painter_at(rect);
                        p.rect_filled(rect, 12.0, ELEVATED);
                        for i in 1..4 {
                            let f = i as f32 / 4.0;
                            let grid = Stroke::new(1.0, Color32::from_white_alpha(if i == 2 { 40 } else { 16 }));
                            p.line_segment([Pos2::new(rect.left() + rect.width() * f, rect.top()), Pos2::new(rect.left() + rect.width() * f, rect.bottom())], grid);
                            p.line_segment([Pos2::new(rect.left(), rect.top() + rect.height() * f), Pos2::new(rect.right(), rect.top() + rect.height() * f)], grid);
                        }
                        let puck = Pos2::new(
                            rect.left() + self.pos[0] * rect.width(),
                            rect.bottom() - self.pos[1] * rect.height(),
                        );
                        let lit = Color32::from_rgb(112, 172, 255);
                        p.line_segment([Pos2::new(puck.x, rect.top()), Pos2::new(puck.x, rect.bottom())], Stroke::new(1.0, lit.gamma_multiply(0.4)));
                        p.line_segment([Pos2::new(rect.left(), puck.y), Pos2::new(rect.right(), puck.y)], Stroke::new(1.0, lit.gamma_multiply(0.4)));
                        p.circle_filled(puck, if self.touching { 13.0 } else { 9.0 }, if self.touching { lit } else { SUBTLE });
                        axis_label(&p, rect, 0, targets[0], current(targets[0]));
                        axis_label(&p, rect, 1, targets[1], current(targets[1]));
                        let hint = if self.absolute() {
                            "Toque no touchpad. F10 ou Esc sai."
                        } else {
                            "Mova o touchpad/mouse. F10 ou Esc sai."
                        };
                        ui.label(RichText::new(hint).size(11.0).color(MUTED));
                    });
            });
    }

    /// Assignment row, shown under the tabs.
    pub fn settings_row(
        &self,
        ui: &mut egui::Ui,
        targets: &mut [XyTarget; 2],
        spring_back: &mut bool,
        sensitivity: &mut f32,
    ) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::section_label("XY PAD · F10"));
            for (axis, name) in ["X", "Y"].into_iter().enumerate() {
                ui.label(RichText::new(name).size(12.0).color(MUTED));
                egui::ComboBox::from_id_salt(("xy-target", axis))
                    .width(150.0)
                    .selected_text(targets[axis].label())
                    .show_ui(ui, |ui| {
                        for t in XyTarget::ALL {
                            if ui.selectable_label(targets[axis] == t, t.label()).clicked() && targets[axis] != t {
                                targets[axis] = t;
                                changed = true;
                            }
                        }
                    });
            }
            changed |= ui
                .checkbox(spring_back, "volta ao soltar")
                .on_hover_text("Ao tirar o dedo (ou sair com F10), os parâmetros voltam ao valor anterior")
                .changed();
            if !self.absolute() {
                ui.label(RichText::new("sensib.").size(12.0).color(MUTED));
                ui.spacing_mut().slider_width = 80.0;
                changed |= ui
                    .add(egui::Slider::new(sensitivity, 0.1..=1.5).show_value(false))
                    .changed();
            }
            ui.label(RichText::new(self.status()).size(11.0).color(SUBTLE));
        });
        changed
    }
}

impl Drop for XyUi {
    fn drop(&mut self) {
        if let Some(pad) = self.pad.as_mut() {
            let _ = pad.set_grab(false);
        }
    }
}

fn axis_label(p: &egui::Painter, rect: Rect, axis: usize, target: XyTarget, value: f32) {
    if target == XyTarget::None {
        return;
    }
    let text = format!("{} {}: {}", ["X", "Y"][axis], target.label(), target.format(value));
    let (pos, align) = if axis == 0 {
        (Pos2::new(rect.right() - 8.0, rect.bottom() - 8.0), egui::Align2::RIGHT_BOTTOM)
    } else {
        (Pos2::new(rect.left() + 8.0, rect.top() + 8.0), egui::Align2::LEFT_TOP)
    };
    p.text(pos, align, text, egui::FontId::proportional(12.0), FG);
}
