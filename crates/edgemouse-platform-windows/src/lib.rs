//! Native Win32 mouse adapter for Windows.

#![cfg(target_os = "windows")]

use edgemouse_core::{
    ButtonState, CaptureMode, KeyCode, KeyState, KeyboardCaptureBackend, KeyboardEvent,
    KeyboardInjectionBackend, MouseButton, MouseCaptureBackend, MouseInjectionBackend,
    PermissionState, PhysicalMouseEvent, PlatformError, Point, RemoteMouseEvent, Vector,
};
use std::collections::{BTreeSet, VecDeque};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::JoinHandle;

const WH_MOUSE_LL: i32 = 14;
const WH_KEYBOARD_LL: i32 = 13;
const HC_ACTION: i32 = 0;
const WM_QUIT: u32 = 0x0012;
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

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PointI32 {
    x: i32,
    y: i32,
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
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn GetCursorPos(point: *mut PointI32) -> i32;
    fn ShowCursor(show: i32) -> i32;
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "Kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn SetConsoleCtrlHandler(handler: Option<ConsoleCtrlHandler>, add: i32) -> i32;
}

struct CallbackState {
    sender: mpsc::SyncSender<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    transitioning: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    last_point: Arc<Mutex<Option<Point>>>,
    coordinate_scale: f64,
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

/// A low-level mouse hook hosted on a dedicated Win32 message-loop thread.
pub struct WindowsMouseCapture {
    receiver: mpsc::Receiver<PhysicalMouseEvent>,
    deferred_events: VecDeque<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    transitioning: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    last_point: Arc<Mutex<Option<Point>>>,
    capture_anchor: Point,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    cursor_hidden: bool,
}

impl WindowsMouseCapture {
    pub fn start(coordinate_scale: f64, capture_anchor: Point) -> Result<Self, PlatformError> {
        if !coordinate_scale.is_finite() || coordinate_scale <= 0.0 {
            return Err(PlatformError::new(
                "Windows coordinate scale must be finite and positive",
            ));
        }
        if !capture_anchor.is_finite() {
            return Err(PlatformError::new("Windows capture anchor must be finite"));
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
        let last_point = Arc::new(Mutex::new(None));
        let callback_last_point = Arc::clone(&last_point);
        let thread = std::thread::Builder::new()
            .name("edgemouse-win32-hook".to_owned())
            .spawn(move || {
                run_hook_thread(
                    event_sender,
                    callback_suppress,
                    callback_transitioning,
                    callback_overflowed,
                    callback_last_point,
                    coordinate_scale,
                    |result| {
                        drop(startup_sender.send(result));
                    },
                );
            })
            .map_err(|error| PlatformError::new(format!("failed to start mouse hook: {error}")))?;
        let thread_id = startup_receiver
            .recv()
            .map_err(|_| PlatformError::new("mouse hook exited during startup"))??;
        Ok(Self {
            receiver: event_receiver,
            deferred_events: VecDeque::new(),
            suppress,
            transitioning,
            overflowed,
            last_point,
            capture_anchor,
            thread_id,
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
    sender: mpsc::SyncSender<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    transitioning: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    last_point: Arc<Mutex<Option<Point>>>,
    coordinate_scale: f64,
    report_startup: impl FnOnce(Result<u32, PlatformError>),
) {
    let state = Box::new(CallbackState {
        sender,
        suppress,
        transitioning,
        overflowed,
        last_point,
        coordinate_scale,
    });
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
        // SAFETY: message is valid writable storage and the thread owns this message loop.
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

    CALLBACK_STATE.store(ptr::null_mut(), Ordering::Release);
    // SAFETY: The message loop has stopped and this thread owns the hook and state.
    unsafe {
        UnhookWindowsHookEx(hook);
        drop(Box::from_raw(state_ptr));
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
        // SAFETY: Synthetic input must remain visible to other hooks and applications.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    }

    if w_param as u32 == WM_MOUSEMOVE && state.transitioning.load(Ordering::Acquire) {
        // SetCursorPos can surface as a low-level move. Do not treat a mode-transition
        // warp (or a concurrent physical move) as relative input for the new owner.
        return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
    }

    let suppress = state.suppress.load(Ordering::Acquire);
    let event = hook_event(
        w_param as u32,
        data,
        &state.last_point,
        suppress,
        state.coordinate_scale,
    );
    if let Some(event) = event {
        match state.sender.try_send(event) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                state.overflowed.store(true, Ordering::Release);
                state.suppress.store(false, Ordering::Release);
                // SAFETY: On overflow, fail open so user input cannot be trapped.
                return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                // SAFETY: The receiver is gone, so never trap user input.
                return unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) };
            }
        }
        if suppress {
            return 1;
        }
    }
    // SAFETY: Forward any event that EdgeMouse is not actively suppressing.
    unsafe { CallNextHookEx(ptr::null_mut(), code, w_param, l_param) }
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

fn rounded_i32(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

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
