/// Map a raw abs axis into MIDI 7-bit. `invert` flips the pad so up is 127.
pub fn map_axis(value: i32, min: i32, max: i32, invert: bool) -> u8 {
    if max <= min {
        return 0;
    }
    let t = (value - min) as f64 / (max - min) as f64;
    let t = if invert { 1.0 - t } else { t };
    (t.clamp(0.0, 1.0) * 127.0).round() as u8
}

/// Score a candidate evdev name. Touchpad wins; mice/virtual/GT-100 lose.
pub fn score_pad(name: &str) -> i32 {
    let l = name.to_ascii_lowercase();
    if l.contains("gt-100")
        || l.contains("gt100")
        || l.contains("ydotoold")
        || l.contains("virtual")
        || l.contains("monitor")
    {
        return -100;
    }
    let mut s = 0;
    if l.contains("touchpad") {
        s += 80;
    }
    if l.contains("synaptics") || l.contains("elan") || l.contains("goodix") || l.contains("dell")
    {
        s += 20;
    }
    if l.contains("mouse") && !l.contains("touchpad") {
        s -= 50;
    }
    s
}

pub const CC_X: u8 = 16;
pub const CC_Y: u8 = 17;
pub const MIDI_CH: u8 = 0;

pub fn cc_msg(cc: u8, value: u8) -> [u8; 3] {
    [0xB0 | MIDI_CH, cc, value]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_axis_edges() {
        assert_eq!(map_axis(0, 0, 1919, false), 0);
        assert_eq!(map_axis(1919, 0, 1919, false), 127);
        assert_eq!(map_axis(0, 0, 1079, true), 127);
        assert_eq!(map_axis(1079, 0, 1079, true), 0);
    }

    #[test]
    fn map_axis_center() {
        assert_eq!(map_axis(960, 0, 1920, false), 64);
    }

    #[test]
    fn prefers_dell_touchpad() {
        assert!(score_pad("DELL0B24:00 27C6:0D42 Touchpad") > score_pad("PS/2 Generic Mouse"));
        assert!(score_pad("DELL0B24:00 27C6:0D42 Touchpad") >= 40);
        assert!(score_pad("ydotoold virtual device") < 0);
        assert!(score_pad("GT-100 Analog Surround 4.0") < 0);
    }

    #[test]
    fn cc_status_byte() {
        assert_eq!(cc_msg(16, 64), [0xB0, 16, 64]);
    }
}
