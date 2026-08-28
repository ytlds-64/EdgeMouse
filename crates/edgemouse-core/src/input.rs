use crate::{NodeId, Point, ScreenId, Vector};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MouseButton {
    Primary,
    Secondary,
    Middle,
    Back,
    Forward,
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Pressed,
    Released,
}

/// A platform-neutral USB HID keyboard usage. Keeping the wire representation
/// physical makes shortcuts and punctuation independent from the sender's
/// active text input method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyCode(u16);

impl KeyCode {
    pub const A: Self = Self(0x04);
    pub const Z: Self = Self(0x1d);
    pub const DIGIT_1: Self = Self(0x1e);
    pub const DIGIT_0: Self = Self(0x27);
    pub const ENTER: Self = Self(0x28);
    pub const ESCAPE: Self = Self(0x29);
    pub const BACKSPACE: Self = Self(0x2a);
    pub const TAB: Self = Self(0x2b);
    pub const SPACE: Self = Self(0x2c);
    pub const MINUS: Self = Self(0x2d);
    pub const EQUAL: Self = Self(0x2e);
    pub const LEFT_BRACKET: Self = Self(0x2f);
    pub const RIGHT_BRACKET: Self = Self(0x30);
    pub const BACKSLASH: Self = Self(0x31);
    pub const SEMICOLON: Self = Self(0x33);
    pub const QUOTE: Self = Self(0x34);
    pub const BACKQUOTE: Self = Self(0x35);
    pub const COMMA: Self = Self(0x36);
    pub const PERIOD: Self = Self(0x37);
    pub const SLASH: Self = Self(0x38);
    pub const CAPS_LOCK: Self = Self(0x39);
    pub const F1: Self = Self(0x3a);
    pub const F12: Self = Self(0x45);
    pub const PRINT_SCREEN: Self = Self(0x46);
    pub const SCROLL_LOCK: Self = Self(0x47);
    pub const PAUSE: Self = Self(0x48);
    pub const INSERT: Self = Self(0x49);
    pub const HOME: Self = Self(0x4a);
    pub const PAGE_UP: Self = Self(0x4b);
    pub const DELETE: Self = Self(0x4c);
    pub const END: Self = Self(0x4d);
    pub const PAGE_DOWN: Self = Self(0x4e);
    pub const ARROW_RIGHT: Self = Self(0x4f);
    pub const ARROW_LEFT: Self = Self(0x50);
    pub const ARROW_DOWN: Self = Self(0x51);
    pub const ARROW_UP: Self = Self(0x52);
    pub const NUM_LOCK: Self = Self(0x53);
    pub const NUMPAD_DIVIDE: Self = Self(0x54);
    pub const NUMPAD_MULTIPLY: Self = Self(0x55);
    pub const NUMPAD_SUBTRACT: Self = Self(0x56);
    pub const NUMPAD_ADD: Self = Self(0x57);
    pub const NUMPAD_ENTER: Self = Self(0x58);
    pub const NUMPAD_1: Self = Self(0x59);
    pub const NUMPAD_9: Self = Self(0x61);
    pub const NUMPAD_0: Self = Self(0x62);
    pub const NUMPAD_DECIMAL: Self = Self(0x63);
    pub const APPLICATION: Self = Self(0x65);
    pub const NUMPAD_EQUAL: Self = Self(0x67);
    pub const F13: Self = Self(0x68);
    pub const F20: Self = Self(0x6f);
    pub const LEFT_CONTROL: Self = Self(0xe0);
    pub const LEFT_SHIFT: Self = Self(0xe1);
    pub const LEFT_ALT: Self = Self(0xe2);
    pub const LEFT_META: Self = Self(0xe3);
    pub const RIGHT_CONTROL: Self = Self(0xe4);
    pub const RIGHT_SHIFT: Self = Self(0xe5);
    pub const RIGHT_ALT: Self = Self(0xe6);
    pub const RIGHT_META: Self = Self(0xe7);

    #[must_use]
    pub const fn from_usage(usage: u16) -> Option<Self> {
        if matches!(usage, 0x04..=0x65 | 0x67..=0x6f | 0xe0..=0xe7) {
            Some(Self(usage))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn usage(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn is_modifier(self) -> bool {
        matches!(self.0, 0xe0..=0xe7)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardEvent {
    pub key: KeyCode,
    pub state: KeyState,
    pub repeat: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutedKeyboardEvent {
    pub sequence: u64,
    pub event: KeyboardEvent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PhysicalMouseEvent {
    Move {
        movement: Vector,
    },
    Button {
        button: MouseButton,
        state: ButtonState,
    },
    Wheel {
        horizontal: f64,
        vertical: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RemoteMouseEvent {
    Enter {
        screen: ScreenId,
        position: Point,
    },
    MoveAbsolute {
        screen: ScreenId,
        position: Point,
    },
    Button {
        button: MouseButton,
        state: ButtonState,
    },
    Wheel {
        horizontal: f64,
        vertical: f64,
    },
    Leave,
    ReleaseAll,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutedEvent {
    pub sequence: u64,
    pub event: RemoteMouseEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDisposition {
    PassThrough,
    Suppress,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    CapturePointer {
        anchor: Point,
    },
    ReleasePointer {
        screen: ScreenId,
        restore_position: Point,
    },
    Send {
        peer: NodeId,
        event: RoutedEvent,
    },
    SendKeyboard {
        peer: NodeId,
        event: RoutedKeyboardEvent,
    },
    PeerTimedOut {
        peer: NodeId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputResult {
    pub disposition: InputDisposition,
    pub effects: Vec<Effect>,
}

impl InputResult {
    #[must_use]
    pub fn pass_through() -> Self {
        Self {
            disposition: InputDisposition::PassThrough,
            effects: Vec::new(),
        }
    }

    #[must_use]
    pub fn suppress(effects: Vec<Effect>) -> Self {
        Self {
            disposition: InputDisposition::Suppress,
            effects,
        }
    }
}
