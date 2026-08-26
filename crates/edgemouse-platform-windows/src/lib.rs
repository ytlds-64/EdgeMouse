//! Native Win32 mouse adapter for Windows.

#![cfg(target_os = "windows")]

use edgemouse_core::{
    ButtonState, CaptureMode, MouseButton, MouseCaptureBackend, MouseInjectionBackend,
    PermissionState, PhysicalMouseEvent, PlatformError, Point, RemoteMouseEvent, Vector,
};
use std::collections::BTreeSet;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

const WH_MOUSE_LL: i32 = 14;
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
const PM_NOREMOVE: u32 = 0;

const LLMHF_INJECTED: u32 = 0x0000_0001;
const XBUTTON1: u16 = 1;
const XBUTTON2: u16 = 2;
const WHEEL_DELTA: f64 = 120.0;
const EVENT_MARKER: usize = 0x4544_4745;
const CAPTURE_QUEUE_CAPACITY: usize = 4_096;

const INPUT_MOUSE: u32 = 0;
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
#[derive(Debug, Clone, Copy)]
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
union InputValue {
    mouse: MouseInput,
}

#[repr(C)]
struct Input {
    input_type: u32,
    value: InputValue,
}

type HookProc = unsafe extern "system" fn(code: i32, w_param: usize, l_param: isize) -> isize;

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
}

struct CallbackState {
    sender: mpsc::SyncSender<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    last_point: Arc<Mutex<Option<PointI32>>>,
}

static CALLBACK_STATE: AtomicPtr<CallbackState> = AtomicPtr::new(ptr::null_mut());

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
    suppress: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    last_point: Arc<Mutex<Option<PointI32>>>,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    cursor_hidden: bool,
}

impl WindowsMouseCapture {
    pub fn start() -> Result<Self, PlatformError> {
        if !CALLBACK_STATE.load(Ordering::Acquire).is_null() {
            return Err(PlatformError::new(
                "only one Windows mouse capture instance is supported",
            ));
        }
        let (event_sender, event_receiver) = mpsc::sync_channel(CAPTURE_QUEUE_CAPACITY);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let suppress = Arc::new(AtomicBool::new(false));
        let callback_suppress = Arc::clone(&suppress);
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
                    callback_overflowed,
                    callback_last_point,
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
            suppress,
            overflowed,
            last_point,
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
        let point = PointI32 {
            x: rounded_i32(position.x),
            y: rounded_i32(position.y),
        };
        let mut last = self
            .last_point
            .lock()
            .map_err(|_| PlatformError::new("Windows cursor reference lock was poisoned"))?;
        *last = Some(point);
        Ok(())
    }

    fn show_cursor(&mut self) {
        if self.cursor_hidden {
            // SAFETY: Calls are balanced with hide_cursor for this adapter instance.
            unsafe { ShowCursor(1) };
            self.cursor_hidden = false;
        }
    }
}

impl MouseCaptureBackend for WindowsMouseCapture {
    fn permission_state(&self) -> PermissionState {
        PermissionState::NotRequired
    }

    fn set_mode(&mut self, mode: CaptureMode) -> Result<(), PlatformError> {
        match mode {
            CaptureMode::Local { restore } => {
                self.suppress.store(false, Ordering::Release);
                if let Some(position) = restore {
                    self.set_reference_point(position)?;
                    Self::warp(position)?;
                }
                self.show_cursor();
                Ok(())
            }
            CaptureMode::Remote { anchor } => {
                self.set_reference_point(anchor)?;
                Self::warp(anchor)?;
                self.hide_cursor();
                self.suppress.store(true, Ordering::Release);
                Ok(())
            }
            CaptureMode::ReceivingRemote { position } => {
                self.set_reference_point(position)?;
                Self::warp(position)?;
                self.show_cursor();
                self.suppress.store(true, Ordering::Release);
                Ok(())
            }
        }
    }

    fn try_next_event(&mut self) -> Result<Option<PhysicalMouseEvent>, PlatformError> {
        if self.overflowed.swap(false, Ordering::AcqRel) {
            return Err(PlatformError::new(
                "Windows capture queue overflowed; local input was released",
            ));
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

fn run_hook_thread(
    sender: mpsc::SyncSender<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    last_point: Arc<Mutex<Option<PointI32>>>,
    report_startup: impl FnOnce(Result<u32, PlatformError>),
) {
    let state = Box::new(CallbackState {
        sender,
        suppress,
        overflowed,
        last_point,
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

    let suppress = state.suppress.load(Ordering::Acquire);
    let event = hook_event(w_param as u32, data, &state.last_point, suppress);
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
    last_point: &Mutex<Option<PointI32>>,
    remote: bool,
) -> Option<PhysicalMouseEvent> {
    match message {
        WM_MOUSEMOVE => {
            let mut last = last_point.lock().ok()?;
            let movement = last.map_or(Vector::new(0.0, 0.0), |previous| {
                Vector::new(
                    f64::from(data.point.x) - f64::from(previous.x),
                    f64::from(data.point.y) - f64::from(previous.y),
                )
            });
            if !remote {
                *last = Some(data.point);
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
}
