//! Native `CoreGraphics` mouse adapter for macOS.

#![cfg(target_os = "macos")]

use edgemouse_core::{
    ButtonState, CaptureMode, DisplayGeometry, KeyCode, KeyState, KeyboardCaptureBackend,
    KeyboardEvent, KeyboardInjectionBackend, MouseButton, MouseCaptureBackend,
    MouseInjectionBackend, PermissionState, PhysicalMouseEvent, PlatformError, Point, Rect,
    RemoteMouseEvent, Vector,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const EVENT_TAP_SESSION: u32 = 1;
const EVENT_TAP_HEAD: u32 = 0;
const EVENT_TAP_ACTIVE: u32 = 0;
const EVENT_SOURCE_USER_DATA: u32 = 42;
const EVENT_MARKER: i64 = 0x4544_4745_4d4f_5553;
const SCROLL_UNIT_PIXEL: u32 = 0;
const CAPTURE_QUEUE_CAPACITY: usize = 4_096;
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE: f64 = 5.0;
const MAX_CLICK_STATE: i64 = 3;
const MOTION_FRAME_INTERVAL: Duration = Duration::from_millis(4);
const MOTION_SNAP_DISTANCE: f64 = 0.25;
const MOTION_MAX_FRAME_DISTANCE: f64 = 240.0;
const MOTION_MIN_SAMPLE_INTERVAL: Duration = Duration::from_millis(1);
const MOTION_MAX_SAMPLES: usize = 32;

const EVENT_LEFT_DOWN: u32 = 1;
const EVENT_LEFT_UP: u32 = 2;
const EVENT_RIGHT_DOWN: u32 = 3;
const EVENT_RIGHT_UP: u32 = 4;
const EVENT_MOUSE_MOVED: u32 = 5;
const EVENT_LEFT_DRAGGED: u32 = 6;
const EVENT_RIGHT_DRAGGED: u32 = 7;
const EVENT_SCROLL_WHEEL: u32 = 22;
const EVENT_KEY_DOWN: u32 = 10;
const EVENT_KEY_UP: u32 = 11;
const EVENT_FLAGS_CHANGED: u32 = 12;
const EVENT_OTHER_DOWN: u32 = 25;
const EVENT_OTHER_UP: u32 = 26;
const EVENT_OTHER_DRAGGED: u32 = 27;
const EVENT_TAP_DISABLED_TIMEOUT: u32 = 0xffff_fffe;
const EVENT_TAP_DISABLED_USER_INPUT: u32 = 0xffff_ffff;

const FIELD_MOUSE_CLICK_STATE: u32 = 1;
const FIELD_MOUSE_BUTTON_NUMBER: u32 = 3;
const FIELD_MOUSE_DELTA_X: u32 = 4;
const FIELD_MOUSE_DELTA_Y: u32 = 5;
const FIELD_SCROLL_FIXED_VERTICAL: u32 = 93;
const FIELD_SCROLL_FIXED_HORIZONTAL: u32 = 94;
const FIELD_SCROLL_POINT_VERTICAL: u32 = 96;
const FIELD_SCROLL_POINT_HORIZONTAL: u32 = 97;
const FIELD_KEYBOARD_AUTOREPEAT: u32 = 8;
const FIELD_KEYBOARD_KEYCODE: u32 = 9;

const FLAG_ALPHA_SHIFT: u64 = 1 << 16;
const FLAG_SHIFT: u64 = 1 << 17;
const FLAG_CONTROL: u64 = 1 << 18;
const FLAG_ALTERNATE: u64 = 1 << 19;
const FLAG_COMMAND: u64 = 1 << 20;
const FLAG_NUMERIC_PAD: u64 = 1 << 21;
const FLAG_SECONDARY_FN: u64 = 1 << 23;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

type EventTapCallback = unsafe extern "C" fn(
    proxy: *mut c_void,
    event_type: u32,
    event: *mut c_void,
    user_info: *mut c_void,
) -> *mut c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn _CGSDefaultConnection() -> i32;
    fn CGSSetConnectionProperty(
        connection: i32,
        target: i32,
        key: *const c_void,
        value: *const c_void,
    ) -> i32;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: EventTapCallback,
        user_info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
    fn CGEventGetDoubleValueField(event: *mut c_void, field: u32) -> f64;
    fn CGEventGetFlags(event: *mut c_void) -> u64;
    fn CGEventSourceFlagsState(state_id: i32) -> u64;
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    fn CGEventCreateMouseEvent(
        source: *mut c_void,
        event_type: u32,
        position: CGPoint,
        button: u32,
    ) -> *mut c_void;
    fn CGEventCreateScrollWheelEvent2(
        source: *mut c_void,
        units: u32,
        wheel_count: u32,
        wheel_1: i32,
        wheel_2: i32,
        wheel_3: i32,
    ) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(
        source: *mut c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> *mut c_void;
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CGMainDisplayID() -> u32;
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: u32) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: *const c_void;
    static kCFRunLoopCommonModes: *const c_void;
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        string: *const i8,
        encoding: u32,
    ) -> *mut c_void;
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *mut c_void,
        order: isize,
    ) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFRunLoopStop(run_loop: *mut c_void);
    fn CFRunLoopWakeUp(run_loop: *mut c_void);
    fn CFRelease(value: *const c_void);
}

#[must_use]
pub fn accessibility_trusted() -> bool {
    // SAFETY: This function takes no pointers and returns a CoreServices Boolean.
    unsafe { AXIsProcessTrusted() != 0 }
}

pub fn current_pointer() -> Result<Point, PlatformError> {
    // SAFETY: A null source requests a snapshot from the combined event state.
    let event = unsafe { CGEventCreate(ptr::null_mut()) };
    if event.is_null() {
        return Err(PlatformError::new(
            "CGEventCreate failed while reading cursor position",
        ));
    }
    // SAFETY: `event` is a live CGEventRef and remains valid for the call.
    let location = unsafe { CGEventGetLocation(event) };
    // SAFETY: This function owns the create-rule reference.
    unsafe { CFRelease(event) };
    let point = Point::new(location.x, location.y);
    if point.is_finite() {
        Ok(point)
    } else {
        Err(PlatformError::new(
            "CoreGraphics returned a non-finite cursor position",
        ))
    }
}

/// Returns the union of all active displays in CoreGraphics global point coordinates.
/// Rotation and scaled display modes are already reflected in each display's bounds.
pub fn desktop_geometry() -> Result<(Rect, f64, Vec<DisplayGeometry>), PlatformError> {
    const MAX_DISPLAYS: usize = 32;
    let mut displays = [0_u32; MAX_DISPLAYS];
    let mut count = 0_u32;
    // SAFETY: `displays` and `count` are writable for the documented number of entries.
    let result = unsafe {
        CGGetActiveDisplayList(
            u32::try_from(MAX_DISPLAYS).unwrap(),
            displays.as_mut_ptr(),
            &raw mut count,
        )
    };
    if result != 0 {
        return Err(PlatformError::new(format!(
            "CGGetActiveDisplayList failed with code {result}"
        )));
    }
    if count == 0 {
        // Background launch contexts can briefly expose no active list while the
        // WindowServer is settling. The main display remains the safe fallback.
        displays[0] = unsafe { CGMainDisplayID() };
        count = 1;
    }

    let main_display = unsafe { CGMainDisplayID() };
    let mut left = f64::INFINITY;
    let mut top = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut bottom = f64::NEG_INFINITY;
    let mut geometry = Vec::with_capacity(usize::try_from(count).unwrap_or(MAX_DISPLAYS));
    for display in displays
        .iter()
        .take(usize::try_from(count).unwrap_or(MAX_DISPLAYS))
    {
        // SAFETY: display identifiers were returned by CoreGraphics in this call.
        let bounds = unsafe { CGDisplayBounds(*display) };
        left = left.min(bounds.origin.x);
        top = top.min(bounds.origin.y);
        right = right.max(bounds.origin.x + bounds.size.width);
        bottom = bottom.max(bounds.origin.y + bounds.size.height);
        let display_bounds = Rect::new(
            Point::new(bounds.origin.x, bounds.origin.y),
            bounds.size.width,
            bounds.size.height,
        )
        .map_err(|error| PlatformError::new(format!("invalid macOS display bounds: {error}")))?;
        let pixel_width = u32::try_from(unsafe { CGDisplayPixelsWide(*display) })
            .map_err(|_| PlatformError::new("macOS display width exceeds supported range"))?;
        let pixel_height = u32::try_from(unsafe { CGDisplayPixelsHigh(*display) })
            .map_err(|_| PlatformError::new("macOS display height exceeds supported range"))?;
        if pixel_width == 0 || pixel_height == 0 {
            return Err(PlatformError::new("macOS reported an empty display mode"));
        }
        let display_scale = (f64::from(pixel_width) / bounds.size.width).max(1.0);
        geometry.push(DisplayGeometry {
            bounds: display_bounds,
            pixel_width,
            pixel_height,
            scale_factor: display_scale,
            primary: *display == main_display,
        });
    }
    let bounds = Rect::new(Point::new(left, top), right - left, bottom - top)
        .map_err(|error| PlatformError::new(format!("invalid macOS desktop bounds: {error}")))?;

    // CoreGraphics input coordinates are points. The backing scale is advertised
    // for diagnostics and future UI rendering, not for pointer coordinate conversion.
    // SAFETY: the main display identifier is owned by CoreGraphics.
    let main_bounds = unsafe { CGDisplayBounds(main_display) };
    let scale_factor = if main_bounds.size.width > 0.0 {
        (unsafe { CGDisplayPixelsWide(main_display) } as f64 / main_bounds.size.width).max(1.0)
    } else {
        1.0
    };
    Ok((bounds, scale_factor, geometry))
}

struct CallbackState {
    sender: mpsc::SyncSender<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    tap: AtomicUsize,
}

/// An active session event tap running on its own `CFRunLoop` thread.
pub struct MacMouseCapture {
    receiver: mpsc::Receiver<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    run_loop: usize,
    thread: Option<JoinHandle<()>>,
    cursor_hidden: bool,
}

impl MacMouseCapture {
    pub fn start() -> Result<Self, PlatformError> {
        if !accessibility_trusted() {
            return Err(PlatformError::new(
                "macOS Accessibility permission is required for mouse capture",
            ));
        }

        let (event_sender, event_receiver) = mpsc::sync_channel(CAPTURE_QUEUE_CAPACITY);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let suppress = Arc::new(AtomicBool::new(false));
        let callback_suppress = Arc::clone(&suppress);
        let overflowed = Arc::new(AtomicBool::new(false));
        let callback_overflowed = Arc::clone(&overflowed);
        let thread = std::thread::Builder::new()
            .name("edgemouse-cgevent-tap".to_owned())
            .spawn(move || {
                run_event_tap(
                    event_sender,
                    callback_suppress,
                    callback_overflowed,
                    |result| drop(startup_sender.send(result)),
                );
            })
            .map_err(|error| PlatformError::new(format!("failed to start event tap: {error}")))?;

        let run_loop = startup_receiver
            .recv()
            .map_err(|_| PlatformError::new("event tap exited during startup"))??;
        Ok(Self {
            receiver: event_receiver,
            suppress,
            overflowed,
            run_loop,
            thread: Some(thread),
            cursor_hidden: false,
        })
    }

    fn hide_cursor(&mut self) -> Result<(), PlatformError> {
        if self.cursor_hidden {
            return Ok(());
        }
        Self::allow_background_cursor_control()?;
        // SAFETY: The display ID comes from CoreGraphics and has no ownership.
        let result = unsafe { CGDisplayHideCursor(CGMainDisplayID()) };
        if result != 0 {
            return Err(PlatformError::new(format!(
                "CGDisplayHideCursor failed with code {result}"
            )));
        }
        self.cursor_hidden = true;
        // Re-associating immediately after hiding avoids a WindowServer race in
        // which a background event-tap process has its cursor made visible again.
        // Remote mode disconnects physical motion again after this returns.
        Self::associate_cursor(true)?;
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), PlatformError> {
        if !self.cursor_hidden {
            return Ok(());
        }
        Self::allow_background_cursor_control()?;
        // SAFETY: The display ID comes from CoreGraphics and has no ownership.
        let result = unsafe { CGDisplayShowCursor(CGMainDisplayID()) };
        if result != 0 {
            return Err(PlatformError::new(format!(
                "CGDisplayShowCursor failed with code {result}"
            )));
        }
        self.cursor_hidden = false;
        Ok(())
    }

    fn allow_background_cursor_control() -> Result<(), PlatformError> {
        // CGDisplayHideCursor normally only honors foreground applications. This
        // WindowServer property is the same compatibility path used by mature
        // software KVMs so a login/background agent can own cursor visibility.
        const PROPERTY: &[u8] = b"SetsCursorInBackground\0";
        const MAC_ROMAN: u32 = 0;
        // SAFETY: PROPERTY is a static nul-terminated ASCII string and the create
        // call returns an owned CoreFoundation object.
        let property = unsafe {
            CFStringCreateWithCString(ptr::null(), PROPERTY.as_ptr().cast::<i8>(), MAC_ROMAN)
        };
        if property.is_null() {
            return Err(PlatformError::new(
                "failed to create the macOS background cursor property",
            ));
        }
        // SAFETY: The connection identifiers are supplied by WindowServer; the
        // property and singleton Boolean remain valid for the duration of the call.
        let result = unsafe {
            let connection = _CGSDefaultConnection();
            CGSSetConnectionProperty(connection, connection, property, kCFBooleanTrue)
        };
        // SAFETY: This function owns the create-rule reference.
        unsafe { CFRelease(property) };
        if result == 0 {
            Ok(())
        } else {
            Err(PlatformError::new(format!(
                "failed to enable background cursor control (WindowServer code {result})"
            )))
        }
    }

    fn warp(position: Point) -> Result<(), PlatformError> {
        if !position.is_finite() {
            return Err(PlatformError::new("cursor restore position is not finite"));
        }
        // Use a marked absolute move instead of CGWarpMouseCursorPosition. A raw
        // warp is reported back through the event tap without source metadata
        // and can be mistaken for physical takeover input.
        let event = unsafe {
            CGEventCreateMouseEvent(
                ptr::null_mut(),
                EVENT_MOUSE_MOVED,
                CGPoint {
                    x: position.x,
                    y: position.y,
                },
                0,
            )
        };
        if event.is_null() {
            return Err(PlatformError::new(
                "CGEventCreateMouseEvent failed while moving the cursor",
            ));
        }
        // SAFETY: `event` is a live create-rule reference and posting retains
        // everything CoreGraphics needs before the reference is released.
        unsafe {
            CGEventSetIntegerValueField(event, EVENT_SOURCE_USER_DATA, EVENT_MARKER);
            CGEventPost(EVENT_TAP_SESSION, event);
            CFRelease(event);
        }
        Ok(())
    }

    fn associate_cursor(connected: bool) -> Result<(), PlatformError> {
        // SAFETY: CoreGraphics accepts a Boolean flag and owns no caller memory.
        let result = unsafe { CGAssociateMouseAndMouseCursorPosition(u32::from(connected)) };
        if result == 0 {
            Ok(())
        } else {
            Err(PlatformError::new(format!(
                "CGAssociateMouseAndMouseCursorPosition failed with code {result}"
            )))
        }
    }
}

impl MouseCaptureBackend for MacMouseCapture {
    fn permission_state(&self) -> PermissionState {
        if accessibility_trusted() {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        }
    }

    fn set_mode(&mut self, mode: CaptureMode) -> Result<(), PlatformError> {
        match mode {
            CaptureMode::Local { restore } => {
                self.suppress.store(false, Ordering::Release);
                Self::associate_cursor(true)?;
                if let Some(position) = restore {
                    Self::warp(position)?;
                }
                self.show_cursor()
            }
            CaptureMode::Remote { anchor } => {
                Self::warp(anchor)?;
                self.hide_cursor()?;
                Self::associate_cursor(false)?;
                self.suppress.store(true, Ordering::Release);
                Ok(())
            }
            CaptureMode::ReceivingRemote { position } => {
                Self::warp(position)?;
                self.show_cursor()?;
                Self::associate_cursor(false)?;
                self.suppress.store(true, Ordering::Release);
                Ok(())
            }
        }
    }

    fn try_next_event(&mut self) -> Result<Option<PhysicalMouseEvent>, PlatformError> {
        if self.overflowed.swap(false, Ordering::AcqRel) {
            return Err(PlatformError::new(
                "macOS capture queue overflowed; local input was released",
            ));
        }
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(PlatformError::new("macOS event tap stopped"))
            }
        }
    }
}

impl Drop for MacMouseCapture {
    fn drop(&mut self) {
        self.suppress.store(false, Ordering::Release);
        drop(Self::associate_cursor(true));
        drop(self.show_cursor());
        if self.run_loop != 0 {
            let run_loop = self.run_loop as *mut c_void;
            // SAFETY: The run loop stays alive until its owning thread exits.
            unsafe {
                CFRunLoopStop(run_loop);
                CFRunLoopWakeUp(run_loop);
            }
        }
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

#[derive(Default)]
struct KeyboardRoutingState {
    remote: bool,
    local_pressed: BTreeSet<KeyCode>,
    captured_pressed: BTreeSet<KeyCode>,
    passthrough_pressed: BTreeSet<KeyCode>,
    caps_lock_on: bool,
    function_key_down: bool,
}

impl KeyboardRoutingState {
    fn key_is_pressed(&self, key: KeyCode) -> bool {
        self.local_pressed.contains(&key)
            || self.captured_pressed.contains(&key)
            || self.passthrough_pressed.contains(&key)
    }
}

struct KeyboardCallbackState {
    sender: mpsc::SyncSender<KeyboardEvent>,
    routing: Arc<Mutex<KeyboardRoutingState>>,
    overflowed: Arc<AtomicBool>,
    emergency_release: Arc<AtomicBool>,
    tap: AtomicUsize,
}

/// A dedicated macOS keyboard event tap. It fails open on contention or queue
/// overflow and only suppresses keys pressed after control moves to the peer.
pub struct MacKeyboardCapture {
    receiver: mpsc::Receiver<KeyboardEvent>,
    routing: Arc<Mutex<KeyboardRoutingState>>,
    overflowed: Arc<AtomicBool>,
    emergency_release: Arc<AtomicBool>,
    run_loop: usize,
    thread: Option<JoinHandle<()>>,
}

impl MacKeyboardCapture {
    pub fn start() -> Result<Self, PlatformError> {
        if !accessibility_trusted() {
            return Err(PlatformError::new(
                "macOS Accessibility permission is required for keyboard capture",
            ));
        }

        let (event_sender, event_receiver) = mpsc::sync_channel(CAPTURE_QUEUE_CAPACITY);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        // SAFETY: Combined-session flags contain no borrowed memory and give the
        // initial Caps Lock toggle state before the event tap starts.
        let caps_lock_on = unsafe { CGEventSourceFlagsState(0) } & FLAG_ALPHA_SHIFT != 0;
        let routing = Arc::new(Mutex::new(KeyboardRoutingState {
            caps_lock_on,
            ..KeyboardRoutingState::default()
        }));
        let overflowed = Arc::new(AtomicBool::new(false));
        let emergency_release = Arc::new(AtomicBool::new(false));
        let callback_routing = Arc::clone(&routing);
        let callback_overflowed = Arc::clone(&overflowed);
        let callback_emergency = Arc::clone(&emergency_release);
        let thread = std::thread::Builder::new()
            .name("edgemouse-cgevent-keyboard-tap".to_owned())
            .spawn(move || {
                run_keyboard_event_tap(
                    event_sender,
                    callback_routing,
                    callback_overflowed,
                    callback_emergency,
                    |result| drop(startup_sender.send(result)),
                );
            })
            .map_err(|error| {
                PlatformError::new(format!("failed to start keyboard event tap: {error}"))
            })?;
        let run_loop = startup_receiver
            .recv()
            .map_err(|_| PlatformError::new("keyboard event tap exited during startup"))??;
        Ok(Self {
            receiver: event_receiver,
            routing,
            overflowed,
            emergency_release,
            run_loop,
            thread: Some(thread),
        })
    }

    fn discard_queue(&mut self) -> Result<(), PlatformError> {
        loop {
            match self.receiver.try_recv() {
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(PlatformError::new("macOS keyboard event tap stopped"));
                }
            }
        }
    }
}

impl KeyboardCaptureBackend for MacKeyboardCapture {
    fn permission_state(&self) -> PermissionState {
        if accessibility_trusted() {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        }
    }

    fn set_remote(&mut self, remote: bool) -> Result<(), PlatformError> {
        self.discard_queue()?;
        let mut routing = self
            .routing
            .lock()
            .map_err(|_| PlatformError::new("macOS keyboard routing lock was poisoned"))?;
        if remote && !routing.remote {
            let local_pressed = std::mem::take(&mut routing.local_pressed);
            routing.passthrough_pressed.extend(local_pressed);
        }
        routing.remote = remote;
        Ok(())
    }

    fn try_next_event(&mut self) -> Result<Option<KeyboardEvent>, PlatformError> {
        if self.overflowed.swap(false, Ordering::AcqRel) {
            return Err(PlatformError::new(
                "macOS keyboard capture queue overflowed; local input was released",
            ));
        }
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(PlatformError::new("macOS keyboard event tap stopped"))
            }
        }
    }

    fn take_emergency_release(&self) -> bool {
        self.emergency_release.swap(false, Ordering::AcqRel)
    }
}

impl Drop for MacKeyboardCapture {
    fn drop(&mut self) {
        if let Ok(mut routing) = self.routing.lock() {
            routing.remote = false;
        }
        if self.run_loop != 0 {
            let run_loop = self.run_loop as *mut c_void;
            // SAFETY: The run loop stays alive until its owning thread exits.
            unsafe {
                CFRunLoopStop(run_loop);
                CFRunLoopWakeUp(run_loop);
            }
        }
        if let Some(thread) = self.thread.take() {
            drop(thread.join());
        }
    }
}

/// Posts marked synthetic mouse events into the macOS session event stream.
pub struct MacMouseInjector {
    position: Point,
    pressed: BTreeSet<MouseButton>,
    clicks: ClickTracker,
    motion: MotionSmoother,
}

#[derive(Debug, Clone, Copy)]
struct TimedPosition {
    position: Point,
    at: Instant,
}

#[derive(Debug)]
struct MotionSmoother {
    displayed: Point,
    target: Point,
    latest: Point,
    played: TimedPosition,
    samples: VecDeque<TimedPosition>,
    last_frame: Instant,
    next_frame: Instant,
    pending: bool,
    buffer_delay: Duration,
    time_constant_seconds: f64,
}

impl MotionSmoother {
    fn new(position: Point, now: Instant, level: u8) -> Self {
        let level = f64::from(level.min(100));
        Self {
            displayed: position,
            target: position,
            latest: position,
            played: TimedPosition { position, at: now },
            samples: VecDeque::new(),
            last_frame: now,
            next_frame: now + MOTION_FRAME_INTERVAL,
            pending: false,
            buffer_delay: Duration::from_micros((level * 180.0).round() as u64),
            time_constant_seconds: 0.0015 + level * 0.000_065,
        }
    }

    fn jump(&mut self, position: Point, now: Instant) {
        self.displayed = position;
        self.target = position;
        self.latest = position;
        self.played = TimedPosition { position, at: now };
        self.samples.clear();
        self.last_frame = now;
        self.next_frame = now + MOTION_FRAME_INTERVAL;
        self.pending = false;
    }

    fn set_target(&mut self, position: Point, received_at: Instant) {
        self.latest = position;
        let last_sample_at = self
            .samples
            .back()
            .map_or(self.played.at, |sample| sample.at);
        let sample_at = received_at.max(last_sample_at + MOTION_MIN_SAMPLE_INTERVAL);
        if self.samples.len() == MOTION_MAX_SAMPLES {
            let _ = self.samples.pop_front();
        }
        self.samples.push_back(TimedPosition {
            position,
            at: sample_at,
        });
        self.pending = true;
    }

    fn desired_position(&mut self, playback_at: Instant) -> (Point, bool) {
        while self
            .samples
            .front()
            .is_some_and(|sample| sample.at <= playback_at)
        {
            self.played = self
                .samples
                .pop_front()
                .expect("the front sample was checked");
        }

        if let Some(next) = self.samples.front()
            && playback_at > self.played.at
        {
            let span = next.at.saturating_duration_since(self.played.at);
            if !span.is_zero() {
                let progress = playback_at
                    .saturating_duration_since(self.played.at)
                    .as_secs_f64()
                    / span.as_secs_f64();
                return (
                    lerp_point(
                        self.played.position,
                        next.position,
                        progress.clamp(0.0, 1.0),
                    ),
                    true,
                );
            }
        }

        (self.played.position, !self.samples.is_empty())
    }

    fn sample(&mut self, now: Instant) -> Option<Point> {
        if !self.pending || now < self.next_frame {
            return None;
        }

        let playback_at = now.checked_sub(self.buffer_delay).unwrap_or(now);
        let (desired, keep_polling) = self.desired_position(playback_at);
        self.target = desired;
        let previous_displayed = self.displayed;
        let elapsed = now.saturating_duration_since(self.last_frame).as_secs_f64();
        let dx = self.target.x - self.displayed.x;
        let dy = self.target.y - self.displayed.y;
        let distance = dx.hypot(dy);
        let alpha = 1.0 - (-elapsed / self.time_constant_seconds).exp();
        let frame_fraction = if distance <= MOTION_SNAP_DISTANCE {
            1.0
        } else {
            alpha.min(MOTION_MAX_FRAME_DISTANCE / distance).min(1.0)
        };
        let mut next = Point::new(
            self.displayed.x + dx * frame_fraction,
            self.displayed.y + dy * frame_fraction,
        );
        if (self.target.x - next.x).hypot(self.target.y - next.y) <= MOTION_SNAP_DISTANCE {
            next = self.target;
        }
        self.pending = keep_polling
            || !self.samples.is_empty()
            || (self.target.x - next.x).hypot(self.target.y - next.y) > MOTION_SNAP_DISTANCE;
        self.displayed = next;
        self.last_frame = now;
        self.next_frame = now + MOTION_FRAME_INTERVAL;
        (next != previous_displayed).then_some(next)
    }

    fn finish(&mut self, now: Instant) -> Option<Point> {
        if !self.pending && self.latest == self.displayed {
            return None;
        }
        let target = self.latest;
        self.jump(target, now);
        Some(target)
    }
}

fn lerp_point(start: Point, end: Point, progress: f64) -> Point {
    Point::new(
        start.x + (end.x - start.x) * progress,
        start.y + (end.y - start.y) * progress,
    )
}

#[derive(Debug, Clone, Copy)]
struct ActiveClick {
    pressed_at: Instant,
    position: Point,
    state: i64,
    moved_too_far: bool,
}

#[derive(Debug, Clone, Copy)]
struct CompletedClick {
    pressed_at: Instant,
    position: Point,
    state: i64,
}

#[derive(Debug, Default)]
struct ClickTracker {
    active: BTreeMap<MouseButton, ActiveClick>,
    completed: BTreeMap<MouseButton, CompletedClick>,
}

impl ClickTracker {
    fn press(&mut self, button: MouseButton, position: Point, now: Instant) -> i64 {
        let state = self
            .completed
            .get(&button)
            .filter(|previous| {
                now.saturating_duration_since(previous.pressed_at) <= DOUBLE_CLICK_INTERVAL
                    && points_are_close(previous.position, position)
            })
            .map_or(1, |previous| (previous.state + 1).min(MAX_CLICK_STATE));
        self.active.insert(
            button,
            ActiveClick {
                pressed_at: now,
                position,
                state,
                moved_too_far: false,
            },
        );
        state
    }

    fn release(&mut self, button: MouseButton) -> i64 {
        let Some(active) = self.active.remove(&button) else {
            self.completed.remove(&button);
            return 1;
        };
        if active.moved_too_far {
            self.completed.remove(&button);
        } else {
            self.completed.insert(
                button,
                CompletedClick {
                    pressed_at: active.pressed_at,
                    position: active.position,
                    state: active.state,
                },
            );
        }
        active.state
    }

    fn note_movement(&mut self, position: Point) {
        for active in self.active.values_mut() {
            active.moved_too_far |= !points_are_close(active.position, position);
        }
    }

    fn active_state(&self, button: MouseButton) -> Option<i64> {
        self.active.get(&button).map(|click| click.state)
    }

    fn reset(&mut self) {
        self.active.clear();
        self.completed.clear();
    }
}

fn points_are_close(left: Point, right: Point) -> bool {
    let dx = left.x - right.x;
    let dy = left.y - right.y;
    dx.mul_add(dx, dy * dy) <= DOUBLE_CLICK_DISTANCE * DOUBLE_CLICK_DISTANCE
}

impl MacMouseInjector {
    #[must_use]
    pub fn new(initial_position: Point) -> Self {
        Self::new_with_smoothing(initial_position, 52)
    }

    #[must_use]
    pub fn new_with_smoothing(initial_position: Point, smoothing: u8) -> Self {
        let now = Instant::now();
        Self {
            position: initial_position,
            pressed: BTreeSet::new(),
            clicks: ClickTracker::default(),
            motion: MotionSmoother::new(initial_position, now, smoothing),
        }
    }

    fn post_mouse(
        &self,
        event_type: u32,
        button: u32,
        click_state: Option<i64>,
    ) -> Result<(), PlatformError> {
        // SAFETY: A null source requests the default event source. The returned
        // create-rule object is checked and released after synchronous posting.
        let event = unsafe {
            CGEventCreateMouseEvent(
                ptr::null_mut(),
                event_type,
                CGPoint {
                    x: self.position.x,
                    y: self.position.y,
                },
                button,
            )
        };
        if event.is_null() {
            return Err(PlatformError::new("CGEventCreateMouseEvent failed"));
        }
        // SAFETY: `event` is a live CGEventRef owned by this function.
        unsafe {
            CGEventSetIntegerValueField(event, EVENT_SOURCE_USER_DATA, EVENT_MARKER);
            if let Some(click_state) = click_state {
                CGEventSetIntegerValueField(event, FIELD_MOUSE_CLICK_STATE, click_state);
            }
            CGEventPost(EVENT_TAP_SESSION, event);
            CFRelease(event);
        }
        Ok(())
    }

    fn post_scroll(&self, horizontal: f64, vertical: f64) -> Result<(), PlatformError> {
        if !horizontal.is_finite() || !vertical.is_finite() {
            return Err(PlatformError::new("scroll deltas must be finite"));
        }
        let vertical = rounded_i32(vertical);
        let horizontal = rounded_i32(horizontal);
        // SAFETY: A null source requests the default event source. Two axes are supplied.
        let event = unsafe {
            CGEventCreateScrollWheelEvent2(
                ptr::null_mut(),
                SCROLL_UNIT_PIXEL,
                2,
                vertical,
                horizontal,
                0,
            )
        };
        if event.is_null() {
            return Err(PlatformError::new("CGEventCreateScrollWheelEvent2 failed"));
        }
        // SAFETY: `event` is a live CGEventRef owned by this function.
        unsafe {
            CGEventSetIntegerValueField(event, EVENT_SOURCE_USER_DATA, EVENT_MARKER);
            CGEventPost(EVENT_TAP_SESSION, event);
            CFRelease(event);
        }
        Ok(())
    }

    fn movement_type(&self) -> (u32, u32) {
        if self.pressed.contains(&MouseButton::Primary) {
            (EVENT_LEFT_DRAGGED, 0)
        } else if self.pressed.contains(&MouseButton::Secondary) {
            (EVENT_RIGHT_DRAGGED, 1)
        } else if let Some(button) = self.pressed.iter().next().copied() {
            (EVENT_OTHER_DRAGGED, mac_button_number(button))
        } else {
            (EVENT_MOUSE_MOVED, 0)
        }
    }

    fn post_movement_now(&mut self, position: Point) -> Result<(), PlatformError> {
        if !position.is_finite() {
            return Err(PlatformError::new("mouse position must be finite"));
        }
        self.position = position;
        self.clicks.note_movement(position);
        let (event_type, button) = self.movement_type();
        let click_state = self
            .pressed
            .iter()
            .next()
            .and_then(|button| self.clicks.active_state(*button));
        self.post_mouse(event_type, button, click_state)
    }

    fn flush_movement(&mut self) -> Result<(), PlatformError> {
        if let Some(position) = self.motion.finish(Instant::now()) {
            self.post_movement_now(position)?;
        }
        Ok(())
    }

    fn post_button(
        &mut self,
        button: MouseButton,
        state: ButtonState,
    ) -> Result<(), PlatformError> {
        let event_type = match (button, state) {
            (MouseButton::Primary, ButtonState::Pressed) => EVENT_LEFT_DOWN,
            (MouseButton::Primary, ButtonState::Released) => EVENT_LEFT_UP,
            (MouseButton::Secondary, ButtonState::Pressed) => EVENT_RIGHT_DOWN,
            (MouseButton::Secondary, ButtonState::Released) => EVENT_RIGHT_UP,
            (_, ButtonState::Pressed) => EVENT_OTHER_DOWN,
            (_, ButtonState::Released) => EVENT_OTHER_UP,
        };
        let click_state = match state {
            ButtonState::Pressed => self.clicks.press(button, self.position, Instant::now()),
            ButtonState::Released => self.clicks.release(button),
        };
        self.post_mouse(event_type, mac_button_number(button), Some(click_state))?;
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
}

impl MouseInjectionBackend for MacMouseInjector {
    fn permission_state(&self) -> PermissionState {
        if accessibility_trusted() {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        }
    }

    fn inject(&mut self, event: RemoteMouseEvent) -> Result<(), PlatformError> {
        self.inject_received(event, Instant::now())
    }

    fn inject_received(
        &mut self,
        event: RemoteMouseEvent,
        received_at: Instant,
    ) -> Result<(), PlatformError> {
        match event {
            RemoteMouseEvent::Enter { position, .. } => {
                self.clicks.reset();
                self.motion.jump(position, received_at);
                self.post_movement_now(position)
            }
            RemoteMouseEvent::MoveAbsolute { position, .. } => {
                if !position.is_finite() {
                    return Err(PlatformError::new("mouse position must be finite"));
                }
                self.motion.set_target(position, received_at);
                Ok(())
            }
            RemoteMouseEvent::Button { button, state } => {
                self.flush_movement()?;
                self.post_button(button, state)
            }
            RemoteMouseEvent::Wheel {
                horizontal,
                vertical,
            } => {
                self.flush_movement()?;
                self.post_scroll(horizontal, vertical)
            }
            RemoteMouseEvent::Leave => {
                self.flush_movement()?;
                self.release_all()
            }
            RemoteMouseEvent::ReleaseAll => self.release_all(),
        }
    }

    fn poll(&mut self) -> Result<(), PlatformError> {
        if let Some(position) = self.motion.sample(Instant::now()) {
            self.post_movement_now(position)?;
        }
        Ok(())
    }

    fn release_all(&mut self) -> Result<(), PlatformError> {
        let pressed: Vec<_> = self.pressed.iter().copied().collect();
        let mut first_error = None;
        for button in pressed {
            if let Err(error) = self.post_button(button, ButtonState::Released) {
                first_error.get_or_insert(error);
            }
        }
        self.clicks.reset();
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(())
        }
    }
}

/// Posts marked synthetic keyboard events into the macOS session event stream.
#[derive(Default)]
pub struct MacKeyboardInjector {
    pressed: BTreeSet<KeyCode>,
}

impl MacKeyboardInjector {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pressed: BTreeSet::new(),
        }
    }

    fn post(&mut self, input: KeyboardEvent) -> Result<(), PlatformError> {
        let virtual_key = mac_virtual_key(input.key).ok_or_else(|| {
            PlatformError::new(format!(
                "macOS has no mapping for keyboard usage {:#06x}",
                input.key.usage()
            ))
        })?;
        match input.state {
            KeyState::Pressed => {
                self.pressed.insert(input.key);
            }
            KeyState::Released => {
                self.pressed.remove(&input.key);
            }
        }
        // SAFETY: A null source requests the default event source. The create-rule
        // object is checked, marked, posted synchronously, and then released.
        let event = unsafe {
            CGEventCreateKeyboardEvent(
                ptr::null_mut(),
                virtual_key,
                input.state == KeyState::Pressed,
            )
        };
        if event.is_null() {
            return Err(PlatformError::new("CGEventCreateKeyboardEvent failed"));
        }
        let flags = mac_modifier_flags(&self.pressed);
        // SAFETY: `event` is a live CGEventRef owned by this function.
        unsafe {
            CGEventSetIntegerValueField(event, EVENT_SOURCE_USER_DATA, EVENT_MARKER);
            CGEventSetIntegerValueField(event, FIELD_KEYBOARD_AUTOREPEAT, i64::from(input.repeat));
            CGEventSetFlags(event, flags);
            CGEventPost(EVENT_TAP_SESSION, event);
            CFRelease(event);
        }
        Ok(())
    }
}

impl KeyboardInjectionBackend for MacKeyboardInjector {
    fn permission_state(&self) -> PermissionState {
        if accessibility_trusted() {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        }
    }

    fn inject(&mut self, event: KeyboardEvent) -> Result<(), PlatformError> {
        self.post(event)
    }

    fn release_all(&mut self) -> Result<(), PlatformError> {
        let pressed: Vec<_> = self.pressed.iter().copied().collect();
        let mut first_error = None;
        for key in pressed {
            if let Err(error) = self.post(KeyboardEvent {
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

fn mac_modifier_flags(pressed: &BTreeSet<KeyCode>) -> u64 {
    let has = |left, right| pressed.contains(&left) || pressed.contains(&right);
    let mut flags = 0;
    if has(KeyCode::LEFT_SHIFT, KeyCode::RIGHT_SHIFT) {
        flags |= FLAG_SHIFT;
    }
    if has(KeyCode::LEFT_CONTROL, KeyCode::RIGHT_CONTROL) {
        flags |= FLAG_CONTROL;
    }
    if has(KeyCode::LEFT_ALT, KeyCode::RIGHT_ALT) {
        flags |= FLAG_ALTERNATE;
    }
    if has(KeyCode::LEFT_META, KeyCode::RIGHT_META) {
        flags |= FLAG_COMMAND;
    }
    if pressed.contains(&KeyCode::CAPS_LOCK) {
        flags |= FLAG_ALPHA_SHIFT;
    }
    if pressed
        .iter()
        .any(|key| matches!(key.usage(), 0x53..=0x63 | 0x67))
    {
        flags |= FLAG_NUMERIC_PAD;
    }
    flags
}

fn mac_virtual_key(key: KeyCode) -> Option<u16> {
    Some(match key.usage() {
        0x04 => 0,
        0x05 => 11,
        0x06 => 8,
        0x07 => 2,
        0x08 => 14,
        0x09 => 3,
        0x0a => 5,
        0x0b => 4,
        0x0c => 34,
        0x0d => 38,
        0x0e => 40,
        0x0f => 37,
        0x10 => 46,
        0x11 => 45,
        0x12 => 31,
        0x13 => 35,
        0x14 => 12,
        0x15 => 15,
        0x16 => 1,
        0x17 => 17,
        0x18 => 32,
        0x19 => 9,
        0x1a => 13,
        0x1b => 7,
        0x1c => 16,
        0x1d => 6,
        0x1e => 18,
        0x1f => 19,
        0x20 => 20,
        0x21 => 21,
        0x22 => 23,
        0x23 => 22,
        0x24 => 26,
        0x25 => 28,
        0x26 => 25,
        0x27 => 29,
        0x28 => 36,
        0x29 => 53,
        0x2a => 51,
        0x2b => 48,
        0x2c => 49,
        0x2d => 27,
        0x2e => 24,
        0x2f => 33,
        0x30 => 30,
        0x31 => 42,
        0x33 => 41,
        0x34 => 39,
        0x35 => 50,
        0x36 => 43,
        0x37 => 47,
        0x38 => 44,
        0x39 => 57,
        0x3a => 122,
        0x3b => 120,
        0x3c => 99,
        0x3d => 118,
        0x3e => 96,
        0x3f => 97,
        0x40 => 98,
        0x41 => 100,
        0x42 => 101,
        0x43 => 109,
        0x44 => 103,
        0x45 => 111,
        0x46 => 105,
        0x47 => 107,
        0x48 => 113,
        0x49 => 114,
        0x4a => 115,
        0x4b => 116,
        0x4c => 117,
        0x4d => 119,
        0x4e => 121,
        0x4f => 124,
        0x50 => 123,
        0x51 => 125,
        0x52 => 126,
        0x53 => 71,
        0x54 => 75,
        0x55 => 67,
        0x56 => 78,
        0x57 => 69,
        0x58 => 76,
        0x59 => 83,
        0x5a => 84,
        0x5b => 85,
        0x5c => 86,
        0x5d => 87,
        0x5e => 88,
        0x5f => 89,
        0x60 => 91,
        0x61 => 92,
        0x62 => 82,
        0x63 => 65,
        0x64 => 10,
        0x67 => 81,
        0x68 => 105,
        0x69 => 107,
        0x6a => 113,
        0x6b => 106,
        0x6c => 64,
        0x6d => 79,
        0x6e => 80,
        0x6f => 90,
        0xe0 => 59,
        0xe1 => 56,
        0xe2 => 58,
        0xe3 => 55,
        0xe4 => 62,
        0xe5 => 60,
        0xe6 => 61,
        0xe7 => 54,
        _ => return None,
    })
}

fn run_keyboard_event_tap(
    sender: mpsc::SyncSender<KeyboardEvent>,
    routing: Arc<Mutex<KeyboardRoutingState>>,
    overflowed: Arc<AtomicBool>,
    emergency_release: Arc<AtomicBool>,
    report_startup: impl FnOnce(Result<usize, PlatformError>),
) {
    let state = Box::new(KeyboardCallbackState {
        sender,
        routing,
        overflowed,
        emergency_release,
        tap: AtomicUsize::new(0),
    });
    let state_ptr = Box::into_raw(state);
    let mask = [EVENT_KEY_DOWN, EVENT_KEY_UP, EVENT_FLAGS_CHANGED]
        .into_iter()
        .fold(0_u64, |mask, event_type| mask | (1_u64 << event_type));

    // SAFETY: `state_ptr` remains allocated until the run loop stops. The callback
    // signature and event mask follow the CoreGraphics API contract.
    let tap = unsafe {
        CGEventTapCreate(
            EVENT_TAP_SESSION,
            EVENT_TAP_HEAD,
            EVENT_TAP_ACTIVE,
            mask,
            keyboard_event_tap_callback,
            state_ptr.cast(),
        )
    };
    if tap.is_null() {
        report_startup(Err(PlatformError::new(
            "CGEventTapCreate failed for keyboard capture; check Accessibility permission",
        )));
        // SAFETY: No callback can run because tap creation failed.
        drop(unsafe { Box::from_raw(state_ptr) });
        return;
    }
    // SAFETY: state_ptr is valid for the run-loop lifetime.
    unsafe { (*state_ptr).tap.store(tap as usize, Ordering::Release) };

    // SAFETY: The tap is a valid CFMachPort and CoreFoundation owns the source
    // references while the run loop is active.
    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), tap, 0) };
    if source.is_null() {
        report_startup(Err(PlatformError::new(
            "CFMachPortCreateRunLoopSource failed for keyboard capture",
        )));
        // SAFETY: Both objects were created by this function and are no longer active.
        unsafe {
            CFRelease(tap);
            drop(Box::from_raw(state_ptr));
        }
        return;
    }
    // SAFETY: These functions operate on the current thread's run loop.
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    unsafe {
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    report_startup(Ok(run_loop as usize));
    // SAFETY: Runs until Drop asks this exact run loop to stop.
    unsafe { CFRunLoopRun() };

    // SAFETY: The run loop has stopped, so no callback is active.
    unsafe {
        CFRelease(source);
        CFRelease(tap);
        drop(Box::from_raw(state_ptr));
    }
}

unsafe extern "C" fn keyboard_event_tap_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: *mut c_void,
    user_info: *mut c_void,
) -> *mut c_void {
    if event.is_null() || user_info.is_null() {
        return event;
    }
    // SAFETY: CoreGraphics calls us with the state pointer supplied to the tap.
    let state = unsafe { &*user_info.cast::<KeyboardCallbackState>() };
    if matches!(
        event_type,
        EVENT_TAP_DISABLED_TIMEOUT | EVENT_TAP_DISABLED_USER_INPUT
    ) {
        let tap = state.tap.load(Ordering::Acquire) as *mut c_void;
        if !tap.is_null() {
            // SAFETY: The tap is retained for the callback-state lifetime.
            unsafe { CGEventTapEnable(tap, true) };
        }
        return event;
    }
    // SAFETY: event is a live CGEventRef for this callback.
    if unsafe { CGEventGetIntegerValueField(event, EVENT_SOURCE_USER_DATA) } == EVENT_MARKER {
        return event;
    }
    // SAFETY: The keycode field is valid for keyboard and flags-changed events.
    let virtual_key = unsafe { CGEventGetIntegerValueField(event, FIELD_KEYBOARD_KEYCODE) };
    let Ok(virtual_key) = u16::try_from(virtual_key) else {
        return event;
    };
    // SAFETY: Flags are available on every CGEvent.
    let flags = unsafe { CGEventGetFlags(event) };
    let Ok(mut routing) = state.routing.try_lock() else {
        // Never block WindowServer's event-tap callback.
        return event;
    };

    if is_mac_language_key(virtual_key) {
        let pressed = mac_language_key_pressed(event_type, virtual_key, flags, &routing);
        if virtual_key == 63 {
            routing.function_key_down = pressed;
        }
        let suppress = routing.remote;
        drop(routing);
        if suppress && pressed {
            if queue_windows_language_toggle(state) {
                return ptr::null_mut();
            }
            return event;
        }
        return if suppress { ptr::null_mut() } else { event };
    }

    let Some(key) = mac_key_code(virtual_key) else {
        return event;
    };

    if event_type == EVENT_FLAGS_CHANGED && key == KeyCode::CAPS_LOCK {
        let caps_lock_on = flags & FLAG_ALPHA_SHIFT != 0;
        if caps_lock_on == routing.caps_lock_on {
            return if routing.remote {
                ptr::null_mut()
            } else {
                event
            };
        }
        routing.caps_lock_on = caps_lock_on;
        let suppress = routing.remote;
        drop(routing);
        if suppress && queue_windows_language_toggle(state) {
            return ptr::null_mut();
        }
        return event;
    }

    let modifier_events = if matches!(event_type, EVENT_KEY_DOWN | EVENT_KEY_UP) {
        reconcile_mac_modifier_flags(&mut routing, flags)
    } else {
        Vec::new()
    };
    let Some(key_state) = mac_key_state(event_type, key, flags, &routing) else {
        return event;
    };
    let (send, suppress, repeat) = route_mac_keyboard_event(&mut routing, key, key_state);
    let emergency = suppress
        && key == KeyCode::ESCAPE
        && key_state == KeyState::Pressed
        && has_mac_emergency_modifiers(&routing.captured_pressed);
    if emergency {
        routing.remote = false;
        state.emergency_release.store(true, Ordering::Release);
    }
    drop(routing);

    for modifier_event in modifier_events {
        if !queue_mac_keyboard_event(state, modifier_event) {
            return event;
        }
    }
    if send && !emergency {
        let forwarded = KeyboardEvent {
            key: remote_mac_key_code(key),
            state: key_state,
            repeat,
        };
        if !queue_mac_keyboard_event(state, forwarded) {
            return event;
        }
    }
    if suppress { ptr::null_mut() } else { event }
}

fn queue_windows_language_toggle(state: &KeyboardCallbackState) -> bool {
    // Win+Space is Windows' layout-independent input-source switch. Sending the
    // complete chord preserves the Mac 中/英 key's user-facing meaning instead of
    // incorrectly turning it into Windows Caps Lock.
    [
        (KeyCode::LEFT_META, KeyState::Pressed),
        (KeyCode::SPACE, KeyState::Pressed),
        (KeyCode::SPACE, KeyState::Released),
        (KeyCode::LEFT_META, KeyState::Released),
    ]
    .into_iter()
    .all(|(key, key_state)| {
        queue_mac_keyboard_event(
            state,
            KeyboardEvent {
                key,
                state: key_state,
                repeat: false,
            },
        )
    })
}

fn is_mac_language_key(virtual_key: u16) -> bool {
    // Globe/Fn, JIS Eisu and JIS Kana. Caps Lock is handled separately because
    // it reports its toggle through FLAG_ALPHA_SHIFT.
    matches!(virtual_key, 63 | 102 | 104)
}

fn mac_language_key_pressed(
    event_type: u32,
    virtual_key: u16,
    flags: u64,
    routing: &KeyboardRoutingState,
) -> bool {
    match event_type {
        EVENT_KEY_DOWN => true,
        EVENT_KEY_UP => false,
        EVENT_FLAGS_CHANGED if virtual_key == 63 => {
            flags & FLAG_SECONDARY_FN != 0 && !routing.function_key_down
        }
        _ => false,
    }
}

fn queue_mac_keyboard_event(state: &KeyboardCallbackState, event: KeyboardEvent) -> bool {
    match state.sender.try_send(event) {
        Ok(()) => true,
        Err(mpsc::TrySendError::Full(_) | mpsc::TrySendError::Disconnected(_)) => {
            state.overflowed.store(true, Ordering::Release);
            if let Ok(mut routing) = state.routing.try_lock() {
                routing.remote = false;
            }
            false
        }
    }
}

fn mac_key_state(
    event_type: u32,
    key: KeyCode,
    flags: u64,
    routing: &KeyboardRoutingState,
) -> Option<KeyState> {
    match event_type {
        EVENT_KEY_DOWN => Some(KeyState::Pressed),
        EVENT_KEY_UP => Some(KeyState::Released),
        EVENT_FLAGS_CHANGED => {
            let flag = mac_modifier_flag(key)?;
            if flags & flag == 0 || routing.key_is_pressed(key) {
                Some(KeyState::Released)
            } else {
                Some(KeyState::Pressed)
            }
        }
        _ => None,
    }
}

fn mac_modifier_flag(key: KeyCode) -> Option<u64> {
    match key {
        KeyCode::LEFT_SHIFT | KeyCode::RIGHT_SHIFT => Some(FLAG_SHIFT),
        KeyCode::LEFT_CONTROL | KeyCode::RIGHT_CONTROL => Some(FLAG_CONTROL),
        KeyCode::LEFT_ALT | KeyCode::RIGHT_ALT => Some(FLAG_ALTERNATE),
        KeyCode::LEFT_META | KeyCode::RIGHT_META => Some(FLAG_COMMAND),
        _ => None,
    }
}

fn reconcile_mac_modifier_flags(
    routing: &mut KeyboardRoutingState,
    flags: u64,
) -> Vec<KeyboardEvent> {
    if !routing.remote {
        return Vec::new();
    }
    let groups = [
        (FLAG_SHIFT, KeyCode::LEFT_SHIFT, KeyCode::RIGHT_SHIFT),
        (FLAG_CONTROL, KeyCode::LEFT_CONTROL, KeyCode::RIGHT_CONTROL),
        (FLAG_ALTERNATE, KeyCode::LEFT_ALT, KeyCode::RIGHT_ALT),
        (FLAG_COMMAND, KeyCode::LEFT_META, KeyCode::RIGHT_META),
    ];
    let mut events = Vec::new();
    for (flag, left, right) in groups {
        let left_down = routing.captured_pressed.contains(&left);
        let right_down = routing.captured_pressed.contains(&right);
        if flags & flag != 0 && !left_down && !right_down {
            routing.captured_pressed.insert(left);
            events.push(KeyboardEvent {
                key: remote_mac_key_code(left),
                state: KeyState::Pressed,
                repeat: false,
            });
        } else if flags & flag == 0 {
            for key in [left, right] {
                if routing.captured_pressed.remove(&key) {
                    events.push(KeyboardEvent {
                        key: remote_mac_key_code(key),
                        state: KeyState::Released,
                        repeat: false,
                    });
                }
            }
        }
    }
    events
}

fn route_mac_keyboard_event(
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

/// Map Mac shortcut muscle memory to Windows: Command becomes Control, while
/// physical Control remains available as the Windows key.
fn remote_mac_key_code(key: KeyCode) -> KeyCode {
    match key {
        KeyCode::LEFT_CONTROL => KeyCode::LEFT_META,
        KeyCode::RIGHT_CONTROL => KeyCode::RIGHT_META,
        KeyCode::LEFT_META => KeyCode::LEFT_CONTROL,
        KeyCode::RIGHT_META => KeyCode::RIGHT_CONTROL,
        _ => key,
    }
}

fn has_mac_emergency_modifiers(pressed: &BTreeSet<KeyCode>) -> bool {
    let control_or_command = pressed.contains(&KeyCode::LEFT_CONTROL)
        || pressed.contains(&KeyCode::RIGHT_CONTROL)
        || pressed.contains(&KeyCode::LEFT_META)
        || pressed.contains(&KeyCode::RIGHT_META);
    control_or_command
        && (pressed.contains(&KeyCode::LEFT_ALT) || pressed.contains(&KeyCode::RIGHT_ALT))
        && (pressed.contains(&KeyCode::LEFT_SHIFT) || pressed.contains(&KeyCode::RIGHT_SHIFT))
}

fn mac_key_code(virtual_key: u16) -> Option<KeyCode> {
    let usage = match virtual_key {
        0 => 0x04,
        11 => 0x05,
        8 => 0x06,
        2 => 0x07,
        14 => 0x08,
        3 => 0x09,
        5 => 0x0a,
        4 => 0x0b,
        34 => 0x0c,
        38 => 0x0d,
        40 => 0x0e,
        37 => 0x0f,
        46 => 0x10,
        45 => 0x11,
        31 => 0x12,
        35 => 0x13,
        12 => 0x14,
        15 => 0x15,
        1 => 0x16,
        17 => 0x17,
        32 => 0x18,
        9 => 0x19,
        13 => 0x1a,
        7 => 0x1b,
        16 => 0x1c,
        6 => 0x1d,
        18 => 0x1e,
        19 => 0x1f,
        20 => 0x20,
        21 => 0x21,
        23 => 0x22,
        22 => 0x23,
        26 => 0x24,
        28 => 0x25,
        25 => 0x26,
        29 => 0x27,
        36 => 0x28,
        53 => 0x29,
        51 => 0x2a,
        48 => 0x2b,
        49 => 0x2c,
        27 => 0x2d,
        24 => 0x2e,
        33 => 0x2f,
        30 => 0x30,
        42 => 0x31,
        10 => 0x64,
        41 => 0x33,
        39 => 0x34,
        50 => 0x35,
        43 => 0x36,
        47 => 0x37,
        44 => 0x38,
        57 => 0x39,
        122 => 0x3a,
        120 => 0x3b,
        99 => 0x3c,
        118 => 0x3d,
        96 => 0x3e,
        97 => 0x3f,
        98 => 0x40,
        100 => 0x41,
        101 => 0x42,
        109 => 0x43,
        103 => 0x44,
        111 => 0x45,
        114 => 0x49,
        115 => 0x4a,
        116 => 0x4b,
        117 => 0x4c,
        119 => 0x4d,
        121 => 0x4e,
        124 => 0x4f,
        123 => 0x50,
        125 => 0x51,
        126 => 0x52,
        71 => 0x53,
        75 => 0x54,
        67 => 0x55,
        78 => 0x56,
        69 => 0x57,
        76 => 0x58,
        83 => 0x59,
        84 => 0x5a,
        85 => 0x5b,
        86 => 0x5c,
        87 => 0x5d,
        88 => 0x5e,
        89 => 0x5f,
        91 => 0x60,
        92 => 0x61,
        82 => 0x62,
        65 => 0x63,
        81 => 0x67,
        105 => 0x68,
        107 => 0x69,
        113 => 0x6a,
        106 => 0x6b,
        64 => 0x6c,
        79 => 0x6d,
        80 => 0x6e,
        90 => 0x6f,
        59 => 0xe0,
        56 => 0xe1,
        58 => 0xe2,
        55 => 0xe3,
        62 => 0xe4,
        60 => 0xe5,
        61 => 0xe6,
        54 => 0xe7,
        _ => return None,
    };
    KeyCode::from_usage(usage)
}

fn run_event_tap(
    sender: mpsc::SyncSender<PhysicalMouseEvent>,
    suppress: Arc<AtomicBool>,
    overflowed: Arc<AtomicBool>,
    report_startup: impl FnOnce(Result<usize, PlatformError>),
) {
    let state = Box::new(CallbackState {
        sender,
        suppress,
        overflowed,
        tap: AtomicUsize::new(0),
    });
    let state_ptr = Box::into_raw(state);
    let mask = [
        EVENT_LEFT_DOWN,
        EVENT_LEFT_UP,
        EVENT_RIGHT_DOWN,
        EVENT_RIGHT_UP,
        EVENT_MOUSE_MOVED,
        EVENT_LEFT_DRAGGED,
        EVENT_RIGHT_DRAGGED,
        EVENT_SCROLL_WHEEL,
        EVENT_OTHER_DOWN,
        EVENT_OTHER_UP,
        EVENT_OTHER_DRAGGED,
    ]
    .into_iter()
    .fold(0_u64, |mask, event_type| mask | (1_u64 << event_type));

    // SAFETY: `state_ptr` remains allocated until the run loop stops. The callback
    // signature and event mask follow the CoreGraphics API contract.
    let tap = unsafe {
        CGEventTapCreate(
            EVENT_TAP_SESSION,
            EVENT_TAP_HEAD,
            EVENT_TAP_ACTIVE,
            mask,
            event_tap_callback,
            state_ptr.cast(),
        )
    };
    if tap.is_null() {
        report_startup(Err(PlatformError::new(
            "CGEventTapCreate failed; check Accessibility permission",
        )));
        // SAFETY: No callback can run because tap creation failed.
        drop(unsafe { Box::from_raw(state_ptr) });
        return;
    }
    // SAFETY: state_ptr is valid for the run-loop lifetime.
    unsafe { (*state_ptr).tap.store(tap as usize, Ordering::Release) };

    // SAFETY: The tap is a valid CFMachPort and CoreFoundation owns the source
    // references while the run loop is active.
    let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null(), tap, 0) };
    if source.is_null() {
        report_startup(Err(PlatformError::new(
            "CFMachPortCreateRunLoopSource failed",
        )));
        // SAFETY: Both objects were created by this function and are no longer active.
        unsafe {
            CFRelease(tap);
            drop(Box::from_raw(state_ptr));
        }
        return;
    }
    // SAFETY: These functions operate on the current thread's run loop.
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    unsafe {
        CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
    }
    report_startup(Ok(run_loop as usize));
    // SAFETY: Runs until Drop asks this exact run loop to stop.
    unsafe { CFRunLoopRun() };

    // SAFETY: The run loop has stopped, so no callback is active. Release create-rule objects.
    unsafe {
        CFRelease(source);
        CFRelease(tap);
        drop(Box::from_raw(state_ptr));
    }
}

unsafe extern "C" fn event_tap_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: *mut c_void,
    user_info: *mut c_void,
) -> *mut c_void {
    if event.is_null() || user_info.is_null() {
        return event;
    }
    // SAFETY: CoreGraphics calls us with the CallbackState pointer supplied to
    // CGEventTapCreate, which lives until the run loop stops.
    let state = unsafe { &*user_info.cast::<CallbackState>() };
    if matches!(
        event_type,
        EVENT_TAP_DISABLED_TIMEOUT | EVENT_TAP_DISABLED_USER_INPUT
    ) {
        let tap = state.tap.load(Ordering::Acquire) as *mut c_void;
        if !tap.is_null() {
            // SAFETY: The tap is retained for the whole callback-state lifetime.
            unsafe { CGEventTapEnable(tap, true) };
        }
        return event;
    }
    // SAFETY: event is a live CGEventRef for the duration of this callback.
    if unsafe { CGEventGetIntegerValueField(event, EVENT_SOURCE_USER_DATA) } == EVENT_MARKER {
        return event;
    }

    if let Some(physical) = physical_event(event_type, event) {
        match state.sender.try_send(physical) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                state.overflowed.store(true, Ordering::Release);
                state.suppress.store(false, Ordering::Release);
                return event;
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return event,
        }
        if state.suppress.load(Ordering::Acquire) {
            return ptr::null_mut();
        }
    }
    event
}

fn physical_event(event_type: u32, event: *mut c_void) -> Option<PhysicalMouseEvent> {
    match event_type {
        EVENT_MOUSE_MOVED | EVENT_LEFT_DRAGGED | EVENT_RIGHT_DRAGGED | EVENT_OTHER_DRAGGED => {
            // SAFETY: Fields are valid for mouse movement events.
            let dx = unsafe { CGEventGetIntegerValueField(event, FIELD_MOUSE_DELTA_X) } as f64;
            // SAFETY: Fields are valid for mouse movement events.
            let dy = unsafe { CGEventGetIntegerValueField(event, FIELD_MOUSE_DELTA_Y) } as f64;
            Some(PhysicalMouseEvent::Move {
                movement: Vector::new(dx, dy),
            })
        }
        EVENT_LEFT_DOWN => Some(button_event(MouseButton::Primary, ButtonState::Pressed)),
        EVENT_LEFT_UP => Some(button_event(MouseButton::Primary, ButtonState::Released)),
        EVENT_RIGHT_DOWN => Some(button_event(MouseButton::Secondary, ButtonState::Pressed)),
        EVENT_RIGHT_UP => Some(button_event(MouseButton::Secondary, ButtonState::Released)),
        EVENT_OTHER_DOWN | EVENT_OTHER_UP => {
            // SAFETY: This field is valid for other-button events.
            let number = unsafe { CGEventGetIntegerValueField(event, FIELD_MOUSE_BUTTON_NUMBER) };
            let button = mouse_button_from_number(number);
            let state = if event_type == EVENT_OTHER_DOWN {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            };
            Some(button_event(button, state))
        }
        EVENT_SCROLL_WHEEL => {
            // Prefer pixel deltas and fall back to fixed-point/line values.
            // SAFETY: Scroll fields are valid for scroll-wheel events.
            let mut vertical =
                unsafe { CGEventGetDoubleValueField(event, FIELD_SCROLL_POINT_VERTICAL) };
            // SAFETY: Scroll fields are valid for scroll-wheel events.
            let mut horizontal =
                unsafe { CGEventGetDoubleValueField(event, FIELD_SCROLL_POINT_HORIZONTAL) };
            if vertical == 0.0 {
                // SAFETY: Fallback scroll field is valid for scroll-wheel events.
                vertical =
                    unsafe { CGEventGetDoubleValueField(event, FIELD_SCROLL_FIXED_VERTICAL) };
            }
            if horizontal == 0.0 {
                // SAFETY: Fallback scroll field is valid for scroll-wheel events.
                horizontal =
                    unsafe { CGEventGetDoubleValueField(event, FIELD_SCROLL_FIXED_HORIZONTAL) };
            }
            Some(PhysicalMouseEvent::Wheel {
                horizontal,
                vertical,
            })
        }
        _ => None,
    }
}

fn button_event(button: MouseButton, state: ButtonState) -> PhysicalMouseEvent {
    PhysicalMouseEvent::Button { button, state }
}

fn mouse_button_from_number(number: i64) -> MouseButton {
    match number {
        0 => MouseButton::Primary,
        1 => MouseButton::Secondary,
        2 => MouseButton::Middle,
        3 => MouseButton::Back,
        4 => MouseButton::Forward,
        other => MouseButton::Other(other.clamp(0, i64::from(u8::MAX)) as u8),
    }
}

fn mac_button_number(button: MouseButton) -> u32 {
    match button {
        MouseButton::Primary => 0,
        MouseButton::Secondary => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(number) => u32::from(number),
    }
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
    fn maps_standard_and_extended_buttons() {
        assert_eq!(mouse_button_from_number(2), MouseButton::Middle);
        assert_eq!(mouse_button_from_number(3), MouseButton::Back);
        assert_eq!(mouse_button_from_number(4), MouseButton::Forward);
        assert_eq!(mouse_button_from_number(9), MouseButton::Other(9));
    }

    #[test]
    fn maps_common_windows_keys_to_mac_virtual_keys() {
        assert_eq!(mac_virtual_key(KeyCode::A), Some(0));
        assert_eq!(mac_virtual_key(KeyCode::ENTER), Some(36));
        assert_eq!(mac_virtual_key(KeyCode::LEFT_META), Some(55));
        assert_eq!(mac_virtual_key(KeyCode::RIGHT_ALT), Some(61));
        assert_eq!(mac_virtual_key(KeyCode::ARROW_LEFT), Some(123));
    }

    #[test]
    fn maps_common_mac_virtual_keys_to_hid_usages() {
        assert_eq!(mac_key_code(0), Some(KeyCode::A));
        assert_eq!(mac_key_code(36), Some(KeyCode::ENTER));
        assert_eq!(mac_key_code(55), Some(KeyCode::LEFT_META));
        assert_eq!(mac_key_code(61), Some(KeyCode::RIGHT_ALT));
        assert_eq!(mac_key_code(123), Some(KeyCode::ARROW_LEFT));
        assert_eq!(mac_key_code(u16::MAX), None);
    }

    #[test]
    fn swaps_command_and_control_for_windows_shortcut_semantics() {
        assert_eq!(
            remote_mac_key_code(KeyCode::LEFT_META),
            KeyCode::LEFT_CONTROL
        );
        assert_eq!(
            remote_mac_key_code(KeyCode::RIGHT_META),
            KeyCode::RIGHT_CONTROL
        );
        assert_eq!(
            remote_mac_key_code(KeyCode::LEFT_CONTROL),
            KeyCode::LEFT_META
        );
        assert_eq!(
            remote_mac_key_code(KeyCode::RIGHT_CONTROL),
            KeyCode::RIGHT_META
        );
        assert_eq!(remote_mac_key_code(KeyCode::LEFT_ALT), KeyCode::LEFT_ALT);
        assert_eq!(remote_mac_key_code(KeyCode::A), KeyCode::A);
    }

    #[test]
    fn mac_keys_held_before_handoff_remain_local_until_released() {
        let mut routing = KeyboardRoutingState::default();
        assert_eq!(
            route_mac_keyboard_event(&mut routing, KeyCode::LEFT_META, KeyState::Pressed),
            (false, false, false)
        );
        let local_pressed = std::mem::take(&mut routing.local_pressed);
        routing.passthrough_pressed.extend(local_pressed);
        routing.remote = true;
        assert_eq!(
            route_mac_keyboard_event(&mut routing, KeyCode::LEFT_META, KeyState::Released),
            (false, false, false)
        );
        assert!(routing.passthrough_pressed.is_empty());
    }

    #[test]
    fn mac_remote_keys_stay_suppressed_until_physically_released() {
        let mut routing = KeyboardRoutingState {
            remote: true,
            ..KeyboardRoutingState::default()
        };
        assert_eq!(
            route_mac_keyboard_event(&mut routing, KeyCode::A, KeyState::Pressed),
            (true, true, false)
        );
        assert_eq!(
            route_mac_keyboard_event(&mut routing, KeyCode::A, KeyState::Pressed),
            (true, true, true)
        );
        routing.remote = false;
        assert_eq!(
            route_mac_keyboard_event(&mut routing, KeyCode::A, KeyState::Released),
            (false, true, false)
        );
        assert!(routing.captured_pressed.is_empty());
    }

    #[test]
    fn flags_changed_distinguishes_left_and_right_modifier_releases() {
        let mut routing = KeyboardRoutingState::default();
        routing.local_pressed.insert(KeyCode::LEFT_SHIFT);
        routing.local_pressed.insert(KeyCode::RIGHT_SHIFT);
        assert_eq!(
            mac_key_state(
                EVENT_FLAGS_CHANGED,
                KeyCode::RIGHT_SHIFT,
                FLAG_SHIFT,
                &routing
            ),
            Some(KeyState::Released)
        );
        routing.local_pressed.remove(&KeyCode::RIGHT_SHIFT);
        assert_eq!(
            mac_key_state(
                EVENT_FLAGS_CHANGED,
                KeyCode::RIGHT_SHIFT,
                FLAG_SHIFT,
                &routing
            ),
            Some(KeyState::Pressed)
        );
    }

    #[test]
    fn ordinary_key_events_recover_a_missed_modifier_transition() {
        let mut routing = KeyboardRoutingState {
            remote: true,
            ..KeyboardRoutingState::default()
        };
        assert_eq!(
            reconcile_mac_modifier_flags(&mut routing, FLAG_COMMAND),
            vec![KeyboardEvent {
                key: KeyCode::LEFT_CONTROL,
                state: KeyState::Pressed,
                repeat: false,
            }]
        );
        assert!(routing.captured_pressed.contains(&KeyCode::LEFT_META));
        assert!(reconcile_mac_modifier_flags(&mut routing, FLAG_COMMAND).is_empty());
        assert_eq!(
            reconcile_mac_modifier_flags(&mut routing, 0),
            vec![KeyboardEvent {
                key: KeyCode::LEFT_CONTROL,
                state: KeyState::Released,
                repeat: false,
            }]
        );
        assert!(routing.captured_pressed.is_empty());
    }

    #[test]
    fn modifier_recovery_covers_option_and_physical_control() {
        let mut routing = KeyboardRoutingState {
            remote: true,
            ..KeyboardRoutingState::default()
        };
        assert_eq!(
            reconcile_mac_modifier_flags(&mut routing, FLAG_ALTERNATE | FLAG_CONTROL),
            vec![
                KeyboardEvent {
                    key: KeyCode::LEFT_META,
                    state: KeyState::Pressed,
                    repeat: false,
                },
                KeyboardEvent {
                    key: KeyCode::LEFT_ALT,
                    state: KeyState::Pressed,
                    repeat: false,
                },
            ]
        );
    }

    #[test]
    fn mac_language_keys_trigger_only_on_press() {
        let mut routing = KeyboardRoutingState::default();
        assert!(is_mac_language_key(63));
        assert!(is_mac_language_key(102));
        assert!(is_mac_language_key(104));
        assert!(!is_mac_language_key(57));
        assert!(mac_language_key_pressed(
            EVENT_FLAGS_CHANGED,
            63,
            FLAG_SECONDARY_FN,
            &routing
        ));
        routing.function_key_down = true;
        assert!(!mac_language_key_pressed(
            EVENT_FLAGS_CHANGED,
            63,
            FLAG_SECONDARY_FN,
            &routing
        ));
        assert!(!mac_language_key_pressed(
            EVENT_FLAGS_CHANGED,
            63,
            0,
            &routing
        ));
        assert!(mac_language_key_pressed(EVENT_KEY_DOWN, 102, 0, &routing));
        assert!(!mac_language_key_pressed(EVENT_KEY_UP, 102, 0, &routing));
    }

    #[test]
    fn modifier_flags_follow_pressed_keys() {
        let pressed = BTreeSet::from([KeyCode::LEFT_SHIFT, KeyCode::LEFT_META, KeyCode::NUMPAD_1]);
        assert_eq!(
            mac_modifier_flags(&pressed),
            FLAG_SHIFT | FLAG_COMMAND | FLAG_NUMERIC_PAD
        );
    }

    #[test]
    fn clamps_scroll_values_for_core_graphics() {
        assert_eq!(rounded_i32(f64::MAX), i32::MAX);
        assert_eq!(rounded_i32(f64::MIN), i32::MIN);
        assert_eq!(rounded_i32(12.6), 13);
    }

    #[test]
    fn marks_fast_nearby_clicks_as_a_double_click() {
        let mut clicks = ClickTracker::default();
        let start = Instant::now();
        let position = Point::new(100.0, 200.0);

        assert_eq!(clicks.press(MouseButton::Primary, position, start), 1);
        assert_eq!(clicks.release(MouseButton::Primary), 1);
        assert_eq!(
            clicks.press(
                MouseButton::Primary,
                Point::new(102.0, 201.0),
                start + Duration::from_millis(120),
            ),
            2
        );
        assert_eq!(clicks.release(MouseButton::Primary), 2);
    }

    #[test]
    fn resets_click_count_after_timeout_or_drag() {
        let mut clicks = ClickTracker::default();
        let start = Instant::now();
        let position = Point::new(100.0, 200.0);

        assert_eq!(clicks.press(MouseButton::Primary, position, start), 1);
        assert_eq!(clicks.release(MouseButton::Primary), 1);
        assert_eq!(
            clicks.press(
                MouseButton::Primary,
                position,
                start + DOUBLE_CLICK_INTERVAL + Duration::from_millis(1),
            ),
            1
        );
        clicks.note_movement(Point::new(120.0, 200.0));
        assert_eq!(clicks.release(MouseButton::Primary), 1);
        assert_eq!(
            clicks.press(
                MouseButton::Primary,
                Point::new(120.0, 200.0),
                start + DOUBLE_CLICK_INTERVAL + Duration::from_millis(100),
            ),
            1
        );
    }

    #[test]
    fn motion_smoother_waits_for_its_jitter_buffer() {
        let start = Instant::now();
        let mut motion = MotionSmoother::new(Point::new(0.0, 0.0), start, 52);
        motion.set_target(Point::new(100.0, 0.0), start + Duration::from_millis(4));

        assert_eq!(motion.sample(start + Duration::from_millis(3)), None);
        assert_eq!(motion.sample(start + Duration::from_millis(4)), None);
        assert_eq!(motion.sample(start + Duration::from_millis(8)), None);
        let first = motion
            .sample(start + Duration::from_millis(12))
            .expect("the buffered sample should become ready");
        assert!(first.x > 0.0 && first.x < 100.0);
        assert_eq!(first.y, 0.0);

        let mut latest = first;
        for frame in 4..=16 {
            if let Some(position) = motion.sample(start + Duration::from_millis(frame * 4)) {
                latest = position;
            }
        }
        assert!((100.0 - latest.x).abs() <= MOTION_SNAP_DISTANCE);
    }

    #[test]
    fn motion_smoother_interpolates_between_buffered_samples() {
        let start = Instant::now();
        let mut motion = MotionSmoother::new(Point::new(0.0, 0.0), start, 52);
        motion.set_target(Point::new(40.0, 0.0), start + Duration::from_millis(4));
        motion.set_target(Point::new(80.0, 0.0), start + Duration::from_millis(8));

        let (desired, keep_polling) = motion.desired_position(start + Duration::from_millis(6));
        assert_eq!(desired, Point::new(60.0, 0.0));
        assert!(keep_polling);
    }

    #[test]
    fn motion_smoother_never_predicts_past_the_latest_real_sample() {
        let start = Instant::now();
        let mut motion = MotionSmoother::new(Point::new(0.0, 0.0), start, 52);
        motion.set_target(Point::new(10.0, 0.0), start + Duration::from_millis(4));
        motion.set_target(Point::new(20.0, 0.0), start + Duration::from_millis(8));
        let _ = motion.desired_position(start + Duration::from_millis(8));

        let (stopped, keep_polling) = motion.desired_position(start + Duration::from_millis(80));
        assert_eq!(stopped, Point::new(20.0, 0.0));
        assert!(!keep_polling);
    }

    #[test]
    fn reliable_control_flushes_the_latest_buffered_position() {
        let start = Instant::now();
        let mut motion = MotionSmoother::new(Point::new(0.0, 0.0), start, 52);
        motion.set_target(Point::new(1_000.0, 0.0), start + Duration::from_millis(1));

        assert_eq!(
            motion.finish(start + Duration::from_millis(2)),
            Some(Point::new(1_000.0, 0.0))
        );
        assert!(motion.samples.is_empty());
    }
}
