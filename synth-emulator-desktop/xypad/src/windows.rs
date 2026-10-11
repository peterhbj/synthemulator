//! Precision Touchpad raw-input backend (HID usage page 0x0D, usage 0x05).

use super::{Touch, Touchpad};

pub struct WindowsTouchpad {
    name: String,
}

pub fn open() -> Result<WindowsTouchpad, String> {
    todo!("xypad windows agent")
}

impl Touchpad for WindowsTouchpad {
    fn name(&self) -> &str {
        &self.name
    }

    fn latest(&self) -> Touch {
        todo!("xypad windows agent")
    }

    fn set_grab(&mut self, grab: bool) -> Result<(), String> {
        let _ = grab;
        todo!("xypad windows agent")
    }

    fn grabbed(&self) -> bool {
        todo!("xypad windows agent")
    }
}
