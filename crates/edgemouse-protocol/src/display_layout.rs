//! Optional, bounded per-monitor layout extension. Rectangles identify the
//! selected OS monitor geometry; unavailable monitors never select a substitute.
use crate::{
    DecodeError, EncodeError, MAX_DISPLAY_COUNT, Reader, decode_edge, encode_edge, put_f64,
    put_u64, put_u128,
};
use edgemouse_core::{Edge, NodeId, Point, Rect};

pub const CAPABILITY_DISPLAY_LAYOUT: u32 = 1 << 5;

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayLayout {
    /// The side of Windows on which Mac is placed.
    pub mac_on: Edge,
    pub windows: Vec<Rect>,
    pub mac: Vec<Rect>,
    pub positions: Option<DisplayPositions>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayPositions {
    pub windows: Vec<Rect>,
    pub mac: Vec<Rect>,
}

impl DisplayLayout {
    pub fn valid(&self) -> bool {
        if let Some(positions) = &self.positions {
            if positions.windows.len() != self.windows.len()
                || positions.mac.len() != self.mac.len()
            {
                return false;
            }
            let all: Vec<_> = positions.windows.iter().chain(&positions.mac).collect();
            if all.iter().enumerate().any(|(index, r)| {
                !r.origin.is_finite()
                    || r.left().abs() > 1_000_000.0
                    || r.top().abs() > 1_000_000.0
                    || !(0.001..=65536.0).contains(&r.width)
                    || !(0.001..=65536.0).contains(&r.height)
                    || all[..index].iter().any(|other| {
                        r.left().max(other.left()) < r.right().min(other.right()) - 0.000001
                            && r.top().max(other.top()) < r.bottom().min(other.bottom()) - 0.000001
                    })
            }) {
                return false;
            }
        }
        [&self.windows, &self.mac].into_iter().all(|displays| {
            displays.len() <= MAX_DISPLAY_COUNT
                && displays.iter().enumerate().all(|(index, rect)| {
                    rect.origin.is_finite()
                        && rect.origin.x.abs() <= 1_000_000.0
                        && rect.origin.y.abs() <= 1_000_000.0
                        && (1.0..=65_536.0).contains(&rect.width)
                        && (1.0..=65_536.0).contains(&rect.height)
                        && !displays[..index].contains(rect)
                })
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayLayoutState {
    pub revision: u64,
    pub author: NodeId,
    /// None automatically selects all detected displays using the shared device direction.
    pub layout: Option<DisplayLayout>,
}

impl DisplayLayoutState {
    pub fn stamp(&self) -> (u64, NodeId) {
        (self.revision, self.author)
    }
}

pub(super) fn encode(payload: &mut Vec<u8>, state: &DisplayLayoutState) -> Result<(), EncodeError> {
    put_u64(payload, state.revision);
    put_u128(payload, state.author.0);
    let Some(layout) = &state.layout else {
        payload.push(0);
        return Ok(());
    };
    if !layout.valid() {
        return Err(EncodeError::InvalidDisplayLayout);
    }
    payload.push(1);
    payload.push(encode_edge(layout.mac_on));
    for displays in [&layout.windows, &layout.mac] {
        payload.push(displays.len() as u8);
        for rect in displays {
            for value in [rect.left(), rect.top(), rect.width, rect.height] {
                put_f64(payload, value);
            }
        }
    }
    if let Some(positions) = &layout.positions {
        payload.push(1);
        for rect in positions.windows.iter().chain(&positions.mac) {
            for value in [rect.left(), rect.top(), rect.width, rect.height] {
                put_f64(payload, value);
            }
        }
    } else {
        payload.push(0);
    }
    Ok(())
}

pub(super) fn decode(payload: &mut Reader<'_>) -> Result<DisplayLayoutState, DecodeError> {
    let revision = payload.u64()?;
    let author = NodeId(payload.u128()?);
    let layout = match payload.u8()? {
        0 => None,
        1 => {
            let mac_on = decode_edge(payload.u8()?)?;
            let mut read_displays = || -> Result<Vec<Rect>, DecodeError> {
                let count = usize::from(payload.u8()?);
                if count > MAX_DISPLAY_COUNT {
                    return Err(DecodeError::InvalidLength);
                }
                let mut displays = Vec::with_capacity(count);
                for _ in 0..count {
                    let origin = Point::new(payload.f64()?, payload.f64()?);
                    let rect = Rect::new(origin, payload.f64()?, payload.f64()?)
                        .map_err(|_| DecodeError::InvalidEnum)?;
                    displays.push(rect);
                }
                Ok(displays)
            };
            let mut layout = DisplayLayout {
                mac_on,
                windows: read_displays()?,
                mac: read_displays()?,
                positions: None,
            };
            layout.positions = match payload.u8()? {
                0 => None,
                1 => {
                    let mut read = |count| -> Result<Vec<Rect>, DecodeError> {
                        (0..count)
                            .map(|_| {
                                Rect::new(
                                    Point::new(payload.f64()?, payload.f64()?),
                                    payload.f64()?,
                                    payload.f64()?,
                                )
                                .map_err(|_| DecodeError::InvalidEnum)
                            })
                            .collect()
                    };
                    Some(DisplayPositions {
                        windows: read(layout.windows.len())?,
                        mac: read(layout.mac.len())?,
                    })
                }
                _ => return Err(DecodeError::InvalidEnum),
            };
            if !layout.valid() {
                return Err(DecodeError::InvalidEnum);
            }
            Some(layout)
        }
        _ => return Err(DecodeError::InvalidEnum),
    };
    Ok(DisplayLayoutState {
        revision,
        author,
        layout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WireMessage, decode_frame, encode_frame};

    #[test]
    fn custom_and_default_monitor_layouts_roundtrip_and_reject_bad_geometry() {
        let rect = Rect::new(Point::new(-253.0, -1080.0), 1920.0, 1080.0).unwrap();
        let layout = DisplayLayout {
            mac_on: Edge::Left,
            windows: vec![rect],
            mac: vec![rect],
            positions: Some(DisplayPositions {
                windows: vec![Rect::new(Point::new(0.0, 0.0), 100.0, 100.0).unwrap()],
                mac: vec![Rect::new(Point::new(-100.0, 0.0), 100.0, 100.0).unwrap()],
            }),
        };
        for layout in [None, Some(layout.clone())] {
            let message = WireMessage::DisplayLayoutUpdate {
                request_id: 42,
                state: DisplayLayoutState {
                    revision: 8,
                    author: NodeId(5),
                    layout,
                },
            };
            let encoded = encode_frame(&message).unwrap();
            assert_eq!(decode_frame(&encoded).unwrap(), message);
            for n in 0..encoded.len() {
                assert!(decode_frame(&encoded[..n]).is_err());
            }
        }
        let mut overlapping = layout.clone();
        let positions = overlapping.positions.as_mut().unwrap();
        positions.mac[0] = positions.windows[0];
        assert!(!overlapping.valid());
        let mut invalid = layout;
        invalid.mac.push(rect);
        assert!(!invalid.valid());
        invalid.mac = vec![Rect {
            width: f64::NAN,
            ..rect
        }];
        assert!(
            encode_frame(&WireMessage::DisplayLayoutUpdate {
                request_id: 1,
                state: DisplayLayoutState {
                    revision: 1,
                    author: NodeId(5),
                    layout: Some(invalid)
                }
            })
            .is_err()
        );
    }
}
