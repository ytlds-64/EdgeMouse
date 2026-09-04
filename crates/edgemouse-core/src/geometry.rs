use std::error::Error;
use std::fmt::{Display, Formatter};

/// A point in logical desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl std::ops::Add<Vector> for Point {
    type Output = Self;

    fn add(self, rhs: Vector) -> Self::Output {
        Self::new(self.x + rhs.dx, self.y + rhs.dy)
    }
}

impl std::ops::Sub for Point {
    type Output = Vector;

    fn sub(self, rhs: Self) -> Self::Output {
        Vector::new(self.x - rhs.x, self.y - rhs.y)
    }
}

/// A two-dimensional mouse movement in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector {
    pub dx: f64,
    pub dy: f64,
}

impl Vector {
    #[must_use]
    pub const fn new(dx: f64, dy: f64) -> Self {
        Self { dx, dy }
    }

    #[must_use]
    pub fn scaled(self, factor: f64) -> Self {
        Self::new(self.dx * factor, self.dy * factor)
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        self.dx.is_finite() && self.dy.is_finite()
    }
}

/// One side of a rectangular screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }

    #[must_use]
    pub const fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// A screen rectangle in logical desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub width: f64,
    pub height: f64,
}

/// Geometry for one physical display inside an operating-system desktop.
///
/// `bounds` uses the platform's global logical coordinate space, while
/// `pixel_width` and `pixel_height` retain the native mode resolution for UI
/// diagnostics. Keeping both lets Retina/DPI-scaled displays be drawn in the
/// correct relative position without losing their physical resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayGeometry {
    pub bounds: Rect,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale_factor: f64,
    pub primary: bool,
}

impl Rect {
    pub fn new(origin: Point, width: f64, height: f64) -> Result<Self, GeometryError> {
        if !origin.is_finite() || !width.is_finite() || !height.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        if width <= 0.0 || height <= 0.0 {
            return Err(GeometryError::NonPositiveSize);
        }
        Ok(Self {
            origin,
            width,
            height,
        })
    }

    #[must_use]
    pub fn left(self) -> f64 {
        self.origin.x
    }

    #[must_use]
    pub fn right(self) -> f64 {
        self.origin.x + self.width
    }

    #[must_use]
    pub fn top(self) -> f64 {
        self.origin.y
    }

    #[must_use]
    pub fn bottom(self) -> f64 {
        self.origin.y + self.height
    }

    #[must_use]
    pub fn contains(self, point: Point) -> bool {
        point.is_finite()
            && point.x >= self.left()
            && point.x < self.right()
            && point.y >= self.top()
            && point.y < self.bottom()
    }

    #[must_use]
    pub fn clamp_inside(self, point: Point, inset: f64) -> Point {
        let safe_inset = inset.max(0.0).min(self.width.min(self.height) / 2.0);
        Point::new(
            point
                .x
                .clamp(self.left() + safe_inset, self.right() - safe_inset),
            point
                .y
                .clamp(self.top() + safe_inset, self.bottom() - safe_inset),
        )
    }

    #[must_use]
    pub fn distance_from_edge(self, point: Point, edge: Edge) -> f64 {
        match edge {
            Edge::Left => point.x - self.left(),
            Edge::Right => self.right() - point.x,
            Edge::Top => point.y - self.top(),
            Edge::Bottom => self.bottom() - point.y,
        }
    }

    #[must_use]
    pub fn edge_fraction(self, point: Point, edge: Edge) -> f64 {
        let value = match edge {
            Edge::Left | Edge::Right => (point.y - self.top()) / self.height,
            Edge::Top | Edge::Bottom => (point.x - self.left()) / self.width,
        };
        value.clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn point_on_edge(self, edge: Edge, fraction: f64, inset: f64) -> Point {
        let fraction = fraction.clamp(0.0, 1.0);
        let inset = inset.max(0.0);
        match edge {
            Edge::Left => Point::new(self.left() + inset, self.top() + fraction * self.height),
            Edge::Right => Point::new(self.right() - inset, self.top() + fraction * self.height),
            Edge::Top => Point::new(self.left() + fraction * self.width, self.top() + inset),
            Edge::Bottom => Point::new(self.left() + fraction * self.width, self.bottom() - inset),
        }
    }

    /// Finds the first boundary crossed by a line segment that starts inside the rectangle.
    #[must_use]
    pub fn first_exit(self, from: Point, to: Point) -> Option<SegmentExit> {
        if !from.is_finite() || !to.is_finite() || !self.contains(from) || self.contains(to) {
            return None;
        }

        let delta = to - from;
        let mut best: Option<SegmentExit> = None;
        let mut consider = |edge: Edge, t: f64, point: Point| {
            if !(0.0..=1.0).contains(&t) {
                return;
            }
            let on_span = if edge.is_vertical() {
                point.y >= self.top() && point.y <= self.bottom()
            } else {
                point.x >= self.left() && point.x <= self.right()
            };
            if on_span && best.is_none_or(|current| t < current.t) {
                best = Some(SegmentExit { edge, t, point });
            }
        };

        if delta.dx < 0.0 && to.x < self.left() {
            let t = (self.left() - from.x) / delta.dx;
            consider(Edge::Left, t, from + delta.scaled(t));
        }
        if delta.dx > 0.0 && to.x >= self.right() {
            let t = (self.right() - from.x) / delta.dx;
            consider(Edge::Right, t, from + delta.scaled(t));
        }
        if delta.dy < 0.0 && to.y < self.top() {
            let t = (self.top() - from.y) / delta.dy;
            consider(Edge::Top, t, from + delta.scaled(t));
        }
        if delta.dy > 0.0 && to.y >= self.bottom() {
            let t = (self.bottom() - from.y) / delta.dy;
            consider(Edge::Bottom, t, from + delta.scaled(t));
        }

        best
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentExit {
    pub edge: Edge,
    pub t: f64,
    pub point: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryError {
    NonFinite,
    NonPositiveSize,
}

impl Display for GeometryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFinite => formatter.write_str("geometry values must be finite"),
            Self::NonPositiveSize => formatter.write_str("rectangle size must be positive"),
        }
    }
}

impl Error for GeometryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::new(Point::new(0.0, 0.0), 100.0, 50.0).unwrap()
    }

    #[test]
    fn detects_first_exit_for_diagonal_motion() {
        let exit = rect()
            .first_exit(Point::new(50.0, 25.0), Point::new(150.0, 100.0))
            .unwrap();
        assert_eq!(exit.edge, Edge::Bottom);
        assert!((exit.t - (1.0 / 3.0)).abs() < f64::EPSILON);
        assert!((exit.point.x - (250.0 / 3.0)).abs() < 0.001);
        assert_eq!(exit.point.y, 50.0);
    }

    #[test]
    fn clamps_to_an_interior_point() {
        assert_eq!(
            rect().clamp_inside(Point::new(200.0, -20.0), 1.0),
            Point::new(99.0, 1.0)
        );
    }
}
