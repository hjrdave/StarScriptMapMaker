//! Editing the stored points of an outline: moving, inserting and deleting them, and repairing
//! the outline when an edit makes it cross itself.

use geo::BooleanOps;

use crate::geometry::{MultiPolygon, NodeRef, Point, Polygon};

use super::{normalised, to_geo};

/// Why a node edit could not be made.
#[derive(Clone, PartialEq, Debug, thiserror::Error)]
pub enum NodeEditError {
    /// The outline has no node there.
    #[error("the outline has no point at part {}, ring {}, point {}", .0.part, .0.ring, .0.index)]
    NoSuchNode(NodeRef),
    /// The edit would leave a ring with fewer than three points.
    #[error("a ring needs at least three points")]
    TooFewPoints,
    /// The edit would leave nothing of the outline.
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
