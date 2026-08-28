//! Length-prefixed, versioned wire messages for EdgeMouse.
//!
//! The transport is expected to provide confidentiality and peer identity.
//! This codec still treats every byte as untrusted and rejects oversized,
//! truncated, trailing, or non-finite values.

#![forbid(unsafe_code)]

use edgemouse_core::{
    ButtonState, KeyCode, KeyState, KeyboardEvent, MouseButton, NodeId, Point, RemoteMouseEvent,
    RoutedEvent, RoutedKeyboardEvent, ScreenId,
};
use std::error::Error;
use std::fmt::{Display, Formatter};

pub const PROTOCOL_VERSION: u16 = 3;
pub const HEADER_LEN: usize = 12;
pub const MAX_FRAME_LEN: usize = 64 * 1024;
pub const MOUSE_DATAGRAM_FRAME_LEN: usize = HEADER_LEN + 6 * std::mem::size_of::<u64>();
const MAGIC: [u8; 4] = *b"EMOU";

#[derive(Debug, Clone, PartialEq)]
pub enum WireMessage {
    Hello {
        node: NodeId,
        name: String,
        capabilities: u32,
    },
    Mouse {
        session_id: u64,
        event: RoutedEvent,
    },
    Keyboard {
        session_id: u64,
        event: RoutedKeyboardEvent,
    },
    /// An unreliable absolute movement update. `after_sequence` identifies the
    /// newest reliable mouse event that must be applied before this position.
    MouseDatagram {
        session_id: u64,
        after_sequence: u64,
        sequence: u64,
        screen: ScreenId,
        position: Point,
    },
    Heartbeat {
        session_id: u64,
        monotonic_ms: u64,
    },
    Goodbye {
        session_id: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    NameTooLong,
    NonFiniteNumber,
    FrameTooLarge,
}

impl Display for EncodeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameTooLong => formatter.write_str("peer name exceeds the protocol limit"),
            Self::NonFiniteNumber => formatter.write_str("wire numbers must be finite"),
            Self::FrameTooLarge => formatter.write_str("encoded frame exceeds the size limit"),
        }
    }
}

impl Error for EncodeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    BadMagic,
    UnsupportedVersion(u16),
    InvalidFlags(u8),
    InvalidTag(u8),
    InvalidLength,
    FrameTooLarge,
    TrailingBytes,
    InvalidUtf8,
    InvalidEnum,
    NonFiniteNumber,
}

impl Display for DecodeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("truncated frame"),
            Self::BadMagic => formatter.write_str("invalid protocol magic"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported protocol version {version}")
            }
            Self::InvalidFlags(flags) => write!(formatter, "unsupported frame flags {flags:#04x}"),
            Self::InvalidTag(tag) => write!(formatter, "unknown message tag {tag}"),
            Self::InvalidLength => formatter.write_str("frame length does not match its header"),
            Self::FrameTooLarge => formatter.write_str("frame exceeds the size limit"),
            Self::TrailingBytes => formatter.write_str("message payload has trailing bytes"),
            Self::InvalidUtf8 => formatter.write_str("peer name is not valid UTF-8"),
            Self::InvalidEnum => formatter.write_str("message contains an invalid enum value"),
            Self::NonFiniteNumber => formatter.write_str("message contains a non-finite number"),
        }
    }
}

impl Error for DecodeError {}

pub fn encode_frame(message: &WireMessage) -> Result<Vec<u8>, EncodeError> {
    let tag = tag_for(message);
    let mut payload = Vec::with_capacity(64);
    match message {
        WireMessage::Hello {
            node,
            name,
            capabilities,
        } => {
            let name = name.as_bytes();
            let name_len = u16::try_from(name.len()).map_err(|_| EncodeError::NameTooLong)?;
            put_u128(&mut payload, node.0);
            put_u32(&mut payload, *capabilities);
            put_u16(&mut payload, name_len);
            payload.extend_from_slice(name);
        }
        WireMessage::Mouse { session_id, event } => {
            put_u64(&mut payload, *session_id);
            put_u64(&mut payload, event.sequence);
            encode_mouse_payload(&mut payload, event.event)?;
        }
        WireMessage::Keyboard { session_id, event } => {
            put_u64(&mut payload, *session_id);
            put_u64(&mut payload, event.sequence);
            put_u16(&mut payload, event.event.key.usage());
            payload.push(match event.event.state {
                KeyState::Pressed => 1,
                KeyState::Released => 0,
            });
            payload.push(u8::from(event.event.repeat));
        }
        WireMessage::MouseDatagram {
            session_id,
            after_sequence,
            sequence,
            screen,
            position,
        } => {
            put_u64(&mut payload, *session_id);
            put_u64(&mut payload, *after_sequence);
            put_u64(&mut payload, *sequence);
            put_u64(&mut payload, screen.0);
            require_finite(position.x)?;
            require_finite(position.y)?;
            put_f64(&mut payload, position.x);
            put_f64(&mut payload, position.y);
        }
        WireMessage::Heartbeat {
            session_id,
            monotonic_ms,
        } => {
            put_u64(&mut payload, *session_id);
            put_u64(&mut payload, *monotonic_ms);
        }
        WireMessage::Goodbye { session_id } => put_u64(&mut payload, *session_id),
    }

    let payload_len = u32::try_from(payload.len()).map_err(|_| EncodeError::FrameTooLarge)?;
    if HEADER_LEN + payload.len() > MAX_FRAME_LEN {
        return Err(EncodeError::FrameTooLarge);
    }

    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&MAGIC);
    put_u16(&mut frame, PROTOCOL_VERSION);
    frame.push(tag);
    frame.push(0);
    put_u32(&mut frame, payload_len);
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_frame(frame: &[u8]) -> Result<WireMessage, DecodeError> {
    if frame.len() < HEADER_LEN {
        return Err(DecodeError::Truncated);
    }
    if frame.len() > MAX_FRAME_LEN {
        return Err(DecodeError::FrameTooLarge);
    }
    if frame[..4] != MAGIC {
        return Err(DecodeError::BadMagic);
    }

    let mut header = Reader::new(&frame[4..HEADER_LEN]);
    let version = header.u16()?;
    if version != PROTOCOL_VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let tag = header.u8()?;
    let flags = header.u8()?;
    if flags != 0 {
        return Err(DecodeError::InvalidFlags(flags));
    }
    let payload_len = usize::try_from(header.u32()?).map_err(|_| DecodeError::InvalidLength)?;
    if payload_len > MAX_FRAME_LEN - HEADER_LEN {
        return Err(DecodeError::FrameTooLarge);
    }
    if frame.len() != HEADER_LEN + payload_len {
        return Err(DecodeError::InvalidLength);
    }

    let mut payload = Reader::new(&frame[HEADER_LEN..]);
    let message = match tag {
        1 => decode_hello(&mut payload)?,
        2..=7 => decode_mouse(tag, &mut payload)?,
        8 => WireMessage::Heartbeat {
            session_id: payload.u64()?,
            monotonic_ms: payload.u64()?,
        },
        9 => WireMessage::Goodbye {
            session_id: payload.u64()?,
        },
        10 => decode_mouse_datagram(&mut payload)?,
        11 => decode_keyboard(&mut payload)?,
        other => return Err(DecodeError::InvalidTag(other)),
    };
    if !payload.is_empty() {
        return Err(DecodeError::TrailingBytes);
    }
    Ok(message)
}

/// Returns the complete frame size once a full header is available.
pub fn expected_frame_len(prefix: &[u8]) -> Result<Option<usize>, DecodeError> {
    if prefix.len() < HEADER_LEN {
        return Ok(None);
    }
    if prefix[..4] != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let payload_len = u32::from_be_bytes(prefix[8..12].try_into().unwrap());
    let payload_len = usize::try_from(payload_len).map_err(|_| DecodeError::InvalidLength)?;
    let total = HEADER_LEN
        .checked_add(payload_len)
        .ok_or(DecodeError::FrameTooLarge)?;
    if total > MAX_FRAME_LEN {
        return Err(DecodeError::FrameTooLarge);
    }
    Ok(Some(total))
}

fn tag_for(message: &WireMessage) -> u8 {
    match message {
        WireMessage::Hello { .. } => 1,
        WireMessage::Mouse { event, .. } => match event.event {
            RemoteMouseEvent::Enter { .. } => 2,
            RemoteMouseEvent::MoveAbsolute { .. } => 3,
            RemoteMouseEvent::Button { .. } => 4,
            RemoteMouseEvent::Wheel { .. } => 5,
            RemoteMouseEvent::Leave => 6,
            RemoteMouseEvent::ReleaseAll => 7,
        },
        WireMessage::MouseDatagram { .. } => 10,
        WireMessage::Keyboard { .. } => 11,
        WireMessage::Heartbeat { .. } => 8,
        WireMessage::Goodbye { .. } => 9,
    }
}

fn encode_mouse_payload(payload: &mut Vec<u8>, event: RemoteMouseEvent) -> Result<(), EncodeError> {
    match event {
        RemoteMouseEvent::Enter { screen, position }
        | RemoteMouseEvent::MoveAbsolute { screen, position } => {
            require_finite(position.x)?;
            require_finite(position.y)?;
            put_u64(payload, screen.0);
            put_f64(payload, position.x);
            put_f64(payload, position.y);
        }
        RemoteMouseEvent::Button { button, state } => {
            put_u16(payload, encode_button(button));
            payload.push(match state {
                ButtonState::Pressed => 1,
                ButtonState::Released => 0,
            });
        }
        RemoteMouseEvent::Wheel {
            horizontal,
            vertical,
        } => {
            require_finite(horizontal)?;
            require_finite(vertical)?;
            put_f64(payload, horizontal);
            put_f64(payload, vertical);
        }
        RemoteMouseEvent::Leave | RemoteMouseEvent::ReleaseAll => {}
    }
    Ok(())
}

fn decode_hello(payload: &mut Reader<'_>) -> Result<WireMessage, DecodeError> {
    let node = NodeId(payload.u128()?);
    let capabilities = payload.u32()?;
    let name_len = usize::from(payload.u16()?);
    let name = std::str::from_utf8(payload.take(name_len)?)
        .map_err(|_| DecodeError::InvalidUtf8)?
        .to_owned();
    Ok(WireMessage::Hello {
        node,
        name,
        capabilities,
    })
}

fn decode_mouse(tag: u8, payload: &mut Reader<'_>) -> Result<WireMessage, DecodeError> {
    let session_id = payload.u64()?;
    let sequence = payload.u64()?;
    let event = match tag {
        2 => RemoteMouseEvent::Enter {
            screen: ScreenId(payload.u64()?),
            position: payload.point()?,
        },
        3 => RemoteMouseEvent::MoveAbsolute {
            screen: ScreenId(payload.u64()?),
            position: payload.point()?,
        },
        4 => RemoteMouseEvent::Button {
            button: decode_button(payload.u16()?)?,
            state: match payload.u8()? {
                0 => ButtonState::Released,
                1 => ButtonState::Pressed,
                _ => return Err(DecodeError::InvalidEnum),
            },
        },
        5 => RemoteMouseEvent::Wheel {
            horizontal: payload.f64()?,
            vertical: payload.f64()?,
        },
        6 => RemoteMouseEvent::Leave,
        7 => RemoteMouseEvent::ReleaseAll,
        _ => return Err(DecodeError::InvalidTag(tag)),
    };
    Ok(WireMessage::Mouse {
        session_id,
        event: RoutedEvent { sequence, event },
    })
}

fn decode_mouse_datagram(payload: &mut Reader<'_>) -> Result<WireMessage, DecodeError> {
    let session_id = payload.u64()?;
    let after_sequence = payload.u64()?;
    let sequence = payload.u64()?;
    let screen = ScreenId(payload.u64()?);
    let position = payload.point()?;
    Ok(WireMessage::MouseDatagram {
        session_id,
        after_sequence,
        sequence,
        screen,
        position,
    })
}

fn decode_keyboard(payload: &mut Reader<'_>) -> Result<WireMessage, DecodeError> {
    let session_id = payload.u64()?;
    let sequence = payload.u64()?;
    let key = KeyCode::from_usage(payload.u16()?).ok_or(DecodeError::InvalidEnum)?;
    let state = match payload.u8()? {
        0 => KeyState::Released,
        1 => KeyState::Pressed,
        _ => return Err(DecodeError::InvalidEnum),
    };
    let repeat = match payload.u8()? {
        0 => false,
        1 => true,
        _ => return Err(DecodeError::InvalidEnum),
    };
    if state == KeyState::Released && repeat {
        return Err(DecodeError::InvalidEnum);
    }
    Ok(WireMessage::Keyboard {
        session_id,
        event: RoutedKeyboardEvent {
            sequence,
            event: KeyboardEvent { key, state, repeat },
        },
    })
}

fn encode_button(button: MouseButton) -> u16 {
    match button {
        MouseButton::Primary => 0,
        MouseButton::Secondary => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(value) => 0x100 | u16::from(value),
    }
}

fn decode_button(value: u16) -> Result<MouseButton, DecodeError> {
    match value {
        0 => Ok(MouseButton::Primary),
        1 => Ok(MouseButton::Secondary),
        2 => Ok(MouseButton::Middle),
        3 => Ok(MouseButton::Back),
        4 => Ok(MouseButton::Forward),
        other if other & 0xff00 == 0x0100 => Ok(MouseButton::Other(other.to_be_bytes()[1])),
        _ => Err(DecodeError::InvalidEnum),
    }
}

fn require_finite(value: f64) -> Result<(), EncodeError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(EncodeError::NonFiniteNumber)
    }
}

fn put_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_u128(output: &mut Vec<u8>, value: u128) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_f64(output: &mut Vec<u8>, value: f64) {
    put_u64(output, value.to_bits());
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn is_empty(&self) -> bool {
        self.position == self.bytes.len()
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .position
            .checked_add(len)
            .ok_or(DecodeError::Truncated)?;
        let result = self
            .bytes
            .get(self.position..end)
            .ok_or(DecodeError::Truncated)?;
        self.position = end;
        Ok(result)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn u128(&mut self) -> Result<u128, DecodeError> {
        Ok(u128::from_be_bytes(self.take(16)?.try_into().unwrap()))
    }

    fn f64(&mut self) -> Result<f64, DecodeError> {
        let value = f64::from_bits(self.u64()?);
        if value.is_finite() {
            Ok(value)
        } else {
            Err(DecodeError::NonFiniteNumber)
        }
    }

    fn point(&mut self) -> Result<Point, DecodeError> {
        Ok(Point::new(self.f64()?, self.f64()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(message: WireMessage) {
        let encoded = encode_frame(&message).unwrap();
        assert_eq!(expected_frame_len(&encoded).unwrap(), Some(encoded.len()));
        assert_eq!(decode_frame(&encoded).unwrap(), message);
    }

    #[test]
    fn round_trips_every_message_family() {
        round_trip(WireMessage::Hello {
            node: NodeId(0xfeed_beef),
            name: "MacBook Pro".to_owned(),
            capabilities: 0b111,
        });
        for event in [
            RemoteMouseEvent::Enter {
                screen: ScreenId(7),
                position: Point::new(1.5, 2.5),
            },
            RemoteMouseEvent::MoveAbsolute {
                screen: ScreenId(7),
                position: Point::new(100.0, 200.0),
            },
            RemoteMouseEvent::Button {
                button: MouseButton::Back,
                state: ButtonState::Pressed,
            },
            RemoteMouseEvent::Button {
                button: MouseButton::Other(42),
                state: ButtonState::Released,
            },
            RemoteMouseEvent::Wheel {
                horizontal: -2.0,
                vertical: 120.0,
            },
            RemoteMouseEvent::Leave,
            RemoteMouseEvent::ReleaseAll,
        ] {
            round_trip(WireMessage::Mouse {
                session_id: 99,
                event: RoutedEvent {
                    sequence: 123,
                    event,
                },
            });
        }
        round_trip(WireMessage::MouseDatagram {
            session_id: 99,
            after_sequence: 120,
            sequence: 123,
            screen: ScreenId(7),
            position: Point::new(100.25, 200.75),
        });
        round_trip(WireMessage::Keyboard {
            session_id: 99,
            event: RoutedKeyboardEvent {
                sequence: 124,
                event: KeyboardEvent {
                    key: KeyCode::LEFT_META,
                    state: KeyState::Pressed,
                    repeat: false,
                },
            },
        });
        round_trip(WireMessage::Heartbeat {
            session_id: 99,
            monotonic_ms: 1_000,
        });
        round_trip(WireMessage::Goodbye { session_id: 99 });
    }

    #[test]
    fn rejects_truncated_and_trailing_frames() {
        let encoded = encode_frame(&WireMessage::Goodbye { session_id: 9 }).unwrap();
        assert_eq!(decode_frame(&encoded[..10]), Err(DecodeError::Truncated));

        let mut trailing = encoded;
        trailing.push(0);
        assert_eq!(decode_frame(&trailing), Err(DecodeError::InvalidLength));
    }

    #[test]
    fn mouse_datagram_has_the_documented_fixed_size() {
        let encoded = encode_frame(&WireMessage::MouseDatagram {
            session_id: 99,
            after_sequence: 120,
            sequence: 123,
            screen: ScreenId(7),
            position: Point::new(100.25, 200.75),
        })
        .unwrap();
        assert_eq!(encoded.len(), MOUSE_DATAGRAM_FRAME_LEN);
    }

    #[test]
    fn rejects_non_finite_coordinates() {
        let message = WireMessage::Mouse {
            session_id: 1,
            event: RoutedEvent {
                sequence: 1,
                event: RemoteMouseEvent::MoveAbsolute {
                    screen: ScreenId(1),
                    position: Point::new(f64::NAN, 0.0),
                },
            },
        };
        assert_eq!(encode_frame(&message), Err(EncodeError::NonFiniteNumber));
    }

    #[test]
    fn rejects_noncanonical_button_codes() {
        let message = WireMessage::Mouse {
            session_id: 1,
            event: RoutedEvent {
                sequence: 1,
                event: RemoteMouseEvent::Button {
                    button: MouseButton::Back,
                    state: ButtonState::Pressed,
                },
            },
        };
        let mut frame = encode_frame(&message).unwrap();
        frame[28..30].copy_from_slice(&0x0200_u16.to_be_bytes());
        assert_eq!(decode_frame(&frame), Err(DecodeError::InvalidEnum));
    }

    #[test]
    fn reports_partial_headers_without_guessing() {
        assert_eq!(expected_frame_len(b"EMOU"), Ok(None));
    }
}
