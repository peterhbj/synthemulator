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

/// Frames to wait for the pinned cursor to reach the window before the grab
/// starts anyway.
const PIN_WAIT_FRAMES: u8 = 3;
/// How close (logical px) the pointer must get to the pin spot to grab early.
const PIN_NEAR_PX: f32 = 48.0;

/// How the touchpad is opened. A field so tests can stand in a fake pad.
type OpenPad = fn() -> Result<Box<dyn Touchpad>, String>;

pub struct XyUi {
    pub active: bool,
    pad: Option<Box<dyn Touchpad>>,
    pad_err: Option<String>,
    opener: OpenPad,
    relative: RelativePad,
    pos: [f32; 2],
    touching: bool,
    /// Parameter values from before the finger landed (spring back).
    saved: Option<[(XyTarget, f32); 2]>,
    last_sent: [Option<f32>; 2],
    /// Pad that cannot stop clicks: the grab waits until the pinned cursor
    /// is inside the window. Holds the frames waited so far.
    pending_grab: Option<u8>,
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
            opener: xypad::open_touchpad,
            relative: RelativePad::new(),
            pos: [0.5, 0.5],
            touching: false,
            saved: None,
            last_sent: [None, None],
            pending_grab: None,
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
        // A pad whose reader ended (device unplugged or re-enumerated) is opened again.
        if self.pad.as_ref().is_some_and(|p| !p.is_alive()) {
            self.pad = None;
        }
        if self.pad.is_none() {
            self.open_pad();
        }
        self.start_grab(ctx);
        self.pos = [targets[0].to_norm(current(targets[0])), targets[1].to_norm(current(targets[1]))];
        self.relative.set(self.pos[0], self.pos[1]);
        self.touching = false;
        self.saved = None;
        self.last_sent = [None, None];
        self.active = true;
    }

    fn open_pad(&mut self) {
        match (self.opener)() {
            Ok(pad) => {
                self.pad = Some(pad);
                self.pad_err = None;
            }
            Err(e) => {
                self.pad = None;
                self.pad_err = Some(e);
            }
        }
    }

    /// Start grabbing. An absolute pad that cannot stop clicks gets its cursor
    /// pinned inside the window first, so taps land on the shield, not on
    /// whatever app was under the cursor.
    fn start_grab(&mut self, ctx: &egui::Context) {
        if self.pad.as_ref().is_some_and(|p| !p.grab_stops_clicks()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(ctx.screen_rect().center()));
            self.pending_grab = Some(0);
        } else {
            self.finish_grab(ctx);
        }
    }

    /// Grab now: the pad, or the window's cursor when there is no pad.
    fn finish_grab(&mut self, ctx: &egui::Context) {
        self.pending_grab = None;
        if let Some(pad) = self.pad.as_mut() {
            match pad.set_grab(true) {
                Ok(()) => return,
                Err(e) => {
                    self.pad_err = Some(e);
                    self.pad = None;
                }
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::Confined));
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
    }

    /// Grab once the pointer is near the pin spot, or after a few frames.
    fn wait_for_pin(&mut self, ctx: &egui::Context, waited: u8) {
        let screen = ctx.screen_rect();
        let target = screen.center();
        let arrived = ctx
            .input(|i| i.pointer.latest_pos())
            .is_some_and(|p| screen.contains(p) && p.distance(target) <= PIN_NEAR_PX);
        if arrived || waited + 1 >= PIN_WAIT_FRAMES {
            self.finish_grab(ctx);
        } else {
            self.pending_grab = Some(waited + 1);
        }
    }

    /// Leave XY mode and give the touchpad / cursor back.
    pub fn exit(&mut self, ctx: &egui::Context, spring_back: bool) -> Vec<XyWrite> {
        if !self.active {
            return Vec::new();
        }
        self.active = false;
        self.pending_grab = None;
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

    /// Finger lifted: spring the parameters back (when enabled) and forget the touch.
    fn lift_finger(&mut self, targets: [XyTarget; 2], spring_back: bool) -> Vec<XyWrite> {
        self.touching = false;
        let mut writes = Vec::new();
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
        writes
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
        // Backends release the grab when nothing polls them, so frames keep coming.
        ctx.request_repaint();
        if self.pad.as_ref().is_some_and(|p| !p.is_alive()) {
            self.pad = None;
            self.open_pad();
            self.start_grab(ctx);
        }
        if let Some(waited) = self.pending_grab {
            self.wait_for_pin(ctx, waited);
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
                return self.lift_finger(targets, spring_back);
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
            for (axis, &target) in targets.iter().enumerate() {
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

    /// Relative mode and a pad that does not stop clicks leave taps and clicks
    /// reaching the widgets under the pointer, so the overlay has to shield them.
    fn shields_clicks(&self) -> bool {
        self.pad.as_ref().is_none_or(|p| !p.grab_stops_clicks())
    }

    /// Big translucent pad in the middle of the window while active.
    pub fn overlay(&self, ctx: &egui::Context, targets: [XyTarget; 2], current: &dyn Fn(XyTarget) -> f32) {
        if !self.active {
            return;
        }
        let screen = ctx.screen_rect();
        if self.shields_clicks() {
            // Covers the window: taps, clicks, drags and wheel stop here, so no
            // deck button, waveform or header chip underneath reacts.
            egui::Area::new(egui::Id::new("xy-shield"))
                .order(egui::Order::Foreground)
                .fixed_pos(screen.min)
                .show(ctx, |ui| {
                    ui.allocate_rect(screen, Sense::CLICK | Sense::DRAG);
                });
        }
        let side = (screen.width().min(screen.height()) * 0.5).clamp(220.0, 420.0);
        egui::Area::new(egui::Id::new("xy-overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(Pos2::new(screen.center().x - side * 0.5 - 16.0, screen.center().y - side * 0.5 - 52.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(SURFACE.gamma_multiply(0.96))
                    .stroke(Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.5)))
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
                            let grid = Stroke::new(1.0_f32, Color32::from_white_alpha(if i == 2 { 40 } else { 16 }));
                            p.line_segment([Pos2::new(rect.left() + rect.width() * f, rect.top()), Pos2::new(rect.left() + rect.width() * f, rect.bottom())], grid);
                            p.line_segment([Pos2::new(rect.left(), rect.top() + rect.height() * f), Pos2::new(rect.right(), rect.top() + rect.height() * f)], grid);
                        }
                        let puck = Pos2::new(
                            rect.left() + self.pos[0] * rect.width(),
                            rect.bottom() - self.pos[1] * rect.height(),
                        );
                        let lit = Color32::from_rgb(112, 172, 255);
                        p.line_segment([Pos2::new(puck.x, rect.top()), Pos2::new(puck.x, rect.bottom())], Stroke::new(1.0_f32, lit.gamma_multiply(0.4)));
                        p.line_segment([Pos2::new(rect.left(), puck.y), Pos2::new(rect.right(), puck.y)], Stroke::new(1.0_f32, lit.gamma_multiply(0.4)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use eframe::egui::{Event, Modifiers, PointerButton, RawInput, ViewportId};

    const TARGETS: [XyTarget; 2] = [XyTarget::MasterFilter, XyTarget::EchoMix];

    /// Stands in for a touchpad backend. `alive` is shared so a test can kill it.
    struct FakePad {
        name: &'static str,
        stops_clicks: bool,
        alive: Arc<AtomicBool>,
        grabbed: bool,
    }

    impl FakePad {
        fn boxed(name: &'static str, stops_clicks: bool) -> (Box<dyn Touchpad>, Arc<AtomicBool>) {
            let alive = Arc::new(AtomicBool::new(true));
            let pad = FakePad {
                name,
                stops_clicks,
                alive: alive.clone(),
                grabbed: false,
            };
            (Box::new(pad), alive)
        }
    }

    impl Touchpad for FakePad {
        fn name(&self) -> &str {
            self.name
        }
        fn latest(&self) -> xypad::Touch {
            xypad::Touch::default()
        }
        fn set_grab(&mut self, grab: bool) -> Result<(), String> {
            self.grabbed = grab;
            Ok(())
        }
        fn grabbed(&self) -> bool {
            self.grabbed
        }
        fn grab_stops_clicks(&self) -> bool {
            self.stops_clicks
        }
        fn is_alive(&self) -> bool {
            self.alive.load(Ordering::Relaxed)
        }
    }

    fn open_reopened() -> Result<Box<dyn Touchpad>, String> {
        Ok(FakePad::boxed("reopened", true).0)
    }

    fn open_fails() -> Result<Box<dyn Touchpad>, String> {
        Err("sem permissão (teste)".into())
    }

    /// One egui frame with `events`; `run` draws inside it.
    fn frame(ctx: &egui::Context, events: Vec<Event>, mut run: impl FnMut(&egui::Context)) -> egui::FullOutput {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 860.0))),
            focused: true,
            events,
            ..Default::default()
        };
        ctx.run(input, |ctx| run(ctx))
    }

    fn grabbed(xy: &XyUi) -> bool {
        xy.pad.as_ref().is_some_and(|p| p.grabbed())
    }

    /// Presses and releases a button under the pad, one frame each. Returns
    /// how often the button was clicked.
    fn click_button_under_pad(shield: bool) -> u32 {
        let ctx = egui::Context::default();
        let mut xy = XyUi::new();
        xy.active = shield;
        let button = Rect::from_min_size(Pos2::new(120.0, 590.0), Vec2::new(120.0, 40.0));
        let at = button.center();
        let clicks = Cell::new(0);
        let draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if ui.put(button, egui::Button::new("PLAY")).clicked() {
                    clicks.set(clicks.get() + 1);
                }
            });
            xy.overlay(ctx, TARGETS, &|_| 0.0);
        };
        let press = |pressed: bool| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(&ctx, vec![Event::PointerMoved(at)], &draw);
        frame(&ctx, vec![Event::PointerMoved(at), press(true)], &draw);
        frame(&ctx, vec![press(false)], &draw);
        clicks.get()
    }

    #[test]
    fn control_button_under_the_pad_is_clicked_without_the_pad() {
        assert_eq!(click_button_under_pad(false), 1);
    }

    #[test]
    fn tap_while_xy_is_active_does_not_click_the_widget_underneath() {
        assert_eq!(click_button_under_pad(true), 0);
    }

    #[test]
    fn absolute_pad_that_cannot_stop_clicks_grabs_once_the_pointer_is_pinned() {
        let ctx = egui::Context::default();
        let mut xy = XyUi::new();
        xy.pad = Some(FakePad::boxed("win", false).0);
        frame(&ctx, vec![], |ctx| {
            xy.toggle(ctx, TARGETS, true, &|_| 0.0);
        });
        assert!(xy.active);
        assert_eq!(xy.pending_grab, Some(0));
        assert!(!grabbed(&xy), "grab must wait for the cursor to be pinned");

        // The pointer reaches the middle of the window: grab straight away.
        let centre = Pos2::new(600.0, 430.0);
        frame(&ctx, vec![Event::PointerMoved(centre)], |ctx| {
            xy.update(ctx, TARGETS, true, 0.35, &|_| 0.0);
        });
        assert!(grabbed(&xy));
        assert_eq!(xy.pending_grab, None);
    }

    #[test]
    fn pinned_cursor_that_never_arrives_is_grabbed_after_three_frames() {
        let ctx = egui::Context::default();
        let mut xy = XyUi::new();
        xy.pad = Some(FakePad::boxed("win", false).0);
        frame(&ctx, vec![], |ctx| {
            xy.toggle(ctx, TARGETS, true, &|_| 0.0);
        });
        for _ in 0..2 {
            frame(&ctx, vec![], |ctx| {
                xy.update(ctx, TARGETS, true, 0.35, &|_| 0.0);
            });
            assert!(!grabbed(&xy));
        }
        frame(&ctx, vec![], |ctx| {
            xy.update(ctx, TARGETS, true, 0.35, &|_| 0.0);
        });
        assert!(grabbed(&xy));
        assert_eq!(xy.pending_grab, None);
    }

    #[test]
    fn pad_that_stops_clicks_is_grabbed_at_once() {
        let ctx = egui::Context::default();
        let mut xy = XyUi::new();
        xy.pad = Some(FakePad::boxed("evdev", true).0);
        frame(&ctx, vec![], |ctx| {
            xy.toggle(ctx, TARGETS, true, &|_| 0.0);
        });
        assert!(grabbed(&xy));
        assert_eq!(xy.pending_grab, None);
        // No shield: the reader's grab already swallows clicks.
        assert!(!xy.shields_clicks());
    }

    #[test]
    fn dead_pad_is_opened_again_on_enter() {
        let ctx = egui::Context::default();
        let mut xy = XyUi::new();
        xy.opener = open_reopened;
        let (pad, alive) = FakePad::boxed("old", true);
        alive.store(false, Ordering::Relaxed);
        xy.pad = Some(pad);
        frame(&ctx, vec![], |ctx| {
            xy.toggle(ctx, TARGETS, true, &|_| 0.0);
        });
        assert_eq!(xy.pad.as_ref().map(|p| p.name()), Some("reopened"));
        assert!(grabbed(&xy));
    }

    #[test]
    fn dead_pad_falls_back_to_relative_when_reopening_fails() {
        let ctx = egui::Context::default();
        let mut xy = XyUi::new();
        xy.opener = open_fails;
        let (pad, alive) = FakePad::boxed("old", true);
        alive.store(false, Ordering::Relaxed);
        xy.pad = Some(pad);
        frame(&ctx, vec![], |ctx| {
            xy.toggle(ctx, TARGETS, true, &|_| 0.0);
        });
        assert!(xy.active);
        assert!(xy.pad.is_none());
        assert_eq!(xy.pad_err.as_deref(), Some("sem permissão (teste)"));
    }

    #[test]
    fn pad_that_dies_while_active_is_replaced() {
        let ctx = egui::Context::default();
        let mut xy = XyUi::new();
        xy.opener = open_reopened;
        let (pad, alive) = FakePad::boxed("old", true);
        xy.pad = Some(pad);
        frame(&ctx, vec![], |ctx| {
            xy.toggle(ctx, TARGETS, true, &|_| 0.0);
        });
        assert_eq!(xy.pad.as_ref().map(|p| p.name()), Some("old"));

        alive.store(false, Ordering::Relaxed);
        frame(&ctx, vec![], |ctx| {
            xy.update(ctx, TARGETS, true, 0.35, &|_| 0.0);
        });
        assert_eq!(xy.pad.as_ref().map(|p| p.name()), Some("reopened"));
        assert!(grabbed(&xy));
    }

    #[test]
    fn active_pad_asks_for_frames_so_its_grab_stays_alive() {
        let ctx = egui::Context::default();
        let mut xy = XyUi::new();
        xy.active = true;
        xy.pad = Some(FakePad::boxed("evdev", true).0);
        let out = frame(&ctx, vec![], |ctx| {
            xy.update(ctx, TARGETS, true, 0.35, &|_| 0.0);
        });
        assert_eq!(out.viewport_output[&ViewportId::ROOT].repaint_delay, Duration::ZERO);
    }
}
