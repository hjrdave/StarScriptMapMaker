//! Outline geometry. Landmass and Lake outlines are `f64` polygon rings (ADR 0002).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A point in Canvas units, as `[x, y]`.
pub type Point = [f64; 2];

/// One part of a region: an outer boundary and optional holes. Rings are implicitly closed.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Polygon {
    pub exterior: Vec<Point>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<Point>>,
}

/// One stored vertex of an outline: `part` is the Polygon, `ring` is 0 for its exterior and
/// 1 and up for its holes, and `index` is the point within the ring.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
pub struct NodeRef {
    pub part: usize,
    pub ring: usize,
    pub index: usize,
}

/// A region made of zero or more Polygons.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct MultiPolygon(pub Vec<Polygon>);

/// An axis-aligned box in Canvas units.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Rect {
    pub fn width(&self) -> f64 {
        self.max[0] - self.min[0]
    }

    pub fn height(&self) -> f64 {
        self.max[1] - self.min[1]
    }

    /// The smallest box holding both.
    pub fn union(&self, other: &Rect) -> Rect {
        Rect {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }

    fn around(points: impl IntoIterator<Item = Point>) -> Option<Rect> {
        points.into_iter().fold(None, |rect: Option<Rect>, p| {
            let point = Rect { min: p, max: p };
            Some(rect.map_or(point, |r| r.union(&point)))
        })
    }
}

/// An affine transform `[a, b, c, d, e, f]`, mapping `(x, y)` to
/// `(a·x + c·y + e, b·x + d·y + f)` as in SVG.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(offset: [f64; 2]) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, offset[0], offset[1]])
    }

    /// Maps the box `from` onto the box `to`, as dragging a resize handle does. A zero-sized
    /// side of `from` keeps its scale, since it cannot be stretched.
    pub fn rect_to_rect(from: &Rect, to: &Rect) -> Affine {
        let scale = |from: f64, to: f64| if from == 0.0 { 1.0 } else { to / from };
        let sx = scale(from.width(), to.width());
        let sy = scale(from.height(), to.height());
        Affine([
            sx,
            0.0,
            0.0,
            sy,
            to.min[0] - from.min[0] * sx,
            to.min[1] - from.min[1] * sy,
        ])
    }

    pub fn apply(&self, [x, y]: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        [a * x + c * y + e, b * x + d * y + f]
    }

    pub fn determinant(&self) -> f64 {
        self.0[0] * self.0[3] - self.0[1] * self.0[2]
    }

    /// Finite and not collapsing shapes to a line or a point.
    pub fn is_invertible(&self) -> bool {
        self.0.iter().all(|v| v.is_finite()) && self.determinant() != 0.0
    }
}

impl Polygon {
    /// Every ring, exterior first.
    pub fn rings(&self) -> impl Iterator<Item = &[Point]> {
        std::iter::once(self.exterior.as_slice()).chain(self.holes.iter().map(Vec::as_slice))
    }

    /// The ring `ring` names: 0 is the exterior, 1 and up are holes.
    pub fn ring(&self, ring: usize) -> Option<&Vec<Point>> {
        match ring {
            0 => Some(&self.exterior),
            hole => self.holes.get(hole - 1),
        }
    }

    fn ring_mut(&mut self, ring: usize) -> Option<&mut Vec<Point>> {
        match ring {
            0 => Some(&mut self.exterior),
            hole => self.holes.get_mut(hole - 1),
        }
    }

    /// Inside the exterior and outside every hole.
    pub fn contains(&self, point: Point) -> bool {
        ring_contains(&self.exterior, point)
            && !self.holes.iter().any(|hole| ring_contains(hole, point))
    }
}

impl MultiPolygon {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The box around every exterior, or `None` when there are no points.
    pub fn bounds(&self) -> Option<Rect> {
        Rect::around(self.0.iter().flat_map(|p| p.exterior.iter().copied()))
    }

    /// Inside any part. A point in a hole is outside: the hole shows what lies beneath.
    pub fn contains(&self, point: Point) -> bool {
        self.0.iter().any(|polygon| polygon.contains(point))
    }

    /// The stored point a node names.
    pub fn node(&self, node: NodeRef) -> Option<Point> {
        self.0
            .get(node.part)?
            .ring(node.ring)?
            .get(node.index)
            .copied()
    }

    /// The ring a node (or edge) names, mutably.
    pub(crate) fn ring_mut(&mut self, part: usize, ring: usize) -> Option<&mut Vec<Point>> {
        self.0.get_mut(part)?.ring_mut(ring)
    }

    pub fn transformed(&self, transform: &Affine) -> MultiPolygon {
        let ring = |ring: &[Point]| ring.iter().map(|&p| transform.apply(p)).collect();
        MultiPolygon(
            self.0
                .iter()
                .map(|polygon| Polygon {
                    exterior: ring(&polygon.exterior),
                    holes: polygon.holes.iter().map(|hole| ring(hole)).collect(),
                })
                .collect(),
        )
    }

    /// Why this cannot be a stored outline, if it cannot: every part needs at least one
    /// ring, every ring at least three points, and every coordinate must be finite.
    pub fn problem(&self) -> Option<String> {
        if self.0.is_empty() {
            return Some("an outline needs at least one part".into());
        }
        for (part, polygon) in self.0.iter().enumerate() {
            for (ring, points) in polygon.rings().enumerate() {
                let name = match ring {
                    0 => format!("part {part}'s exterior"),
                    hole => format!("part {part}'s hole {}", hole - 1),
                };
                if points.len() < 3 {
                    return Some(format!("{name} has fewer than three points"));
                }
                if points.iter().flatten().any(|v| !v.is_finite()) {
                    return Some(format!("{name} has a coordinate that is not finite"));
                }
            }
        }
        None
    }
}

/// Even-odd ray casting against one implicitly closed ring.
fn ring_contains(ring: &[Point], [x, y]: Point) -> bool {
    let mut inside = false;
    let mut previous = match ring.last() {
        Some(&last) => last,
        None => return false,
    };
    for &current in ring {
        let ([x0, y0], [x1, y1]) = (previous, current);
        if (y0 > y) != (y1 > y) && x < x0 + (y - y0) / (y1 - y0) * (x1 - x0) {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(min: f64, max: f64) -> Vec<Point> {
        vec![[min, min], [max, min], [max, max], [min, max]]
    }

    #[test]
    fn holes_are_outside() {
        let island = MultiPolygon(vec![Polygon {
            exterior: square(0.0, 100.0),
            holes: vec![square(40.0, 60.0)],
        }]);
        assert!(island.contains([10.0, 10.0]));
        assert!(!island.contains([50.0, 50.0]));
        assert!(!island.contains([150.0, 50.0]));
        assert_eq!(
            island.bounds(),
            Some(Rect {
                min: [0.0, 0.0],
                max: [100.0, 100.0]
            })
        );
    }

    #[test]
    fn rect_to_rect_maps_corners() {
        let from = Rect {
            min: [10.0, 20.0],
            max: [30.0, 60.0],
        };
        let to = Rect {
            min: [0.0, 0.0],
            max: [10.0, 80.0],
        };
        let t = Affine::rect_to_rect(&from, &to);
        assert_eq!(t.apply(from.min), to.min);
        assert_eq!(t.apply(from.max), to.max);
        let flat = Rect {
            min: [0.0, 5.0],
            max: [10.0, 5.0],
        };
        assert!(Affine::rect_to_rect(&flat, &to).is_invertible());
    }

    #[test]
    fn outline_problems_are_named() {
        assert!(MultiPolygon::default().problem().is_some());
        let mut island = MultiPolygon(vec![Polygon {
            exterior: square(0.0, 1.0),
            holes: vec![vec![[0.0, 0.0], [1.0, 1.0]]],
        }]);
        assert_eq!(
            island.problem().as_deref(),
            Some("part 0's hole 0 has fewer than three points")
        );
        island.0[0].holes.clear();
        island.0[0].exterior[2][0] = f64::NAN;
        assert!(island.problem().unwrap().contains("not finite"));
    }
}
