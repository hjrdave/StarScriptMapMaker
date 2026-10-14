//! Land edits as polygon operations (ADR 0002): brush strokes become regions, and regions are
//! united with or cut out of stored outlines. geo does the work, on i_overlay underneath.
//!
//! Everything this module returns is a valid stored outline: parts never overlap, exteriors
//! are counter-clockwise and holes clockwise (positive and negative shoelace area).

use geo::orient::Direction;
use geo::{
    BooleanOps, Buffer, ChaikinSmoothing, Intersects, LineString, Orient, Simplify,
    buffer::BufferStyle,
};

use crate::command::Command;
use crate::geometry::{MultiPolygon, Point, Polygon};
use crate::id::Id;
use crate::map::Map;
use crate::node::{Node, ObjectKind};
use crate::roughness::RoughnessCache;

/// Chaikin passes over a stroke's centreline before it becomes a region. Each pass doubles
/// the points and rounds every corner.
const SMOOTHING_PASSES: usize = 2;

/// How far simplification may move a brush stroke's outline, in Canvas units: a small share of
/// the brush radius, but never finer than half a screen pixel, which nobody would see.
pub fn simplify_tolerance(radius: f64, canvas_units_per_pixel: f64) -> f64 {
    (radius * 0.04).max(canvas_units_per_pixel * 0.5)
}

/// The area a round brush of `radius` covers along `points`, with round caps and joins. A
/// single point gives a disc. This is the live preview of a stroke; [`brush_region`] is what
/// a finished stroke commits.
pub fn stroke_region(points: &[Point], radius: f64) -> MultiPolygon {
    if points.is_empty() || !(radius.is_finite() && radius > 0.0) {
        return MultiPolygon::default();
    }
    from_geo(&centreline(points).buffer_with_style(BufferStyle::new(radius)))
}

/// The region a finished brush stroke adds or erases: the centreline smoothed (Chaikin),
/// stroked with round caps and joins, and its outline simplified (Ramer–Douglas–Peucker)
/// to within `tolerance`, so stored outlines stay light and every node stays editable.
pub fn brush_region(points: &[Point], radius: f64, tolerance: f64) -> MultiPolygon {
    if points.is_empty() || !(radius.is_finite() && radius > 0.0) {
        return MultiPolygon::default();
    }
    let mut kept: Vec<Point> = Vec::with_capacity(points.len());
    for &point in points {
        // Points closer than the tolerance only add noise for the smoothing to round off.
        if kept
            .last()
            .is_none_or(|&last| distance(last, point) >= tolerance)
        {
            kept.push(point);
        }
    }
    if let (Some(&last), true) = (points.last(), kept.len() > 1) {
        *kept.last_mut().expect("not empty") = last;
    }
    let line = centreline(&kept).chaikin_smoothing(SMOOTHING_PASSES);
    let stroked = line.buffer_with_style(BufferStyle::new(radius));
    let simplified = stroked.simplify(tolerance.max(0.0));
    // Simplifying can make a ring cross itself; uniting it with nothing cleans that up.
    normalised(simplified.union(&geo::MultiPolygon::<f64>::new(Vec::new())))
}

/// One finished land brush stroke, ready to become a Command.
#[derive(Clone, PartialEq, Debug)]
pub struct BrushStroke {
    /// What the stroke covers, from [`brush_region`].
    pub region: MultiPolygon,
    /// Where the stroke began, in Canvas units.
    pub start: Point,
    /// Erasing (the modifier was held) rather than adding.
    pub erase: bool,
}

/// What a land brush stroke does, given the editor's Selection and active Layer:
///
/// - **Add** merges into the selected Landmass or Lake (the last selected, if several). With
///   none selected, it merges into the topmost Landmass under the stroke's start, or creates a
///   Landmass with id `new_id` in the active Layer if the stroke starts on water.
/// - **Erase** cuts the stroke out of the selected Lakes, or with none selected, out of every
///   Landmass directly in the active Layer that it touches.
///
/// Returns `None` when an erase touches nothing, and [`BrushError::NoLayer`] when a stroke
/// needs the active Layer and there is none. Locked targets are kept, so the Command fails and
/// says why.
pub fn brush_command(
    map: &Map,
    roughness: &mut RoughnessCache,
    selection: &[Id],
    active_layer: Option<Id>,
    stroke: BrushStroke,
    new_id: Id,
) -> Result<Option<Command>, BrushError> {
    let selected = |want: fn(&ObjectKind) -> bool| -> Vec<Id> {
        selection
            .iter()
            .copied()
            .filter(|&id| matches!(map.node(id), Some(Node::Object(o)) if want(&o.kind)))
            .collect()
    };
    let is_outline = |kind: &ObjectKind| kind.geometry().is_some();
    let is_lake = |kind: &ObjectKind| matches!(kind, ObjectKind::Lake(_));
    let is_landmass = |kind: &ObjectKind| matches!(kind, ObjectKind::Landmass(_));
    let layer = active_layer.filter(|&id| matches!(map.node(id), Some(Node::Layer(_))));
    let BrushStroke {
        region,
        start,
        erase,
    } = stroke;

    if erase {
        let lakes = selected(is_lake);
        let ids = if lakes.is_empty() {
            let layer = layer.ok_or(BrushError::NoLayer)?;
            map.children(Some(layer))
                .iter()
                .copied()
                .filter(|&id| match map.node(id) {
                    Some(Node::Object(o)) if is_landmass(&o.kind) => {
                        intersects(o.kind.geometry().expect("a Landmass"), &region)
                    }
                    _ => false,
                })
                .collect()
        } else {
            lakes
        };
        return Ok((!ids.is_empty()).then_some(Command::SubtractRegion { ids, region }));
    }

    if let Some(&id) = selected(is_outline).last() {
        return Ok(Some(Command::AddRegion { id, region }));
    }
    let under_start = map.draw_order().into_iter().rev().find(|&id| {
        matches!(map.node(id), Some(Node::Object(o)) if is_landmass(&o.kind))
            && roughness
                .outline(map, id)
                .is_some_and(|outline| outline.contains(start))
    });
    Ok(Some(match under_start {
        Some(id) => Command::AddRegion { id, region },
        None => Command::CreateLandmass {
            id: new_id,
            parent: layer.ok_or(BrushError::NoLayer)?,
            index: None,
            geometry: region,
            fill: None,
        },
    }))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum BrushError {
    #[error("choose a Layer to paint land into")]
    NoLayer,
}

/// `outline` with `region` added.
pub fn union(outline: &MultiPolygon, region: &MultiPolygon) -> MultiPolygon {
    normalised(to_geo(outline).union(&to_geo(region)))
}

/// `outline` with `region` cut out. Parts it splits stay parts of one outline.
pub fn difference(outline: &MultiPolygon, region: &MultiPolygon) -> MultiPolygon {
    normalised(to_geo(outline).difference(&to_geo(region)))
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

/// The signed shoelace area of an implicitly closed ring: positive when counter-clockwise.
pub fn signed_area(ring: &[Point]) -> f64 {
    let Some(&last) = ring.last() else {
        return 0.0;
    };
    let mut previous = last;
    let mut twice = 0.0;
    for &point in ring {
        twice += previous[0] * point[1] - point[0] * previous[1];
        previous = point;
    }
    twice / 2.0
}

fn centreline(points: &[Point]) -> LineString<f64> {
    LineString::from(points.to_vec())
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Exteriors counter-clockwise and holes clockwise, as stored outlines keep them.
fn normalised(region: geo::MultiPolygon<f64>) -> MultiPolygon {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn square(min: f64, max: f64) -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: vec![[min, min], [max, min], [max, max], [min, max]],
            holes: Vec::new(),
        }])
    }

    fn area(region: &MultiPolygon) -> f64 {
        region
            .0
            .iter()
            .flat_map(Polygon::rings)
            .map(signed_area)
            .sum()
    }

    fn assert_oriented(region: &MultiPolygon) {
        for polygon in &region.0 {
            assert!(
                signed_area(&polygon.exterior) > 0.0,
                "exterior is counter-clockwise"
            );
            for hole in &polygon.holes {
                assert!(signed_area(hole) < 0.0, "hole is clockwise");
            }
        }
    }

    #[test]
    fn a_click_is_a_disc_and_a_drag_a_capsule() {
        let disc = stroke_region(&[[50.0, 50.0]], 10.0);
        assert_eq!(disc.0.len(), 1);
        let expected = std::f64::consts::PI * 100.0;
        assert!((area(&disc) - expected).abs() < expected * 0.02);

        let capsule = stroke_region(&[[0.0, 0.0], [100.0, 0.0]], 10.0);
        let expected = 100.0 * 20.0 + std::f64::consts::PI * 100.0;
        assert!((area(&capsule) - expected).abs() < expected * 0.02);
        assert!(capsule.contains([-9.0, 0.0]) && capsule.contains([109.0, 0.0]));
        assert!(!capsule.contains([50.0, 11.0]));
    }

    #[test]
    fn a_committed_stroke_is_lighter_than_its_preview() {
        let points: Vec<Point> = (0..200)
            .map(|i| {
                let t = f64::from(i) / 10.0;
                [t * 20.0, t.sin() * 40.0]
            })
            .collect();
        let preview = stroke_region(&points, 15.0);
        let committed = brush_region(&points, 15.0, simplify_tolerance(15.0, 1.0));
        let count = |r: &MultiPolygon| r.0.iter().map(|p| p.exterior.len()).sum::<usize>();
        assert!(count(&committed) < count(&preview));
        assert!(committed.problem().is_none());
        assert_oriented(&committed);
        assert!((area(&committed) - area(&preview)).abs() < area(&preview) * 0.05);
    }

    #[test]
    fn union_and_difference_keep_outlines_valid() {
        let joined = union(&square(0.0, 10.0), &square(5.0, 15.0));
        assert_eq!(joined.0.len(), 1);
        assert!((area(&joined) - 175.0).abs() < 1e-9);
        assert_oriented(&joined);

        let holed = difference(&square(0.0, 10.0), &square(4.0, 6.0));
        assert_eq!(holed.0[0].holes.len(), 1);
        assert_oriented(&holed);

        // A cut across the middle leaves two parts of one outline.
        let band = MultiPolygon(vec![Polygon {
            exterior: vec![[4.0, -1.0], [6.0, -1.0], [6.0, 11.0], [4.0, 11.0]],
            holes: Vec::new(),
        }]);
        let split = difference(&square(0.0, 10.0), &band);
        assert_eq!(split.0.len(), 2);
        assert_oriented(&split);

        assert!(difference(&square(0.0, 10.0), &square(-1.0, 11.0)).is_empty());
    }

    #[test]
    fn clockwise_input_comes_out_counter_clockwise() {
        let mut clockwise = square(0.0, 10.0);
        clockwise.0[0].exterior.reverse();
        assert!(signed_area(&clockwise.0[0].exterior) < 0.0);
        assert_oriented(&union(&clockwise, &square(20.0, 30.0)));
    }

    #[test]
    fn intersects_needs_shared_area_not_just_shared_bounds() {
        let l_shape = MultiPolygon(vec![Polygon {
            exterior: vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 2.0],
                [2.0, 2.0],
                [2.0, 10.0],
                [0.0, 10.0],
            ],
            holes: Vec::new(),
        }]);
        assert!(intersects(&l_shape, &square(1.0, 3.0)));
        assert!(!intersects(&l_shape, &square(5.0, 9.0)));
        assert!(!intersects(&l_shape, &square(20.0, 30.0)));
    }
}
