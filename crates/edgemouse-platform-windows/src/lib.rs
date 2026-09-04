//! Native Win32 mouse adapter for Windows.

#![cfg(target_os = "windows")]

use edgemouse_core::{
    ButtonState, CaptureMode, DisplayGeometry, KeyCode, KeyState, KeyboardCaptureBackend,
    KeyboardEvent, KeyboardInjectionBackend, MouseButton, MouseCaptureBackend,
    MouseInjectionBackend, PermissionState, PhysicalMouseEvent, PlatformError, Point, Rect,
    RemoteMouseEvent, Vector,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::c_void;
use std::mem::{MaybeUninit, size_of};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::JoinHandle;

const WH_MOUSE_LL: i32 = 14;
const WH_KEYBOARD_LL: i32 = 13;
const HC_ACTION: i32 = 0;
const WM_QUIT: u32 = 0x0012;
const WM_INPUT: u32 = 0x00ff;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_MOUSEWHEEL: u32 = 0x020a;
const WM_XBUTTONDOWN: u32 = 0x020b;
const WM_XBUTTONUP: u32 = 0x020c;
const WM_MOUSEHWHEEL: u32 = 0x020e;
const CTRL_C_EVENT: u32 = 0;
const CTRL_BREAK_EVENT: u32 = 1;
const CTRL_CLOSE_EVENT: u32 = 2;
const CTRL_LOGOFF_EVENT: u32 = 5;
const CTRL_SHUTDOWN_EVENT: u32 = 6;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const PM_NOREMOVE: u32 = 0;

const HID_USAGE_PAGE_GENERIC: u16 = 0x01;
const HID_USAGE_GENERIC_MOUSE: u16 = 0x02;
const RIDEV_REMOVE: u32 = 0x0000_0001;
const RIDEV_INPUTSINK: u32 = 0x0000_0100;
const RID_INPUT: u32 = 0x1000_0003;
const RIM_TYPEMOUSE: u32 = 0;
const MOUSE_MOVE_ABSOLUTE: u16 = 0x0001;
const MOUSE_VIRTUAL_DESKTOP: u16 = 0x0002;
const RI_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x0001;
const RI_MOUSE_LEFT_BUTTON_UP: u16 = 0x0002;
const RI_MOUSE_RIGHT_BUTTON_DOWN: u16 = 0x0004;
const RI_MOUSE_RIGHT_BUTTON_UP: u16 = 0x0008;
const RI_MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x0010;
const RI_MOUSE_MIDDLE_BUTTON_UP: u16 = 0x0020;
const RI_MOUSE_BUTTON_4_DOWN: u16 = 0x0040;
const RI_MOUSE_BUTTON_4_UP: u16 = 0x0080;
const RI_MOUSE_BUTTON_5_DOWN: u16 = 0x0100;
const RI_MOUSE_BUTTON_5_UP: u16 = 0x0200;
const RI_MOUSE_WHEEL: u16 = 0x0400;
const RI_MOUSE_HORIZONTAL_WHEEL: u16 = 0x0800;

const LLMHF_INJECTED: u32 = 0x0000_0001;
const LLKHF_EXTENDED: u32 = 0x0000_0001;
const LLKHF_INJECTED: u32 = 0x0000_0010;
const XBUTTON1: u16 = 1;
const XBUTTON2: u16 = 2;
const WHEEL_DELTA: f64 = 120.0;
const EVENT_MARKER: usize = 0x4544_4745;
const CAPTURE_QUEUE_CAPACITY: usize = 4_096;

const INPUT_MOUSE: u32 = 0;
const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_EXTENDEDKEY: u32 = 0x0001;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
const MOUSEEVENTF_MIDDLEDOWN: u32 = 0x0020;
const MOUSEEVENTF_MIDDLEUP: u32 = 0x0040;
const MOUSEEVENTF_XDOWN: u32 = 0x0080;
const MOUSEEVENTF_XUP: u32 = 0x0100;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const MOUSEEVENTF_HWHEEL: u32 = 0x1000;
const MOUSEEVENTF_VIRTUALDESK: u32 = 0x4000;
const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;

const SM_XVIRTUALSCREEN: i32 = 76;
const SM_YVIRTUALSCREEN: i32 = 77;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;
const SM_CMONITORS: i32 = 80;
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;
const HWND_MESSAGE: isize = -3;
const MONITORINFOF_PRIMARY: u32 = 1;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PointI32 {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RectI32 {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: RectI32,
    work: RectI32,
    flags: u32,
}

#[repr(C)]
struct MouseHookData {
    point: PointI32,
    mouse_data: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
struct KeyboardHookData {
    virtual_key: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
struct Message {
    window: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    point: PointI32,
    private: u32,
}

#[repr(C)]
struct RawInputDevice {
    usage_page: u16,
    usage: u16,
    flags: u32,
    target: *mut c_void,
}

#[repr(C)]
struct RawInputHeader {
    input_type: u32,
    size: u32,
    device: *mut c_void,
    w_param: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct RawMouse {
    flags: u16,
    alignment: u16,
    buttons: u32,
    raw_buttons: u32,
    last_x: i32,
    last_y: i32,
    extra_information: u32,
}

#[repr(C)]
struct RawMouseInput {
    header: RawInputHeader,
    mouse: RawMouse,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MouseInput {
    dx: i32,
    dy: i32,
    mouse_data: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KeyboardInput {
    virtual_key: u16,
    scan_code: u16,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
union InputValue {
    mouse: MouseInput,
    keyboard: KeyboardInput,
}

#[repr(C)]
struct Input {
    input_type: u32,
    value: InputValue,
}

type HookProc = unsafe extern "system" fn(code: i32, w_param: usize, l_param: isize) -> isize;
type ConsoleCtrlHandler = unsafe extern "system" fn(control_type: u32) -> i32;

#[link(name = "User32")]
unsafe extern "system" {
    fn SetWindowsHookExW(
        hook_id: i32,
        callback: Option<HookProc>,
        module: *mut c_void,
        thread_id: u32,
    ) -> *mut c_void;
    fn UnhookWindowsHookEx(hook: *mut c_void) -> i32;
    fn CallNextHookEx(hook: *mut c_void, code: i32, w_param: usize, l_param: isize) -> isize;
    fn GetMessageW(
        message: *mut Message,
        window: *mut c_void,
        min_filter: u32,
        max_filter: u32,
    ) -> i32;
    fn PeekMessageW(
        message: *mut Message,
        window: *mut c_void,
        min_filter: u32,
        max_filter: u32,
        remove: u32,
    ) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
    fn PostThreadMessageW(thread_id: u32, message: u32, w_param: usize, l_param: isize) -> i32;
    fn CreateWindowExW(
        extended_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: *mut c_void,
        menu: *mut c_void,
        instance: *mut c_void,
        parameter: *mut c_void,
    ) -> *mut c_void;
    fn DestroyWindow(window: *mut c_void) -> i32;
    fn RegisterRawInputDevices(devices: *const RawInputDevice, count: u32, size: u32) -> i32;
    fn GetRawInputData(
        raw_input: *mut c_void,
        command: u32,
        data: *mut c_void,
        size: *mut u32,
        header_size: u32,
    ) -> u32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn GetCursorPos(point: *mut PointI32) -> i32;
    fn ShowCursor(show: i32) -> i32;
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetDpiForSystem() -> u32;
    fn SetProcessDpiAwarenessContext(value: isize) -> i32;
    fn EnumDisplayMonitors(
        device_context: *mut c_void,
        clip: *const RectI32,
        callback: Option<
            unsafe extern "system" fn(*mut c_void, *mut c_void, *mut RectI32, isize) -> i32,
        >,
        data: isize,
    ) -> i32;
    fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
}

#[link(name = "Shcore")]
unsafe extern "system" {
    fn GetDpiForMonitor(
        monitor: *mut c_void,
        dpi_type: u32,
        dpi_x: *mut u32,
        dpi_y: *mut u32,
    ) -> i32;
}

#[link(name = "Kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetTickCount() -> u32;
    fn SetConsoleCtrlHandler(handler: Option<ConsoleCtrlHandler>, add: i32) -> i32;
}

struct CallbackState {
    sender: mpsc::SyncSender<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    transitioning: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    last_point: Arc<Mutex<Option<Point>>>,
    coordinate_scale: f64,
    raw_input_requested: bool,
    raw_input_active: AtomicBool,
    raw_cutoff_time: Arc<AtomicU32>,
}

#[derive(Debug, Clone, Copy)]
struct MouseCaptureStartup {
    thread_id: u32,
    raw_input: bool,
}

#[derive(Default)]
struct RawMovementState {
    absolute_positions: BTreeMap<usize, Point>,
}

static CALLBACK_STATE: AtomicPtr<CallbackState> = AtomicPtr::new(ptr::null_mut());
static KEYBOARD_CALLBACK_STATE: AtomicPtr<KeyboardCallbackState> = AtomicPtr::new(ptr::null_mut());
static KEYBOARD_REMOTE_ACTIVE: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_STATE: OnceLock<Arc<AtomicBool>> = OnceLock::new();

#[derive(Default)]
struct KeyboardRoutingState {
    remote: bool,
    local_pressed: BTreeSet<KeyCode>,
    captured_pressed: BTreeSet<KeyCode>,
    passthrough_pressed: BTreeSet<KeyCode>,
}

struct KeyboardCallbackState {
    sender: mpsc::SyncSender<KeyboardEvent>,
    routing: Arc<Mutex<KeyboardRoutingState>>,
    overflowed: Arc<AtomicBool>,
    emergency_release: Arc<AtomicBool>,
}

pub fn current_pointer() -> Result<Point, PlatformError> {
    let mut point = PointI32 { x: 0, y: 0 };
    // SAFETY: `point` is initialized writable storage for one Win32 POINT.
    if unsafe { GetCursorPos(&raw mut point) } == 0 {
        Err(PlatformError::new("GetCursorPos failed"))
    } else {
        Ok(Point::new(f64::from(point.x), f64::from(point.y)))
    }
}

/// Returns the Windows virtual desktop in per-monitor-aware physical coordinates.
/// This includes rotated and secondary displays and avoids DPI virtualization in
/// the low-level hook coordinate stream.
pub fn desktop_geometry() -> Result<(Rect, f64, Vec<DisplayGeometry>), PlatformError> {
    // SAFETY: this changes process coordinate interpretation and retains no Rust memory.
    // A zero result is also expected when an embedding manifest already selected DPI mode.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    // SAFETY: GetSystemMetrics and GetDpiForSystem have no pointer parameters.
    let (left, top, width, height, count, dpi) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
            GetSystemMetrics(SM_CMONITORS),
            GetDpiForSystem(),
        )
    };
    if width <= 0 || height <= 0 || count <= 0 {
        return Err(PlatformError::new(
            "Windows reported invalid virtual desktop geometry",
        ));
    }
    let bounds = Rect::new(
        Point::new(f64::from(left), f64::from(top)),
        f64::from(width),
        f64::from(height),
    )
    .map_err(|error| PlatformError::new(format!("invalid Windows desktop bounds: {error}")))?;
    let scale_factor = (f64::from(dpi) / 96.0).max(1.0);
    let mut displays = Vec::with_capacity(
        usize::try_from(count).map_err(|_| PlatformError::new("invalid Windows display count"))?,
    );
    // SAFETY: the callback only appends to `displays` during this synchronous call.
    let enumerated = unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(collect_display_geometry),
            (&raw mut displays) as isize,
        )
    };
    if enumerated == 0 || displays.is_empty() {
        return Err(PlatformError::new(
            "Windows failed to enumerate active displays",
        ));
    }
    Ok((bounds, scale_factor, displays))
}

unsafe extern "system" fn collect_display_geometry(
    monitor: *mut c_void,
    _device_context: *mut c_void,
    monitor_rect: *mut RectI32,
    data: isize,
) -> i32 {
    if monitor_rect.is_null() || data == 0 {
        return 0;
    }
    // SAFETY: EnumDisplayMonitors supplies a valid rectangle for the callback duration.
    let rect = unsafe { *monitor_rect };
    let width = rect.right.saturating_sub(rect.left);
    let height = rect.bottom.saturating_sub(rect.top);
    if width <= 0 || height <= 0 {
        return 1;
    }
    let Ok(bounds) = Rect::new(
        Point::new(f64::from(rect.left), f64::from(rect.top)),
        f64::from(width),
        f64::from(height),
    ) else {
        return 1;
    };
    let mut info = MonitorInfo {
        size: u32::try_from(size_of::<MonitorInfo>()).expect("MONITORINFO size fits in u32"),
        monitor: rect,
        work: rect,
        flags: 0,
    };
    // SAFETY: `info` is initialized writable storage for one MONITORINFO value.
    let primary = unsafe { GetMonitorInfoW(monitor, &raw mut info) } != 0
        && info.flags & MONITORINFOF_PRIMARY != 0;
    let mut dpi_x = 96_u32;
    let mut dpi_y = 96_u32;
    // SAFETY: the monitor handle comes from EnumDisplayMonitors and both DPI outputs are writable.
    if unsafe { GetDpiForMonitor(monitor, 0, &raw mut dpi_x, &raw mut dpi_y) } != 0 {
        dpi_x = 96;
    }
    let display = DisplayGeometry {
        bounds,
        pixel_width: u32::try_from(width).expect("positive i32 width fits in u32"),
        pixel_height: u32::try_from(height).expect("positive i32 height fits in u32"),
        scale_factor: (f64::from(dpi_x) / 96.0).max(1.0),
        primary,
    };
    // SAFETY: `data` is the exclusive Vec pointer passed to the synchronous enumeration call.
    unsafe { &mut *(data as *mut Vec<DisplayGeometry>) }.push(display);
    1
}

/// A low-level mouse hook hosted on a dedicated Win32 message-loop thread.
pub struct WindowsMouseCapture {
    receiver: mpsc::Receiver<PhysicalMouseEvent>,
    deferred_events: VecDeque<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    transitioning: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    last_point: Arc<Mutex<Option<Point>>>,
    raw_cutoff_time: Arc<AtomicU32>,
    capture_anchor: Point,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    cursor_hidden: bool,
}

impl WindowsMouseCapture {
    pub fn start(
        coordinate_scale: f64,
        capture_anchor: Point,
        initial_pointer: Point,
        raw_input_enabled: bool,
    ) -> Result<Self, PlatformError> {
        if !coordinate_scale.is_finite() || coordinate_scale <= 0.0 {
            return Err(PlatformError::new(
                "Windows coordinate scale must be finite and positive",
            ));
        }
        if !capture_anchor.is_finite() {
            return Err(PlatformError::new("Windows capture anchor must be finite"));
        }
        if !initial_pointer.is_finite() {
            return Err(PlatformError::new("Windows initial pointer must be finite"));
        }
        if !CALLBACK_STATE.load(Ordering::Acquire).is_null() {
            return Err(PlatformError::new(
                "only one Windows mouse capture instance is supported",
            ));
        }
        let (event_sender, event_receiver) = mpsc::sync_channel(CAPTURE_QUEUE_CAPACITY);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let suppress = Arc::new(AtomicBool::new(false));
        let callback_suppress = Arc::clone(&suppress);
        let transitioning = Arc::new(AtomicBool::new(false));
        let callback_transitioning = Arc::clone(&transitioning);
        let overflowed = Arc::new(AtomicBool::new(false));
        let callback_overflowed = Arc::clone(&overflowed);
        // Seed the hook reference with the same normalized position used by the
        // routing session. This preserves the very first outward delta when the
        // program becomes ready while the cursor is already on a screen edge.
        let last_point = Arc::new(Mutex::new(Some(initial_pointer)));
        let callback_last_point = Arc::clone(&last_point);
        // SAFETY: GetTickCount has no pointer parameters and supplies the same
        // timestamp domain used by Win32 input messages.
        let raw_cutoff_time = Arc::new(AtomicU32::new(unsafe { GetTickCount() }));
        let callback_raw_cutoff_time = Arc::clone(&raw_cutoff_time);
        let thread = std::thread::Builder::new()
            .name("edgemouse-win32-hook".to_owned())
            .spawn(move || {
                run_hook_thread(
                    CallbackState {
                        sender: event_sender,
                        suppress: callback_suppress,
                        transitioning: callback_transitioning,
                        overflowed: callback_overflowed,
                        last_point: callback_last_point,
                        coordinate_scale,
                        raw_input_requested: raw_input_enabled,
                        raw_input_active: AtomicBool::new(false),
                        raw_cutoff_time: callback_raw_cutoff_time,
                    },
                    |result| {
                        drop(startup_sender.send(result));
                    },
                );
            })
            .map_err(|error| PlatformError::new(format!("failed to start mouse hook: {error}")))?;
        let startup = startup_receiver
            .recv()
            .map_err(|_| PlatformError::new("mouse hook exited during startup"))??;
        if startup.raw_input {
            println!("Windows mouse capture: Raw Input movement + low-level safety hook");
        } else if raw_input_enabled {
            eprintln!(
                "Windows Raw Input is unavailable; using the low-level mouse hook for movement"
            );
        } else {
            println!("Windows mouse capture: low-level hook (Raw Input disabled in config)");
        }
        Ok(Self {
            receiver: event_receiver,
            deferred_events: VecDeque::new(),
            suppress,
            transitioning,
            overflowed,
            last_point,
            raw_cutoff_time,
            capture_anchor,
            thread_id: startup.thread_id,
            thread: Some(thread),
            cursor_hidden: false,
        })
    }

    fn warp(position: Point) -> Result<(), PlatformError> {
        if !position.is_finite() {
            return Err(PlatformError::new("cursor position must be finite"));
        }
        // SAFETY: SetCursorPos copies two integer coordinates and owns no memory.
        if unsafe { SetCursorPos(rounded_i32(position.x), rounded_i32(position.y)) } == 0 {
            Err(PlatformError::new("SetCursorPos failed"))
        } else {
            Ok(())
        }
    }

    fn hide_cursor(&mut self) {
        if !self.cursor_hidden {
            // SAFETY: Calls are balanced by show_cursor for this adapter instance.
            unsafe { ShowCursor(0) };
            self.cursor_hidden = true;
        }
    }

    fn set_reference_point(&self, position: Point) -> Result<(), PlatformError> {
        let mut last = self
            .last_point
            .lock()
            .map_err(|_| PlatformError::new("Windows cursor reference lock was poisoned"))?;
        *last = Some(position);
        Ok(())
    }

    fn show_cursor(&mut self) {
        if self.cursor_hidden {
            // SAFETY: Calls are balanced with hide_cursor for this adapter instance.
            unsafe { ShowCursor(1) };
            self.cursor_hidden = false;
        }
    }

    fn discard_queued_movements(&mut self) -> Result<(), PlatformError> {
        loop {
            match self.receiver.try_recv() {
                Ok(PhysicalMouseEvent::Move { .. }) => {}
                Ok(event) => self.deferred_events.push_back(event),
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(PlatformError::new("Windows mouse hook stopped"));
                }
            }
        }
    }

    fn transition_pointer(
        &mut self,
        position: Option<Point>,
        suppress: bool,
        hide_cursor: bool,
    ) -> Result<(), PlatformError> {
        self.transitioning.store(true, Ordering::Release);
        self.suppress.store(false, Ordering::Release);

        let result = (|| {
            self.discard_queued_movements()?;
            if let Some(position) = position {
                self.set_reference_point(position)?;
                Self::warp(position)?;
            }
            self.discard_queued_movements()
        })();

        if result.is_ok() {
            if hide_cursor {
                self.hide_cursor();
            } else {
                self.show_cursor();
            }
            self.suppress.store(suppress, Ordering::Release);
        } else {
            self.show_cursor();
            self.suppress.store(false, Ordering::Release);
        }
        // SAFETY: GetTickCount has no pointer parameters. Raw events at or before
        // this transition boundary belong to the previous pointer owner.
        self.raw_cutoff_time
            .store(unsafe { GetTickCount() }, Ordering::Release);
        self.transitioning.store(false, Ordering::Release);
        result
    }
}

impl MouseCaptureBackend for WindowsMouseCapture {
    fn permission_state(&self) -> PermissionState {
        PermissionState::NotRequired
    }

    fn set_mode(&mut self, mode: CaptureMode) -> Result<(), PlatformError> {
        match mode {
            CaptureMode::Local { restore } => self.transition_pointer(restore, false, false),
            CaptureMode::Remote { anchor: _ } => {
                self.transition_pointer(Some(self.capture_anchor), true, true)
            }
            CaptureMode::ReceivingRemote { position } => {
                self.transition_pointer(Some(position), true, false)
            }
        }
    }

    fn try_next_event(&mut self) -> Result<Option<PhysicalMouseEvent>, PlatformError> {
        if self.overflowed.swap(false, Ordering::AcqRel) {
            return Err(PlatformError::new(
                "Windows capture queue overflowed; local input was released",
            ));
        }
        if let Some(event) = self.deferred_events.pop_front() {
            return Ok(Some(event));
        }
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(PlatformError::new("Windows mouse hook stopped"))
            }
        }
    }
}

impl Drop for WindowsMouseCapture {
    fn drop(&mut self) {
        self.transitioning.store(false, Ordering::Release);
        self.suppress.store(false, Ordering::Release);
        self.show_cursor();
        if self.thread_id != 0 {
            // SAFETY: thread_id belongs to the live message-loop thread.
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        }
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

/// A low-level keyboard hook that only suppresses keys while this computer's
/// pointer is controlled by the peer. Keys already held during a handoff keep
/// passing through until released, preventing stuck local modifiers.
pub struct WindowsKeyboardCapture {
    receiver: mpsc::Receiver<KeyboardEvent>,
    routing: Arc<Mutex<KeyboardRoutingState>>,
    overflowed: Arc<AtomicBool>,
    emergency_release: Arc<AtomicBool>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl WindowsKeyboardCapture {
    pub fn start() -> Result<Self, PlatformError> {
        if !KEYBOARD_CALLBACK_STATE.load(Ordering::Acquire).is_null() {
            return Err(PlatformError::new(
                "only one Windows keyboard capture instance is supported",
            ));
        }
        let (event_sender, event_receiver) = mpsc::sync_channel(CAPTURE_QUEUE_CAPACITY);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let routing = Arc::new(Mutex::new(KeyboardRoutingState::default()));
        let overflowed = Arc::new(AtomicBool::new(false));
        let emergency_release = Arc::new(AtomicBool::new(false));
        let thread_routing = Arc::clone(&routing);
        let thread_overflowed = Arc::clone(&overflowed);
        let thread_emergency = Arc::clone(&emergency_release);
        let thread = std::thread::Builder::new()
            .name("edgemouse-win32-keyboard-hook".to_owned())
            .spawn(move || {
                run_keyboard_hook_thread(
                    event_sender,
                    thread_routing,
                    thread_overflowed,
                    thread_emergency,
                    |result| drop(startup_sender.send(result)),
                );
            })
            .map_err(|error| {
                PlatformError::new(format!("failed to start keyboard hook: {error}"))
            })?;
        let thread_id = startup_receiver
            .recv()
            .map_err(|_| PlatformError::new("keyboard hook exited during startup"))??;
        Ok(Self {
            receiver: event_receiver,
            routing,
            overflowed,
            emergency_release,
            thread_id,
            thread: Some(thread),
        })
    }

    fn discard_queue(&mut self) -> Result<(), PlatformError> {
        loop {
            match self.receiver.try_recv() {
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(PlatformError::new("Windows keyboard hook stopped"));
                }
            }
        }
    }
}

impl KeyboardCaptureBackend for WindowsKeyboardCapture {
    fn permission_state(&self) -> PermissionState {
        PermissionState::NotRequired
    }

    fn set_remote(&mut self, remote: bool) -> Result<(), PlatformError> {
        self.discard_queue()?;
        let mut routing = self
            .routing
            .lock()
            .map_err(|_| PlatformError::new("Windows keyboard routing lock was poisoned"))?;
        if remote && !routing.remote {
            let local_pressed = std::mem::take(&mut routing.local_pressed);
            routing.passthrough_pressed.extend(local_pressed);
        }
        routing.remote = remote;
        KEYBOARD_REMOTE_ACTIVE.store(remote, Ordering::Release);
        Ok(())
    }

    fn try_next_event(&mut self) -> Result<Option<KeyboardEvent>, PlatformError> {
        if self.overflowed.swap(false, Ordering::AcqRel) {
            return Err(PlatformError::new(
                "Windows keyboard capture queue overflowed; local input was released",
            ));
        }
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(PlatformError::new("Windows keyboard hook stopped"))
            }
        }
    }

    fn take_emergency_release(&self) -> bool {
        self.emergency_release.swap(false, Ordering::AcqRel)
    }
}

impl Drop for WindowsKeyboardCapture {
    fn drop(&mut self) {
        KEYBOARD_REMOTE_ACTIVE.store(false, Ordering::Release);
        if let Ok(mut routing) = self.routing.lock() {
            routing.remote = false;
        }
        if self.thread_id != 0 {
            // SAFETY: thread_id belongs to the live message-loop thread.
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        }
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

/// Returns whether physical Windows keyboard input currently belongs to the
/// remote computer. The console shutdown handler uses this to avoid treating a
/// forwarded Ctrl+C shortcut as a request to stop EdgeMouse.
#[must_use]
pub fn keyboard_capture_is_remote() -> bool {
    KEYBOARD_REMOTE_ACTIVE.load(Ordering::Acquire)
}

/// Installs a console handler that preserves Ctrl+C as a forwarded shortcut
/// while remote input is active, without ignoring close/logoff/shutdown events.
pub fn install_shutdown_handler(stopping: Arc<AtomicBool>) -> Result<(), PlatformError> {
    SHUTDOWN_STATE
        .set(stopping)
        .map_err(|_| PlatformError::new("Windows shutdown handler is already installed"))?;
    // SAFETY: The callback has system ABI and its state is process-lifetime storage.
    if unsafe { SetConsoleCtrlHandler(Some(console_control_handler), 1) } == 0 {
        return Err(PlatformError::new("SetConsoleCtrlHandler failed"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsoleControlAction {
    PassThrough,
    IgnoreForwardedShortcut,
    Stop,
}

const fn console_control_action(control_type: u32, keyboard_remote: bool) -> ConsoleControlAction {
    match control_type {
        CTRL_C_EVENT if keyboard_remote => ConsoleControlAction::IgnoreForwardedShortcut,
        CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT
        | CTRL_SHUTDOWN_EVENT => ConsoleControlAction::Stop,
        _ => ConsoleControlAction::PassThrough,
    }
}

unsafe extern "system" fn console_control_handler(control_type: u32) -> i32 {
    match console_control_action(control_type, keyboard_capture_is_remote()) {
        ConsoleControlAction::PassThrough => 0,
        ConsoleControlAction::IgnoreForwardedShortcut => 1,
        ConsoleControlAction::Stop => {
            let Some(stopping) = SHUTDOWN_STATE.get() else {
                return 0;
            };
            stopping.store(true, Ordering::Release);
            1
        }
    }
}

/// Injects marked absolute pointer, button, and wheel events with `SendInput`.
pub struct WindowsMouseInjector {
    position: Point,
    pressed: BTreeSet<MouseButton>,
}

impl WindowsMouseInjector {
    #[must_use]
    pub fn new(initial_position: Point) -> Self {
        Self {
            position: initial_position,
            pressed: BTreeSet::new(),
        }
    }

    fn send(&self, flags: u32, mouse_data: u32, dx: i32, dy: i32) -> Result<(), PlatformError> {
        let input = Input {
            input_type: INPUT_MOUSE,
            value: InputValue {
                mouse: MouseInput {
                    dx,
                    dy,
                    mouse_data,
                    flags,
                    time: 0,
                    extra_info: EVENT_MARKER,
                },
            },
        };
        // SAFETY: input points to one initialized INPUT value for the duration of the call.
        let sent = unsafe { SendInput(1, &raw const input, size_of::<Input>() as i32) };
        if sent == 1 {
            Ok(())
        } else {
            Err(PlatformError::new(
                "SendInput failed; UIPI may be blocking a higher-integrity target",
            ))
        }
    }

    fn send_absolute_move(&self) -> Result<(), PlatformError> {
        if !self.position.is_finite() {
            return Err(PlatformError::new("mouse position must be finite"));
        }
        // SAFETY: GetSystemMetrics has no pointer parameters.
        let (left, top, width, height) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        if width <= 1 || height <= 1 {
            return Err(PlatformError::new("invalid Windows virtual desktop bounds"));
        }
        let dx = normalized_absolute(self.position.x, left, width);
        let dy = normalized_absolute(self.position.y, top, height);
        self.send(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            0,
            dx,
            dy,
        )
    }

    fn send_button(
        &mut self,
        button: MouseButton,
        state: ButtonState,
    ) -> Result<(), PlatformError> {
        let (flags, data) = button_flags(button, state)?;
        self.send(flags, data, 0, 0)?;
        match state {
            ButtonState::Pressed => {
                self.pressed.insert(button);
            }
            ButtonState::Released => {
                self.pressed.remove(&button);
            }
        }
        Ok(())
    }

    fn send_wheel(&self, horizontal: f64, vertical: f64) -> Result<(), PlatformError> {
        if !horizontal.is_finite() || !vertical.is_finite() {
            return Err(PlatformError::new("wheel deltas must be finite"));
        }
        if vertical != 0.0 {
            self.send(MOUSEEVENTF_WHEEL, signed_mouse_data(vertical), 0, 0)?;
        }
        if horizontal != 0.0 {
            self.send(MOUSEEVENTF_HWHEEL, signed_mouse_data(horizontal), 0, 0)?;
        }
        Ok(())
    }
}

impl MouseInjectionBackend for WindowsMouseInjector {
    fn permission_state(&self) -> PermissionState {
        PermissionState::NotRequired
    }

    fn inject(&mut self, event: RemoteMouseEvent) -> Result<(), PlatformError> {
        match event {
            RemoteMouseEvent::Enter { position, .. }
            | RemoteMouseEvent::MoveAbsolute { position, .. } => {
                self.position = position;
                self.send_absolute_move()
            }
            RemoteMouseEvent::Button { button, state } => self.send_button(button, state),
            RemoteMouseEvent::Wheel {
                horizontal,
                vertical,
            } => self.send_wheel(horizontal, vertical),
            RemoteMouseEvent::Leave | RemoteMouseEvent::ReleaseAll => self.release_all(),
        }
    }

    fn release_all(&mut self) -> Result<(), PlatformError> {
        let pressed: Vec<_> = self.pressed.iter().copied().collect();
        let mut first_error = None;
        for button in pressed {
            if let Err(error) = self.send_button(button, ButtonState::Released) {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
pub struct WindowsKeyboardInjector {
    pressed: BTreeSet<KeyCode>,
}

impl WindowsKeyboardInjector {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pressed: BTreeSet::new(),
        }
    }

    fn send(&mut self, event: KeyboardEvent) -> Result<(), PlatformError> {
        let (virtual_key, extended) = windows_virtual_key(event.key).ok_or_else(|| {
            PlatformError::new(format!(
                "Windows has no mapping for keyboard usage {:#06x}",
                event.key.usage()
            ))
        })?;
        let mut flags = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
        if event.state == KeyState::Released {
            flags |= KEYEVENTF_KEYUP;
        }
        let input = Input {
            input_type: INPUT_KEYBOARD,
            value: InputValue {
                keyboard: KeyboardInput {
                    virtual_key,
                    scan_code: 0,
                    flags,
                    time: 0,
                    extra_info: EVENT_MARKER,
                },
            },
        };
        // SAFETY: input points to one initialized INPUT value for the call.
        let sent = unsafe { SendInput(1, &raw const input, size_of::<Input>() as i32) };
        if sent != 1 {
            return Err(PlatformError::new(
                "SendInput keyboard injection failed; UIPI may be blocking the target",
            ));
        }
        match event.state {
            KeyState::Pressed => {
                self.pressed.insert(event.key);
            }
            KeyState::Released => {
                self.pressed.remove(&event.key);
            }
        }
        Ok(())
    }
}

impl KeyboardInjectionBackend for WindowsKeyboardInjector {
    fn permission_state(&self) -> PermissionState {
        PermissionState::NotRequired
    }

    fn inject(&mut self, event: KeyboardEvent) -> Result<(), PlatformError> {
        self.send(event)
    }

    fn release_all(&mut self) -> Result<(), PlatformError> {
        let pressed: Vec<_> = self.pressed.iter().copied().collect();
        let mut first_error = None;
        for key in pressed {
            if let Err(error) = self.send(KeyboardEvent {
                key,
                state: KeyState::Released,
                repeat: false,
            }) {
                first_error.get_or_insert(error);
            }
        }
        self.pressed.clear();
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }
}

fn run_hook_thread(
    state: CallbackState,
    report_startup: impl FnOnce(Result<MouseCaptureStartup, PlatformError>),
) {
    let raw_input_requested = state.raw_input_requested;
    let state = Box::new(state);
    let state_ptr = Box::into_raw(state);
    if CALLBACK_STATE
        .compare_exchange(
            ptr::null_mut(),
            state_ptr,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        report_startup(Err(PlatformError::new(
            "another Windows mouse hook is already active",
        )));
        // SAFETY: The pointer was never published by this thread.
        drop(unsafe { Box::from_raw(state_ptr) });
        return;
    }

    // SAFETY: The callback has static lifetime and low-level hooks accept a null module handle.
    let hook =
        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_callback), ptr::null_mut(), 0) };
    if hook.is_null() {
        CALLBACK_STATE.store(ptr::null_mut(), Ordering::Release);
        report_startup(Err(PlatformError::new("SetWindowsHookExW failed")));
        // SAFETY: No callback can run because hook creation failed.
        drop(unsafe { Box::from_raw(state_ptr) });
        return;
    }

    let raw_window = raw_input_requested.then(create_raw_input_window).flatten();
    let raw_input = raw_window.is_some_and(register_raw_mouse);
    // SAFETY: state_ptr remains owned by this thread until the message loop exits.
    unsafe {
        (*state_ptr)
            .raw_input_active
            .store(raw_input, Ordering::Release);
    }

    let mut message = Message {
        window: ptr::null_mut(),
        message: 0,
        w_param: 0,
        l_param: 0,
        time: 0,
        point: PointI32 { x: 0, y: 0 },
        private: 0,
    };
    // SAFETY: PeekMessage creates this thread's message queue without removing a message.
    unsafe { PeekMessageW(&raw mut message, ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    // SAFETY: Returns the ID of this live thread.
    report_startup(Ok(MouseCaptureStartup {
        thread_id: unsafe { GetCurrentThreadId() },
        raw_input,
    }));

    let mut raw_movement = RawMovementState::default();

    loop {
        // SAFETY: message is valid writable storage and the thread owns this message loop.
        let result = unsafe { GetMessageW(&raw mut message, ptr::null_mut(), 0, 0) };
        if result <= 0 {
            break;
        }
        if message.message == WM_INPUT
            && unsafe { (*state_ptr).raw_input_active.load(Ordering::Acquire) }
            && process_raw_input_message(
                message.l_param,
                message.time,
                unsafe { &*state_ptr },
                &mut raw_movement,
            )
            .is_err()
        {
            // Keep the safety hook active and fail over permanently for this run.
            unsafe {
                (*state_ptr)
                    .raw_input_active
                    .store(false, Ordering::Release);
            }
            eprintln!("Windows Raw Input read failed; falling back to the low-level mouse hook");
        }
        // SAFETY: message was initialized by GetMessageW.
        unsafe {
            TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }

    CALLBACK_STATE.store(ptr::null_mut(), Ordering::Release);
    if raw_input {
        unregister_raw_mouse();
    }
    if let Some(window) = raw_window {
        // SAFETY: This thread created and still owns the hidden window.
        unsafe { DestroyWindow(window) };
    }
    // SAFETY: The message loop has stopped and this thread owns the hook and state.
    unsafe {
        UnhookWindowsHookEx(hook);
        drop(Box::from_raw(state_ptr));
    }
}

fn create_raw_input_window() -> Option<*mut c_void> {
    const STATIC_CLASS: [u16; 7] = [83, 84, 65, 84, 73, 67, 0];
    // SAFETY: STATIC_CLASS is a static nul-terminated Win32 system class name.
    // HWND_MESSAGE creates a non-visible, message-only target owned by this thread.
    let window = unsafe {
        CreateWindowExW(
            0,
            STATIC_CLASS.as_ptr(),
            ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE as *mut c_void,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    (!window.is_null()).then_some(window)
}

fn register_raw_mouse(window: *mut c_void) -> bool {
    let device = RawInputDevice {
        usage_page: HID_USAGE_PAGE_GENERIC,
        usage: HID_USAGE_GENERIC_MOUSE,
        flags: RIDEV_INPUTSINK,
        target: window,
    };
    // SAFETY: device points to one initialized registration record for this call.
    unsafe {
        RegisterRawInputDevices(
            &raw const device,
            1,
            u32::try_from(size_of::<RawInputDevice>()).unwrap(),
        ) != 0
    }
}

fn unregister_raw_mouse() {
    let device = RawInputDevice {
        usage_page: HID_USAGE_PAGE_GENERIC,
        usage: HID_USAGE_GENERIC_MOUSE,
        flags: RIDEV_REMOVE,
        target: ptr::null_mut(),
    };
    // SAFETY: RIDEV_REMOVE with a null target unregisters this process' mouse TLC.
    unsafe {
        RegisterRawInputDevices(
            &raw const device,
            1,
            u32::try_from(size_of::<RawInputDevice>()).unwrap(),
        );
    }
}

fn process_raw_input_message(
    raw_handle: isize,
    message_time: u32,
    state: &CallbackState,
    movement_state: &mut RawMovementState,
) -> Result<(), PlatformError> {
    if raw_handle == 0 {
        return Err(PlatformError::new(
            "WM_INPUT did not contain an input handle",
        ));
    }
    let mut raw = MaybeUninit::<RawMouseInput>::uninit();
    let mut bytes = u32::try_from(size_of::<RawMouseInput>()).unwrap();
    // SAFETY: raw is aligned writable storage for a mouse RAWINPUT record and
    // bytes/header_size describe its exact capacity and header layout.
    let copied = unsafe {
        GetRawInputData(
            raw_handle as *mut c_void,
            RID_INPUT,
            raw.as_mut_ptr().cast(),
            &raw mut bytes,
            u32::try_from(size_of::<RawInputHeader>()).unwrap(),
        )
    };
    if copied == u32::MAX || usize::try_from(copied).ok() != Some(size_of::<RawMouseInput>()) {
        return Err(PlatformError::new(format!(
            "GetRawInputData returned {copied} bytes for a {}-byte mouse record",
            size_of::<RawMouseInput>()
        )));
    }
    // SAFETY: GetRawInputData reported that it initialized the complete record.
    let raw = unsafe { raw.assume_init() };
    if raw.header.input_type != RIM_TYPEMOUSE {
        return Ok(());
    }

    let capture = state.suppress.load(Ordering::Acquire)
        && !state.transitioning.load(Ordering::Acquire)
        && win32_time_is_after(message_time, state.raw_cutoff_time.load(Ordering::Acquire));
    let events = raw_mouse_events(&raw, movement_state, capture);
    for event in events {
        if !queue_mouse_event(state, event) {
            break;
        }
    }
    Ok(())
}

fn raw_mouse_events(
    raw: &RawMouseInput,
    movement_state: &mut RawMovementState,
    capture: bool,
) -> Vec<PhysicalMouseEvent> {
    let mut events = Vec::new();
    let movement = raw_mouse_movement(raw, movement_state);
    if capture {
        match movement {
            Some(movement) if movement.dx != 0.0 || movement.dy != 0.0 => {
                events.push(PhysicalMouseEvent::Move { movement });
            }
            Some(_) | None => {}
        }
    }
    if !capture {
        return events;
    }

    let flags = low_word(raw.mouse.buttons);
    let mappings = [
        (
            RI_MOUSE_LEFT_BUTTON_DOWN,
            MouseButton::Primary,
            ButtonState::Pressed,
        ),
        (
            RI_MOUSE_LEFT_BUTTON_UP,
            MouseButton::Primary,
            ButtonState::Released,
        ),
        (
            RI_MOUSE_RIGHT_BUTTON_DOWN,
            MouseButton::Secondary,
            ButtonState::Pressed,
        ),
        (
            RI_MOUSE_RIGHT_BUTTON_UP,
            MouseButton::Secondary,
            ButtonState::Released,
        ),
        (
            RI_MOUSE_MIDDLE_BUTTON_DOWN,
            MouseButton::Middle,
            ButtonState::Pressed,
        ),
        (
            RI_MOUSE_MIDDLE_BUTTON_UP,
            MouseButton::Middle,
            ButtonState::Released,
        ),
        (
            RI_MOUSE_BUTTON_4_DOWN,
            MouseButton::Back,
            ButtonState::Pressed,
        ),
        (
            RI_MOUSE_BUTTON_4_UP,
            MouseButton::Back,
            ButtonState::Released,
        ),
        (
            RI_MOUSE_BUTTON_5_DOWN,
            MouseButton::Forward,
            ButtonState::Pressed,
        ),
        (
            RI_MOUSE_BUTTON_5_UP,
            MouseButton::Forward,
            ButtonState::Released,
        ),
    ];
    for (flag, button, state) in mappings {
        if flags & flag != 0 {
            events.push(button_event(button, state));
        }
    }
    let wheel = f64::from(i16::from_ne_bytes(
        high_word(raw.mouse.buttons).to_ne_bytes(),
    ));
    if flags & RI_MOUSE_WHEEL != 0 {
        events.push(PhysicalMouseEvent::Wheel {
            horizontal: 0.0,
            vertical: wheel,
        });
    }
    if flags & RI_MOUSE_HORIZONTAL_WHEEL != 0 {
        events.push(PhysicalMouseEvent::Wheel {
            horizontal: wheel,
            vertical: 0.0,
        });
    }
    events
}

fn raw_mouse_movement(
    raw: &RawMouseInput,
    movement_state: &mut RawMovementState,
) -> Option<Vector> {
    if raw.mouse.flags & MOUSE_MOVE_ABSOLUTE == 0 {
        return Some(Vector::new(
            f64::from(raw.mouse.last_x),
            f64::from(raw.mouse.last_y),
        ));
    }

    let virtual_desktop = raw.mouse.flags & MOUSE_VIRTUAL_DESKTOP != 0;
    // SAFETY: GetSystemMetrics has no pointer parameters.
    let (left, top, width, height) = unsafe {
        if virtual_desktop {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        } else {
            (
                0,
                0,
                GetSystemMetrics(SM_CXSCREEN),
                GetSystemMetrics(SM_CYSCREEN),
            )
        }
    };
    if width <= 0 || height <= 0 {
        return None;
    }
    let position = Point::new(
        f64::from(left) + f64::from(raw.mouse.last_x) * f64::from(width) / 65_535.0,
        f64::from(top) + f64::from(raw.mouse.last_y) * f64::from(height) / 65_535.0,
    );
    let previous = movement_state
        .absolute_positions
        .insert(raw.header.device as usize, position)?;
    Some(Vector::new(
        position.x - previous.x,
        position.y - previous.y,
    ))
}

fn win32_time_is_after(candidate: u32, boundary: u32) -> bool {
    (candidate.wrapping_sub(boundary) as i32) > 0
}

fn queue_mouse_event(state: &CallbackState, event: PhysicalMouseEvent) -> bool {
    match state.sender.try_send(event) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(_)) => {
            state.overflowed.store(true, Ordering::Release);
            state.suppress.store(false, Ordering::Release);
            false
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {
            state.suppress.store(false, Ordering::Release);
            false
        }
    }
}

unsafe extern "system" fn mouse_hook_callback(code: i32, w_param: usize, l_param: isize) -> isize {
    if code != HC_ACTION || l_param == 0 {
        // SAFETY: Forwarding unknown/non-action callbacks is required by the hook contract.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    }
    let state_ptr = CALLBACK_STATE.load(Ordering::Acquire);
    if state_ptr.is_null() {
        // SAFETY: No local state exists, so forward the callback unchanged.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    }
    // SAFETY: The hook thread keeps both pointers alive throughout callback execution.
    let state = unsafe { &*state_ptr };
    let data = unsafe { &*(l_param as *const MouseHookData) };
    if data.extra_info == EVENT_MARKER || data.flags & LLMHF_INJECTED != 0 {
        if w_param as u32 == WM_MOUSEMOVE {
            update_hook_reference(
                &state.last_point,
                logical_hook_point(data.point, state.coordinate_scale),
            );
        }
        // SAFETY: Synthetic input must remain visible to other hooks and applications.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    }

    if w_param as u32 == WM_MOUSEMOVE && state.transitioning.load(Ordering::Acquire) {
        // SetCursorPos can surface as a low-level move. Do not treat a mode-transition
        // warp (or a concurrent physical move) as relative input for the new owner.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    }

    let suppress = state.suppress.load(Ordering::Acquire);
    if suppress
        && state.raw_input_active.load(Ordering::Acquire)
        && is_captured_mouse_message(w_param as u32)
    {
        // Raw Input owns physical event ordering while remote. The low-level
        // hook remains responsible for preventing the local OS from acting on it.
        return 1;
    }
    let event = hook_event(
        w_param as u32,
        data,
        &state.last_point,
        suppress,
        state.coordinate_scale,
    );
    if let Some(event) = event {
        if !queue_mouse_event(state, event) {
            // SAFETY: On queue failure, fail open so user input cannot be trapped.
            return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
        }
        if suppress {
            return 1;
        }
    }
    // SAFETY: Forward any event that EdgeMouse is not actively suppressing.
    unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) }
}

fn is_captured_mouse_message(message: u32) -> bool {
    matches!(
        message,
        WM_MOUSEMOVE
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_RBUTTONDOWN
            | WM_RBUTTONUP
            | WM_MBUTTONDOWN
            | WM_MBUTTONUP
            | WM_MOUSEWHEEL
            | WM_XBUTTONDOWN
            | WM_XBUTTONUP
            | WM_MOUSEHWHEEL
    )
}

fn hook_event(
    message: u32,
    data: &MouseHookData,
    last_point: &Mutex<Option<Point>>,
    remote: bool,
    coordinate_scale: f64,
) -> Option<PhysicalMouseEvent> {
    match message {
        WM_MOUSEMOVE => {
            let point = logical_hook_point(data.point, coordinate_scale);
            let mut last = last_point.lock().ok()?;
            let movement = last.map_or(Vector::new(0.0, 0.0), |previous| {
                Vector::new(point.x - previous.x, point.y - previous.y)
            });
            if !remote {
                *last = Some(point);
            }
            Some(PhysicalMouseEvent::Move { movement })
        }
        WM_LBUTTONDOWN => Some(button_event(MouseButton::Primary, ButtonState::Pressed)),
        WM_LBUTTONUP => Some(button_event(MouseButton::Primary, ButtonState::Released)),
        WM_RBUTTONDOWN => Some(button_event(MouseButton::Secondary, ButtonState::Pressed)),
        WM_RBUTTONUP => Some(button_event(MouseButton::Secondary, ButtonState::Released)),
        WM_MBUTTONDOWN => Some(button_event(MouseButton::Middle, ButtonState::Pressed)),
        WM_MBUTTONUP => Some(button_event(MouseButton::Middle, ButtonState::Released)),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let button = match high_word(data.mouse_data) {
                XBUTTON1 => MouseButton::Back,
                XBUTTON2 => MouseButton::Forward,
                other => MouseButton::Other(other.to_be_bytes()[1]),
            };
            let state = if message == WM_XBUTTONDOWN {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            };
            Some(button_event(button, state))
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = f64::from(i16::from_ne_bytes(high_word(data.mouse_data).to_ne_bytes()));
            if message == WM_MOUSEWHEEL {
                Some(PhysicalMouseEvent::Wheel {
                    horizontal: 0.0,
                    vertical: delta,
                })
            } else {
                Some(PhysicalMouseEvent::Wheel {
                    horizontal: delta,
                    vertical: 0.0,
                })
            }
        }
        _ => None,
    }
}

fn run_keyboard_hook_thread(
    sender: mpsc::SyncSender<KeyboardEvent>,
    routing: Arc<Mutex<KeyboardRoutingState>>,
    overflowed: Arc<AtomicBool>,
    emergency_release: Arc<AtomicBool>,
    report_startup: impl FnOnce(Result<u32, PlatformError>),
) {
    let state = Box::new(KeyboardCallbackState {
        sender,
        routing,
        overflowed,
        emergency_release,
    });
    let state_ptr = Box::into_raw(state);
    if KEYBOARD_CALLBACK_STATE
        .compare_exchange(
            ptr::null_mut(),
            state_ptr,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        report_startup(Err(PlatformError::new(
            "another Windows keyboard hook is already active",
        )));
        // SAFETY: The pointer was never published by this thread.
        drop(unsafe { Box::from_raw(state_ptr) });
        return;
    }
    // SAFETY: The callback has static lifetime and low-level hooks accept a null module handle.
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_hook_callback),
            ptr::null_mut(),
            0,
        )
    };
    if hook.is_null() {
        KEYBOARD_CALLBACK_STATE.store(ptr::null_mut(), Ordering::Release);
        report_startup(Err(PlatformError::new(
            "SetWindowsHookExW failed for keyboard capture",
        )));
        // SAFETY: No callback can run because hook creation failed.
        drop(unsafe { Box::from_raw(state_ptr) });
        return;
    }
    let mut message = Message {
        window: ptr::null_mut(),
        message: 0,
        w_param: 0,
        l_param: 0,
        time: 0,
        point: PointI32 { x: 0, y: 0 },
        private: 0,
    };
    // SAFETY: PeekMessage creates this thread's message queue without removing a message.
    unsafe { PeekMessageW(&raw mut message, ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    // SAFETY: Returns the ID of this live thread.
    report_startup(Ok(unsafe { GetCurrentThreadId() }));
    loop {
        // SAFETY: message is writable storage owned by this message loop.
        let result = unsafe { GetMessageW(&raw mut message, ptr::null_mut(), 0, 0) };
        if result <= 0 {
            break;
        }
        // SAFETY: message was initialized by GetMessageW.
        unsafe {
            TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
    }
    KEYBOARD_CALLBACK_STATE.store(ptr::null_mut(), Ordering::Release);
    // SAFETY: The message loop has stopped and this thread owns hook and state.
    unsafe {
        UnhookWindowsHookEx(hook);
        drop(Box::from_raw(state_ptr));
    }
}

unsafe extern "system" fn keyboard_hook_callback(
    code: i32,
    w_param: usize,
    l_param: isize,
) -> isize {
    if code != HC_ACTION || l_param == 0 {
        // SAFETY: Forwarding unknown/non-action callbacks is required by the hook contract.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    }
    let state_ptr = KEYBOARD_CALLBACK_STATE.load(Ordering::Acquire);
    if state_ptr.is_null() {
        // SAFETY: No local state exists, so forward the callback unchanged.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    }
    // SAFETY: The hook thread keeps both pointers alive throughout callback execution.
    let state = unsafe { &*state_ptr };
    let data = unsafe { &*(l_param as *const KeyboardHookData) };
    if data.extra_info == EVENT_MARKER || data.flags & LLKHF_INJECTED != 0 {
        // SAFETY: Synthetic input must remain visible to other hooks and applications.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    }
    let key_state = match w_param as u32 {
        WM_KEYDOWN | WM_SYSKEYDOWN => KeyState::Pressed,
        WM_KEYUP | WM_SYSKEYUP => KeyState::Released,
        _ => {
            return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
        }
    };
    let Some(key) = windows_key_code(data) else {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    };
    let Ok(mut routing) = state.routing.try_lock() else {
        // The callback always fails open instead of blocking the system hook thread.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    };

    let (send, suppress, repeat) = route_keyboard_event(&mut routing, key, key_state);
    let emergency = suppress
        && key == KeyCode::ESCAPE
        && key_state == KeyState::Pressed
        && has_emergency_modifiers(&routing.captured_pressed);
    if emergency {
        routing.remote = false;
        KEYBOARD_REMOTE_ACTIVE.store(false, Ordering::Release);
        state.emergency_release.store(true, Ordering::Release);
    }
    drop(routing);

    if send && !emergency {
        let event = KeyboardEvent {
            key: remote_key_code(key),
            state: key_state,
            repeat,
        };
        match state.sender.try_send(event) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                state.overflowed.store(true, Ordering::Release);
                KEYBOARD_REMOTE_ACTIVE.store(false, Ordering::Release);
                if let Ok(mut routing) = state.routing.try_lock() {
                    routing.remote = false;
                }
                return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                KEYBOARD_REMOTE_ACTIVE.store(false, Ordering::Release);
                if let Ok(mut routing) = state.routing.try_lock() {
                    routing.remote = false;
                }
                return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
            }
        }
    }
    if suppress {
        1
    } else {
        unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) }
    }
}

fn route_keyboard_event(
    routing: &mut KeyboardRoutingState,
    key: KeyCode,
    state: KeyState,
) -> (bool, bool, bool) {
    if routing.passthrough_pressed.contains(&key) {
        if state == KeyState::Released {
            routing.passthrough_pressed.remove(&key);
        }
        return (false, false, false);
    }
    if routing.remote {
        return match state {
            KeyState::Pressed => {
                let repeat = !routing.captured_pressed.insert(key);
                (true, true, repeat)
            }
            KeyState::Released if routing.captured_pressed.remove(&key) => (true, true, false),
            KeyState::Released => (false, false, false),
        };
    }
    if routing.captured_pressed.contains(&key) {
        if state == KeyState::Released {
            routing.captured_pressed.remove(&key);
        }
        return (false, true, false);
    }
    match state {
        KeyState::Pressed => {
            routing.local_pressed.insert(key);
        }
        KeyState::Released => {
            routing.local_pressed.remove(&key);
        }
    }
    (false, false, false)
}

/// Preserve Windows shortcut muscle memory on macOS by swapping the physical
/// Control and Windows-key usages on the wire. Ctrl+C/V/A therefore becomes
/// Command+C/V/A, while the Windows key remains available as macOS Control.
fn remote_key_code(key: KeyCode) -> KeyCode {
    match key {
        KeyCode::LEFT_CONTROL => KeyCode::LEFT_META,
        KeyCode::RIGHT_CONTROL => KeyCode::RIGHT_META,
        KeyCode::LEFT_META => KeyCode::LEFT_CONTROL,
        KeyCode::RIGHT_META => KeyCode::RIGHT_CONTROL,
        _ => key,
    }
}

fn has_emergency_modifiers(pressed: &BTreeSet<KeyCode>) -> bool {
    (pressed.contains(&KeyCode::LEFT_CONTROL) || pressed.contains(&KeyCode::RIGHT_CONTROL))
        && (pressed.contains(&KeyCode::LEFT_ALT) || pressed.contains(&KeyCode::RIGHT_ALT))
        && (pressed.contains(&KeyCode::LEFT_SHIFT) || pressed.contains(&KeyCode::RIGHT_SHIFT))
}

fn windows_key_code(data: &KeyboardHookData) -> Option<KeyCode> {
    let vk = data.virtual_key;
    let extended = data.flags & LLKHF_EXTENDED != 0;
    let usage = match vk {
        0x41..=0x5a => 0x04 + u16::try_from(vk - 0x41).ok()?,
        0x31..=0x39 => 0x1e + u16::try_from(vk - 0x31).ok()?,
        0x30 => 0x27,
        0x0d if extended => 0x58,
        0x0d => 0x28,
        0x1b => 0x29,
        0x08 => 0x2a,
        0x09 => 0x2b,
        0x20 => 0x2c,
        0xbd => 0x2d,
        0xbb => 0x2e,
        0xdb => 0x2f,
        0xdd => 0x30,
        0xdc => 0x31,
        0xba => 0x33,
        0xde => 0x34,
        0xc0 => 0x35,
        0xbc => 0x36,
        0xbe => 0x37,
        0xbf => 0x38,
        0x14 => 0x39,
        0x70..=0x7b => 0x3a + u16::try_from(vk - 0x70).ok()?,
        0x2c => 0x46,
        0x91 => 0x47,
        0x13 => 0x48,
        0x2d => 0x49,
        0x24 => 0x4a,
        0x21 => 0x4b,
        0x2e => 0x4c,
        0x23 => 0x4d,
        0x22 => 0x4e,
        0x27 => 0x4f,
        0x25 => 0x50,
        0x28 => 0x51,
        0x26 => 0x52,
        0x90 => 0x53,
        0x6f => 0x54,
        0x6a => 0x55,
        0x6d => 0x56,
        0x6b => 0x57,
        0x61..=0x69 => 0x59 + u16::try_from(vk - 0x61).ok()?,
        0x60 => 0x62,
        0x6e => 0x63,
        0x5d => 0x65,
        0x7c..=0x83 => 0x68 + u16::try_from(vk - 0x7c).ok()?,
        0xa0 => 0xe1,
        0xa1 => 0xe5,
        0xa2 => 0xe0,
        0xa3 => 0xe4,
        0xa4 => 0xe2,
        0xa5 => 0xe6,
        0x5b => 0xe3,
        0x5c => 0xe7,
        0x10 if data.scan_code == 0x36 => 0xe5,
        0x10 => 0xe1,
        0x11 if extended => 0xe4,
        0x11 => 0xe0,
        0x12 if extended => 0xe6,
        0x12 => 0xe2,
        _ => return None,
    };
    KeyCode::from_usage(usage)
}

fn windows_virtual_key(key: KeyCode) -> Option<(u16, bool)> {
    Some(match key.usage() {
        0x04..=0x1d => (0x41 + key.usage() - 0x04, false),
        0x1e..=0x26 => (0x31 + key.usage() - 0x1e, false),
        0x27 => (0x30, false),
        0x28 => (0x0d, false),
        0x29 => (0x1b, false),
        0x2a => (0x08, false),
        0x2b => (0x09, false),
        0x2c => (0x20, false),
        0x2d => (0xbd, false),
        0x2e => (0xbb, false),
        0x2f => (0xdb, false),
        0x30 => (0xdd, false),
        0x31 => (0xdc, false),
        0x33 => (0xba, false),
        0x34 => (0xde, false),
        0x35 => (0xc0, false),
        0x36 => (0xbc, false),
        0x37 => (0xbe, false),
        0x38 => (0xbf, false),
        0x39 => (0x14, false),
        0x3a..=0x45 => (0x70 + key.usage() - 0x3a, false),
        0x46 => (0x2c, true),
        0x47 => (0x91, false),
        0x48 => (0x13, false),
        0x49 => (0x2d, true),
        0x4a => (0x24, true),
        0x4b => (0x21, true),
        0x4c => (0x2e, true),
        0x4d => (0x23, true),
        0x4e => (0x22, true),
        0x4f => (0x27, true),
        0x50 => (0x25, true),
        0x51 => (0x28, true),
        0x52 => (0x26, true),
        0x53 => (0x90, true),
        0x54 => (0x6f, true),
        0x55 => (0x6a, false),
        0x56 => (0x6d, false),
        0x57 => (0x6b, false),
        0x58 => (0x0d, true),
        0x59..=0x61 => (0x61 + key.usage() - 0x59, false),
        0x62 => (0x60, false),
        0x63 => (0x6e, false),
        0x65 => (0x5d, true),
        0x68..=0x6f => (0x7c + key.usage() - 0x68, false),
        0xe0 => (0xa2, false),
        0xe1 => (0xa0, false),
        0xe2 => (0xa4, false),
        0xe3 => (0x5b, true),
        0xe4 => (0xa3, true),
        0xe5 => (0xa1, false),
        0xe6 => (0xa5, true),
        0xe7 => (0x5c, true),
        _ => return None,
    })
}

fn logical_hook_point(point: PointI32, coordinate_scale: f64) -> Point {
    Point::new(
        f64::from(point.x) / coordinate_scale,
        f64::from(point.y) / coordinate_scale,
    )
}

fn update_hook_reference(reference: &Mutex<Option<Point>>, point: Point) {
    if let Ok(mut reference) = reference.lock() {
        *reference = Some(point);
    }
}

fn button_event(button: MouseButton, state: ButtonState) -> PhysicalMouseEvent {
    PhysicalMouseEvent::Button { button, state }
}

fn button_flags(button: MouseButton, state: ButtonState) -> Result<(u32, u32), PlatformError> {
    let result = match (button, state) {
        (MouseButton::Primary, ButtonState::Pressed) => (MOUSEEVENTF_LEFTDOWN, 0),
        (MouseButton::Primary, ButtonState::Released) => (MOUSEEVENTF_LEFTUP, 0),
        (MouseButton::Secondary, ButtonState::Pressed) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (MouseButton::Secondary, ButtonState::Released) => (MOUSEEVENTF_RIGHTUP, 0),
        (MouseButton::Middle, ButtonState::Pressed) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (MouseButton::Middle, ButtonState::Released) => (MOUSEEVENTF_MIDDLEUP, 0),
        (MouseButton::Back, ButtonState::Pressed) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON1)),
        (MouseButton::Back, ButtonState::Released) => (MOUSEEVENTF_XUP, u32::from(XBUTTON1)),
        (MouseButton::Forward, ButtonState::Pressed) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON2)),
        (MouseButton::Forward, ButtonState::Released) => (MOUSEEVENTF_XUP, u32::from(XBUTTON2)),
        (MouseButton::Other(_), _) => {
            return Err(PlatformError::new(
                "Windows SendInput supports five mapped mouse buttons in the MVP",
            ));
        }
    };
    Ok(result)
}

fn normalized_absolute(value: f64, origin: i32, extent: i32) -> i32 {
    (((value - f64::from(origin)) * 65_535.0) / f64::from(extent - 1))
        .round()
        .clamp(0.0, 65_535.0) as i32
}

fn signed_mouse_data(delta: f64) -> u32 {
    let value = (delta / WHEEL_DELTA * WHEEL_DELTA)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    u32::from_ne_bytes(value.to_ne_bytes())
}

fn high_word(value: u32) -> u16 {
    value.to_ne_bytes()[2..4]
        .try_into()
        .map(u16::from_ne_bytes)
        .unwrap()
}

fn low_word(value: u32) -> u16 {
    value.to_ne_bytes()[0..2]
        .try_into()
        .map(u16::from_ne_bytes)
        .unwrap()
}

fn rounded_i32(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_mouse_input(
        flags: u16,
        button_flags: u16,
        button_data: i16,
        x: i32,
        y: i32,
    ) -> RawMouseInput {
        RawMouseInput {
            header: RawInputHeader {
                input_type: RIM_TYPEMOUSE,
                size: u32::try_from(size_of::<RawMouseInput>()).unwrap(),
                device: ptr::dangling_mut::<c_void>(),
                w_param: 0,
            },
            mouse: RawMouse {
                flags,
                alignment: 0,
                buttons: u32::from(button_flags)
                    | (u32::from(u16::from_ne_bytes(button_data.to_ne_bytes())) << 16),
                raw_buttons: 0,
                last_x: x,
                last_y: y,
                extra_information: 0,
            },
        }
    }

    #[test]
    fn maps_virtual_desktop_coordinates() {
        assert_eq!(normalized_absolute(-1920.0, -1920, 3840), 0);
        assert_eq!(normalized_absolute(1919.0, -1920, 3840), 65_535);
    }

    #[test]
    fn maps_supported_buttons() {
        assert_eq!(
            button_flags(MouseButton::Back, ButtonState::Pressed).unwrap(),
            (MOUSEEVENTF_XDOWN, u32::from(XBUTTON1))
        );
        assert!(button_flags(MouseButton::Other(8), ButtonState::Pressed).is_err());
    }

    #[test]
    fn normalizes_per_monitor_hook_coordinates_to_logical_points() {
        assert_eq!(
            logical_hook_point(PointI32 { x: 3_838, y: 2_158 }, 2.0),
            Point::new(1_919.0, 1_079.0)
        );
    }

    #[test]
    fn preserves_fractional_logical_hook_coordinates() {
        assert_eq!(
            logical_hook_point(PointI32 { x: 101, y: 203 }, 2.0),
            Point::new(50.5, 101.5)
        );
    }

    #[test]
    fn first_hooked_move_uses_the_seeded_pointer_reference() {
        let reference = Mutex::new(Some(Point::new(1_919.0, 500.0)));
        let data = MouseHookData {
            point: PointI32 { x: 1_920, y: 500 },
            mouse_data: 0,
            flags: 0,
            time: 0,
            extra_info: 0,
        };

        assert_eq!(
            hook_event(WM_MOUSEMOVE, &data, &reference, false, 1.0),
            Some(PhysicalMouseEvent::Move {
                movement: Vector::new(1.0, 0.0),
            })
        );
    }

    #[test]
    fn raw_input_layout_matches_the_win32_mouse_abi() {
        assert_eq!(size_of::<RawMouse>(), 24);
        if size_of::<usize>() == 8 {
            assert_eq!(size_of::<RawInputHeader>(), 24);
            assert_eq!(size_of::<RawMouseInput>(), 48);
        } else {
            assert_eq!(size_of::<RawInputHeader>(), 16);
            assert_eq!(size_of::<RawMouseInput>(), 40);
        }
    }

    #[test]
    fn raw_mouse_keeps_movement_button_and_wheel_order() {
        let raw = raw_mouse_input(0, RI_MOUSE_LEFT_BUTTON_DOWN | RI_MOUSE_WHEEL, 120, -7, 11);
        let mut movement = RawMovementState::default();
        assert_eq!(
            raw_mouse_events(&raw, &mut movement, true),
            vec![
                PhysicalMouseEvent::Move {
                    movement: Vector::new(-7.0, 11.0),
                },
                button_event(MouseButton::Primary, ButtonState::Pressed),
                PhysicalMouseEvent::Wheel {
                    horizontal: 0.0,
                    vertical: 120.0,
                },
            ]
        );
    }

    #[test]
    fn raw_mouse_is_drained_but_not_emitted_while_local() {
        let raw = raw_mouse_input(0, RI_MOUSE_RIGHT_BUTTON_DOWN, 0, 4, 5);
        assert!(raw_mouse_events(&raw, &mut RawMovementState::default(), false).is_empty());
    }

    #[test]
    fn transition_timestamp_rejects_old_raw_input_across_wraparound() {
        assert!(!win32_time_is_after(100, 100));
        assert!(!win32_time_is_after(99, 100));
        assert!(win32_time_is_after(101, 100));
        assert!(win32_time_is_after(2, u32::MAX - 2));
    }

    #[test]
    fn safety_hook_recognizes_every_raw_mouse_message() {
        for message in [
            WM_MOUSEMOVE,
            WM_LBUTTONDOWN,
            WM_LBUTTONUP,
            WM_RBUTTONDOWN,
            WM_RBUTTONUP,
            WM_MBUTTONDOWN,
            WM_MBUTTONUP,
            WM_XBUTTONDOWN,
            WM_XBUTTONUP,
            WM_MOUSEWHEEL,
            WM_MOUSEHWHEEL,
        ] {
            assert!(is_captured_mouse_message(message));
        }
        assert!(!is_captured_mouse_message(WM_KEYDOWN));
    }

    #[test]
    fn synthetic_remote_moves_refresh_the_physical_takeover_reference() {
        let reference = Mutex::new(Some(Point::new(10.0, 20.0)));
        update_hook_reference(&reference, Point::new(300.0, 400.0));

        assert_eq!(*reference.lock().unwrap(), Some(Point::new(300.0, 400.0)));
    }

    #[test]
    fn maps_common_virtual_keys_to_hid_usages() {
        let data = |virtual_key, scan_code, flags| KeyboardHookData {
            virtual_key,
            scan_code,
            flags,
            time: 0,
            extra_info: 0,
        };
        assert_eq!(windows_key_code(&data(0x41, 0, 0)), Some(KeyCode::A));
        assert_eq!(
            windows_key_code(&data(0x11, 0, LLKHF_EXTENDED)),
            Some(KeyCode::RIGHT_CONTROL)
        );
        assert_eq!(
            windows_key_code(&data(0x0d, 0, LLKHF_EXTENDED)),
            Some(KeyCode::NUMPAD_ENTER)
        );
    }

    #[test]
    fn swaps_control_and_meta_for_mac_shortcut_semantics() {
        assert_eq!(remote_key_code(KeyCode::LEFT_CONTROL), KeyCode::LEFT_META);
        assert_eq!(remote_key_code(KeyCode::RIGHT_CONTROL), KeyCode::RIGHT_META);
        assert_eq!(remote_key_code(KeyCode::LEFT_META), KeyCode::LEFT_CONTROL);
        assert_eq!(remote_key_code(KeyCode::RIGHT_META), KeyCode::RIGHT_CONTROL);
        assert_eq!(remote_key_code(KeyCode::LEFT_ALT), KeyCode::LEFT_ALT);
        assert_eq!(remote_key_code(KeyCode::A), KeyCode::A);
    }

    #[test]
    fn only_remote_ctrl_c_is_ignored_as_a_console_shutdown_signal() {
        assert_eq!(
            console_control_action(CTRL_C_EVENT, true),
            ConsoleControlAction::IgnoreForwardedShortcut
        );
        assert_eq!(
            console_control_action(CTRL_C_EVENT, false),
            ConsoleControlAction::Stop
        );
        assert_eq!(
            console_control_action(CTRL_CLOSE_EVENT, true),
            ConsoleControlAction::Stop
        );
        assert_eq!(
            console_control_action(u32::MAX, true),
            ConsoleControlAction::PassThrough
        );
    }

    #[test]
    fn keys_held_before_handoff_are_not_captured() {
        let mut routing = KeyboardRoutingState::default();
        assert_eq!(
            route_keyboard_event(&mut routing, KeyCode::LEFT_CONTROL, KeyState::Pressed),
            (false, false, false)
        );
        let local_pressed = std::mem::take(&mut routing.local_pressed);
        routing.passthrough_pressed.extend(local_pressed);
        routing.remote = true;
        assert_eq!(
            route_keyboard_event(&mut routing, KeyCode::LEFT_CONTROL, KeyState::Released),
            (false, false, false)
        );
        assert!(routing.passthrough_pressed.is_empty());
    }

    #[test]
    fn remote_keys_stay_suppressed_until_physically_released() {
        let mut routing = KeyboardRoutingState {
            remote: true,
            ..KeyboardRoutingState::default()
        };
        assert_eq!(
            route_keyboard_event(&mut routing, KeyCode::A, KeyState::Pressed),
            (true, true, false)
        );
        routing.remote = false;
        assert_eq!(
            route_keyboard_event(&mut routing, KeyCode::A, KeyState::Pressed),
            (false, true, false)
        );
        assert_eq!(
            route_keyboard_event(&mut routing, KeyCode::A, KeyState::Released),
            (false, true, false)
        );
        assert!(routing.captured_pressed.is_empty());
    }
}
