//! evdev backend. Needs read access to /dev/input/event* (group `input`).
//!
//! `open` probes each `event*` node itself rather than through
//! `evdev::enumerate`, which silently drops nodes it cannot open. That lets
//! "every node refused" (no permission) be told apart from "no touchpad".
//!
//! The chosen device is opened non-blocking and owned by one reader thread.
//! That thread is also the only one that issues `EVIOCGRAB`, on the same fd
//! it reads from: grabbing through another fd would starve the reader. Each
//! poll makes the grab match the request, reads up to a few batches, then
//! sleeps 4 ms.

use super::{normalize_axis, Touch, Touchpad};
use evdev::{AbsoluteAxisCode, Device, DeviceState, EventSummary, InputEvent, KeyCode, PropType};
use std::fs;
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

const PERMISSION_MSG: &str =
    "Sem permissão para ler o touchpad. Rode: sudo usermod -aG input $USER e faça logout/login.";
const NOT_FOUND_MSG: &str = "Nenhum touchpad encontrado.";

/// Idle wait between polls of the non-blocking device.
const POLL_INTERVAL: Duration = Duration::from_millis(4);

/// Reads per poll before the reader sleeps. A longer burst carries over to the
/// next poll, which keeps one poll from running on for long.
const MAX_READS_PER_POLL: usize = 8;

/// Name fragments of common touchpad drivers, matched case-insensitively.
const NAME_HINTS: [&str; 6] = [
    "touchpad",
    "trackpad",
    "synaptics",
    "elan",
    "alps",
    "glidepoint",
];

pub struct LinuxTouchpad {
    name: String,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// State written by the reader thread and read by the UI.
struct Shared {
    /// `f32` bits. 0 = left edge.
    x: AtomicU32,
    /// `f32` bits. 1 = top edge.
    y: AtomicU32,
    touching: AtomicBool,
    /// Grab state requested by the UI.
    grab_requested: AtomicBool,
    /// Grab state the reader thread has actually applied.
    grabbed: AtomicBool,
    stop: AtomicBool,
}

impl Shared {
    fn new(touch: Touch) -> Self {
        Self {
            x: AtomicU32::new(touch.x.to_bits()),
            y: AtomicU32::new(touch.y.to_bits()),
            touching: AtomicBool::new(touch.touching),
            grab_requested: AtomicBool::new(false),
            grabbed: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        }
    }

    fn touch(&self) -> Touch {
        Touch {
            x: f32::from_bits(self.x.load(Ordering::Relaxed)),
            y: f32::from_bits(self.y.load(Ordering::Relaxed)),
            touching: self.touching.load(Ordering::Relaxed),
        }
    }
}

/// What the classifier needs to know about a device, as plain flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Caps {
    /// ABS_X and ABS_Y.
    abs_xy: bool,
    /// ABS_MT_POSITION_X and ABS_MT_POSITION_Y.
    mt_xy: bool,
    btn_touch: bool,
    btn_tool_finger: bool,
    prop_pointer: bool,
    prop_direct: bool,
}

/// A touchpad reports an absolute position, `BTN_TOUCH`, and either a finger
/// tool or the pointer property. A direct-input device (touchscreen) is not one.
fn is_touchpad(c: Caps) -> bool {
    (c.abs_xy || c.mt_xy) && c.btn_touch && (c.btn_tool_finger || c.prop_pointer) && !c.prop_direct
}

/// Whether a device name carries a known touchpad driver fragment.
fn name_preferred(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    NAME_HINTS.iter().any(|hint| lower.contains(hint))
}

/// Index of the candidate to use: the first with a preferred name, otherwise
/// the first candidate. `None` when there are no candidates.
fn pick_index(names: &[&str]) -> Option<usize> {
    names
        .iter()
        .position(|name| name_preferred(name))
        .or_else(|| (!names.is_empty()).then_some(0))
}

/// Axes the position is read from: ABS_X/ABS_Y when present, otherwise the
/// multitouch position axes (slot 0 only). The flag is true for multitouch.
fn axes_for(c: Caps) -> (AbsoluteAxisCode, AbsoluteAxisCode, bool) {
    if c.abs_xy {
        (AbsoluteAxisCode::ABS_X, AbsoluteAxisCode::ABS_Y, false)
    } else {
        (
            AbsoluteAxisCode::ABS_MT_POSITION_X,
            AbsoluteAxisCode::ABS_MT_POSITION_Y,
            true,
        )
    }
}

fn caps_of(dev: &Device) -> Caps {
    let abs = |code: AbsoluteAxisCode| {
        dev.supported_absolute_axes()
            .is_some_and(|axes| axes.contains(code))
    };
    let key = |code: KeyCode| dev.supported_keys().is_some_and(|keys| keys.contains(code));
    let props = dev.properties();
    Caps {
        abs_xy: abs(AbsoluteAxisCode::ABS_X) && abs(AbsoluteAxisCode::ABS_Y),
        mt_xy: abs(AbsoluteAxisCode::ABS_MT_POSITION_X) && abs(AbsoluteAxisCode::ABS_MT_POSITION_Y),
        btn_touch: key(KeyCode::BTN_TOUCH),
        btn_tool_finger: key(KeyCode::BTN_TOOL_FINGER),
        prop_pointer: props.contains(PropType::POINTER),
        prop_direct: props.contains(PropType::DIRECT),
    }
}

/// Message when no touchpad was found. Any node refused for permission means
/// the user may lack access, even if other nodes (a joystick, say) opened
/// fine, so the permission hint wins over "not found".
fn no_touchpad_msg(refused: usize) -> &'static str {
    if refused > 0 {
        PERMISSION_MSG
    } else {
        NOT_FOUND_MSG
    }
}

/// `/dev/input/event*` nodes, sorted. Empty when the directory cannot be read.
fn event_nodes() -> Vec<PathBuf> {
    let Ok(dir) = fs::read_dir("/dev/input") else {
        return Vec::new();
    };
    let mut nodes: Vec<PathBuf> = dir
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("event"))
        })
        .collect();
    nodes.sort();
    nodes
}

/// Logical (min, max) and current value of one absolute axis, or `None` if
/// the device does not report it.
fn axis_info(dev: &Device, code: AbsoluteAxisCode) -> io::Result<Option<(i32, i32, i32)>> {
    Ok(dev
        .get_absinfo()?
        .find(|(axis, _)| *axis == code)
        .map(|(_, info)| (info.minimum(), info.maximum(), info.value())))
}

fn read_failed(e: io::Error) -> String {
    format!("Não foi possível ler o touchpad: {e}")
}

/// Opens the built-in touchpad and starts its reader thread. `Err` is a
/// user-facing Portuguese message.
pub fn open() -> Result<LinuxTouchpad, String> {
    let mut candidates: Vec<(Device, String)> = Vec::new();
    let mut refused = 0_usize;
    for path in event_nodes() {
        match Device::open(&path) {
            Ok(dev) => {
                if is_touchpad(caps_of(&dev)) {
                    let name = dev.name().unwrap_or_default().to_owned();
                    candidates.push((dev, name));
                }
            }
            Err(e) if e.kind() == ErrorKind::PermissionDenied => refused += 1,
            Err(_) => {}
        }
    }
    if candidates.is_empty() {
        return Err(no_touchpad_msg(refused).into());
    }
    let index = {
        let names: Vec<&str> = candidates.iter().map(|(_, name)| name.as_str()).collect();
        pick_index(&names).unwrap_or(0)
    };
    let (dev, name) = candidates.swap_remove(index);
    drop(candidates);
    start(dev, name)
}

/// Reads the logical ranges and initial state, switches the device to
/// non-blocking reads, and spawns the reader thread that owns it.
fn start(dev: Device, name: String) -> Result<LinuxTouchpad, String> {
    let (x_code, y_code, mt) = axes_for(caps_of(&dev));
    let missing = || "Não foi possível ler o touchpad: eixo de posição ausente.".to_owned();
    let (x_min, x_max, x_now) = axis_info(&dev, x_code)
        .map_err(read_failed)?
        .ok_or_else(missing)?;
    let (y_min, y_max, y_now) = axis_info(&dev, y_code)
        .map_err(read_failed)?
        .ok_or_else(missing)?;
    // Multitouch: the slot the kernel is writing to now. It only sends
    // ABS_MT_SLOT when the slot changes, so starting from 0 on a device that is
    // already on another slot would read that slot's positions as slot 0's.
    let slot = if mt {
        axis_info(&dev, AbsoluteAxisCode::ABS_MT_SLOT)
            .map_err(read_failed)?
            .map_or(0, |(_, _, current)| current)
    } else {
        0
    };
    let touching = dev
        .get_key_state()
        .map_err(read_failed)?
        .contains(KeyCode::BTN_TOUCH);
    // Non-blocking, so the reader can apply grab requests between reads.
    dev.set_nonblocking(true).map_err(read_failed)?;

    let track = Track {
        x: x_code,
        y: y_code,
        mt,
        x_range: (x_min, x_max),
        y_range: (y_min, y_max),
    };
    // Multitouch positions are only trusted on slot 0, so a start on another
    // slot begins at the centre until slot 0 reports.
    let (x, y) = if !mt || slot == 0 {
        (x_of(&track, x_now), y_of(&track, y_now))
    } else {
        (0.5, 0.5)
    };
    let shared = Arc::new(Shared::new(Touch { x, y, touching }));
    let worker = Arc::clone(&shared);
    let thread = thread::Builder::new()
        .name("xypad-evdev".into())
        .spawn(move || run(dev, worker, track, slot))
        .map_err(|e| format!("Não foi possível iniciar a leitura do touchpad: {e}"))?;
    Ok(LinuxTouchpad {
        name,
        shared,
        thread: Some(thread),
    })
}

/// The axes the position is read from, with their logical ranges.
#[derive(Clone, Copy, Debug)]
struct Track {
    x: AbsoluteAxisCode,
    y: AbsoluteAxisCode,
    /// Multitouch-only device: only slot 0 drives the position.
    mt: bool,
    x_range: (i32, i32),
    y_range: (i32, i32),
}

/// The grab calls the reader makes, behind a trait so the retry rule can be
/// tested without a device.
trait Grab {
    fn grab(&mut self) -> io::Result<()>;
    fn ungrab(&mut self) -> io::Result<()>;
    fn is_grabbed(&self) -> bool;
}

impl Grab for Device {
    fn grab(&mut self) -> io::Result<()> {
        Device::grab(self)
    }

    fn ungrab(&mut self) -> io::Result<()> {
        Device::ungrab(self)
    }

    fn is_grabbed(&self) -> bool {
        Device::is_grabbed(self)
    }
}

/// Makes the grab match `want`, judged by the device's real state rather than
/// by the last request. evdev records a grab only after the ioctl succeeds, so
/// a refused call leaves the two different and the next poll tries again.
fn reconcile_grab<G: Grab>(dev: &mut G, want: bool) {
    if dev.is_grabbed() != want {
        // EVIOCGRAB on the fd we read from. A failure is retried on the next poll.
        let _ = if want { dev.grab() } else { dev.ungrab() };
    }
}

/// What one poll found on the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Poll {
    /// Nothing more to read now, or the read was interrupted.
    Idle,
    /// The device stopped reading (for example it was unplugged).
    Gone,
}

/// Runs `read` up to `MAX_READS_PER_POLL` times, stopping at the first
/// `WouldBlock`. Any other error means the device is gone.
fn drain(mut read: impl FnMut() -> io::Result<()>) -> Poll {
    for _ in 0..MAX_READS_PER_POLL {
        match read() {
            Ok(()) => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
                return Poll::Idle;
            }
            Err(_) => return Poll::Gone,
        }
    }
    Poll::Idle
}

/// Whether evdev has resynchronised its cache since `seen`. evdev stamps the
/// cache with the time of each kernel sync, so a new stamp means the cache now
/// holds the whole kernel state. Updates `seen` when it returns true.
fn resynced(seen: &mut SystemTime, stamp: SystemTime) -> bool {
    if *seen == stamp {
        false
    } else {
        *seen = stamp;
        true
    }
}

/// Reader loop. Owns the device; keeps the grab matching the request on its
/// own fd; folds events into `shared`. Ends on `stop` or when the device stops
/// reading (for example it was unplugged), always ungrabbing first.
fn run(mut dev: Device, shared: Arc<Shared>, track: Track, start_slot: i32) {
    let mut slot = start_slot;
    let mut seen = dev.cached_state().timestamp();
    loop {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        reconcile_grab(&mut dev, shared.grab_requested.load(Ordering::Acquire));
        shared.grabbed.store(dev.is_grabbed(), Ordering::Relaxed);

        let poll = drain(|| {
            let read = match dev.fetch_events() {
                Ok(events) => {
                    for ev in events {
                        fold(&shared, &track, &mut slot, ev);
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            };
            // Checked after every read, WouldBlock included: a resync lost to
            // WouldBlock still left the cache correct, and a lost BTN_TOUCH
            // release would otherwise stick.
            if resynced(&mut seen, dev.cached_state().timestamp()) {
                sync_from_cache(&dev, &shared, &track);
            }
            read
        });
        if poll == Poll::Gone {
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    let _ = dev.ungrab();
    // Closing the fd releases any grab too, so the state is clear either way.
    shared.grabbed.store(false, Ordering::Relaxed);
    shared.touching.store(false, Ordering::Relaxed);
}

/// Takes the touch state from evdev's cache. Only valid after a resync (see
/// `resynced`): the cache starts zero-filled, not read from the kernel, so
/// before a sync it says nothing about keys or axes that have not reported.
fn sync_from_cache(dev: &Device, shared: &Shared, track: &Track) {
    let cache: &DeviceState = dev.cached_state();
    let touching = cache
        .key_vals()
        .is_some_and(|keys| keys.contains(KeyCode::BTN_TOUCH));
    let raw = |code: AbsoluteAxisCode| {
        if track.mt {
            None
        } else {
            cache
                .abs_vals()
                .and_then(|abs| abs.get(code.0 as usize))
                .map(|info| info.value)
        }
    };
    apply_snapshot(shared, track, touching, raw(track.x), raw(track.y));
}

/// Writes a full snapshot of the touch state. `raw_x` and `raw_y` are raw axis
/// values, or `None` to keep the current position (multitouch).
fn apply_snapshot(
    shared: &Shared,
    track: &Track,
    touching: bool,
    raw_x: Option<i32>,
    raw_y: Option<i32>,
) {
    shared.touching.store(touching, Ordering::Relaxed);
    if let Some(raw) = raw_x {
        shared
            .x
            .store(x_of(track, raw).to_bits(), Ordering::Relaxed);
    }
    if let Some(raw) = raw_y {
        shared
            .y
            .store(y_of(track, raw).to_bits(), Ordering::Relaxed);
    }
}

/// Touch x for a raw X value: 0 = left edge.
fn x_of(track: &Track, raw: i32) -> f32 {
    normalize_axis(raw, track.x_range.0, track.x_range.1)
}

/// Touch y for a raw Y value. Raw Y grows downward; `Touch::y` grows upward.
fn y_of(track: &Track, raw: i32) -> f32 {
    1.0 - normalize_axis(raw, track.y_range.0, track.y_range.1)
}

/// Folds one kernel event into the shared touch state.
fn fold(shared: &Shared, track: &Track, slot: &mut i32, ev: InputEvent) {
    match ev.destructure() {
        EventSummary::AbsoluteAxis(_, code, value) => {
            if code == AbsoluteAxisCode::ABS_MT_SLOT {
                *slot = value;
            } else if !track.mt || *slot == 0 {
                if code == track.x {
                    shared
                        .x
                        .store(x_of(track, value).to_bits(), Ordering::Relaxed);
                } else if code == track.y {
                    shared
                        .y
                        .store(y_of(track, value).to_bits(), Ordering::Relaxed);
                }
            }
        }
        EventSummary::Key(_, code, value) if code == KeyCode::BTN_TOUCH => {
            shared.touching.store(value != 0, Ordering::Relaxed);
        }
        _ => {}
    }
}

impl Drop for LinuxTouchpad {
    fn drop(&mut self) {
        self.shared.grab_requested.store(false, Ordering::Release);
        self.shared.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            // The reader polls every 4 ms, so this returns promptly. It has
            // ungrabbed by the time it finishes.
            let _ = thread.join();
        }
    }
}

impl Touchpad for LinuxTouchpad {
    fn name(&self) -> &str {
        &self.name
    }

    fn latest(&self) -> Touch {
        self.shared.touch()
    }

    fn set_grab(&mut self, grab: bool) -> Result<(), String> {
        self.shared.grab_requested.store(grab, Ordering::Release);
        Ok(())
    }

    fn grabbed(&self) -> bool {
        self.shared.grabbed.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::EventType;

    const TOUCHPAD: Caps = Caps {
        abs_xy: true,
        mt_xy: false,
        btn_touch: true,
        btn_tool_finger: true,
        prop_pointer: true,
        prop_direct: false,
    };

    fn abs(code: AbsoluteAxisCode, value: i32) -> InputEvent {
        InputEvent::new(EventType::ABSOLUTE.0, code.0, value)
    }

    fn key(code: KeyCode, value: i32) -> InputEvent {
        InputEvent::new(EventType::KEY.0, code.0, value)
    }

    fn fresh() -> Shared {
        Shared::new(Touch {
            x: 0.0,
            y: 0.5,
            touching: false,
        })
    }

    fn xy_track() -> Track {
        Track {
            x: AbsoluteAxisCode::ABS_X,
            y: AbsoluteAxisCode::ABS_Y,
            mt: false,
            x_range: (0, 1000),
            y_range: (0, 1000),
        }
    }

    #[test]
    fn touchpad_is_accepted() {
        assert!(is_touchpad(TOUCHPAD));
    }

    #[test]
    fn pointer_property_stands_in_for_finger_tool() {
        let c = Caps {
            btn_tool_finger: false,
            ..TOUCHPAD
        };
        assert!(is_touchpad(c));
    }

    #[test]
    fn multitouch_axes_are_enough() {
        let c = Caps {
            abs_xy: false,
            mt_xy: true,
            ..TOUCHPAD
        };
        assert!(is_touchpad(c));
    }

    #[test]
    fn touchscreen_is_rejected() {
        let c = Caps {
            prop_direct: true,
            ..TOUCHPAD
        };
        assert!(!is_touchpad(c));
    }

    #[test]
    fn mouse_is_rejected() {
        // A mouse: relative axes and BTN_LEFT, no absolute position or touch.
        let mouse = Caps {
            prop_pointer: true,
            ..Caps::default()
        };
        assert!(!is_touchpad(mouse));
    }

    #[test]
    fn keyboard_is_rejected() {
        assert!(!is_touchpad(Caps::default()));
    }

    #[test]
    fn absolute_pointer_without_touch_is_rejected() {
        let c = Caps {
            btn_touch: false,
            ..TOUCHPAD
        };
        assert!(!is_touchpad(c));
    }

    #[test]
    fn device_without_finger_or_pointer_is_rejected() {
        let c = Caps {
            btn_tool_finger: false,
            prop_pointer: false,
            ..TOUCHPAD
        };
        assert!(!is_touchpad(c));
    }

    #[test]
    fn device_without_position_axes_is_rejected() {
        let c = Caps {
            abs_xy: false,
            mt_xy: false,
            ..TOUCHPAD
        };
        assert!(!is_touchpad(c));
    }

    #[test]
    fn preferred_name_wins_over_first_candidate() {
        let names = ["Logitech USB Receiver", "SynPS/2 Synaptics TouchPad"];
        assert_eq!(pick_index(&names), Some(1));
    }

    #[test]
    fn first_candidate_wins_without_a_hint() {
        assert_eq!(pick_index(&["Alpha", "Beta"]), Some(0));
    }

    #[test]
    fn first_hinted_candidate_wins_among_hinted() {
        let names = ["Foo Pad", "ELAN Touchpad", "Synaptics"];
        assert_eq!(pick_index(&names), Some(1));
    }

    #[test]
    fn no_candidates_means_no_pick() {
        assert_eq!(pick_index(&[]), None);
    }

    #[test]
    fn name_hints_are_case_insensitive() {
        assert!(name_preferred("GlidePoint"));
        assert!(name_preferred("TRACKPAD"));
        assert!(name_preferred("Elantech"));
        assert!(name_preferred("ALPS DualPoint"));
        assert!(!name_preferred("Power Button"));
    }

    #[test]
    fn absolute_axes_are_preferred_over_multitouch() {
        let c = Caps {
            abs_xy: true,
            mt_xy: true,
            ..TOUCHPAD
        };
        let (x, y, mt) = axes_for(c);
        assert_eq!(
            (x, y, mt),
            (AbsoluteAxisCode::ABS_X, AbsoluteAxisCode::ABS_Y, false)
        );
        let (x, y, mt) = axes_for(Caps { abs_xy: false, ..c });
        assert_eq!(
            (x, y, mt),
            (
                AbsoluteAxisCode::ABS_MT_POSITION_X,
                AbsoluteAxisCode::ABS_MT_POSITION_Y,
                true
            )
        );
    }

    #[test]
    fn absolute_x_is_normalized() {
        let shared = fresh();
        let mut slot = 0;
        fold(
            &shared,
            &xy_track(),
            &mut slot,
            abs(AbsoluteAxisCode::ABS_X, 250),
        );
        assert!((shared.touch().x - 0.25).abs() < 1e-6);
    }

    #[test]
    fn absolute_y_is_inverted_so_top_is_one() {
        let shared = fresh();
        let mut slot = 0;
        let track = xy_track();
        fold(&shared, &track, &mut slot, abs(AbsoluteAxisCode::ABS_Y, 0));
        assert_eq!(shared.touch().y, 1.0);
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_Y, 1000),
        );
        assert_eq!(shared.touch().y, 0.0);
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_Y, 250),
        );
        assert!((shared.touch().y - 0.75).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_values_clamp() {
        let shared = fresh();
        let mut slot = 0;
        fold(
            &shared,
            &xy_track(),
            &mut slot,
            abs(AbsoluteAxisCode::ABS_X, 5000),
        );
        assert_eq!(shared.touch().x, 1.0);
        fold(
            &shared,
            &xy_track(),
            &mut slot,
            abs(AbsoluteAxisCode::ABS_X, -5000),
        );
        assert_eq!(shared.touch().x, 0.0);
    }

    #[test]
    fn degenerate_axis_range_reads_as_centre() {
        let shared = fresh();
        let mut slot = 0;
        let track = Track {
            y_range: (7, 7),
            ..xy_track()
        };
        fold(&shared, &track, &mut slot, abs(AbsoluteAxisCode::ABS_Y, 3));
        assert_eq!(shared.touch().y, 0.5);
    }

    #[test]
    fn btn_touch_drives_touching_and_other_keys_do_not() {
        let shared = fresh();
        let mut slot = 0;
        let track = xy_track();
        fold(&shared, &track, &mut slot, key(KeyCode::BTN_TOUCH, 1));
        assert!(shared.touch().touching);
        fold(&shared, &track, &mut slot, key(KeyCode::BTN_LEFT, 1));
        assert!(shared.touch().touching);
        fold(&shared, &track, &mut slot, key(KeyCode::BTN_TOUCH, 0));
        assert!(!shared.touch().touching);
    }

    #[test]
    fn multitouch_only_follows_slot_zero() {
        let shared = fresh();
        let mut slot = 0;
        let track = Track {
            x: AbsoluteAxisCode::ABS_MT_POSITION_X,
            y: AbsoluteAxisCode::ABS_MT_POSITION_Y,
            mt: true,
            x_range: (0, 1000),
            y_range: (0, 1000),
        };
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 1),
        );
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_MT_POSITION_X, 800),
        );
        assert_eq!(shared.touch().x, 0.0, "slot 1 must not move the position");
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 0),
        );
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_MT_POSITION_X, 200),
        );
        assert!((shared.touch().x - 0.2).abs() < 1e-6);
    }

    #[test]
    fn unrelated_events_are_ignored() {
        let shared = fresh();
        let before = shared.touch();
        let mut slot = 0;
        fold(
            &shared,
            &xy_track(),
            &mut slot,
            abs(AbsoluteAxisCode::ABS_Z, 900),
        );
        assert_eq!(shared.touch(), before);
    }

    #[test]
    fn permission_hint_when_any_node_was_refused() {
        // Some nodes readable (a joystick under uaccess) must not hide the hint.
        assert_eq!(no_touchpad_msg(3), PERMISSION_MSG);
        assert_eq!(no_touchpad_msg(1), PERMISSION_MSG);
        assert_eq!(no_touchpad_msg(2), PERMISSION_MSG);
    }

    #[test]
    fn not_found_when_no_node_was_refused() {
        assert_eq!(no_touchpad_msg(0), NOT_FOUND_MSG);
    }

    #[test]
    fn multitouch_starting_on_another_slot_ignores_its_positions() {
        // Regression: the kernel is already on slot 1 and sends no ABS_MT_SLOT
        // for its positions, so they must not be read as slot 0.
        let shared = fresh();
        let track = Track {
            x: AbsoluteAxisCode::ABS_MT_POSITION_X,
            y: AbsoluteAxisCode::ABS_MT_POSITION_Y,
            mt: true,
            x_range: (0, 1000),
            y_range: (0, 1000),
        };
        let mut slot = 1;
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_MT_POSITION_X, 800),
        );
        assert_eq!(shared.touch().x, 0.0, "slot 1 position read as slot 0");
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_MT_SLOT, 0),
        );
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_MT_POSITION_X, 200),
        );
        assert!((shared.touch().x - 0.2).abs() < 1e-6);
    }

    /// Scripted device for the grab rule: refuses the first `refuse` calls.
    struct FakeGrab {
        refuse: usize,
        grabbed: bool,
        calls: usize,
    }

    impl FakeGrab {
        fn attempt(&mut self, grab: bool) -> io::Result<()> {
            self.calls += 1;
            if self.refuse > 0 {
                self.refuse -= 1;
                return Err(io::Error::from(ErrorKind::ResourceBusy));
            }
            self.grabbed = grab;
            Ok(())
        }
    }

    impl Grab for FakeGrab {
        fn grab(&mut self) -> io::Result<()> {
            self.attempt(true)
        }

        fn ungrab(&mut self) -> io::Result<()> {
            self.attempt(false)
        }

        fn is_grabbed(&self) -> bool {
            self.grabbed
        }
    }

    #[test]
    fn refused_grab_is_retried_until_the_kernel_grants_it() {
        // Regression: a refused grab used to be attempted once and never again.
        let mut dev = FakeGrab {
            refuse: 2,
            grabbed: false,
            calls: 0,
        };
        reconcile_grab(&mut dev, true);
        reconcile_grab(&mut dev, true);
        assert!(!dev.is_grabbed());
        reconcile_grab(&mut dev, true);
        assert!(dev.is_grabbed());
        assert_eq!(dev.calls, 3);
        reconcile_grab(&mut dev, true);
        assert_eq!(dev.calls, 3, "no call once the grab is held");
    }

    #[test]
    fn refused_ungrab_is_retried_until_released() {
        let mut dev = FakeGrab {
            refuse: 1,
            grabbed: true,
            calls: 0,
        };
        reconcile_grab(&mut dev, false);
        assert!(dev.is_grabbed(), "the refused release did not take effect");
        reconcile_grab(&mut dev, false);
        assert!(!dev.is_grabbed());
        assert_eq!(dev.calls, 2);
    }

    #[test]
    fn grab_is_left_alone_when_already_as_requested() {
        let mut dev = FakeGrab {
            refuse: 0,
            grabbed: false,
            calls: 0,
        };
        reconcile_grab(&mut dev, false);
        assert_eq!(dev.calls, 0);
    }

    #[test]
    fn a_poll_reads_until_would_block() {
        let mut reads = 0;
        let poll = drain(|| {
            reads += 1;
            if reads <= 3 {
                Ok(())
            } else {
                Err(io::Error::from(ErrorKind::WouldBlock))
            }
        });
        assert_eq!(poll, Poll::Idle);
        assert_eq!(reads, 4);
    }

    #[test]
    fn a_poll_reads_at_most_the_cap() {
        // Regression: one 32-event batch per poll could not keep up with bursts.
        let mut reads = 0;
        let poll = drain(|| {
            reads += 1;
            Ok(())
        });
        assert_eq!(poll, Poll::Idle);
        assert_eq!(reads, MAX_READS_PER_POLL);
    }

    #[test]
    fn a_device_error_other_than_would_block_ends_the_reader() {
        let mut reads = 0;
        let poll = drain(|| {
            reads += 1;
            Err(io::Error::other("ENODEV"))
        });
        assert_eq!(poll, Poll::Gone);
        assert_eq!(reads, 1);
    }

    #[test]
    fn an_interrupted_read_is_not_a_device_error() {
        let poll = drain(|| Err(io::Error::from(ErrorKind::Interrupted)));
        assert_eq!(poll, Poll::Idle);
    }

    #[test]
    fn a_stuck_touch_is_cleared_by_a_resync_snapshot() {
        // Regression: the BTN_TOUCH release was lost in a SYN_DROPPED resync,
        // so no event arrives. The resynced cache says the finger is up.
        let shared = fresh();
        let track = xy_track();
        let mut slot = 0;
        fold(&shared, &track, &mut slot, key(KeyCode::BTN_TOUCH, 1));
        assert!(shared.touch().touching);
        apply_snapshot(&shared, &track, false, Some(500), Some(500));
        assert!(!shared.touch().touching);
    }

    #[test]
    fn resync_snapshot_sets_position_from_raw_axes() {
        let shared = fresh();
        apply_snapshot(&shared, &xy_track(), true, Some(250), Some(250));
        let t = shared.touch();
        assert!(t.touching);
        assert!((t.x - 0.25).abs() < 1e-6);
        // Raw Y grows downward, so 250 of 1000 is near the top.
        assert!((t.y - 0.75).abs() < 1e-6);
    }

    #[test]
    fn resync_snapshot_keeps_position_when_not_reported() {
        // Multitouch passes no raw axes: only the touch state changes.
        let shared = fresh();
        let track = xy_track();
        let mut slot = 0;
        fold(
            &shared,
            &track,
            &mut slot,
            abs(AbsoluteAxisCode::ABS_X, 800),
        );
        apply_snapshot(&shared, &track, true, None, None);
        let t = shared.touch();
        assert!((t.x - 0.8).abs() < 1e-6);
        assert!(t.touching);
    }

    #[test]
    fn a_new_cache_stamp_is_one_resync() {
        let mut seen = SystemTime::UNIX_EPOCH;
        let stamp = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
        assert!(!resynced(&mut seen, SystemTime::UNIX_EPOCH));
        assert!(resynced(&mut seen, stamp));
        assert!(!resynced(&mut seen, stamp));
    }

    #[test]
    fn user_facing_messages_are_portuguese_and_exact() {
        assert_eq!(
            PERMISSION_MSG,
            "Sem permissão para ler o touchpad. Rode: sudo usermod -aG input $USER e faça logout/login."
        );
        assert_eq!(NOT_FOUND_MSG, "Nenhum touchpad encontrado.");
    }

    #[test]
    fn open_does_not_panic_without_input_devices() {
        // Accepts Ok (a real touchpad on this machine) or Err (a container).
        let _ = open();
    }
}
