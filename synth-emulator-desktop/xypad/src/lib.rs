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
    /// Grab: the touchpad stops moving the system cursor (and stops
    /// clicking) until released. Release must always succeed.
    fn set_grab(&mut self, grab: bool) -> Result<(), String>;
    fn grabbed(&self) -> bool;
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
    let _ = (value, min, max);
    todo!("xypad agent")
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
        let _ = (dx, dy);
        todo!("xypad agent")
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
