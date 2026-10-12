//! Touchpad as an XY pad.
//!
//! Absolute backends read the finger position straight from the touchpad
//! and can grab it so the system cursor stops moving while the pad is in
//! use: evdev on Linux, Precision Touchpad raw input on Windows.
//! [`RelativePad`] is the portable fallback: the app locks its own cursor
//! and feeds mouse deltas in.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;
#[cfg(any(windows, test))] mod hid_logic;

/// Finger position, normalized. `y` grows upwards (top of the pad = 1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Touch {
    /// 0 = left edge, 1 = right edge.
    pub x: f32,
    /// 0 = bottom edge, 1 = top edge.
    pub y: f32,
    pub touching: bool,
}

/// An absolute touchpad, read on a background thread.
pub trait Touchpad: Send {
    /// Device name for the UI.
    fn name(&self) -> &str;
    /// Most recent finger state. Cheap; call every frame.
    fn latest(&self) -> Touch;
    /// Grab: the touchpad stops moving the system cursor until released.
    /// Whether taps and clicks stop too depends on the backend, see
    /// [`Touchpad::grab_stops_clicks`]. Release must always succeed.
    fn set_grab(&mut self, grab: bool) -> Result<(), String>;
    fn grabbed(&self) -> bool;
    /// True when a grab also swallows taps, clicks and scrolling (Linux
    /// EVIOCGRAB). False when they still reach the window under the pinned
    /// cursor (Windows), so the app must shield its own widgets.
    fn grab_stops_clicks(&self) -> bool {
        false
    }
    /// False once the device is gone (unplugged, re-enumerated after resume)
    /// or its reader thread has ended; the app should open it again.
    fn is_alive(&self) -> bool {
        true
    }
}

/// Open the built-in touchpad. `Err` carries a short, user-facing reason in
/// Portuguese (e.g. missing permission on Linux) so the app can show it and
/// fall back to [`RelativePad`].
pub fn open_touchpad() -> Result<Box<dyn Touchpad>, String> {
    #[cfg(target_os = "linux")]
    {
        linux::open().map(|t| Box::new(t) as Box<dyn Touchpad>)
    }
    #[cfg(windows)]
    {
        windows::open().map(|t| Box::new(t) as Box<dyn Touchpad>)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Err("touchpad absoluto não suportado neste sistema".into())
    }
}

/// Map a raw axis value to 0..=1 given its logical range. Values outside
/// the range clamp. A degenerate range (max <= min) returns 0.5.
pub fn normalize_axis(value: i32, min: i32, max: i32) -> f32 {
    if max <= min {
        return 0.5;
    }
    // f64 keeps the span exact even for the full i32 range.
    let span = f64::from(max) - f64::from(min);
    let t = (f64::from(value) - f64::from(min)) / span;
    t.clamp(0.0, 1.0) as f32
}

/// Clamp to 0..=1; NaN (which `clamp` would keep) becomes the centre.
fn unit(v: f32) -> f32 {
    if v.is_nan() {
        0.5
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// Relative fallback: integrates pointer deltas (in logical pixels) into a
/// position, `sensitivity` = fraction of the pad per 100 px of motion.
#[derive(Clone, Copy, Debug)]
pub struct RelativePad {
    x: f32,
    y: f32,
    pub sensitivity: f32,
}

impl Default for RelativePad {
    fn default() -> Self {
        Self::new()
    }
}

impl RelativePad {
    /// Starts centred, sensitivity 0.35.
    pub fn new() -> Self {
        Self {
            x: 0.5,
            y: 0.5,
            sensitivity: 0.35,
        }
    }

    /// Screen deltas: `dy > 0` is downwards on screen, so it lowers `y`.
    pub fn apply_delta(&mut self, dx: f32, dy: f32) -> Touch {
        // A non-finite sensitivity would poison the position: treat it as no motion.
        let k = if self.sensitivity.is_finite() {
            self.sensitivity / 100.0
        } else {
            0.0
        };
        let mut x = unit(self.x);
        let mut y = unit(self.y);
        if dx.is_finite() {
            x = unit(x + dx * k);
        }
        if dy.is_finite() {
            y = unit(y - dy * k);
        }
        self.x = x;
        self.y = y;
        self.position()
    }

    pub fn set(&mut self, x: f32, y: f32) {
        self.x = x.clamp(0.0, 1.0);
        self.y = y.clamp(0.0, 1.0);
    }

    pub fn position(&self) -> Touch {
        Touch {
            x: self.x,
            y: self.y,
            touching: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn normalize_maps_ends_and_middle() {
        assert_eq!(normalize_axis(0, 0, 100), 0.0);
        assert_eq!(normalize_axis(100, 0, 100), 1.0);
        assert!(close(normalize_axis(50, 0, 100), 0.5));
        assert!(close(normalize_axis(2500, 1000, 4000), 0.5));
        assert!(close(normalize_axis(1750, 1000, 4000), 0.25));
        assert!(close(normalize_axis(-50, -100, 100), 0.25));
    }

    #[test]
    fn normalize_clamps_outside_range() {
        assert_eq!(normalize_axis(-5, 0, 100), 0.0);
        assert_eq!(normalize_axis(101, 0, 100), 1.0);
        assert_eq!(normalize_axis(i32::MIN, 0, 100), 0.0);
        assert_eq!(normalize_axis(i32::MAX, 0, 100), 1.0);
    }

    #[test]
    fn normalize_full_i32_range_does_not_overflow() {
        assert_eq!(normalize_axis(i32::MAX, i32::MIN, i32::MAX), 1.0);
        assert_eq!(normalize_axis(i32::MIN, i32::MIN, i32::MAX), 0.0);
        assert!(close(normalize_axis(0, i32::MIN, i32::MAX), 0.5));
    }

    #[test]
    fn normalize_is_monotonic() {
        let mut prev = -1.0_f32;
        for v in (0..=1000).step_by(7) {
            let t = normalize_axis(v, 0, 1000);
            assert!(t >= prev, "not monotonic at {v}");
            prev = t;
        }
    }

    #[test]
    fn normalize_degenerate_range_is_centre() {
        assert_eq!(normalize_axis(7, 5, 5), 0.5);
        assert_eq!(normalize_axis(7, 9, 3), 0.5);
        assert_eq!(normalize_axis(i32::MIN, 10, 10), 0.5);
        assert_eq!(normalize_axis(i32::MAX, 0, 0), 0.5);
    }

    #[test]
    fn relative_starts_centred_and_touching() {
        let p = RelativePad::new().position();
        assert_eq!((p.x, p.y), (0.5, 0.5));
        assert!(p.touching);
    }

    #[test]
    fn relative_x_follows_delta_scaled_by_sensitivity() {
        let mut p = RelativePad::new();
        let t = p.apply_delta(100.0, 0.0);
        assert!(close(t.x, 0.85), "x = {}", t.x);
        assert!(close(t.y, 0.5));
        assert!(t.touching);
    }

    #[test]
    fn relative_y_direction_is_inverted() {
        // dy > 0 is downward on screen, which lowers y (top of pad = 1).
        let mut p = RelativePad::new();
        let down = p.apply_delta(0.0, 100.0);
        assert!(close(down.y, 0.15), "y = {}", down.y);
        let mut p = RelativePad::new();
        let up = p.apply_delta(0.0, -100.0);
        assert!(close(up.y, 0.85), "y = {}", up.y);
        assert!(close(up.x, 0.5));
    }

    #[test]
    fn relative_accumulates_and_matches_position() {
        let mut p = RelativePad::new();
        let mut last = p.position();
        for _ in 0..10 {
            last = p.apply_delta(10.0, 0.0);
        }
        assert!(close(last.x, 0.85), "x = {}", last.x);
        assert_eq!(last, p.position());
    }

    #[test]
    fn relative_clamps_both_axes() {
        let mut p = RelativePad::new();
        assert_eq!(p.apply_delta(1.0e6, 0.0).x, 1.0);
        assert_eq!(p.apply_delta(-1.0e6, 0.0).x, 0.0);
        assert_eq!(p.apply_delta(0.0, -1.0e6).y, 1.0);
        let t = p.apply_delta(0.0, 1.0e6);
        assert_eq!(t.y, 0.0);
        assert!(t.touching);
    }

    #[test]
    fn relative_ignores_non_finite_deltas() {
        let mut p = RelativePad::new();
        p.set(0.3, 0.6);
        let t = p.apply_delta(f32::NAN, f32::INFINITY);
        assert!(close(t.x, 0.3));
        assert!(close(t.y, 0.6));
        assert!(t.touching);

        let t = p.apply_delta(f32::NEG_INFINITY, 100.0);
        assert!(close(t.x, 0.3));
        assert!(close(t.y, 0.6 - 0.35), "y = {}", t.y);
    }

    #[test]
    fn relative_non_finite_sensitivity_means_no_motion() {
        let mut p = RelativePad::new();
        p.sensitivity = f32::NAN;
        let t = p.apply_delta(100.0, 100.0);
        assert_eq!((t.x, t.y), (0.5, 0.5));
        p.sensitivity = f32::INFINITY;
        let t = p.apply_delta(100.0, 100.0);
        assert_eq!((t.x, t.y), (0.5, 0.5));
    }

    #[test]
    fn relative_zero_delta_is_identity() {
        let mut p = RelativePad::new();
        p.set(0.2, 0.9);
        let t = p.apply_delta(0.0, 0.0);
        assert_eq!((t.x, t.y), (0.2, 0.9));
    }
}
