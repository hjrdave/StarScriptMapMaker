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
use crate::geometry::{MultiPolygon, NodeRef, Point, Polygon};
use crate::id::Id;
use crate::map::Map;
use crate::node::{Node, ObjectKind};
use crate::roughness::{RoughnessCache, roughen};

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

/// The region a lasso loop encloses: a freehand path simplified to within `tolerance`, or the
/// clicked corners as they are, with any self-crossing untangled (even-odd, so a figure eight
/// is two loops). Empty when the loop encloses no area.
pub fn lasso_region(points: &[Point], tolerance: f64) -> MultiPolygon {
    if points.len() < 3 {
        return MultiPolygon::default();
    }
    let ring = LineString::from(points.to_vec());
    let ring = if tolerance > 0.0 {
        ring.simplify(tolerance)
    } else {
        ring
    };
    if ring.0.len() < 3 {
        return MultiPolygon::default();
    }
    let polygon = geo::Polygon::new(ring, Vec::new());
    normalised(
        geo::MultiPolygon::new(vec![polygon]).union(&geo::MultiPolygon::<f64>::new(Vec::new())),
    )
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

/// What a water brush stroke does, given the editor's Selection:
///
/// - **Add** merges into the selected Lake (the last selected, if several). With none
///   selected, it merges into the topmost Lake under the stroke's start, or creates a Lake with
///   id `new_id` in `layer` if the stroke starts anywhere else. Narrow strokes make rivers and
///   wide ones lakes; both are Lakes, drawn above land.
/// - **Erase** cuts the stroke out of the selected Lakes, or with none selected, out of every
///   Lake it touches.
///
/// Returns `None` when an erase touches nothing, and [`BrushError::NoLayer`] when a stroke
/// needs a Layer and there is none. Locked targets are kept, so the Command fails and says why.
pub fn water_command(
    map: &Map,
    roughness: &mut RoughnessCache,
    selection: &[Id],
    layer: Option<Id>,
    stroke: BrushStroke,
    new_id: Id,
) -> Result<Option<Command>, BrushError> {
    let is_lake = |id: Id| matches!(map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Lake(_)));
    let BrushStroke {
        region,
        start,
        erase,
    } = stroke;

    if erase {
        let mut ids: Vec<Id> = selection
            .iter()
            .copied()
            .filter(|&id| is_lake(id))
            .collect();
        if ids.is_empty() {
            ids = map
                .draw_order()
                .into_iter()
                .filter(|&id| {
                    matches!(map.node(id), Some(Node::Object(o))
                        if matches!(o.kind, ObjectKind::Lake(_))
                            && intersects(o.kind.geometry().expect("a Lake"), &region))
                })
                .collect();
        }
        return Ok((!ids.is_empty()).then_some(Command::SubtractRegion { ids, region }));
    }

    let target = selection
        .iter()
        .rev()
        .copied()
        .find(|&id| is_lake(id))
        .or_else(|| {
            map.draw_order().into_iter().rev().find(|&id| {
                is_lake(id)
                    && roughness
                        .outline(map, id)
                        .is_some_and(|outline| outline.contains(start))
            })
        });
    Ok(Some(match target {
        Some(id) => Command::AddRegion { id, region },
        None => Command::CreateLake {
            id: new_id,
            parent: layer
                .filter(|&id| matches!(map.node(id), Some(Node::Layer(_))))
                .ok_or(BrushError::NoLayer)?,
            index: None,
            geometry: region,
            fill: None,
        },
    }))
}

/// The live preview of a brush stroke: the area, with its rough shoreline, that `command` (from
/// [`brush_command`] or [`water_command`]) adds to the Map (an add) or takes away (an erase).
/// It is made from the same unions and differences the Command runs, then roughened as a
/// render does, so the outline the user sees while painting is the one the stroke commits.
/// `None` for any other Command, or when nothing changes.
pub fn preview_area(
    map: &Map,
    roughness: &mut RoughnessCache,
    command: &Command,
) -> Option<MultiPolygon> {
    let area = match command {
        Command::CreateLandmass { geometry, .. } | Command::CreateLake { geometry, .. } => {
            roughen(geometry, &map.defaults.roughness)
        }
        Command::AddRegion { id, region } => {
            let geometry = outline_of(map, *id)?;
            let after = roughen(&union(geometry, region), map.roughness(*id)?);
            difference(&after, &*roughness.outline(map, *id)?)
        }
        Command::SubtractRegion { ids, region } => {
            let mut removed = MultiPolygon::default();
            for &id in ids {
                let Some(geometry) = outline_of(map, id) else {
                    continue;
                };
                if !intersects(geometry, region) {
                    continue;
                }
                let before = roughness.outline(map, id)?;
                let after = roughen(&difference(geometry, region), map.roughness(id)?);
                removed = union(&removed, &difference(&before, &after));
            }
            removed
        }
        _ => return None,
    };
    (!area.is_empty()).then_some(area)
}

fn outline_of(map: &Map, id: Id) -> Option<&MultiPolygon> {
    match map.node(id)? {
        Node::Object(o) => o.kind.geometry(),
        _ => None,
    }
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

/// `outline` grown outward by `distance` Canvas units, or shrunk inward if `distance` is
/// negative: the rings that ripples and shading follow. Parts that shrink to nothing vanish.
pub fn offset(outline: &MultiPolygon, distance: f64) -> MultiPolygon {
    if !distance.is_finite() || distance == 0.0 {
        return outline.clone();
    }
    normalised(to_geo(outline).buffer_with_style(BufferStyle::new(distance)))
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

/// Why a node edit could not be made.
#[derive(Clone, PartialEq, Debug, thiserror::Error)]
pub enum NodeEditError {
    #[error("the outline has no node at part {}, ring {}, point {}", .0.part, .0.ring, .0.index)]
    NoSuchNode(NodeRef),
    #[error("a ring needs at least three points")]
    TooFewPoints,
    #[error("the edit would leave nothing of the outline")]
    Collapsed,
}

/// `outline` with each of `nodes` moved by `offset`. A result that crosses itself is repaired.
pub fn move_nodes(
    outline: &MultiPolygon,
    nodes: &[NodeRef],
    offset: Point,
) -> Result<MultiPolygon, NodeEditError> {
    let mut edited = outline.clone();
    let mut moved = std::collections::HashSet::new();
    for &node in nodes {
        let point = edited
            .ring_mut(node.part, node.ring)
            .and_then(|ring| ring.get_mut(node.index))
            .ok_or(NodeEditError::NoSuchNode(node))?;
        // A node named twice still moves once.
        if moved.insert(node) {
            *point = [point[0] + offset[0], point[1] + offset[1]];
        }
    }
    repaired(edited)
}

/// `outline` with a node added at `point` on the edge that starts at `edge` and runs to the
/// next point of its ring (the last point's edge closes the ring).
pub fn insert_node(
    outline: &MultiPolygon,
    edge: NodeRef,
    point: Point,
) -> Result<MultiPolygon, NodeEditError> {
    let mut edited = outline.clone();
    let ring = edited
        .ring_mut(edge.part, edge.ring)
        .filter(|ring| edge.index < ring.len())
        .ok_or(NodeEditError::NoSuchNode(edge))?;
    ring.insert(edge.index + 1, point);
    repaired(edited)
}

/// `outline` without `nodes`. Refuses to leave a ring with fewer than three points.
pub fn delete_nodes(
    outline: &MultiPolygon,
    nodes: &[NodeRef],
) -> Result<MultiPolygon, NodeEditError> {
    let mut doomed = nodes.to_vec();
    doomed.sort_by_key(|n| std::cmp::Reverse((n.part, n.ring, n.index)));
    doomed.dedup();
    let mut edited = outline.clone();
    for node in doomed {
        let ring = edited
            .ring_mut(node.part, node.ring)
            .filter(|ring| node.index < ring.len())
            .ok_or(NodeEditError::NoSuchNode(node))?;
        ring.remove(node.index);
        if ring.len() < 3 {
            return Err(NodeEditError::TooFewPoints);
        }
    }
    repaired(edited)
}

/// A hand-edited outline made valid. One that is already valid is returned as it is, so node
/// indices stay put; one that crosses itself is cleaned up by i_overlay, which can renumber
/// its nodes.
fn repaired(outline: MultiPolygon) -> Result<MultiPolygon, NodeEditError> {
    if outline.problem().is_some() {
        return Err(NodeEditError::TooFewPoints);
    }
    if is_clean(&outline) {
        return Ok(outline);
    }
    let cleaned = normalised(to_geo(&outline).union(&geo::MultiPolygon::<f64>::new(Vec::new())));
    if cleaned.is_empty() {
        Err(NodeEditError::Collapsed)
    } else {
        Ok(cleaned)
    }
}

/// Whether an outline is already a valid stored one: oriented, no edge touching another
/// except where neighbours meet, and no part or hole sitting inside another by drifting there
/// whole. Segments go in an R-tree, so this is near-linear on coastlines (geo's own
/// validation is quadratic and takes seconds at 50k vertices).
fn is_clean(outline: &MultiPolygon) -> bool {
    use geo::algorithm::line_intersection::line_intersection;
    use rstar::primitives::{GeomWithData, Rectangle};

    let oriented = outline.0.iter().all(|polygon| {
        signed_area(&polygon.exterior) > 0.0 && polygon.holes.iter().all(|h| signed_area(h) < 0.0)
    });
    if !oriented {
        return false;
    }
    // (ring number across the whole outline, index of the segment within its ring, ring length)
    type Segment = GeomWithData<Rectangle<Point>, (usize, usize, usize)>;
    let mut segments: Vec<Segment> = Vec::new();
    let mut rings: Vec<&[Point]> = Vec::new();
    for ring in outline.0.iter().flat_map(Polygon::rings) {
        let number = rings.len();
        rings.push(ring);
        for (i, &a) in ring.iter().enumerate() {
            let b = ring[(i + 1) % ring.len()];
            segments.push(GeomWithData::new(
                Rectangle::from_corners(
                    [a[0].min(b[0]), a[1].min(b[1])],
                    [a[0].max(b[0]), a[1].max(b[1])],
                ),
                (number, i, ring.len()),
            ));
        }
    }
    let line = |(ring, i, len): (usize, usize, usize)| {
        let (a, b) = (rings[ring][i], rings[ring][(i + 1) % len]);
        geo::Line::new(
            geo::coord! {x: a[0], y: a[1]},
            geo::coord! {x: b[0], y: b[1]},
        )
    };
    let tree = rstar::RTree::bulk_load(segments);
    for entry in tree.iter() {
        let (ring, i, len) = entry.data;
        let envelope = rstar::AABB::from_corners(entry.geom().lower(), entry.geom().upper());
        for other in tree.locate_in_envelope_intersecting(&envelope) {
            let (other_ring, j, _) = other.data;
            if (other_ring, j) <= (ring, i) {
                continue;
            }
            let neighbours = other_ring == ring
                && (j == (i + 1) % len || i == (j + 1) % len || (len == 3 && i != j));
            if neighbours {
                // Neighbours share an end; they must not overlap along a line as well.
                if let Some(geo::algorithm::line_intersection::LineIntersection::Collinear {
                    ..
                }) = line_intersection(line(entry.data), line(other.data))
                {
                    return false;
                }
            } else if line_intersection(line(entry.data), line(other.data)).is_some() {
                return false;
            }
        }
    }
    // No edges meet, so a ring is wholly inside or outside another: check one vertex each.
    for (number, ring) in rings.iter().enumerate() {
        let probe = ring[0];
        let mut ring_number = 0;
        for polygon in &outline.0 {
            let first = ring_number;
            ring_number += 1 + polygon.holes.len();
            let is_hole = number > first && number < ring_number;
            let own = (first..ring_number).contains(&number);
            if own && number == first {
                continue; // the exterior against its own polygon
            }
            if own && is_hole {
                // A hole belongs inside its own exterior.
                if !ring_contains_point(&polygon.exterior, probe) {
                    return false;
                }
            } else if !own && polygon.contains(probe) {
                // A ring inside another part's fill: overlapping parts.
                return false;
            }
        }
    }
    true
}

fn ring_contains_point(ring: &[Point], point: Point) -> bool {
    Polygon {
        exterior: ring.to_vec(),
        holes: Vec::new(),
    }
    .contains(point)
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
    use geo::Validation;

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

    fn node(part: usize, ring: usize, index: usize) -> NodeRef {
        NodeRef { part, ring, index }
    }

    #[test]
    fn moving_nodes_keeps_indices_when_the_outline_stays_valid() {
        let moved = move_nodes(&square(0.0, 10.0), &[node(0, 0, 2)], [5.0, 5.0]).unwrap();
        assert_eq!(moved.0[0].exterior[2], [15.0, 15.0]);
        assert_eq!(moved.0[0].exterior.len(), 4);
        // Naming a node twice moves it once.
        let moved = move_nodes(&square(0.0, 10.0), &[node(0, 0, 2); 2], [5.0, 0.0]).unwrap();
        assert_eq!(moved.0[0].exterior[2], [15.0, 10.0]);
        assert_eq!(
            move_nodes(&square(0.0, 10.0), &[node(0, 0, 9)], [1.0, 1.0]),
            Err(NodeEditError::NoSuchNode(node(0, 0, 9)))
        );
    }

    #[test]
    fn dragging_a_node_across_the_outline_is_repaired() {
        // Dragging a corner past the opposite one ties the square into a bow-tie.
        let bowtie = move_nodes(&square(0.0, 10.0), &[node(0, 0, 2)], [-20.0, 0.0]).unwrap();
        assert!(to_geo(&bowtie).is_valid());
        assert_oriented(&bowtie);
    }

    #[test]
    fn insert_adds_a_node_after_the_edge_start_and_the_last_edge_wraps() {
        let inserted = insert_node(&square(0.0, 10.0), node(0, 0, 0), [5.0, -3.0]).unwrap();
        assert_eq!(inserted.0[0].exterior[1], [5.0, -3.0]);
        assert_eq!(inserted.0[0].exterior.len(), 5);
        let closing = insert_node(&square(0.0, 10.0), node(0, 0, 3), [-2.0, 5.0]).unwrap();
        assert_eq!(closing.0[0].exterior[4], [-2.0, 5.0]);
        assert_oriented(&closing);
    }

    #[test]
    fn delete_refuses_to_leave_a_ring_with_fewer_than_three_points() {
        let pentagon = MultiPolygon(vec![Polygon {
            exterior: vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [12.0, 6.0],
                [5.0, 10.0],
                [-2.0, 6.0],
            ],
            holes: Vec::new(),
        }]);
        let fewer =
            delete_nodes(&pentagon, &[node(0, 0, 1), node(0, 0, 3), node(0, 0, 3)]).unwrap();
        assert_eq!(fewer.0[0].exterior.len(), 3);
        assert_eq!(
            delete_nodes(&pentagon, &[node(0, 0, 0), node(0, 0, 1), node(0, 0, 2)]),
            Err(NodeEditError::TooFewPoints)
        );
        assert_eq!(
            delete_nodes(&square(0.0, 10.0), &[node(0, 0, 0), node(0, 0, 2)]),
            Err(NodeEditError::TooFewPoints)
        );
    }

    #[test]
    fn a_big_coastline_stays_quick_to_edit() {
        let n = 50_000;
        let ring: Vec<Point> = (0..n)
            .map(|i| {
                let t = f64::from(i) / f64::from(n) * std::f64::consts::TAU;
                let r = 10_000.0 + 400.0 * (t * 37.0).sin();
                [r * t.cos(), r * t.sin()]
            })
            .collect();
        let coast = MultiPolygon(vec![Polygon {
            exterior: ring,
            holes: Vec::new(),
        }]);
        let started = std::time::Instant::now();
        let edited = move_nodes(&coast, &[node(0, 0, 25_000)], [30.0, 0.0]).unwrap();
        assert_eq!(edited.0[0].exterior.len(), 50_000);
        let elapsed = started.elapsed();
        // Generous for debug builds; a drag runs this every frame.
        assert!(elapsed.as_millis() < 1500, "took {elapsed:?}");
    }

    #[test]
    fn offset_grows_and_shrinks_outlines() {
        let grown = offset(&square(0.0, 10.0), 2.0);
        assert!(grown.contains([-1.0, 5.0]) && !grown.contains([-3.0, 5.0]));
        let shrunk = offset(&square(0.0, 10.0), -2.0);
        assert!(shrunk.contains([5.0, 5.0]) && !shrunk.contains([1.0, 5.0]));
        assert!(offset(&square(0.0, 10.0), -6.0).is_empty());
        // A hole grows when the land around it shrinks.
        let ring = difference(&square(0.0, 30.0), &square(10.0, 20.0));
        let inset = offset(&ring, -2.0);
        assert!(!inset.contains([8.5, 15.0]) && inset.contains([7.0, 15.0]));
    }

    #[test]
    fn a_lasso_loop_becomes_a_valid_region_even_when_it_crosses_itself() {
        let square = lasso_region(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], 0.0);
        assert_eq!(square.0.len(), 1);
        assert_oriented(&square);
        assert!((area(&square) - 100.0).abs() < 1e-9);
        // A bow-tie is two triangles.
        let bowtie = lasso_region(&[[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]], 0.0);
        assert_eq!(bowtie.0.len(), 2);
        assert_oriented(&bowtie);
        assert!(lasso_region(&[[0.0, 0.0], [5.0, 5.0]], 0.0).is_empty());
        assert!(lasso_region(&[[0.0, 0.0], [5.0, 5.0], [10.0, 10.0]], 0.0).is_empty());
    }
}
