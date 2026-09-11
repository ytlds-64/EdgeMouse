use super::{Portal, ScreenId, Transition};
use crate::{Edge, Point, Rect, Vector};

#[derive(Debug, Clone)]
struct Segment {
    display: Rect,
    start: f64,
    end: f64,
}

impl Segment {
    fn length(&self) -> f64 {
        self.end - self.start
    }

    fn coordinate(point: Point, edge: Edge) -> f64 {
        if edge.is_vertical() { point.y } else { point.x }
    }

    fn contains_coordinate(&self, point: Point, edge: Edge) -> bool {
        let coordinate = Self::coordinate(point, edge);
        coordinate >= self.start && coordinate < self.end
    }
}

/// Only exposed physical monitor edges participate. Internal monitor borders
/// stay under the operating system's control, including partly aligned screens.
fn exposed(displays: &[Rect], selected: &[Rect], edge: Edge) -> Vec<Segment> {
    let mut result = Vec::new();
    for display in displays.iter().filter(|rect| selected.contains(rect)) {
        if result
            .iter()
            .any(|segment: &Segment| segment.display == *display)
        {
            continue;
        }
        let range = |rect: Rect| {
            if edge.is_vertical() {
                (rect.top(), rect.bottom())
            } else {
                (rect.left(), rect.right())
            }
        };
        let mut ranges = vec![range(*display)];
        for other in displays {
            let covers_edge = match edge {
                Edge::Left => other.left() < display.left() && other.right() >= display.left(),
                Edge::Right => other.right() > display.right() && other.left() <= display.right(),
                Edge::Top => other.top() < display.top() && other.bottom() >= display.top(),
                Edge::Bottom => {
                    other.bottom() > display.bottom() && other.top() <= display.bottom()
                }
            };
            if !covers_edge {
                continue;
            }
            let (a, b) = range(*other);
            ranges = ranges
                .into_iter()
                .flat_map(|(start, end)| {
                    if a >= end || b <= start {
                        vec![(start, end)]
                    } else {
                        [(start, a.min(end)), (b.max(start), end)]
                            .into_iter()
                            .filter(|(start, end)| end > start)
                            .collect()
                    }
                })
                .collect();
        }
        result.extend(ranges.into_iter().map(|(start, end)| Segment {
            display: *display,
            start,
            end,
        }));
    }
    result.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.display.left().total_cmp(&b.display.left()))
            .then(a.display.top().total_cmp(&b.display.top()))
    });
    result
}

#[derive(Debug, Clone)]
struct Route {
    portal: Portal,
    source: Vec<Segment>,
    target: Vec<Segment>,
}

impl Route {
    fn usable(&self) -> bool {
        !self.source.is_empty() && !self.target.is_empty()
    }

    fn at_exit(&self, point: Point, movement: Vector) -> bool {
        if !self.usable() {
            return false;
        }
        let edge = self.portal.from_edge;
        let outward = match edge {
            Edge::Left => movement.dx < 0.0,
            Edge::Right => movement.dx > 0.0,
            Edge::Top => movement.dy < 0.0,
            Edge::Bottom => movement.dy > 0.0,
        };
        outward
            && self.source.iter().any(|segment| {
                segment.display.contains(point)
                    && segment.contains_coordinate(point, edge)
                    && segment.display.distance_from_edge(point, edge) <= 1.0
            })
    }

    fn advance(&self, point: Point, movement: Vector, inset: f64) -> Option<Transition> {
        if !self.usable() {
            return None;
        }
        let edge = self.portal.from_edge;
        let source_total: f64 = self.source.iter().map(Segment::length).sum();
        let target_total: f64 = self.target.iter().map(Segment::length).sum();
        let mut offset = 0.0;
        for source in &self.source {
            if source.display.contains(point)
                && let Some(exit) = source.display.first_exit(point, point + movement)
                && exit.edge == edge
                && source.contains_coordinate(exit.point, edge)
            {
                let fraction =
                    (offset + Segment::coordinate(exit.point, edge) - source.start) / source_total;
                let mut target_offset = fraction.clamp(0.0, 1.0) * target_total;
                for (index, target) in self.target.iter().enumerate() {
                    if target_offset < target.length() || index + 1 == self.target.len() {
                        let coordinate = target.start + target_offset.min(target.length());
                        let rect = target.display;
                        let entry = match self.portal.to_edge {
                            Edge::Left => Point::new(rect.left() + inset, coordinate),
                            Edge::Right => Point::new(rect.right() - inset, coordinate),
                            Edge::Top => Point::new(coordinate, rect.top() + inset),
                            Edge::Bottom => Point::new(coordinate, rect.bottom() - inset),
                        };
                        let mut remaining = movement.scaled(1.0 - exit.t);
                        if edge.is_vertical() {
                            remaining.dy *= target_total / source_total;
                        } else {
                            remaining.dx *= target_total / source_total;
                        }
                        return Some(Transition {
                            from: self.portal.from,
                            to: self.portal.to,
                            from_edge: edge,
                            to_edge: self.portal.to_edge,
                            position: rect.clamp_inside(entry + remaining, inset),
                        });
                    }
                    target_offset -= target.length();
                }
            }
            offset += source.length();
        }
        None
    }
}

#[derive(Debug, Clone)]
pub(super) struct DisplayLinks {
    routes: Vec<Route>,
}

impl DisplayLinks {
    pub(super) fn new(
        portal: Portal,
        from_displays: &[Rect],
        to_displays: &[Rect],
        from_selected: &[Rect],
        to_selected: &[Rect],
    ) -> Self {
        let source = exposed(from_displays, from_selected, portal.from_edge);
        let target = exposed(to_displays, to_selected, portal.to_edge);
        Self {
            routes: vec![
                Route {
                    portal,
                    source: source.clone(),
                    target: target.clone(),
                },
                Route {
                    portal: Portal {
                        from: portal.to,
                        to: portal.from,
                        from_edge: portal.to_edge,
                        to_edge: portal.from_edge,
                    },
                    source: target,
                    target: source,
                },
            ],
        }
    }

    pub(super) fn arranged(
        from: ScreenId,
        to: ScreenId,
        from_displays: &[Rect],
        to_displays: &[Rect],
        from_positions: &[(Rect, Rect)],
        to_positions: &[(Rect, Rect)],
    ) -> Self {
        let mut routes = Vec::new();
        for &(source, source_frame) in from_positions {
            for &(target, target_frame) in to_positions {
                for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
                    let touching = match edge {
                        Edge::Left => source_frame.left() - target_frame.right(),
                        Edge::Right => source_frame.right() - target_frame.left(),
                        Edge::Top => source_frame.top() - target_frame.bottom(),
                        Edge::Bottom => source_frame.bottom() - target_frame.top(),
                    }
                    .abs()
                        < 0.000001;
                    if !touching {
                        continue;
                    }
                    let vertical = edge.is_vertical();
                    let axis = |r: Rect| {
                        if vertical {
                            (r.top(), r.height)
                        } else {
                            (r.left(), r.width)
                        }
                    };
                    let virtual_range = |segment: &Segment, actual: Rect, frame: Rect| {
                        let (origin, size) = axis(actual);
                        let (start, length) = axis(frame);
                        (
                            start + (segment.start - origin) / size * length,
                            start + (segment.end - origin) / size * length,
                        )
                    };
                    let actual_segment = |actual: Rect, frame: Rect, start: f64, end: f64| {
                        let (origin, size) = axis(actual);
                        let (offset, length) = axis(frame);
                        Segment {
                            display: actual,
                            start: origin + (start - offset) / length * size,
                            end: origin + (end - offset) / length * size,
                        }
                    };
                    for a in exposed(from_displays, &[source], edge) {
                        for b in exposed(to_displays, &[target], edge.opposite()) {
                            let (a0, a1) = virtual_range(&a, source, source_frame);
                            let (b0, b1) = virtual_range(&b, target, target_frame);
                            let start = a0.max(b0);
                            let end = a1.min(b1);
                            if end - start <= 0.000001 {
                                continue;
                            }
                            let a = actual_segment(source, source_frame, start, end);
                            let b = actual_segment(target, target_frame, start, end);
                            routes.push(Route {
                                portal: Portal {
                                    from,
                                    to,
                                    from_edge: edge,
                                    to_edge: edge.opposite(),
                                },
                                source: vec![a.clone()],
                                target: vec![b.clone()],
                            });
                            routes.push(Route {
                                portal: Portal {
                                    from: to,
                                    to: from,
                                    from_edge: edge.opposite(),
                                    to_edge: edge,
                                },
                                source: vec![b],
                                target: vec![a],
                            });
                        }
                    }
                }
            }
        }
        Self { routes }
    }

    pub(super) fn portal(&self, screen: ScreenId, edge: Edge) -> Option<Portal> {
        self.routes
            .iter()
            .find(|route| {
                route.portal.from == screen && route.portal.from_edge == edge && route.usable()
            })
            .map(|route| route.portal)
    }

    pub(super) fn at_exit(&self, screen: ScreenId, point: Point, movement: Vector) -> bool {
        self.routes
            .iter()
            .any(|route| route.portal.from == screen && route.at_exit(point, movement))
    }

    pub(super) fn distance(&self, screen: ScreenId, point: Point, edge: Edge) -> f64 {
        self.routes
            .iter()
            .filter(|route| route.portal.from == screen && route.portal.from_edge == edge)
            .flat_map(|route| &route.source)
            .filter(|segment| segment.display.contains(point))
            .map(|segment| segment.display.distance_from_edge(point, edge))
            .min_by(f64::total_cmp)
            .unwrap_or(f64::INFINITY)
    }

    pub(super) fn advance(
        &self,
        screen: ScreenId,
        point: Point,
        movement: Vector,
        blocked: Option<Edge>,
        inset: f64,
    ) -> Option<Transition> {
        self.routes
            .iter()
            .filter(|route| route.portal.from == screen && blocked != Some(route.portal.from_edge))
            .find_map(|route| route.advance(point, movement, inset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Advance, ControlState, NodeId, PhysicalMouseEvent, Screen, Session, SessionConfig, Topology,
    };

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(Point::new(x, y), w, h).unwrap()
    }
    fn displays() -> (Rect, Rect, Rect) {
        (
            rect(0.0, 0.0, 2160.0, 3840.0),
            rect(-253.0, -1080.0, 1920.0, 1080.0),
            rect(0.0, 0.0, 1470.0, 956.0),
        )
    }
    fn topology(mac: &[Rect], edge: Edge) -> Topology {
        let (win, upper, lower) = displays();
        let mut t = Topology::default();
        for (id, bounds) in [(1, win), (2, rect(-253.0, -1080.0, 1920.0, 2036.0))] {
            t.add_screen(
                Screen::new(ScreenId(id), NodeId(id.into()), "desktop", bounds, 1.0).unwrap(),
            )
            .unwrap();
        }
        t.set_displays(ScreenId(1), vec![win]).unwrap();
        t.set_displays(ScreenId(2), vec![upper, lower]).unwrap();
        t.connect_displays_bidirectional(ScreenId(1), edge, ScreenId(2), &[win], mac)
            .unwrap();
        t
    }
    fn enter(t: &Topology, y: f64) -> Point {
        let Advance::Crossed(transition) = t
            .advance(
                ScreenId(1),
                Point::new(1.0, y),
                Vector::new(-4.0, 0.0),
                None,
            )
            .unwrap()
        else {
            panic!("expected crossing");
        };
        transition.position
    }

    #[test]
    fn only_upper_or_only_lower_routes_the_entire_windows_edge_to_that_monitor() {
        let (_, upper, lower) = displays();
        for chosen in [upper, lower] {
            let t = topology(&[chosen], Edge::Left);
            for y in [1.0, 960.0, 1920.0, 2880.0, 3839.0] {
                assert!(chosen.contains(enter(&t, y)));
            }
            let inactive = if chosen == upper { lower } else { upper };
            assert!(!t.can_cross_from(
                ScreenId(2),
                Point::new(inactive.right() - 1.0, inactive.top() + 100.0),
                Vector::new(5.0, 0.0)
            ));
        }
    }

    #[test]
    fn both_mac_monitors_share_windows_edge_in_spatial_order_with_roundtrip_mapping() {
        let (_, upper, lower) = displays();
        let t = topology(&[lower, upper], Edge::Left);
        for (y, expected) in [(960.0, upper), (2880.0, lower)] {
            let position = enter(&t, y);
            assert!(expected.contains(position), "{position:?}");
            let Advance::Crossed(back) = t
                .advance(ScreenId(2), position, Vector::new(10.0, 0.0), None)
                .unwrap()
            else {
                panic!("expected return");
            };
            assert!((back.position.y - y).abs() < 0.0001);
            assert_eq!(back.to, ScreenId(1));
        }
    }

    #[test]
    fn inset_lower_monitor_edge_uses_actual_cursor_and_keeps_entry_guard() {
        let (_, _, lower) = displays();
        let t = topology(&[lower], Edge::Left);
        let mut mac = Session::new(
            NodeId(2),
            t.clone(),
            ScreenId(2),
            Point::new(700.0, 300.0),
            SessionConfig::default(),
        )
        .unwrap();
        mac.handle_input(
            PhysicalMouseEvent::LocalMove {
                position: Point::new(700.0, 300.0),
                movement: Vector::new(5000.0, 0.0),
            },
            1,
        )
        .unwrap();
        assert_eq!(
            mac.state(),
            ControlState::Local,
            "large motion away from the real edge must stay local"
        );
        mac.handle_input(
            PhysicalMouseEvent::LocalMove {
                position: Point::new(1469.0, 300.0),
                movement: Vector::new(4.0, 0.0),
            },
            2,
        )
        .unwrap();
        assert_eq!(mac.state(), ControlState::Remote { peer: NodeId(1) });
        let mut win = Session::new(
            NodeId(1),
            t,
            ScreenId(1),
            Point::new(1.0, 2000.0),
            SessionConfig::default(),
        )
        .unwrap();
        win.handle_input(
            PhysicalMouseEvent::LocalMove {
                position: Point::new(1.0, 2000.0),
                movement: Vector::new(-4.0, 0.0),
            },
            1,
        )
        .unwrap();
        win.handle_input(
            PhysicalMouseEvent::Move {
                movement: Vector::new(10.0, 0.0),
            },
            2,
        )
        .unwrap();
        assert_eq!(
            win.state(),
            ControlState::Remote { peer: NodeId(2) },
            "inset edge must not immediately bounce back"
        );
        win.handle_input(
            PhysicalMouseEvent::Move {
                movement: Vector::new(-20.0, 0.0),
            },
            3,
        )
        .unwrap();
        win.handle_input(
            PhysicalMouseEvent::Move {
                movement: Vector::new(80.0, 0.0),
            },
            4,
        )
        .unwrap();
        assert_eq!(win.state(), ControlState::Local);
    }

    #[test]
    fn internal_monitor_border_stays_local_and_unplugged_selection_never_retargets() {
        let (_, upper, _) = displays();
        let t = topology(&[upper], Edge::Top);
        assert!(!t.can_cross_from(ScreenId(2), Point::new(700.0, -1.0), Vector::new(0.0, 5.0)));
        assert!(t.can_cross_from(ScreenId(2), Point::new(1500.0, -1.0), Vector::new(0.0, 5.0)));
        for selection in [vec![], vec![rect(3000.0, 0.0, 100.0, 100.0)]] {
            let t = topology(&selection, Edge::Left);
            assert!(matches!(
                t.advance(
                    ScreenId(1),
                    Point::new(1.0, 100.0),
                    Vector::new(-20.0, 0.0),
                    None
                )
                .unwrap(),
                Advance::Stayed { .. }
            ));
        }
    }
    #[test]
    fn all_four_directions_map_negative_origins_and_skip_covered_edge_sections() {
        let source = rect(100.0, -200.0, 400.0, 300.0);
        let target = rect(-700.0, 200.0, 800.0, 1000.0);
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            let links = DisplayLinks::new(
                Portal {
                    from: ScreenId(1),
                    to: ScreenId(2),
                    from_edge: edge,
                    to_edge: edge.opposite(),
                },
                &[source],
                &[target],
                &[source],
                &[target],
            );
            let movement = match edge {
                Edge::Left => Vector::new(-10.0, 0.0),
                Edge::Right => Vector::new(10.0, 0.0),
                Edge::Top => Vector::new(0.0, -10.0),
                Edge::Bottom => Vector::new(0.0, 10.0),
            };
            let start = source.point_on_edge(edge, 0.3, 1.0);
            assert!(links.at_exit(ScreenId(1), start, movement));
            let transition = links
                .advance(ScreenId(1), start, movement, None, 1.0)
                .unwrap();
            assert!(target.contains(transition.position));
            let expected = target.point_on_edge(edge.opposite(), 0.3, 1.0);
            let difference = if edge.is_vertical() {
                transition.position.y - expected.y
            } else {
                transition.position.x - expected.x
            };
            assert!(difference.abs() < 1e-9);
            assert!(
                links
                    .advance(ScreenId(1), start, movement, Some(edge), 1.0)
                    .is_none()
            );
        }
        let neighbor = rect(500.0, -50.0, 200.0, 150.0);
        let edges = exposed(&[source, neighbor], &[source], Edge::Right);
        assert_eq!(edges.len(), 1);
        assert_eq!((edges[0].start, edges[0].end), (-200.0, -50.0));
    }
    #[test]
    fn moving_virtual_windows_frame_selects_upper_lower_both_or_no_crossing() {
        let (win, upper, lower) = displays();
        let source_frames = Topology::automatic_display_positions(&[win], Edge::Left);
        let target_frames = Topology::automatic_display_positions(&[upper, lower], Edge::Right);
        let configured = |x: f64, y: f64| {
            let mut t = topology(&[upper, lower], Edge::Left);
            let mut frame = source_frames[0];
            frame.origin = Point::new(x, y);
            t.connect_arranged_displays_bidirectional(
                ScreenId(1),
                ScreenId(2),
                &[(win, frame)],
                &[(upper, target_frames[0]), (lower, target_frames[1])],
            )
            .unwrap();
            t
        };
        let both = configured(0.0, 0.0);
        assert!(upper.contains(enter(&both, 960.0)));
        assert!(lower.contains(enter(&both, 2880.0)));
        let only_upper = configured(0.0, -600.0);
        assert!(upper.contains(enter(&only_upper, 3800.0)));
        assert!(
            !only_upper.can_cross_from(
                ScreenId(1),
                Point::new(1.0, 1000.0),
                Vector::new(-10.0, 0.0)
            ),
            "non-overlapping Windows edge must remain local"
        );
        let only_lower = configured(0.0, 600.0);
        assert!(lower.contains(enter(&only_lower, 960.0)));
        assert!(!only_lower.can_cross_from(
            ScreenId(1),
            Point::new(1.0, 3000.0),
            Vector::new(-10.0, 0.0)
        ));
        let gap = configured(100.0, 0.0);
        assert!(!gap.can_cross_from(ScreenId(1), Point::new(1.0, 960.0), Vector::new(-10.0, 0.0)));
        let mut mixed = both.clone();
        let mut lower_right = target_frames[1];
        lower_right.origin = Point::new(source_frames[0].right(), 0.0);
        mixed
            .connect_arranged_displays_bidirectional(
                ScreenId(1),
                ScreenId(2),
                &[(win, source_frames[0])],
                &[(upper, target_frames[0]), (lower, lower_right)],
            )
            .unwrap();
        assert!(mixed.portal(ScreenId(1), Edge::Left).is_some());
        assert!(mixed.portal(ScreenId(1), Edge::Right).is_some());
        assert!(upper.contains(enter(&mixed, 960.0)));
        let Advance::Crossed(right) = mixed
            .advance(
                ScreenId(1),
                Point::new(2159.0, 960.0),
                Vector::new(4.0, 0.0),
                None,
            )
            .unwrap()
        else {
            panic!("right edge must lead to lower Mac monitor")
        };
        assert!(lower.contains(right.position));
        let mut t = both.clone();
        assert!(
            t.connect_arranged_displays_bidirectional(
                ScreenId(1),
                ScreenId(2),
                &[(win, target_frames[0])],
                &[(upper, target_frames[0])]
            )
            .is_err(),
            "overlapping virtual screens must be rejected"
        );
        for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
            let a = Topology::automatic_display_positions(&[win], edge);
            let b = Topology::automatic_display_positions(&[upper], edge.opposite());
            let mut t = topology(&[upper], edge);
            t.connect_arranged_displays_bidirectional(
                ScreenId(1),
                ScreenId(2),
                &[(win, a[0])],
                &[(upper, b[0])],
            )
            .unwrap();
            assert!(t.portal(ScreenId(1), edge).is_some());
        }
    }
}
