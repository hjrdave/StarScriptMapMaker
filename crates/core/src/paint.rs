//! Paint strokes: the colour a user paints over the Map with the brush (ADR 0017).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::color::Color;
use crate::geometry::Point;
use crate::id::Id;

/// The most points a committed paint stroke may have.
pub const MAX_STROKE_POINTS: usize = 5_000;
/// The most points a stroke may be given before it is simplified.
pub const MAX_RAW_STROKE_POINTS: usize = 100_000;
/// The most paint strokes a Map may have.
pub const MAX_PAINT_STROKES: usize = 20_000;
/// The largest brush size, in Canvas units.
pub const MAX_STROKE_SIZE: f64 = 2_000.0;
/// How far, in Canvas units, a committed stroke's path may stray from the path as drawn.
pub const STROKE_TOLERANCE: f64 = 0.5;

/// One press-drag-release of the paint brush, or of its eraser. Strokes belong to the Map, not
/// to a Landmass, and draw in order.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PaintStroke {
    /// The stroke's identity, which Commands refer to.
    pub id: Id,
    /// The path the brush followed, in Canvas units. Never empty.
    pub points: Vec<Point>,
    /// The paint colour. An eraser stroke ignores it.
    pub color: Color,
    /// The brush diameter, in Canvas units.
    pub size: f64,
    /// How soft the brush edge is, from 0 (hard) to 1 (soft all the way in).
    pub softness: f32,
    /// How strong the stroke is, from 0 to 1.
    pub opacity: f32,
    /// Whether the stroke removes paint where it passes instead of adding it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub erase: bool,
}

/// Why a paint stroke is not valid.
#[derive(Clone, PartialEq, Debug, Error)]
pub enum StrokeError {
    /// The stroke has no points.
    #[error("a paint stroke needs at least one point")]
    NoPoints,
    /// The stroke has more points than allowed.
    #[error("a paint stroke has {len} points; the most is {max}")]
    TooManyPoints {
        /// How many points it has.
        len: usize,
        /// The most it may have.
        max: usize,
    },
    /// A point is not a finite number.
    #[error("a paint stroke point is not finite")]
    NonFinitePoint,
    /// The size is not above 0 and at most [`MAX_STROKE_SIZE`].
    #[error("the brush size {0} must be above 0 and at most {MAX_STROKE_SIZE}")]
    Size(f64),
    /// The softness is not from 0 to 1.
    #[error("the brush softness {0} must be from 0 to 1")]
    Softness(f32),
    /// The opacity is not from 0 to 1.
    #[error("the brush opacity {0} must be from 0 to 1")]
    Opacity(f32),
}

impl PaintStroke {
    /// Checks every rule of a stored stroke.
    pub fn validate(&self) -> Result<(), StrokeError> {
        if self.points.is_empty() {
            return Err(StrokeError::NoPoints);
        }
        if self.points.len() > MAX_STROKE_POINTS {
            return Err(StrokeError::TooManyPoints {
                len: self.points.len(),
                max: MAX_STROKE_POINTS,
            });
        }
        self.validate_settings()?;
        if !self.points.iter().flatten().all(|v| v.is_finite()) {
            return Err(StrokeError::NonFinitePoint);
        }
        Ok(())
    }

    /// Checks the size, softness and opacity.
    pub(crate) fn validate_settings(&self) -> Result<(), StrokeError> {
        if !(self.size.is_finite() && self.size > 0.0 && self.size <= MAX_STROKE_SIZE) {
            return Err(StrokeError::Size(self.size));
        }
        if !(0.0..=1.0).contains(&self.softness) {
            return Err(StrokeError::Softness(self.softness));
        }
        if !(0.0..=1.0).contains(&self.opacity) {
            return Err(StrokeError::Opacity(self.opacity));
        }
        Ok(())
    }

    /// The stroke with its path simplified to within [`STROKE_TOLERANCE`], as committed.
    /// Fails for a stroke that is not valid, and for one with more than
    /// [`MAX_RAW_STROKE_POINTS`] points.
    pub fn committed(mut self) -> Result<Self, StrokeError> {
        if self.points.is_empty() {
            return Err(StrokeError::NoPoints);
        }
        if self.points.len() > MAX_RAW_STROKE_POINTS {
            return Err(StrokeError::TooManyPoints {
                len: self.points.len(),
                max: MAX_RAW_STROKE_POINTS,
            });
        }
        if !self.points.iter().flatten().all(|v| v.is_finite()) {
            return Err(StrokeError::NonFinitePoint);
        }
        self.points = simplify_path(&self.points, STROKE_TOLERANCE);
        self.validate()?;
        Ok(self)
    }
}

/// Simplifies a path with the Ramer-Douglas-Peucker method: the result keeps the first and last
/// point and a subset of the others, and no dropped point is further than `tolerance` from it.
pub fn simplify_path(points: &[Point], tolerance: f64) -> Vec<Point> {
    if points.len() < 3 || tolerance.is_nan() || tolerance < 0.0 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut spans = vec![(0, points.len() - 1)];
    while let Some((lo, hi)) = spans.pop() {
        let mut far = (0.0, None);
        for i in lo + 1..hi {
            let d = distance_to_segment(points[i], points[lo], points[hi]);
            if d > far.0 {
                far = (d, Some(i));
            }
        }
        if let (d, Some(i)) = far
            && d > tolerance
        {
            keep[i] = true;
            spans.push((lo, i));
            spans.push((i, hi));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(&p, keep)| keep.then_some(p))
        .collect()
}

/// Whether `eraser` would remove any paint: whether its path, widened to its brush, meets the
/// widened path of a paint stroke in `strokes`, all of which come before it. An eraser only
/// touches paint drawn before it (ADR 0017), so one that meets none changes nothing and can be
/// left out. Paint that an earlier eraser already removed still counts as met.
pub fn erases_any(strokes: &[PaintStroke], eraser: &PaintStroke) -> bool {
    let path = simplify_path(&eraser.points, STROKE_TOLERANCE);
    let Some(eraser_box) = bounds(&path) else {
        return false;
    };
    strokes.iter().filter(|stroke| !stroke.erase).any(|stroke| {
        let reach = (stroke.size + eraser.size) / 2.0;
        bounds(&stroke.points).is_some_and(|stroke_box| boxes_within(stroke_box, eraser_box, reach))
            && paths_within(&stroke.points, &path, reach)
    })
}

/// The box around `points`, as `[min_x, min_y, max_x, max_y]`; `None` with no points.
fn bounds(points: &[Point]) -> Option<[f64; 4]> {
    let first = points.first()?;
    Some(
        points
            .iter()
            .fold([first[0], first[1], first[0], first[1]], |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            }),
    )
}

/// Whether boxes `a` and `b` come within `reach` of each other.
fn boxes_within(a: [f64; 4], b: [f64; 4], reach: f64) -> bool {
    a[0] - reach <= b[2] && b[0] - reach <= a[2] && a[1] - reach <= b[3] && b[1] - reach <= a[3]
}

/// Whether the paths `a` and `b` (a single point counts as a path) come within `reach` of
/// each other anywhere.
fn paths_within(a: &[Point], b: &[Point], reach: f64) -> bool {
    let segments = |path: &[Point]| -> Vec<(Point, Point)> {
        match path {
            [only] => vec![(*only, *only)],
            _ => path.windows(2).map(|w| (w[0], w[1])).collect(),
        }
    };
    let (a, b) = (segments(a), segments(b));
    a.iter().any(|&(p, q)| {
        let a_box = [
            p[0].min(q[0]),
            p[1].min(q[1]),
            p[0].max(q[0]),
            p[1].max(q[1]),
        ];
        b.iter().any(|&(r, s)| {
            let b_box = [
                r[0].min(s[0]),
                r[1].min(s[1]),
                r[0].max(s[0]),
                r[1].max(s[1]),
            ];
            boxes_within(a_box, b_box, reach) && segment_distance(p, q, r, s) <= reach
        })
    })
}

/// The shortest distance between the segments `p`–`q` and `r`–`s`.
fn segment_distance(p: Point, q: Point, r: Point, s: Point) -> f64 {
    if segments_cross(p, q, r, s) {
        return 0.0;
    }
    distance_to_segment(p, r, s)
        .min(distance_to_segment(q, r, s))
        .min(distance_to_segment(r, p, q))
        .min(distance_to_segment(s, p, q))
}

/// Whether the segments `p`–`q` and `r`–`s` cross (touching ends are caught by the distances).
fn segments_cross(p: Point, q: Point, r: Point, s: Point) -> bool {
    let side = |a: Point, b: Point, c: Point| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let (d1, d2) = (side(r, s, p), side(r, s, q));
    let (d3, d4) = (side(p, q, r), side(p, q, s));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// How far `p` is from the segment `a` to `b`.
fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    };
    (p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_run_collapses_to_its_ends() {
        let path: Vec<Point> = (0..100).map(|i| [f64::from(i), 0.0]).collect();
        assert_eq!(simplify_path(&path, 0.5), vec![[0.0, 0.0], [99.0, 0.0]]);
    }

    #[test]
    fn a_corner_is_kept() {
        let mut path: Vec<Point> = (0..=10).map(|i| [f64::from(i), 0.0]).collect();
        path.extend((1..=10).map(|i| [10.0, f64::from(i)]));
        assert_eq!(
            simplify_path(&path, 0.1),
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]
        );
    }
}
