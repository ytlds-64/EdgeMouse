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
