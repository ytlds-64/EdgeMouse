//! Preserve an existing editable responder across an input handoff. No text,
//! titles, selection, clipboard, or application activation is read or changed.

use std::ffi::c_void;
use std::ptr;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *const c_void,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, seconds: f32) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: *const c_void;
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        bytes: *const i8,
        encoding: u32,
    ) -> *mut c_void;
    fn CFRelease(value: *const c_void);
    fn CFEqual(a: *const c_void, b: *const c_void) -> u8;
    fn CFGetTypeID(value: *const c_void) -> usize;
}

struct OwnedCf(*const c_void);

impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: All instances own a non-null Create/Copy reference.
        unsafe { CFRelease(self.0) };
    }
}

impl OwnedCf {
    fn string(value: &std::ffi::CStr) -> Option<Self> {
        // SAFETY: A valid nul-terminated UTF-8 constant is supplied.
        let value = unsafe { CFStringCreateWithCString(ptr::null(), value.as_ptr(), 0x0800_0100) };
        if value.is_null() {
            None
        } else {
            Some(Self(value))
        }
    }

    fn same(&self, other: &Self) -> bool {
        // SAFETY: Both are live CoreFoundation objects.
        unsafe { CFEqual(self.0, other.0) != 0 }
    }

    fn ax(self) -> Option<Self> {
        // SAFETY: This object is live; reject non-AX attributes before AX calls.
        if unsafe { CFGetTypeID(self.0) != AXUIElementGetTypeID() } {
            return None;
        }
        // Best effort must not let an unresponsive application freeze control.
        unsafe { AXUIElementSetMessagingTimeout(self.0, 0.02) };
        Some(self)
    }

    fn attribute(&self, name: &std::ffi::CStr) -> Result<Option<Self>, ()> {
        let name = Self::string(name).ok_or(())?;
        let mut value = ptr::null();
        // SAFETY: Called only on validated AX objects; Copy returns an owned ref.
        let result = unsafe { AXUIElementCopyAttributeValue(self.0, name.0, &raw mut value) };
        match result {
            0 if !value.is_null() => Ok(Some(Self(value))),
            -25212 => Ok(None), // kAXErrorNoValue, distinct from failed messaging
            _ => Err(()),
        }
    }

    fn element(&self, name: &std::ffi::CStr) -> Option<Self> {
        self.attribute(name).ok()??.ax()
    }
}

fn front() -> Option<(OwnedCf, OwnedCf)> {
    // SAFETY: Create returns an owned system-wide AX object.
    let system = unsafe { AXUIElementCreateSystemWide() };
    if system.is_null() {
        return None;
    }
    let system = OwnedCf(system).ax()?;
    let app = system.element(c"AXFocusedApplication")?;
    let window = app.element(c"AXFocusedWindow")?;
    Some((app, window))
}

#[derive(Clone, Copy, Debug)]
enum CurrentFocus {
    SameInput,
    MissingOrWindow,
    Other,
    Unavailable,
}

fn may_restore(same_app: bool, same_window: bool, current: CurrentFocus) -> bool {
    same_app
        && same_window
        && matches!(
            current,
            CurrentFocus::SameInput | CurrentFocus::MissingOrWindow
        )
}

pub(super) struct InputFocusBookmark {
    app: OwnedCf,
    window: OwnedCf,
    input: OwnedCf,
}

impl InputFocusBookmark {
    pub(super) fn capture() -> Option<Self> {
        let (app, window) = front()?;
        let input = app.element(c"AXFocusedUIElement")?;
        let role = input.attribute(c"AXRole").ok()??;
        let editable = [c"AXTextField", c"AXTextArea", c"AXComboBox"]
            .iter()
            .any(|name| OwnedCf::string(name).is_some_and(|candidate| candidate.same(&role)));
        editable.then_some(Self { app, window, input })
    }

    pub(super) fn restore(self) {
        let Some((app, window)) = front() else { return };
        let current = match app.attribute(c"AXFocusedUIElement") {
            Ok(None) => CurrentFocus::MissingOrWindow,
            Ok(Some(input)) if input.same(&self.input) => CurrentFocus::SameInput,
            Ok(Some(input)) if input.same(&window) => CurrentFocus::MissingOrWindow,
            Ok(Some(_)) => CurrentFocus::Other,
            Err(()) => CurrentFocus::Unavailable,
        };
        if !may_restore(app.same(&self.app), window.same(&self.window), current) {
            return;
        }
        let Some(focused) = OwnedCf::string(c"AXFocused") else {
            return;
        };
        // Reassert only this responder in the still-frontmost original window.
        // Never activate an app, raise a window, or manufacture a mouse click.
        // A stale/closed input simply returns an AX error; do not stop service.
        let result =
            unsafe { AXUIElementSetAttributeValue(self.input.0, focused.0, kCFBooleanTrue) };
        if result == 0 {
            println!("macOS input focus restored in the unchanged foreground window");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handback_reasserts_only_the_original_responder_in_the_same_window() {
        for current in [CurrentFocus::SameInput, CurrentFocus::MissingOrWindow] {
            assert!(may_restore(true, true, current));
            assert!(!may_restore(false, true, current));
            assert!(!may_restore(true, false, current));
        }
    }

    #[test]
    fn handback_never_steals_another_input_or_guesses_after_an_ax_failure() {
        for current in [CurrentFocus::Other, CurrentFocus::Unavailable] {
            assert!(!may_restore(true, true, current));
        }
    }
}
