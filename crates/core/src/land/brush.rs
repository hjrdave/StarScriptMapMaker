//! Brush and lasso strokes: smoothing a stroke into a region, and the Commands that add it to
//! or cut it from an outline.

use geo::{BooleanOps, Buffer, ChaikinSmoothing, LineString, Simplify, buffer::BufferStyle};

use crate::command::Command;
use crate::geometry::{MultiPolygon, Point, Polygon};
use crate::id::Id;
use crate::map::Map;
use crate::node::{Node, ObjectKind};
use crate::roughness::{RoughnessCache, roughen};

use super::{difference, from_geo, intersects, normalised, signed_area, to_geo, union};

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
    if kept.len() > 1
        && let (Some(&last), Some(end)) = (points.last(), kept.last_mut())
    {
        *end = last;
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

/// The largest Irregularity: the biggest lobes then reach out (or in) by half the brush radius.
pub const MAX_IRREGULARITY: f64 = 0.5;

/// Irregularity's noise: (wavelength, amplitude) octaves, the wavelength in brush radii and the
/// amplitude as a share of the largest. Lobes about a brush wide, then smaller bumps on them;
/// anything finer is left to Roughness, which would only add stored points here.
const IRREGULARITY_OCTAVES: [(f64, f64); 2] = [(2.2, 1.0), (1.0, 0.45)];

/// How far apart, in brush radii, points are put along an outline before it is pushed about,
/// so that every lobe has points to move.
const IRREGULARITY_SPACING: f64 = 0.2;

/// Holes smaller than this share of the brush's disc are dropped from an irregular stroke: they
/// are slivers where the outline folded back on itself, not shapes anyone painted.
const MIN_HOLE_SHARE: f64 = 0.02;

/// The region a finished brush stroke adds or erases with Irregularity: [`brush_region`] with
/// its outline pushed out and in, at brush scale, by smooth noise that `seed` picks. The noise
/// is periodic around each ring, so the rings close without a seam. `irregularity` is the
/// largest push as a share of `radius`, up to [`MAX_IRREGULARITY`]; zero (or anything not
/// positive) gives exactly [`brush_region`]. The pushed outline is simplified to within
/// `tolerance` and untangled, so it is a valid stored outline like any other.
pub fn irregular_region(
    points: &[Point],
    radius: f64,
    tolerance: f64,
    irregularity: f64,
    seed: u32,
) -> MultiPolygon {
    let region = brush_region(points, radius, tolerance);
    if region.is_empty() || irregularity.is_nan() || irregularity <= 0.0 {
        return region;
    }
    let irregularity = irregularity.min(MAX_IRREGULARITY);
    let mut phases = Phases::new(seed);
    let pushed = MultiPolygon(
        region
            .0
            .iter()
            .map(|polygon| Polygon {
                exterior: push_ring(&polygon.exterior, radius, irregularity, &mut phases),
                holes: polygon
                    .holes
                    .iter()
                    .map(|hole| push_ring(hole, radius, irregularity, &mut phases))
                    .collect(),
            })
            .collect(),
    );
    let simplified = to_geo(&pushed).simplify(tolerance.max(0.0));
    let mut untangled = normalised(simplified.union(&geo::MultiPolygon::<f64>::new(Vec::new())));
    let min_hole = MIN_HOLE_SHARE * std::f64::consts::PI * radius * radius;
    for polygon in &mut untangled.0 {
        polygon
            .holes
            .retain(|hole| signed_area(hole).abs() >= min_hole);
    }
    untangled
}

/// `ring` pushed along its normals by the Irregularity noise. Each octave makes a whole number
/// of waves around the ring, so the last point meets the first.
fn push_ring(ring: &[Point], radius: f64, irregularity: f64, phases: &mut Phases) -> Vec<Point> {
    let step = radius * IRREGULARITY_SPACING;
    let mut dense: Vec<Point> = Vec::new();
    for (i, &a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        // At most a few thousand points per edge, whatever the radius.
        let pieces = (distance(a, b) / step).ceil().clamp(1.0, 4096.0) as usize;
        for k in 0..pieces {
            let t = k as f64 / pieces as f64;
            dense.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    let n = dense.len();
    let mut along = Vec::with_capacity(n);
    let mut length = 0.0;
    for (i, &p) in dense.iter().enumerate() {
        along.push(length);
        length += distance(p, dense[(i + 1) % n]);
    }
    if length.is_nan() || length <= 0.0 {
        return ring.to_vec();
    }
    let amplitude = radius * irregularity;
    let waves: Vec<(f64, f64, f64)> = IRREGULARITY_OCTAVES
        .iter()
        .map(|&(wavelength, share)| {
            let cycles = (length / (wavelength * radius)).round().max(1.0);
            (cycles, share * amplitude, phases.next())
        })
        .collect();
    (0..n)
        .map(|i| {
            let (prev, p, next) = (dense[(i + n - 1) % n], dense[i], dense[(i + 1) % n]);
            let (tx, ty) = (next[0] - prev[0], next[1] - prev[1]);
            let tangent = tx.hypot(ty);
            if tangent == 0.0 {
                return p;
            }
            let angle = std::f64::consts::TAU * along[i] / length;
            let push: f64 = waves
                .iter()
                .map(|&(cycles, size, phase)| size * (cycles * angle + phase).sin())
                .sum();
            [p[0] + ty / tangent * push, p[1] - tx / tangent * push]
        })
        .collect()
}

/// The phases of the Irregularity noise's waves, a seeded stream (splitmix64).
struct Phases(u64);

impl Phases {
    fn new(seed: u32) -> Self {
        Self(u64::from(seed))
    }

    /// The next phase, in radians.
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1_u64 << 53) as f64 * std::f64::consts::TAU
    }
}

/// One finished land brush stroke, ready to become a Command.
#[derive(Clone, PartialEq, Debug)]
pub struct BrushStroke {
    /// What the stroke covers, from [`brush_region`] or [`irregular_region`].
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
    let is_outline = |kind: &ObjectKind| kind.stored_geometry().is_some();
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
                    Some(Node::Object(o)) if is_landmass(&o.kind) => o
                        .kind
                        .geometry()
                        .is_some_and(|geometry| intersects(geometry, &region)),
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
                            && o.kind.geometry().is_some_and(|geometry| intersects(geometry, &region)))
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

/// Why a land brush stroke could not be made.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum BrushError {
    /// There is no Layer to paint into.
    #[error("choose a Layer to paint land into")]
    NoLayer,
}

fn centreline(points: &[Point]) -> LineString<f64> {
    LineString::from(points.to_vec())
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
