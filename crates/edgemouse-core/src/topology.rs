use crate::{Edge, Point, Rect, Vector};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u128);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenId(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub id: ScreenId,
    pub node: NodeId,
    pub name: String,
    pub bounds: Rect,
    pub scale_factor: f64,
}

impl Screen {
    pub fn new(
        id: ScreenId,
        node: NodeId,
        name: impl Into<String>,
        bounds: Rect,
        scale_factor: f64,
    ) -> Result<Self, TopologyError> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err(TopologyError::InvalidScaleFactor);
        }
        Ok(Self {
            id,
            node,
            name: name.into(),
            bounds,
            scale_factor,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Portal {
    pub from: ScreenId,
    pub from_edge: Edge,
    pub to: ScreenId,
    pub to_edge: Edge,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transition {
    pub from: ScreenId,
    pub to: ScreenId,
    pub from_edge: Edge,
    pub to_edge: Edge,
    pub position: Point,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Advance {
    Stayed { screen: ScreenId, position: Point },
    Crossed(Transition),
}

#[derive(Debug, Clone)]
pub struct Topology {
    screens: BTreeMap<ScreenId, Screen>,
    displays: BTreeMap<ScreenId, Vec<Rect>>,
    portals: BTreeMap<(ScreenId, Edge), Portal>,
    edge_inset: f64,
}

impl Default for Topology {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Topology {
    #[must_use]
    pub fn new(edge_inset: f64) -> Self {
        Self {
            screens: BTreeMap::new(),
            displays: BTreeMap::new(),
            portals: BTreeMap::new(),
            edge_inset: edge_inset.max(0.001),
        }
    }

    pub fn add_screen(&mut self, screen: Screen) -> Result<(), TopologyError> {
        if self.screens.contains_key(&screen.id) {
            return Err(TopologyError::DuplicateScreen(screen.id));
        }
        self.screens.insert(screen.id, screen);
        Ok(())
    }

    pub fn connect_bidirectional(
        &mut self,
        from: ScreenId,
        from_edge: Edge,
        to: ScreenId,
    ) -> Result<(), TopologyError> {
        if from == to {
            return Err(TopologyError::SelfConnection(from));
        }
        self.require_screen(from)?;
        self.require_screen(to)?;

        let to_edge = from_edge.opposite();
        let forward_key = (from, from_edge);
        let reverse_key = (to, to_edge);
        if self.portals.contains_key(&forward_key) {
            return Err(TopologyError::OccupiedEdge(from, from_edge));
        }
        if self.portals.contains_key(&reverse_key) {
            return Err(TopologyError::OccupiedEdge(to, to_edge));
        }

        self.portals.insert(
            forward_key,
            Portal {
                from,
                from_edge,
                to,
                to_edge,
            },
        );
        self.portals.insert(
            reverse_key,
            Portal {
                from: to,
                from_edge: to_edge,
                to: from,
                to_edge: from_edge,
            },
        );
        Ok(())
    }

    /// Real monitor regions, in the same coordinate space as the desktop.
    /// A desktop bounding box may contain large holes beside a portrait monitor.
    pub fn set_displays(&mut self, id: ScreenId, displays: Vec<Rect>) -> Result<(), TopologyError> {
        let desktop = self.require_bounds(id)?;
        if displays.is_empty()
            || displays.iter().any(|rect| {
                Rect::new(rect.origin, rect.width, rect.height).is_err()
                    || rect.left() < desktop.left()
                    || rect.right() > desktop.right()
                    || rect.top() < desktop.top()
                    || rect.bottom() > desktop.bottom()
            })
        {
            return Err(TopologyError::InvalidDisplayGeometry);
        }
        if !displays.iter().any(|r| r.left() == desktop.left())
            || !displays.iter().any(|r| r.right() == desktop.right())
            || !displays.iter().any(|r| r.top() == desktop.top())
            || !displays.iter().any(|r| r.bottom() == desktop.bottom())
        {
            return Err(TopologyError::InvalidDisplayGeometry);
        }
        self.displays.insert(id, displays);
        Ok(())
    }

    pub fn clamp_pointer(&self, id: ScreenId, point: Point) -> Result<Point, TopologyError> {
        if !point.is_finite() {
            return Err(TopologyError::NonFiniteMovement);
        }
        let bounds = self.require_bounds(id)?;
        let Some(displays) = self.displays.get(&id) else {
            return Ok(bounds.clamp_inside(point, self.edge_inset));
        };
        // Do not leave the virtual pointer in a gap where the OS will silently
        // clamp the visible cursor to another monitor edge.
        Ok(displays
            .iter()
            .map(|rect| rect.clamp_inside(point, self.edge_inset))
            .min_by(|a, b| {
                let distance = |p: Point| (p.x - point.x).powi(2) + (p.y - point.y).powi(2);
                distance(*a).total_cmp(&distance(*b))
            })
            .expect("display list validated as nonempty"))
    }

    fn edge_segments(&self, id: ScreenId, edge: Edge) -> Result<Vec<(f64, f64)>, TopologyError> {
        let bounds = self.require_bounds(id)?;
        let fallback = [bounds];
        let displays = self
            .displays
            .get(&id)
            .map_or(fallback.as_slice(), Vec::as_slice);
        let mut segments: Vec<_> = displays
            .iter()
            .filter(|rect| match edge {
                Edge::Left => rect.left() == bounds.left(),
                Edge::Right => rect.right() == bounds.right(),
                Edge::Top => rect.top() == bounds.top(),
                Edge::Bottom => rect.bottom() == bounds.bottom(),
            })
            .map(|rect| {
                if edge.is_vertical() {
                    (rect.top(), rect.bottom())
                } else {
                    (rect.left(), rect.right())
                }
            })
            .collect();
        segments.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (start, end) in segments {
            if let Some(last) = merged.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        Ok(merged)
    }

    fn edge_fraction(&self, id: ScreenId, point: Point, edge: Edge) -> Result<f64, TopologyError> {
        let segments = self.edge_segments(id, edge)?;
        let coordinate = if edge.is_vertical() { point.y } else { point.x };
        let total: f64 = segments.iter().map(|(a, b)| b - a).sum();
        let offset: f64 = segments
            .iter()
            .map(|(a, b)| (coordinate - a).clamp(0.0, b - a))
            .sum();
        Ok(if total > 0.0 { offset / total } else { 0.5 })
    }

    fn entry_point(&self, id: ScreenId, edge: Edge, fraction: f64) -> Result<Point, TopologyError> {
        let bounds = self.require_bounds(id)?;
        let segments = self.edge_segments(id, edge)?;
        let total: f64 = segments.iter().map(|(a, b)| b - a).sum();
        let mut offset = fraction.clamp(0.0, 1.0) * total;
        for (start, end) in segments {
            if offset <= end - start {
                let coordinate = start + offset;
                let point = match edge {
                    Edge::Left => Point::new(bounds.left() + self.edge_inset, coordinate),
                    Edge::Right => Point::new(bounds.right() - self.edge_inset, coordinate),
                    Edge::Top => Point::new(coordinate, bounds.top() + self.edge_inset),
                    Edge::Bottom => Point::new(coordinate, bounds.bottom() - self.edge_inset),
                };
                return self.clamp_pointer(id, point);
            }
            offset -= end - start;
        }
        self.clamp_pointer(id, bounds.point_on_edge(edge, fraction, self.edge_inset))
    }

    #[must_use]
    pub fn screen(&self, id: ScreenId) -> Option<&Screen> {
        self.screens.get(&id)
    }

    pub(crate) fn require_bounds(&self, id: ScreenId) -> Result<Rect, TopologyError> {
        Ok(self.require_screen(id)?.bounds)
    }

    #[must_use]
    pub fn portal(&self, screen: ScreenId, edge: Edge) -> Option<Portal> {
        self.portals.get(&(screen, edge)).copied()
    }

    pub fn advance(
        &self,
        screen_id: ScreenId,
        from: Point,
        movement: Vector,
        blocked_edge: Option<Edge>,
    ) -> Result<Advance, TopologyError> {
        if !from.is_finite() || !movement.is_finite() {
            return Err(TopologyError::NonFiniteMovement);
        }
        let screen = self.require_screen(screen_id)?;
        if !screen.bounds.contains(from) {
            return Err(TopologyError::PointOutsideScreen(screen_id));
        }

        let requested = from + movement;
        let Some(exit) = screen.bounds.first_exit(from, requested) else {
            return Ok(Advance::Stayed {
                screen: screen_id,
                position: self.clamp_pointer(screen_id, requested)?,
            });
        };

        let coordinate = if exit.edge.is_vertical() {
            exit.point.y
        } else {
            exit.point.x
        };
        if !self
            .edge_segments(screen_id, exit.edge)?
            .iter()
            .any(|(a, b)| coordinate >= *a && coordinate <= *b)
        {
            return Ok(Advance::Stayed {
                screen: screen_id,
                position: self.clamp_pointer(screen_id, requested)?,
            });
        }

        if blocked_edge == Some(exit.edge) {
            return Ok(Advance::Stayed {
                screen: screen_id,
                position: self.clamp_pointer(screen_id, requested)?,
            });
        }

        let Some(portal) = self.portal(screen_id, exit.edge) else {
            return Ok(Advance::Stayed {
                screen: screen_id,
                position: self.clamp_pointer(screen_id, requested)?,
            });
        };
        let destination = self.require_screen(portal.to)?;
        let fraction = self.edge_fraction(screen_id, exit.point, exit.edge)?;
        let mut position = self.entry_point(portal.to, portal.to_edge, fraction)?;
        let mut remaining = movement.scaled(1.0 - exit.t);

        if exit.edge.is_vertical() {
            remaining.dy *= destination.bounds.height / screen.bounds.height;
        } else {
            remaining.dx *= destination.bounds.width / screen.bounds.width;
        }
        position = self.clamp_pointer(portal.to, position + remaining)?;

        Ok(Advance::Crossed(Transition {
            from: screen_id,
            to: portal.to,
            from_edge: exit.edge,
            to_edge: portal.to_edge,
            position,
        }))
    }

    fn require_screen(&self, id: ScreenId) -> Result<&Screen, TopologyError> {
        self.screens
            .get(&id)
            .ok_or(TopologyError::MissingScreen(id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TopologyError {
    MissingScreen(ScreenId),
    DuplicateScreen(ScreenId),
    SelfConnection(ScreenId),
    OccupiedEdge(ScreenId, Edge),
    InvalidScaleFactor,
    InvalidDisplayGeometry,
    PointOutsideScreen(ScreenId),
    NonFiniteMovement,
}

impl Display for TopologyError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingScreen(id) => write!(formatter, "missing screen {}", id.0),
            Self::DuplicateScreen(id) => write!(formatter, "duplicate screen {}", id.0),
            Self::SelfConnection(id) => {
                write!(formatter, "screen {} cannot connect to itself", id.0)
            }
            Self::OccupiedEdge(id, edge) => {
                write!(
                    formatter,
                    "screen {} edge {edge:?} already has a portal",
                    id.0
                )
            }
            Self::InvalidScaleFactor => {
                formatter.write_str("scale factor must be finite and positive")
            }
            Self::InvalidDisplayGeometry => {
                formatter.write_str("invalid physical display geometry")
            }
            Self::PointOutsideScreen(id) => {
                write!(formatter, "pointer is outside screen {}", id.0)
            }
            Self::NonFiniteMovement => formatter.write_str("mouse movement must be finite"),
        }
    }
}

impl Error for TopologyError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(id: u64, node: u128, width: f64, height: f64) -> Screen {
        Screen::new(
            ScreenId(id),
            NodeId(node),
            format!("screen-{id}"),
            Rect::new(Point::new(0.0, 0.0), width, height).unwrap(),
            1.0,
        )
        .unwrap()
    }

    #[test]
    fn maps_position_between_different_height_screens() {
        let mut topology = Topology::default();
        topology.add_screen(screen(1, 1, 100.0, 100.0)).unwrap();
        topology.add_screen(screen(2, 2, 200.0, 200.0)).unwrap();
        topology
            .connect_bidirectional(ScreenId(1), Edge::Right, ScreenId(2))
            .unwrap();

        let result = topology
            .advance(
                ScreenId(1),
                Point::new(99.0, 25.0),
                Vector::new(5.0, 0.0),
                None,
            )
            .unwrap();
        let Advance::Crossed(transition) = result else {
            panic!("expected a transition");
        };
        assert_eq!(transition.to, ScreenId(2));
        assert_eq!(transition.to_edge, Edge::Left);
        assert!((transition.position.y - 50.0).abs() < 0.001);
        assert!(transition.position.x > 1.0);
    }

    fn mixed_windows_mac() -> Topology {
        let mut topology = Topology::default();
        let windows = Rect::new(Point::new(-3840.0, 0.0), 6000.0, 3840.0).unwrap();
        let mac = Rect::new(Point::new(-253.0, -1080.0), 1920.0, 2036.0).unwrap();
        for (id, node, bounds) in [(1, 1, windows), (2, 2, mac)] {
            topology
                .add_screen(
                    Screen::new(ScreenId(id), NodeId(node), "desktop", bounds, 1.0).unwrap(),
                )
                .unwrap();
        }
        topology
            .set_displays(
                ScreenId(1),
                vec![
                    Rect::new(Point::new(-3840.0, 0.0), 3840.0, 2160.0).unwrap(),
                    Rect::new(Point::new(0.0, 0.0), 2160.0, 3840.0).unwrap(),
                ],
            )
            .unwrap();
        topology
            .set_displays(
                ScreenId(2),
                vec![
                    Rect::new(Point::new(-253.0, -1080.0), 1920.0, 1080.0).unwrap(),
                    Rect::new(Point::new(0.0, 0.0), 1470.0, 956.0).unwrap(),
                ],
            )
            .unwrap();
        topology
            .connect_bidirectional(ScreenId(2), Edge::Top, ScreenId(1))
            .unwrap();
        topology
    }

    #[test]
    fn diagnostic_handoff_lands_on_the_real_portrait_bottom_not_a_desktop_hole() {
        let topology = mixed_windows_mac();
        for x in [-252.0, 808.2, 1382.8, 1474.0, 1666.0] {
            let Advance::Crossed(enter) = topology
                .advance(
                    ScreenId(2),
                    Point::new(x, -1079.0),
                    Vector::new(0.0, -32.0),
                    None,
                )
                .unwrap()
            else {
                panic!("expected handoff")
            };
            assert!((1.0..2160.0).contains(&enter.position.x), "{enter:?}");
            assert_eq!(enter.position.y, 3808.0);
        }
    }

    #[test]
    fn remote_portrait_lower_half_stays_on_windows_until_the_actual_bottom() {
        let topology = mixed_windows_mac();
        for y in [1280.0, 1920.0, 2160.0, 3000.0, 3600.0] {
            assert_eq!(
                topology
                    .advance(
                        ScreenId(1),
                        Point::new(1000.0, y),
                        Vector::new(0.0, 100.0),
                        None
                    )
                    .unwrap(),
                Advance::Stayed {
                    screen: ScreenId(1),
                    position: Point::new(1000.0, y + 100.0)
                }
            );
        }
        assert!(matches!(
            topology
                .advance(
                    ScreenId(1),
                    Point::new(1000.0, 3839.0),
                    Vector::new(0.0, 4.0),
                    None
                )
                .unwrap(),
            Advance::Crossed(_)
        ));
    }

    #[test]
    fn movement_cannot_accumulate_in_the_empty_region_below_the_landscape() {
        let topology = mixed_windows_mac();
        assert!(matches!(
            topology
                .advance(
                    ScreenId(1),
                    Point::new(-2000.0, 2100.0),
                    Vector::new(0.0, 10000.0),
                    None
                )
                .unwrap(),
            Advance::Stayed { .. }
        ));
        let mut point = Point::new(-2000.0, 2159.0);
        for _ in 0..100 {
            let Advance::Stayed { position, .. } = topology
                .advance(ScreenId(1), point, Vector::new(0.0, 80.0), None)
                .unwrap()
            else {
                panic!("empty desktop space must not cross to Mac")
            };
            point = position;
        }
        assert_eq!(point, Point::new(-2000.0, 2159.0));
        let Advance::Stayed { position, .. } = topology
            .advance(ScreenId(1), point, Vector::new(2500.0, 0.0), None)
            .unwrap()
        else {
            panic!("monitor-to-monitor movement stays local")
        };
        assert_eq!(position, Point::new(500.0, 2159.0));
    }

    #[test]
    fn physical_edge_segments_merge_mirrors_and_skip_gaps_in_both_axes() {
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            let mut topology = Topology::default();
            topology.add_screen(screen(1, 1, 100.0, 100.0)).unwrap();
            let displays = if edge.is_vertical() {
                vec![
                    Rect::new(Point::new(0.0, 0.0), 100.0, 40.0).unwrap(),
                    Rect::new(Point::new(0.0, 60.0), 100.0, 40.0).unwrap(),
                ]
            } else {
                vec![
                    Rect::new(Point::new(0.0, 0.0), 40.0, 100.0).unwrap(),
                    Rect::new(Point::new(60.0, 0.0), 40.0, 100.0).unwrap(),
                ]
            };
            let mut mirrored = displays.clone();
            mirrored.push(displays[0]);
            topology.set_displays(ScreenId(1), mirrored).unwrap();
            for fraction in [0.1, 0.3, 0.7, 0.9] {
                let point = topology.entry_point(ScreenId(1), edge, fraction).unwrap();
                assert!(displays.iter().any(|r| r.contains(point)));
                assert!(
                    (topology.edge_fraction(ScreenId(1), point, edge).unwrap() - fraction).abs()
                        < 0.001
                );
            }
            assert!(topology.set_displays(ScreenId(1), vec![]).is_err());
            assert!(
                topology
                    .set_displays(
                        ScreenId(1),
                        vec![Rect::new(Point::new(-1.0, 0.0), 100.0, 100.0).unwrap()]
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn maps_a_landscape_edge_onto_a_rotated_portrait_screen() {
        let mut topology = Topology::default();
        topology.add_screen(screen(1, 1, 1920.0, 1080.0)).unwrap();
        topology.add_screen(screen(2, 2, 1080.0, 1920.0)).unwrap();
        topology
            .connect_bidirectional(ScreenId(1), Edge::Right, ScreenId(2))
            .unwrap();

        let Advance::Crossed(transition) = topology
            .advance(
                ScreenId(1),
                Point::new(1919.0, 270.0),
                Vector::new(4.0, 0.0),
                None,
            )
            .unwrap()
        else {
            panic!("expected a transition");
        };
        assert_eq!(transition.to, ScreenId(2));
        assert!((transition.position.y - 480.0).abs() < 0.01);
    }

    #[test]
    fn a_blocked_entry_edge_prevents_immediate_bounce() {
        let mut topology = Topology::default();
        topology.add_screen(screen(1, 1, 100.0, 100.0)).unwrap();
        topology.add_screen(screen(2, 2, 100.0, 100.0)).unwrap();
        topology
            .connect_bidirectional(ScreenId(1), Edge::Right, ScreenId(2))
            .unwrap();

        let result = topology
            .advance(
                ScreenId(2),
                Point::new(1.0, 50.0),
                Vector::new(-10.0, 0.0),
                Some(Edge::Left),
            )
            .unwrap();
        assert!(matches!(
            result,
            Advance::Stayed {
                screen: ScreenId(2),
                ..
            }
        ));
    }
}
