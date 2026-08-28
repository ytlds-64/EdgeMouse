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

    #[must_use]
    pub fn screen(&self, id: ScreenId) -> Option<&Screen> {
        self.screens.get(&id)
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
                position: screen.bounds.clamp_inside(requested, self.edge_inset),
            });
        };

        if blocked_edge == Some(exit.edge) {
            return Ok(Advance::Stayed {
                screen: screen_id,
                position: screen.bounds.clamp_inside(requested, self.edge_inset),
            });
        }

        let Some(portal) = self.portal(screen_id, exit.edge) else {
            return Ok(Advance::Stayed {
                screen: screen_id,
                position: screen.bounds.clamp_inside(requested, self.edge_inset),
            });
        };
        let destination = self.require_screen(portal.to)?;
        let fraction = screen.bounds.edge_fraction(exit.point, exit.edge);
        let mut position =
            destination
                .bounds
                .point_on_edge(portal.to_edge, fraction, self.edge_inset);
        let mut remaining = movement.scaled(1.0 - exit.t);

        if exit.edge.is_vertical() {
            remaining.dy *= destination.bounds.height / screen.bounds.height;
        } else {
            remaining.dx *= destination.bounds.width / screen.bounds.width;
        }
        position = destination
            .bounds
            .clamp_inside(position + remaining, self.edge_inset);

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
