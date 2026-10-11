//! Precision Touchpad raw-input backend (HID usage page 0x0D, usage 0x05).
//!
//! A background thread owns a message-only window registered for raw input
//! from the Precision Touchpad top-level collection with `RIDEV_INPUTSINK`,
//! so reports arrive even while Helix is not the foreground app. Each HID
//! report is parsed with the `HidP_*` API against the device's preparsed
//! data; the report-level decisions (which link collection is a finger,
//! hybrid-mode frames, tip switch) live in [`crate::hid_logic`]. The finger
//! position is published through atomics, so [`Touchpad::latest`] is a few
//! relaxed loads.
//!
//! Grab: Windows has no exclusive grab for a touchpad. While grabbed, the
//! thread pins the system cursor with a 1x1 `ClipCursor` rectangle at the
//! position the cursor had when the grab started, re-applied on every report
//! and every 100 ms because Windows drops the clip on focus changes. The
//! cursor stays put, but touchpad clicks (buttons, tap-to-click) still reach
//! the window under the pinned cursor, and system multi-finger gestures
//! still run; that is accepted.
//!
//! Watchdog: the same 100 ms tick lifts a touch that has had no report for
//! `hid_logic::STALE_TOUCH` (150 ms), so a lost lift (dropped report, input
//! taken by another process, locked session) cannot leave the pad stuck
//! "touching".
//!
//! Raw input registration is per process. Every [`open`] takes a new
//! generation number; a thread registers or unregisters only under
//! [`REGISTRATION`], and registers only while its generation is the newest
//! and its instance has not been given up, so a slow, timed-out thread can
//! never take the touchpad away from a newer instance.

#![deny(unsafe_op_in_unsafe_fn)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::{offset_of, size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HidP_GetCaps, HidP_GetUsageValue, HidP_GetUsages, HidP_GetValueCaps, HidP_Input,
    HidP_MaxUsageListLength, HIDP_CAPS, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows_sys::Win32::Foundation::{
    HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, GetRawInputDeviceList, GetRegisteredRawInputDevices,
    RegisterRawInputDevices, HRAWINPUT, RAWHID, RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICELIST,
    RAWINPUTHEADER, RIDEV_DEVNOTIFY, RIDEV_INPUTSINK, RIDEV_REMOVE, RIDI_DEVICEINFO,
    RIDI_PREPARSEDDATA, RID_DEVICE_INFO, RID_INPUT, RIM_TYPEHID,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ClipCursor, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetMessageW, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer, UnregisterClassW,
    GIDC_REMOVAL, HWND_MESSAGE, MSG, WM_APP, WM_CLOSE, WM_DESTROY, WM_INPUT,
    WM_INPUT_DEVICE_CHANGE, WM_TIMER, WNDCLASSW,
};

use super::hid_logic::{
    is_precision_touchpad, layout, pad_change, tip_on, touch_is_stale, usage_span, Contact,
    FrameTracker, Layout, PadChange, Update, ValueCap, PAGE_DIGITIZER, PAGE_GENERIC_DESKTOP,
    USAGE_CONTACT_COUNT, USAGE_TOUCH_PAD, USAGE_X, USAGE_Y,
};
use super::{Touch, Touchpad};

const NO_TOUCHPAD: &str = "Nenhum touchpad de precisão encontrado.";
/// How long [`open`] waits for the input thread to come up.
const OPEN_TIMEOUT: Duration = Duration::from_secs(2);
const TICK_TIMER_ID: usize = 1;
/// Always-on tick: re-applies the grab (Windows drops the cursor clip on
/// focus changes) and runs the stale-touch watchdog.
const TICK_MS: u32 = 100;
/// Posted by the app thread when the grab flag changes.
const WM_APP_GRAB: u32 = WM_APP + 1;
/// Sanity bounds on sizes Windows reports.
const MAX_INPUT_BYTES: u32 = 1 << 20;
const MAX_PREPARSED_BYTES: u32 = 1 << 20;
const MAX_USAGE_LIST: usize = 1024;
/// After a Win32 failure while reading a device's description, skip this
/// many of its reports (about half a second) before trying again.
const OPEN_RETRY_REPORTS: u32 = 64;

/// Unique window class per instance, so one instance unregistering its
/// class can never break another one being created.
static CLASS_SEQ: AtomicU32 = AtomicU32::new(0);
/// Bumped by every [`open`]; only the newest generation may register.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Serialises "check, then register" and "check, then unregister" across
/// instances, since raw input registration is per process.
static REGISTRATION: Mutex<()> = Mutex::new(());

fn registration_lock() -> MutexGuard<'static, ()> {
    // The guarded data is `()`, so a poisoned lock is still usable.
    REGISTRATION.lock().unwrap_or_else(PoisonError::into_inner)
}

/// State shared between the app and the input thread.
struct Shared {
    /// `f32` bits, 0..=1.
    x: AtomicU32,
    /// `f32` bits, 0..=1, top of the pad = 1.
    y: AtomicU32,
    touching: AtomicBool,
    /// Grab requested by the app.
    grab: AtomicBool,
    /// The app is closing the touchpad.
    stop: AtomicBool,
    /// The message window as an integer, 0 while there is none.
    hwnd: AtomicIsize,
}

impl Shared {
    fn new() -> Self {
        Self {
            x: AtomicU32::new(0.5f32.to_bits()),
            y: AtomicU32::new(0.5f32.to_bits()),
            touching: AtomicBool::new(false),
            grab: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            hwnd: AtomicIsize::new(0),
        }
    }

    fn touch(&self, x: f32, y: f32) {
        self.x.store(x.to_bits(), Ordering::Relaxed);
        self.y.store(y.to_bits(), Ordering::Relaxed);
        self.touching.store(true, Ordering::Release);
    }

    fn lift(&self) {
        self.touching.store(false, Ordering::Release);
    }

    /// Post `msg` to the input thread's window. False when there is no
    /// window (thread gone) or posting failed.
    fn post(&self, msg: u32) -> bool {
        let hwnd = self.hwnd.load(Ordering::SeqCst);
        if hwnd == 0 {
            return false;
        }
        // SAFETY: PostMessageW only queues a message carrying no pointers;
        // it validates the handle and fails cleanly if the window is gone.
        unsafe { PostMessageW(hwnd as HWND, msg, 0, 0) != 0 }
    }
}

pub struct WindowsTouchpad {
    name: String,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

pub fn open() -> Result<WindowsTouchpad, String> {
    // From here on, any older instance still starting up may not register.
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
    let shared = Arc::new(Shared::new());
    let (tx, rx) = mpsc::channel();
    let thread_shared = Arc::clone(&shared);
    let thread = thread::Builder::new()
        .name("helix-xypad".into())
        .spawn(move || run(thread_shared, generation, tx))
        .map_err(|_| "Falha ao iniciar a leitura do touchpad.".to_string())?;
    match rx.recv_timeout(OPEN_TIMEOUT) {
        Ok(Ok(name)) => Ok(WindowsTouchpad {
            name,
            shared,
            thread: Some(thread),
        }),
        Ok(Err(reason)) => {
            // The thread reported and is tearing down; this is quick.
            let _ = thread.join();
            Err(reason)
        }
        Err(RecvTimeoutError::Timeout) => {
            // Either the thread sees `stop` after reporting, or it already
            // published its window and gets WM_CLOSE (SeqCst on both).
            shared.stop.store(true, Ordering::SeqCst);
            shared.post(WM_CLOSE);
            Err("O touchpad não respondeu a tempo.".into())
        }
        Err(RecvTimeoutError::Disconnected) => {
            let _ = thread.join();
            Err("Falha ao iniciar a leitura do touchpad.".into())
        }
    }
}

impl Touchpad for WindowsTouchpad {
    fn name(&self) -> &str {
        &self.name
    }

    fn latest(&self) -> Touch {
        let touching = self.shared.touching.load(Ordering::Acquire);
        Touch {
            x: f32::from_bits(self.shared.x.load(Ordering::Relaxed)),
            y: f32::from_bits(self.shared.y.load(Ordering::Relaxed)),
            touching,
        }
    }

    /// Pins the cursor (see the module docs); never fails.
    fn set_grab(&mut self, grab: bool) -> Result<(), String> {
        let was = self.shared.grab.swap(grab, Ordering::SeqCst);
        if was && !grab {
            // Release right away from this thread; the input thread then
            // stops re-applying (and releases again) on WM_APP_GRAB.
            release_cursor();
        }
        if was != grab {
            self.shared.post(WM_APP_GRAB);
        }
        Ok(())
    }

    fn grabbed(&self) -> bool {
        self.shared.grab.load(Ordering::SeqCst)
    }
}

impl Drop for WindowsTouchpad {
    fn drop(&mut self) {
        if self.shared.grab.swap(false, Ordering::SeqCst) {
            release_cursor();
        }
        self.shared.stop.store(true, Ordering::SeqCst);
        let posted = self.shared.post(WM_CLOSE);
        if let Some(thread) = self.thread.take() {
            // Only wait when the thread is known to wake up and leave.
            if posted || thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}

fn release_cursor() {
    // SAFETY: a null rectangle releases the cursor clip; no pointers read.
    unsafe {
        ClipCursor(null());
    }
}

/// NUL-terminated UTF-16.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Input thread: find the touchpad, create the window, register for raw
/// input, report to [`open`], pump messages until WM_CLOSE.
fn run(shared: Arc<Shared>, generation: u64, tx: mpsc::Sender<Result<String, String>>) {
    let name = match find_touchpad() {
        Ok(Some(name)) => name,
        Ok(None) => {
            let _ = tx.send(Err(NO_TOUCHPAD.into()));
            return;
        }
        Err(reason) => {
            let _ = tx.send(Err(reason));
            return;
        }
    };

    // SAFETY: a null module name returns the handle of the running exe.
    let hinstance = unsafe { GetModuleHandleW(null()) };
    let class = wide(&format!(
        "HelixXyPadRawInput{}",
        CLASS_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: null_mut(),
        hCursor: null_mut(),
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
    };
    // SAFETY: wc is fully initialised and its class name outlives the
    // class (unregistered below before `class` is dropped).
    if unsafe { RegisterClassW(&wc) } == 0 {
        let _ = tx.send(Err("Falha ao criar a janela de entrada do touchpad.".into()));
        return;
    }

    // The wndproc finds its state here; it runs on this thread only.
    install_state(ThreadState::new(Arc::clone(&shared)));

    // SAFETY: the class was registered above; HWND_MESSAGE makes a
    // message-only window; all other arguments are plain values or null.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            hinstance,
            null(),
        )
    };
    if hwnd.is_null() {
        teardown(&shared, None, &class, hinstance);
        let _ = tx.send(Err("Falha ao criar a janela de entrada do touchpad.".into()));
        return;
    }
    shared.hwnd.store(hwnd as isize, Ordering::SeqCst);

    if let Err(reason) = register(&shared, generation, hwnd) {
        teardown(&shared, Some(hwnd), &class, hinstance);
        let _ = tx.send(Err(reason.into()));
        return;
    }

    // Always-on tick for the grab and the stale-touch watchdog. Without it
    // the grab is still re-applied on every report, so failure is not fatal.
    // SAFETY: hwnd is this thread's window; without a TIMERPROC the timer
    // posts WM_TIMER to it. The timer dies with the window.
    unsafe {
        SetTimer(hwnd, TICK_TIMER_ID, TICK_MS, None);
    }

    let delivered = tx.send(Ok(name)).is_ok();
    drop(tx);
    if delivered && !shared.stop.load(Ordering::SeqCst) {
        message_loop();
    }
    teardown(&shared, Some(hwnd), &class, hinstance);
}

fn message_loop() {
    // SAFETY: MSG is plain data; all-zero is a valid value.
    let mut msg: MSG = unsafe { zeroed() };
    loop {
        // SAFETY: msg is a valid out pointer; a null window filter takes
        // every message of this thread, including WM_QUIT.
        let r = unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) };
        if r == 0 || r == -1 {
            break;
        }
        // SAFETY: msg was filled in by GetMessageW.
        unsafe {
            DispatchMessageW(&msg);
        }
    }
}

/// Undo everything `run` set up. `hwnd` is `None` when window creation
/// failed. Safe to call with the window already destroyed.
fn teardown(shared: &Shared, hwnd: Option<HWND>, class: &[u16], hinstance: HINSTANCE) {
    shared.hwnd.store(0, Ordering::SeqCst);
    let state = take_state();
    if let Some(hwnd) = hwnd {
        unregister_if_ours(hwnd);
        if !state.as_ref().is_some_and(|s| s.destroyed) {
            // SAFETY: the window belongs to this thread and was not
            // destroyed yet (WM_DESTROY was not seen).
            unsafe {
                DestroyWindow(hwnd);
            }
        }
    }
    if state.as_ref().is_some_and(|s| s.pin.is_some()) {
        release_cursor();
    }
    // SAFETY: class is the NUL-terminated name registered in `run`; no
    // window of it is left. Failure is harmless.
    unsafe {
        UnregisterClassW(class.as_ptr(), hinstance);
    }
    shared.lift();
}

/// Register `hwnd` as the process' Precision Touchpad sink, unless this
/// instance was given up (`stop`) or a newer [`open`] has started.
fn register(shared: &Shared, generation: u64, hwnd: HWND) -> Result<(), &'static str> {
    let _lock = registration_lock();
    if shared.stop.load(Ordering::SeqCst) {
        return Err("O touchpad não respondeu a tempo.");
    }
    if GENERATION.load(Ordering::SeqCst) != generation {
        return Err("Outra leitura do touchpad foi iniciada.");
    }
    let device = RAWINPUTDEVICE {
        usUsagePage: PAGE_DIGITIZER,
        usUsage: USAGE_TOUCH_PAD,
        dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
        hwndTarget: hwnd,
    };
    // SAFETY: one valid RAWINPUTDEVICE, size given in bytes.
    let ok = unsafe { RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) };
    if ok != 0 {
        Ok(())
    } else {
        Err("Falha ao registrar o touchpad para leitura.")
    }
}

/// The process' raw input registrations, `None` if they cannot be read.
fn registered_devices() -> Option<Vec<RAWINPUTDEVICE>> {
    let item = size_of::<RAWINPUTDEVICE>() as u32;
    // Another thread (e.g. the GUI's own raw mouse input) can register
    // between the two calls; retry a few times.
    for _ in 0..4 {
        let mut count = 0u32;
        // SAFETY: a null buffer asks for the number of registrations. The
        // docs say this returns (UINT)-1 with ERROR_INSUFFICIENT_BUFFER,
        // other implementations return 0; only `count` matters here.
        unsafe {
            GetRegisteredRawInputDevices(null_mut(), &mut count, item);
        }
        if count == 0 {
            return Some(Vec::new());
        }
        let mut list = vec![
            RAWINPUTDEVICE {
                usUsagePage: 0,
                usUsage: 0,
                dwFlags: 0,
                hwndTarget: null_mut(),
            };
            count as usize
        ];
        // SAFETY: list has room for `count` entries of `item` bytes each.
        let got = unsafe { GetRegisteredRawInputDevices(list.as_mut_ptr(), &mut count, item) };
        if got != u32::MAX {
            list.truncate(got as usize);
            return Some(list);
        }
    }
    None
}

/// Raw input registration is per process: remove ours only if it still
/// targets this window (a newer instance may have taken it over).
fn unregister_if_ours(hwnd: HWND) {
    let item = size_of::<RAWINPUTDEVICE>() as u32;
    let _lock = registration_lock();
    let Some(list) = registered_devices() else {
        return;
    };
    let ours = list
        .iter()
        .any(|d| is_precision_touchpad(d.usUsagePage, d.usUsage) && d.hwndTarget == hwnd);
    if ours {
        let remove = RAWINPUTDEVICE {
            usUsagePage: PAGE_DIGITIZER,
            usUsage: USAGE_TOUCH_PAD,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: null_mut(),
        };
        // SAFETY: one valid RAWINPUTDEVICE; RIDEV_REMOVE requires a null
        // target window.
        unsafe {
            RegisterRawInputDevices(&remove, 1, item);
        }
    }
}

/// Name of the first Precision Touchpad, `Ok(None)` if there is none.
fn find_touchpad() -> Result<Option<String>, String> {
    let item = size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut list = Vec::new();
    // Devices can arrive between the two calls; retry a few times.
    for _ in 0..4 {
        let mut count = 0u32;
        // SAFETY: a null buffer asks for the device count.
        if unsafe { GetRawInputDeviceList(null_mut(), &mut count, item) } == u32::MAX {
            return Err("Falha ao listar os dispositivos de entrada.".into());
        }
        if count == 0 {
            return Ok(None);
        }
        list = vec![
            RAWINPUTDEVICELIST {
                hDevice: null_mut(),
                dwType: 0,
            };
            count as usize
        ];
        // SAFETY: list has room for `count` entries of `item` bytes each.
        let got = unsafe { GetRawInputDeviceList(list.as_mut_ptr(), &mut count, item) };
        if got != u32::MAX {
            list.truncate(got as usize);
            break;
        }
        list.clear();
    }
    Ok(list
        .iter()
        .filter(|d| d.dwType == RIM_TYPEHID)
        .find_map(|d| touchpad_ids(d.hDevice))
        .map(|(vid, pid)| format!("Touchpad de precisão {vid:04X}:{pid:04X}")))
}

/// Vendor and product id when `device` is a Precision Touchpad collection.
fn touchpad_ids(device: HANDLE) -> Option<(u32, u32)> {
    // SAFETY: RID_DEVICE_INFO is plain data; all-zero is a valid value.
    let mut info: RID_DEVICE_INFO = unsafe { zeroed() };
    info.cbSize = size_of::<RID_DEVICE_INFO>() as u32;
    let mut size = info.cbSize;
    // SAFETY: info is a writable RID_DEVICE_INFO of `size` bytes with
    // cbSize set, as RIDI_DEVICEINFO requires.
    let r = unsafe {
        GetRawInputDeviceInfoW(
            device,
            RIDI_DEVICEINFO,
            (&mut info as *mut RID_DEVICE_INFO).cast(),
            &mut size,
        )
    };
    if r == 0 || r == u32::MAX || info.dwType != RIM_TYPEHID {
        return None;
    }
    // SAFETY: dwType is RIM_TYPEHID, so Windows filled the `hid` variant.
    let hid = unsafe { info.Anonymous.hid };
    is_precision_touchpad(hid.usUsagePage, hid.usUsage).then_some((hid.dwVendorId, hid.dwProductId))
}

thread_local! {
    static STATE: RefCell<Option<ThreadState>> = const { RefCell::new(None) };
}

fn install_state(state: ThreadState) {
    let _ = STATE.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = Some(state);
        }
    });
}

fn take_state() -> Option<ThreadState> {
    STATE
        .try_with(|cell| cell.try_borrow_mut().ok().and_then(|mut slot| slot.take()))
        .ok()
        .flatten()
}

/// Run `f` on this thread's state, if any. Never panics: a re-entrant call
/// (should Windows ever nest messages) is simply skipped.
fn with_state(f: impl FnOnce(&mut ThreadState)) {
    let _ = STATE.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            if let Some(state) = slot.as_mut() {
                f(state);
            }
        }
    });
}

/// Window procedure of the message-only window.
///
/// # Safety
/// Called by Windows with the arguments of a message for our window.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_INPUT => {
            with_state(|s| s.on_input(lparam as HRAWINPUT));
            // SAFETY: forwarding the original message; DefWindowProcW does
            // the raw input cleanup Windows asks for.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_INPUT_DEVICE_CHANGE => {
            let kind = u32::try_from(wparam).unwrap_or(0);
            with_state(|s| s.on_device_change(kind, lparam));
            0
        }
        WM_TIMER if wparam == TICK_TIMER_ID => {
            with_state(ThreadState::on_tick);
            0
        }
        WM_APP_GRAB => {
            with_state(ThreadState::apply_grab);
            0
        }
        WM_CLOSE => {
            // SAFETY: hwnd is our own window and we are on its thread.
            unsafe {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            with_state(|s| s.destroyed = true);
            // SAFETY: plain call; ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: forwarding the original message unchanged.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// What is known about one raw input device handle.
enum Slot {
    /// A Precision Touchpad being parsed.
    Ready(Device),
    /// Definitively not a usable Precision Touchpad; its input is ignored.
    NotUsable,
    /// Reading its description failed; skip this many reports, then retry.
    Retry { skip: u32 },
}

/// Why [`Device::open`] gave up.
enum OpenError {
    /// The device is not a usable Precision Touchpad.
    NotUsable,
    /// A Win32 call failed; worth another try later.
    Failed,
}

/// Input-thread state, reached from the wndproc through `STATE`.
struct ThreadState {
    shared: Arc<Shared>,
    /// Reused `GetRawInputData` buffer; `u64` keeps it 8-byte aligned.
    buf: Vec<u64>,
    /// Per device handle.
    devices: HashMap<isize, Slot>,
    /// Cursor position the grab pins to, while the clip is applied.
    pin: Option<POINT>,
    /// When the last touch report was parsed, for the watchdog.
    last_report: Option<Instant>,
    /// WM_DESTROY was seen.
    destroyed: bool,
}

impl ThreadState {
    fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            buf: Vec::new(),
            devices: HashMap::new(),
            pin: None,
            last_report: None,
            destroyed: false,
        }
    }

    /// Every [`TICK_MS`]: keep the grab applied and lift a stale touch.
    fn on_tick(&mut self) {
        self.apply_grab();
        let touching = self.shared.touching.load(Ordering::Acquire);
        let since = self.last_report.map(|t| t.elapsed());
        if touch_is_stale(touching, since) {
            self.shared.lift();
            // A frame left half-read (lost report) must not carry over.
            for slot in self.devices.values_mut() {
                if let Slot::Ready(device) = slot {
                    device.tracker = FrameTracker::default();
                }
            }
        }
    }

    /// Bring the cursor clip in line with the grab flag.
    fn apply_grab(&mut self) {
        let want =
            self.shared.grab.load(Ordering::SeqCst) && !self.shared.stop.load(Ordering::SeqCst);
        if want {
            if self.pin.is_none() {
                let mut p = POINT { x: 0, y: 0 };
                // SAFETY: p is a valid out pointer.
                if unsafe { GetCursorPos(&mut p) } == 0 {
                    return; // e.g. secure desktop; retried on the next tick
                }
                self.pin = Some(p);
            }
            if let Some(p) = self.pin {
                let rect = RECT {
                    left: p.x,
                    top: p.y,
                    right: p.x.saturating_add(1),
                    bottom: p.y.saturating_add(1),
                };
                // SAFETY: rect is a valid RECT for the duration of the call.
                unsafe {
                    ClipCursor(&rect);
                }
            }
        } else if self.pin.take().is_some() {
            release_cursor();
        }
    }

    fn on_device_change(&mut self, kind: u32, device: isize) {
        // Arrival or removal: forget the cached device either way (handles
        // can be reused). A removed touchpad can no longer be touched.
        let was_touchpad = matches!(self.devices.remove(&device), Some(Slot::Ready(_)));
        if was_touchpad && kind == GIDC_REMOVAL {
            self.shared.lift();
        }
    }

    fn on_input(&mut self, input: HRAWINPUT) {
        self.apply_grab();

        let header_size = size_of::<RAWINPUTHEADER>() as u32;
        let mut size = 0u32;
        // SAFETY: a null buffer asks for the size of this input.
        let r = unsafe { GetRawInputData(input, RID_INPUT, null_mut(), &mut size, header_size) };
        if r != 0 || size == 0 || size > MAX_INPUT_BYTES {
            return;
        }
        let words = (size as usize).div_ceil(size_of::<u64>());
        if self.buf.len() < words {
            self.buf.resize(words, 0);
        }
        // SAFETY: buf is 8-byte aligned (GetRawInputData asks for 4) and
        // has at least `size` writable bytes.
        let copied = unsafe {
            GetRawInputData(
                input,
                RID_INPUT,
                self.buf.as_mut_ptr().cast(),
                &mut size,
                header_size,
            )
        };
        if copied == u32::MAX {
            return;
        }
        let Some(data) = as_bytes_mut(&mut self.buf).get_mut(..copied as usize) else {
            return;
        };
        if data.len() < size_of::<RAWINPUTHEADER>() {
            return;
        }
        // SAFETY: data holds at least a RAWINPUTHEADER; read_unaligned
        // copies it out whatever the buffer's alignment.
        let header: RAWINPUTHEADER =
            unsafe { data.as_ptr().cast::<RAWINPUTHEADER>().read_unaligned() };
        if header.dwType != RIM_TYPEHID {
            return;
        }
        // RAWHID: dwSizeHid, dwCount, then dwCount reports of dwSizeHid bytes.
        let hid = offset_of!(RAWINPUT, data);
        let (Some(size_hid), Some(count)) = (
            read_u32(data, hid + offset_of!(RAWHID, dwSizeHid)),
            read_u32(data, hid + offset_of!(RAWHID, dwCount)),
        ) else {
            return;
        };
        let size_hid = size_hid as usize;
        if size_hid == 0 {
            return;
        }
        let start = hid + offset_of!(RAWHID, bRawData);
        let Some(reports) = size_hid
            .checked_mul(count as usize)
            .and_then(|total| data.get_mut(start..start.checked_add(total)?))
        else {
            return;
        };

        let slot = self
            .devices
            .entry(header.hDevice as isize)
            .or_insert(Slot::Retry { skip: 0 });
        if let Slot::Retry { skip } = slot {
            if *skip > 0 {
                *skip -= 1;
                return;
            }
            *slot = match Device::open(header.hDevice) {
                Ok(device) => Slot::Ready(device),
                Err(OpenError::NotUsable) => Slot::NotUsable,
                Err(OpenError::Failed) => Slot::Retry {
                    skip: OPEN_RETRY_REPORTS,
                },
            };
        }
        let Slot::Ready(device) = slot else {
            return;
        };
        let mut parsed = false;
        for report in reports.chunks_exact_mut(size_hid) {
            let Some(update) = device.process(report) else {
                continue;
            };
            parsed = true;
            match pad_change(&device.layout, update) {
                PadChange::Keep => {}
                PadChange::Touch { x, y } => self.shared.touch(x, y),
                PadChange::Lift => self.shared.lift(),
            }
        }
        if parsed {
            self.last_report = Some(Instant::now());
        }
    }
}

/// The bytes of a `u64` buffer.
fn as_bytes_mut(words: &mut [u64]) -> &mut [u8] {
    let len = size_of_val(words);
    // SAFETY: u8 has no alignment or validity requirements, the `len` bytes
    // are exactly the slice's memory, and the result keeps its borrow.
    unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), len) }
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_ne_bytes(bytes.try_into().ok()?))
}

/// A Precision Touchpad's parsing state.
struct Device {
    /// Preparsed HID data (`RIDI_PREPARSEDDATA`), u64 for alignment.
    preparsed: Vec<u64>,
    layout: Layout,
    /// Scratch for `HidP_GetUsages`.
    usages: Vec<u16>,
    tracker: FrameTracker,
}

impl Device {
    /// Fetch the preparsed data and work out the report layout. A failing
    /// Win32 call is [`OpenError::Failed`] (retried later); a device that
    /// describes itself as something else is [`OpenError::NotUsable`].
    fn open(handle: HANDLE) -> Result<Device, OpenError> {
        let mut size = 0u32;
        // SAFETY: a null buffer asks for the preparsed data size in bytes.
        let r =
            unsafe { GetRawInputDeviceInfoW(handle, RIDI_PREPARSEDDATA, null_mut(), &mut size) };
        if r != 0 || size == 0 {
            return Err(OpenError::Failed);
        }
        if size > MAX_PREPARSED_BYTES {
            return Err(OpenError::NotUsable);
        }
        let mut preparsed = vec![0u64; (size as usize).div_ceil(size_of::<u64>())];
        let mut capacity =
            u32::try_from(size_of_val(preparsed.as_slice())).map_err(|_| OpenError::NotUsable)?;
        // SAFETY: preparsed has `capacity` writable bytes.
        let r = unsafe {
            GetRawInputDeviceInfoW(
                handle,
                RIDI_PREPARSEDDATA,
                preparsed.as_mut_ptr().cast(),
                &mut capacity,
            )
        };
        if r == 0 || r == u32::MAX {
            return Err(OpenError::Failed);
        }
        let pp = preparsed_ptr(&preparsed);

        // SAFETY: HIDP_CAPS is plain data; all-zero is a valid value.
        let mut caps: HIDP_CAPS = unsafe { zeroed() };
        // SAFETY: pp points at preparsed data owned by `preparsed`; caps is
        // a valid out pointer.
        if unsafe { HidP_GetCaps(pp, &mut caps) } != HIDP_STATUS_SUCCESS {
            return Err(OpenError::Failed);
        }
        if !is_precision_touchpad(caps.UsagePage, caps.Usage) || caps.NumberInputValueCaps == 0 {
            return Err(OpenError::NotUsable);
        }
        // SAFETY: HIDP_VALUE_CAPS is plain data; all-zero is a valid value.
        let blank: HIDP_VALUE_CAPS = unsafe { zeroed() };
        let mut raw = vec![blank; usize::from(caps.NumberInputValueCaps)];
        let mut len = caps.NumberInputValueCaps;
        // SAFETY: raw has room for `len` entries; pp as above.
        if unsafe { HidP_GetValueCaps(HidP_Input, raw.as_mut_ptr(), &mut len, pp) }
            != HIDP_STATUS_SUCCESS
        {
            return Err(OpenError::Failed);
        }
        raw.truncate(usize::from(len));
        let caps: Vec<ValueCap> = raw.iter().map(value_cap).collect();
        let layout = layout(&caps).ok_or(OpenError::NotUsable)?;

        // SAFETY: pp as above.
        let max = unsafe { HidP_MaxUsageListLength(HidP_Input, PAGE_DIGITIZER, pp) };
        let usages = vec![0u16; (max as usize).clamp(1, MAX_USAGE_LIST)];
        Ok(Device {
            preparsed,
            layout,
            usages,
            tracker: FrameTracker::default(),
        })
    }

    /// Parse one input report of `InputReportByteLength` bytes. `None` when
    /// it is not a touch report (another report id).
    fn process(&mut self, report: &mut [u8]) -> Option<Update> {
        let Device {
            preparsed,
            layout,
            usages,
            tracker,
        } = self;
        let first = layout.fingers.first()?;
        // Not a touch report: leave the state alone.
        usage_value(
            preparsed,
            PAGE_GENERIC_DESKTOP,
            first.link_collection,
            USAGE_X,
            report,
        )?;
        // Link collection 0: Contact Count sits in the top-level collection.
        let count = if layout.has_contact_count {
            usage_value(preparsed, PAGE_DIGITIZER, 0, USAGE_CONTACT_COUNT, report)
        } else {
            None
        };
        let fingers = &layout.fingers;
        Some(tracker.feed(count, fingers.len(), |i| {
            let f = fingers.get(i)?;
            let link = f.link_collection;
            let x = usage_value(preparsed, PAGE_GENERIC_DESKTOP, link, USAGE_X, report)?;
            let y = usage_value(preparsed, PAGE_GENERIC_DESKTOP, link, USAGE_Y, report)?;
            Some(Contact {
                tip: tip_switch(preparsed, link, usages, report),
                x: f.x.decode(x),
                y: f.y.decode(y),
            })
        }))
    }
}

fn preparsed_ptr(preparsed: &[u64]) -> PHIDP_PREPARSED_DATA {
    preparsed.as_ptr() as PHIDP_PREPARSED_DATA
}

/// `HidP_GetUsageValue` for one input usage, `None` if the report does not
/// carry it.
fn usage_value(preparsed: &[u64], page: u16, link: u16, usage: u16, report: &[u8]) -> Option<u32> {
    let len = u32::try_from(report.len()).ok()?;
    let mut value = 0u32;
    // SAFETY: preparsed holds this device's preparsed data and outlives the
    // call; report is a readable buffer of `len` bytes; value is a valid
    // out pointer.
    let status = unsafe {
        HidP_GetUsageValue(
            HidP_Input,
            page,
            link,
            usage,
            &mut value,
            preparsed_ptr(preparsed),
            report.as_ptr(),
            len,
        )
    };
    (status == HIDP_STATUS_SUCCESS).then_some(value)
}

/// Tip switch of finger collection `link` in this report.
fn tip_switch(preparsed: &[u64], link: u16, usages: &mut [u16], report: &mut [u8]) -> bool {
    let (Ok(len), Ok(mut n)) = (u32::try_from(report.len()), u32::try_from(usages.len())) else {
        return false;
    };
    // SAFETY: usages has room for `n` entries; preparsed and report as in
    // `usage_value` (HidP_GetUsages takes the report as non-const but only
    // reads it).
    let status = unsafe {
        HidP_GetUsages(
            HidP_Input,
            PAGE_DIGITIZER,
            link,
            usages.as_mut_ptr(),
            &mut n,
            preparsed_ptr(preparsed),
            report.as_mut_ptr(),
            len,
        )
    };
    status == HIDP_STATUS_SUCCESS && usages.get(..n as usize).is_some_and(tip_on)
}

fn value_cap(v: &HIDP_VALUE_CAPS) -> ValueCap {
    // SAFETY: both union variants are plain u16 fields over the same bytes,
    // so reading either is defined; IsRange says which one is meaningful.
    let (usage, range_min, range_max) = unsafe {
        (
            v.Anonymous.NotRange.Usage,
            v.Anonymous.Range.UsageMin,
            v.Anonymous.Range.UsageMax,
        )
    };
    let (usage_min, usage_max) = usage_span(v.IsRange != 0, usage, range_min, range_max);
    ValueCap {
        usage_page: v.UsagePage,
        usage_min,
        usage_max,
        link_collection: v.LinkCollection,
        bit_size: v.BitSize,
        logical_min: v.LogicalMin,
        logical_max: v.LogicalMax,
        physical_min: v.PhysicalMin,
        physical_max: v.PhysicalMax,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad_without_thread() -> WindowsTouchpad {
        WindowsTouchpad {
            name: "test".into(),
            shared: Arc::new(Shared::new()),
            thread: None,
        }
    }

    #[test]
    fn rawinput_layout_matches_the_offsets_used() {
        assert_eq!(offset_of!(RAWINPUT, data), size_of::<RAWINPUTHEADER>());
        assert_eq!(offset_of!(RAWHID, dwSizeHid), 0);
        assert_eq!(offset_of!(RAWHID, dwCount), 4);
        assert_eq!(offset_of!(RAWHID, bRawData), 8);
    }

    #[test]
    fn read_u32_is_bounds_checked() {
        let data = 0x1234_5678u32.to_ne_bytes();
        assert_eq!(read_u32(&data, 0), Some(0x1234_5678));
        assert_eq!(read_u32(&data, 1), None);
        assert_eq!(read_u32(&data, usize::MAX), None);
    }

    #[test]
    fn byte_view_covers_the_whole_aligned_buffer() {
        let mut words = vec![0u64; 3];
        let bytes = as_bytes_mut(&mut words);
        assert_eq!(bytes.len(), 24);
        assert_eq!(bytes.as_ptr() as usize % 8, 0);
        bytes[8..16].copy_from_slice(&7u64.to_ne_bytes());
        assert_eq!(words, vec![0, 7, 0]);
        assert!(as_bytes_mut(&mut []).is_empty());
    }

    #[test]
    fn wide_is_nul_terminated() {
        assert_eq!(wide("Ab"), vec![u16::from(b'A'), u16::from(b'b'), 0]);
    }

    #[test]
    fn value_cap_reads_not_range_and_range() {
        // SAFETY: plain data; all-zero is a valid value.
        let mut v: HIDP_VALUE_CAPS = unsafe { zeroed() };
        v.UsagePage = PAGE_GENERIC_DESKTOP;
        v.LinkCollection = 3;
        v.BitSize = 12;
        v.LogicalMin = 0;
        v.LogicalMax = 1216;
        v.PhysicalMin = 0;
        v.PhysicalMax = 1200;
        v.IsRange = 0;
        v.Anonymous.NotRange.Usage = USAGE_Y;
        let c = value_cap(&v);
        assert_eq!((c.usage_min, c.usage_max), (USAGE_Y, USAGE_Y));
        assert_eq!(c.link_collection, 3);
        assert_eq!((c.logical_min, c.logical_max), (0, 1216));
        assert_eq!((c.physical_min, c.physical_max), (0, 1200));
        assert_eq!(c.bit_size, 12);

        v.IsRange = 1;
        v.Anonymous.Range.UsageMin = USAGE_X;
        v.Anonymous.Range.UsageMax = USAGE_Y;
        let c = value_cap(&v);
        assert_eq!((c.usage_min, c.usage_max), (USAGE_X, USAGE_Y));
    }

    #[test]
    fn latest_reflects_shared_state() {
        let pad = pad_without_thread();
        let at = |x, y, touching| Touch { x, y, touching };
        assert_eq!(pad.latest(), at(0.5, 0.5, false));
        pad.shared.touch(0.25, 0.75);
        assert_eq!(pad.latest(), at(0.25, 0.75, true));
        pad.shared.lift();
        assert_eq!(pad.latest(), at(0.25, 0.75, false));
    }

    #[test]
    fn grab_flag_round_trips_without_a_thread() {
        let mut pad = pad_without_thread();
        assert!(!pad.grabbed());
        assert_eq!(pad.set_grab(true), Ok(()));
        assert!(pad.grabbed());
        assert_eq!(pad.set_grab(false), Ok(()));
        assert!(!pad.grabbed());
        assert_eq!(pad.set_grab(false), Ok(()));
    }

    /// Exercises the real Win32 path: on a machine without a Precision
    /// Touchpad it must fail with a user-facing reason; with one it must
    /// open, grab, release and close without hanging.
    #[test]
    fn open_succeeds_or_reports_why() {
        match open() {
            Ok(mut pad) => {
                assert!(!pad.name().is_empty());
                assert!(!pad.grabbed());
                assert_eq!(pad.set_grab(true), Ok(()));
                assert_eq!(pad.set_grab(false), Ok(()));
                let t = pad.latest();
                assert!((0.0..=1.0).contains(&t.x) && (0.0..=1.0).contains(&t.y));
                drop(pad);
            }
            Err(reason) => assert!(!reason.is_empty()),
        }
    }
}
