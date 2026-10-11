//! evdev backend. Needs read access to /dev/input/event* (group `input`).

use super::{Touch, Touchpad};

pub struct LinuxTouchpad {
    name: String,
}

pub fn open() -> Result<LinuxTouchpad, String> {
    todo!("xypad linux agent")
}

impl Touchpad for LinuxTouchpad {
    fn name(&self) -> &str {
        &self.name
    }

    fn latest(&self) -> Touch {
        todo!("xypad linux agent")
    }

    fn set_grab(&mut self, grab: bool) -> Result<(), String> {
        let _ = grab;
        todo!("xypad linux agent")
    }

    fn grabbed(&self) -> bool {
        todo!("xypad linux agent")
    }
}
