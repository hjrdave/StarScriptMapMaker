//! Land edits as polygon operations (ADR 0002): brush strokes become regions, and regions are
//! united with or cut out of stored outlines. geo does the work, on i_overlay underneath.
//!
//! Everything this module returns is a valid stored outline: parts never overlap, exteriors
//! are counter-clockwise and holes clockwise (positive and negative shoelace area).

use geo::bool_ops::FillRule;
use geo::orient::Direction;
use geo::{BooleanOps, Buffer, Intersects, LineString, Orient, Simplify, buffer::BufferStyle};

use crate::geometry::{MultiPolygon, Point, Polygon};

mod brush;
mod nodes;
#[cfg(test)]
mod tests;

pub use brush::{
    BrushError, BrushStroke, brush_command, brush_region, lasso_region, preview_area,
    simplify_tolerance, stroke_region, water_command,
};
pub use nodes::{NodeEditError, delete_nodes, insert_node, move_nodes, signed_area};

/// `outline` with `region` added.
pub fn union(outline: &MultiPolygon, region: &MultiPolygon) -> MultiPolygon {
    normalised(to_geo(outline).union(&to_geo(region)))
}

/// `outline` with `region` cut out. Parts it splits stay parts of one outline.
pub fn difference(outline: &MultiPolygon, region: &MultiPolygon) -> MultiPolygon {
    normalised(to_geo(outline).difference(&to_geo(region)))
}

/// How far [`offset`] may move a ring, as a share of the offset distance.
const OFFSET_TOLERANCE: f64 = 0.02;

/// The least [`offset`] may move a ring, in Canvas units: well under the width of a shore line.
const MIN_OFFSET_TOLERANCE: f64 = 0.25;

/// `outline` grown outward by `distance` Canvas units, or shrunk inward if `distance` is
/// negative: the rings that ripples and shading follow. Parts that shrink to nothing vanish.
///
/// Only drawing uses these rings, so they are approximate: `outline` is simplified first, by a
/// small share of `distance` (but at least [`MIN_OFFSET_TOLERANCE`]). Offsetting already rounds
/// away detail much finer than `distance`, and a rough coastline has so many vertices that
/// offsetting it exactly is far too slow to follow a slider.
pub fn offset(outline: &MultiPolygon, distance: f64) -> MultiPolygon {
    if !distance.is_finite() || distance == 0.0 {
        return outline.clone();
    }
    let tolerance = (distance.abs() * OFFSET_TOLERANCE).max(MIN_OFFSET_TOLERANCE);
    let simplified = to_geo(outline).simplify(tolerance);
    normalised(simplified.buffer_with_style(BufferStyle::new(distance)))
}

/// The parts of `lines` that lie outside `region`, as open lines. Where `region` has parts that
/// overlap, a point in any of them counts as inside, so it can be the water of several Lakes and
/// Rivers at once. A line that never meets `region` comes back whole.
pub fn lines_outside(lines: &[Vec<Point>], region: &MultiPolygon) -> Vec<Vec<Point>> {
    if lines.is_empty() || region.is_empty() {
        return lines.to_vec();
    }
    let lines =
        geo::MultiLineString::new(lines.iter().map(|l| LineString::from(l.clone())).collect());
    // Positive winding, because stored parts are all counter-clockwise: overlapping parts add
    // up instead of cancelling out as even-odd would.
    let outside = to_geo(region).clip_with_fill_rule(&lines, true, FillRule::Positive);
    outside
        .0
        .iter()
        .map(|line| line.coords().map(|c| [c.x, c.y]).collect::<Vec<Point>>())
        .filter(|line| line.len() > 1)
        .collect()
}

/// Whether the two regions share any point.
pub fn intersects(a: &MultiPolygon, b: &MultiPolygon) -> bool {
    match (a.bounds(), b.bounds()) {
        (Some(a_box), Some(b_box))
            if a_box.min[0] <= b_box.max[0]
                && b_box.min[0] <= a_box.max[0]
                && a_box.min[1] <= b_box.max[1]
                && b_box.min[1] <= a_box.max[1] =>
        {
            to_geo(a).intersects(&to_geo(b))
        }
        _ => false,
    }
}

/// Exteriors counter-clockwise and holes clockwise, as stored outlines keep them.
pub(crate) fn normalised(region: geo::MultiPolygon<f64>) -> MultiPolygon {
    from_geo(&region.orient(Direction::Default))
}

fn to_geo(region: &MultiPolygon) -> geo::MultiPolygon<f64> {
    let ring = |points: &[Point]| LineString::from(points.to_vec());
    geo::MultiPolygon::new(
        region
            .0
            .iter()
            .map(|polygon| {
                geo::Polygon::new(
                    ring(&polygon.exterior),
                    polygon.holes.iter().map(|hole| ring(hole)).collect(),
                )
            })
            .collect(),
    )
}

/// Back to stored rings, which are implicitly closed: geo repeats the first point at the end.
fn from_geo(region: &geo::MultiPolygon<f64>) -> MultiPolygon {
    let ring = |line: &LineString<f64>| {
        let mut points: Vec<Point> = line.coords().map(|c| [c.x, c.y]).collect();
        if points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        points
    };
    MultiPolygon(
        region
            .iter()
            .map(|polygon| Polygon {
                exterior: ring(polygon.exterior()),
                holes: polygon.interiors().iter().map(ring).collect(),
            })
            .filter(|polygon| polygon.exterior.len() >= 3)
            .collect(),
    )
}
