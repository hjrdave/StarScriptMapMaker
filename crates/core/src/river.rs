//! Rivers: water that follows a path of points from source to mouth (ADR 0012).
//!
//! A River stores only its points, each with a width, and how much it bends; its outline is
//! generated from them. The path is smoothed through every point, bends are added between the
//! points from an integer hash of the seed, and the banks are offset from that line by half
//! the width, which tapers from point to point. The source ends in a point and the mouth is
//! cut square, so it opens into the water it flows into.
//!
//! The outline is regenerated whenever a River is read or edited, so it must be the same on
//! every platform. As for Roughness (ADR 0002), the computation uses only `+ - * /` and
//! `sqrt`, which are correctly rounded everywhere, and never `sin`, `exp` or `powf`.

use geo::{LineString, unary_union};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::extra::Extra;
use crate::geometry::{Affine, MultiPolygon, Point};
use crate::land;
use crate::node::Lake;
use crate::roughness::{mix, unit};
use crate::scatter::{Placement, Terrain};
use crate::shore::ShoreStyle;

/// The narrowest a River can be at a point, in Canvas units.
pub const MIN_RIVER_WIDTH: f64 = 0.5;

/// The widest a River can be at a point, in Canvas units: the largest brush size.
pub const MAX_RIVER_WIDTH: f64 = 2000.0;

/// The most points a River's path can have.
pub const MAX_RIVER_POINTS: usize = 4096;

/// The strongest bending a River can have; 0 follows the smoothed path exactly.
pub const MAX_MEANDER: f64 = 1.0;

/// The outline is cut into at most this many cross-sections, whatever the River's length.
const MAX_SAMPLES: usize = 6000;

/// Cross-sections are about this share of the local width apart.
const STEP_PER_WIDTH: f64 = 0.35;

/// But never closer than this, in Canvas units.
const MIN_STEP: f64 = 0.5;

/// The length of one bend, as a multiple of the local width, at full bending.
const BEND_LENGTH: f64 = 9.0;

/// The furthest a bend swings from the path, as a multiple of the local width, at full bending.
const BEND_SWING: f64 = 1.4;

/// How far, as a multiple of the local width, the bends take to swing out after a point: the
/// path passes through every point the user placed.
const PIN_RAMP: f64 = 2.5;

/// How much the banks wobble, as a share of the local width, so they look drawn by hand.
const BANK_WOBBLE: f64 = 0.08;

/// The length of one bank wobble, as a multiple of the local width.
const WOBBLE_LENGTH: f64 = 1.6;

/// How far the source's point reaches past the first point, as a multiple of its width.
const SOURCE_TIP: f64 = 0.8;

/// Over this many of its first widths, a River narrows to a fine point at its source.
const SOURCE_TAPER: f64 = 6.0;

/// How wide a River is at its very source, as a share of its first point's width.
const SOURCE_NARROWEST: f64 = 0.3;

/// One point of a River's path.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RiverPoint {
    /// Where the path passes, in Canvas units.
    pub position: Point,
    /// How wide the River is here, in Canvas units.
    pub width: f64,
    /// Whether the River runs to this point without bends, from the point before.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub straight: bool,
}

impl RiverPoint {
    /// A point at `position`, `width` wide, reached through bends.
    pub fn new(position: Point, width: f64) -> Self {
        Self {
            position,
            width,
            straight: false,
        }
    }
}

/// Water that follows a path from source to mouth, tapering between the widths of its points.
/// Its outline is generated from the path, so it is never stored or edited directly.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(from = "RiverData", into = "RiverData")]
#[schemars(with = "RiverData")]
pub struct River {
    data: RiverData,
    outline: MultiPolygon,
    mouth: Option<[Point; 2]>,
}

/// The stored fields of a River: everything but its generated outline.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "River")]
struct RiverData {
    /// The points from source to mouth.
    path: Vec<RiverPoint>,
    /// How much it bends between its points, from 0 (none) to 1.
    #[serde(default = "default_meander")]
    meander: f64,
    /// Picks one of many equally bendy Rivers through the same points.
    #[serde(default)]
    seed: u32,
    /// The water's colour.
    #[serde(default = "default_fill")]
    fill: Color,
    /// Overrides the Map-wide default Shore style for Lakes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shore: Option<ShoreStyle>,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    extra: Extra,
}

fn default_meander() -> f64 {
    River::DEFAULT_MEANDER
}

fn default_fill() -> Color {
    Lake::DEFAULT_FILL
}

impl From<RiverData> for River {
    fn from(data: RiverData) -> Self {
        let (outline, mouth) = generate(&data);
        Self {
            data,
            outline,
            mouth,
        }
    }
}

impl From<River> for RiverData {
    fn from(river: River) -> Self {
        river.data
    }
}

impl River {
    /// How much a new River bends.
    pub const DEFAULT_MEANDER: f64 = 0.5;

    /// A River along `path`, with the default bending, seed, water colour and Shore style.
    pub fn new(path: Vec<RiverPoint>) -> Self {
        RiverData {
            path,
            meander: Self::DEFAULT_MEANDER,
            seed: 0,
            fill: Lake::DEFAULT_FILL,
            shore: None,
            extra: Extra::new(),
        }
        .into()
    }

    fn edited(mut self, edit: impl FnOnce(&mut RiverData)) -> Self {
        edit(&mut self.data);
        self.data.into()
    }

    /// The points from source to mouth.
    pub fn path(&self) -> &[RiverPoint] {
        &self.data.path
    }

    /// How much it bends between its points, from 0 (none) to [`MAX_MEANDER`].
    pub fn meander(&self) -> f64 {
        self.data.meander
    }

    /// Which of many equally bendy Rivers through the same points this is.
    pub fn seed(&self) -> u32 {
        self.data.seed
    }

    /// The water's colour.
    pub fn fill(&self) -> Color {
        self.data.fill
    }

    /// The Shore style override, if set.
    pub fn shore(&self) -> Option<&ShoreStyle> {
        self.data.shore.as_ref()
    }

    /// The generated outline. Empty when the River is invalid (see [`River::problem`]).
    pub fn outline(&self) -> &MultiPolygon {
        &self.outline
    }

    /// The two corners of the square mouth, left bank first. The outline's edge between them
    /// is where the River opens into other water, so no shore line is drawn along it.
    pub fn mouth(&self) -> Option<[Point; 2]> {
        self.mouth
    }

    /// A copy along another path.
    pub fn with_path(self, path: Vec<RiverPoint>) -> Self {
        self.edited(|data| data.path = path)
    }

    /// A copy that bends by `meander`.
    pub fn with_meander(self, meander: f64) -> Self {
        self.edited(|data| data.meander = meander)
    }

    /// A copy with another seed, which keeps the points but changes the bends.
    pub fn with_seed(self, seed: u32) -> Self {
        self.edited(|data| data.seed = seed)
    }

    /// A copy with another water colour.
    pub fn with_fill(mut self, fill: Color) -> Self {
        self.data.fill = fill;
        self
    }

    /// A copy with another Shore style override.
    pub fn with_shore(mut self, shore: Option<ShoreStyle>) -> Self {
        self.data.shore = shore;
        self
    }

    /// A copy with every point moved by `transform`, and every width scaled by how much it
    /// scales areas (the square root of its determinant), kept in range.
    pub fn transformed(&self, transform: &Affine) -> Self {
        let scale = transform.determinant().abs().sqrt();
        let path = self
            .data
            .path
            .iter()
            .map(|p| RiverPoint {
                position: transform.apply(p.position),
                width: (p.width * scale).clamp(MIN_RIVER_WIDTH, MAX_RIVER_WIDTH),
                ..*p
            })
            .collect();
        self.clone().with_path(path)
    }

    pub(crate) fn fill_mut(&mut self) -> &mut Color {
        &mut self.data.fill
    }

    pub(crate) fn shore_mut(&mut self) -> &mut Option<ShoreStyle> {
        &mut self.data.shore
    }

    /// Why this cannot be a River, if it cannot: it needs two to [`MAX_RIVER_POINTS`] points
    /// with finite positions that are not all in one place, widths from [`MIN_RIVER_WIDTH`] to
    /// [`MAX_RIVER_WIDTH`], and bending from 0 to [`MAX_MEANDER`].
    pub fn problem(&self) -> Option<String> {
        let path = &self.data.path;
        if path.len() < 2 {
            return Some("a River needs at least two points".into());
        }
        if path.len() > MAX_RIVER_POINTS {
            return Some(format!(
                "a River can have at most {MAX_RIVER_POINTS} points"
            ));
        }
        if let Some(point) = path
            .iter()
            .position(|p| p.position.iter().any(|v| !v.is_finite()))
        {
            return Some(format!("point {point} has a coordinate that is not finite"));
        }
        if let Some((point, p)) = path
            .iter()
            .enumerate()
            .find(|(_, p)| !(MIN_RIVER_WIDTH..=MAX_RIVER_WIDTH).contains(&p.width))
        {
            return Some(format!(
                "point {point}'s width must be from {MIN_RIVER_WIDTH} to {MAX_RIVER_WIDTH}, not {}",
                p.width
            ));
        }
        if !(0.0..=MAX_MEANDER).contains(&self.data.meander) {
            return Some(format!(
                "the bending must be from 0 to {MAX_MEANDER}, not {}",
                self.data.meander
            ));
        }
        if self.outline.is_empty() {
            return Some("a River's points cannot all be in one place".into());
        }
        None
    }

    /// The outline's boundary without the mouth: the lines a shore outline is drawn along.
    /// Each line is open; a ring the mouth does not touch comes back closed, its first point
    /// repeated at the end.
    pub fn banks(&self) -> Vec<Vec<Point>> {
        self.away_from_mouth(&self.outline, 0.0)
    }

    /// The rings of `region`, a ring offset from the outline, as open lines that leave out
    /// every edge within `reach` of the mouth: the lines shore shading is drawn along, which
    /// stop where the River opens into other water.
    pub fn away_from_mouth(&self, region: &MultiPolygon, reach: f64) -> Vec<Vec<Point>> {
        let mut lines = Vec::new();
        for ring in region.0.iter().flat_map(|polygon| polygon.rings()) {
            let open = |i: usize| {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                !self
                    .mouth
                    .is_some_and(|mouth| near_mouth(mouth, a, b, reach))
            };
            // Start just after a mouth edge, if there is one, so every line runs unbroken.
            let start = (0..ring.len())
                .find(|&i| !open(i))
                .map_or(0, |i| (i + 1) % ring.len());
            let mut line: Vec<Point> = Vec::new();
            for step in 0..ring.len() {
                let i = (start + step) % ring.len();
                if open(i) {
                    if line.is_empty() {
                        line.push(ring[i]);
                    }
                    line.push(ring[(i + 1) % ring.len()]);
                } else if line.len() > 1 {
                    lines.push(std::mem::take(&mut line));
                } else {
                    line.clear();
                }
            }
            if line.len() > 1 {
                lines.push(line);
            }
        }
        lines
    }
}

/// How far past the shore, in Canvas units, the later of a River's two banks reaches when
/// [`ending_at_shore`] cuts its path, so its mouth covers the shore's line.
const SHORE_OVERSHOOT: f64 = 1.0;

/// `path` cut short where it first leaves land after reaching it, if it does: at the coast or
/// at the edge of a Lake or another River. The cut falls just past where the later of its two
/// banks crosses the shore, so the square mouth opens onto the water across its whole width
/// instead of running on into it. The width there is the River's width at that spot. A path
/// that never leaves land comes back as it was.
pub fn ending_at_shore(path: &[RiverPoint], terrain: &Terrain) -> Vec<RiverPoint> {
    let on_land = |p: Point| terrain.allows(Placement::Land, p);
    let Some(first) = path.iter().position(|p| on_land(p.position)) else {
        return path.to_vec();
    };
    for i in first..path.len().saturating_sub(1) {
        let (a, b) = (path[i], path[i + 1]);
        let length = distance(a.position, b.position);
        if length <= 0.0 {
            continue;
        }
        let along = |t: f64, side: f64| {
            let (dx, dy) = (
                (b.position[0] - a.position[0]) / length,
                (b.position[1] - a.position[1]) / length,
            );
            let half = side * (a.width + (b.width - a.width) * t) / 2.0;
            [
                a.position[0] + dx * length * t + dy * half,
                a.position[1] + dy * length * t - dx * half,
            ]
        };
        let Some(centre) = leaves_land(&on_land, |t| along(t, 0.0), length, a.width.min(b.width))
        else {
            continue;
        };
        // Each bank crosses the shore where it does; a bank already in the water at the
        // centre's crossing counts from there.
        let bank = |side: f64| {
            leaves_land(&on_land, |t| along(t, side), length, a.width.min(b.width))
                .unwrap_or(centre)
        };
        let latest = centre.max(bank(1.0)).max(bank(-1.0));
        let end = (latest + SHORE_OVERSHOOT / length).min(1.0);
        let mut cut = path[..=i].to_vec();
        cut.push(RiverPoint::new(
            along(end, 0.0),
            a.width + (b.width - a.width) * end,
        ));
        return cut;
    }
    path.to_vec()
}

/// Where along a line from `t = 0` to `1`, `length` long, the point `at(t)` first leaves land,
/// sampled every half of `width` and then narrowed down by bisection. `None` if it never does.
fn leaves_land(
    on_land: &impl Fn(Point) -> bool,
    at: impl Fn(f64) -> Point,
    length: f64,
    width: f64,
) -> Option<f64> {
    let samples = (length / (width / 2.0).max(1.0)).ceil().max(1.0) as usize;
    let k = (1..=samples).find(|&k| !on_land(at(k as f64 / samples as f64)))?;
    let (mut lo, mut hi) = ((k - 1) as f64 / samples as f64, k as f64 / samples as f64);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if on_land(at(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(hi)
}

/// Whether both ends of the edge from `a` to `b` lie within `reach` of the mouth, or on it
/// when `reach` is 0.
fn near_mouth(mouth: [Point; 2], a: Point, b: Point, reach: f64) -> bool {
    let [m, n] = mouth;
    let (dx, dy) = (n[0] - m[0], n[1] - m[1]);
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0.0 {
        return false;
    }
    let reach = reach.max(length_squared.sqrt() * 1e-6);
    let near = |p: Point| {
        let t = (((p[0] - m[0]) * dx + (p[1] - m[1]) * dy) / length_squared).clamp(0.0, 1.0);
        distance(p, [m[0] + dx * t, m[1] + dy * t]) <= reach
    };
    near(a) && near(b)
}

/// One cross-section of the River along its smoothed path.
#[derive(Clone, Copy, Debug)]
struct Section {
    position: Point,
    width: f64,
    /// Which of the path's segments it lies on, counting from the source.
    segment: usize,
    /// Whether that segment runs without bends.
    straight: bool,
}

/// The outline and mouth of a River, or an empty outline when it has fewer than two distinct
/// points or anything is out of range.
fn generate(data: &RiverData) -> (MultiPolygon, Option<[Point; 2]>) {
    let valid = data.path.len() <= MAX_RIVER_POINTS
        && (0.0..=MAX_MEANDER).contains(&data.meander)
        && data.path.iter().all(|p| {
            p.position.iter().all(|v| v.is_finite())
                && (MIN_RIVER_WIDTH..=MAX_RIVER_WIDTH).contains(&p.width)
        });
    let mut path: Vec<RiverPoint> = Vec::with_capacity(data.path.len());
    for &point in data.path.iter().filter(|_| valid) {
        if path
            .last()
            .is_none_or(|last| distance(last.position, point.position) > 1e-9)
        {
            path.push(point);
        }
    }
    if path.len() < 2 {
        return (MultiPolygon::default(), None);
    }
    let sections = smoothed(&path);
    let line = bent(&sections, data.meander, data.seed);
    banked(&line, data.seed)
}

/// Cross-sections along a centripetal Catmull-Rom curve through every point, which never loops
/// or overshoots between points however unevenly they are spaced. Widths taper linearly.
fn smoothed(path: &[RiverPoint]) -> Vec<Section> {
    let positions: Vec<Point> = path.iter().map(|p| p.position).collect();
    let last = positions.len() - 1;
    // Past the ends, mirror the neighbouring point, so the curve leaves the ends straight.
    let at = |i: isize| -> Point {
        if i < 0 {
            mirror(positions[0], positions[1])
        } else if i as usize > last {
            mirror(positions[last], positions[last - 1])
        } else {
            positions[i as usize]
        }
    };
    let steps: Vec<usize> = (0..last)
        .map(|i| {
            let width = path[i].width.min(path[i + 1].width);
            let step = (width * STEP_PER_WIDTH).max(MIN_STEP);
            (distance(positions[i], positions[i + 1]) / step)
                .ceil()
                .max(1.0) as usize
        })
        .collect();
    let total: usize = steps.iter().sum();
    // Long, narrow Rivers spread their cross-sections out rather than grow without bound.
    let thin = total.div_ceil(MAX_SAMPLES).max(1);
    let mut sections = Vec::with_capacity(total / thin + 2);
    for segment in 0..last {
        let i = segment as isize;
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let n = steps[segment].div_ceil(thin).max(1);
        for k in 0..n {
            let u = k as f64 / n as f64;
            sections.push(Section {
                position: catmull_rom(p0, p1, p2, p3, u),
                width: path[segment].width + (path[segment + 1].width - path[segment].width) * u,
                segment,
                straight: path[segment + 1].straight,
            });
        }
    }
    sections.push(Section {
        position: positions[last],
        width: path[last].width,
        segment: last - 1,
        straight: path[last].straight,
    });
    sections
}

/// The point a share `u` of the way from `p1` to `p2` on the centripetal Catmull-Rom curve
/// through `p0`..`p3` (Barry and Goldman's form).
fn catmull_rom(p0: Point, p1: Point, p2: Point, p3: Point, u: f64) -> Point {
    // Knot spacing is the square root of the distance; never zero, so nothing divides by it.
    let knot = |a: Point, b: Point| distance(a, b).sqrt().max(1e-6);
    let t1 = knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let t = t1 + (t2 - t1) * u;
    let lerp = |a: Point, b: Point, ta: f64, tb: f64| {
        let w = (t - ta) / (tb - ta);
        [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w]
    };
    let a1 = lerp(p0, p1, 0.0, t1);
    let a2 = lerp(p1, p2, t1, t2);
    let a3 = lerp(p2, p3, t2, t3);
    let b1 = lerp(a1, a2, 0.0, t2);
    let b2 = lerp(a2, a3, t1, t3);
    lerp(b1, b2, t1, t2)
}

/// The cross-sections swung sideways into bends. Each segment between two points bends on its
/// own, seeded from the River's seed and the segment's number, so moving a point changes only
/// the bends next to it. The swing fades to nothing at every point.
fn bent(sections: &[Section], meander: f64, seed: u32) -> Vec<Section> {
    if meander <= 0.0 {
        return sections.to_vec();
    }
    let normals = normals(sections);
    // How far along its own segment each section is, and how long each segment is.
    let last = sections.len() - 1;
    let mut along = vec![0.0; sections.len()];
    let mut lengths = vec![0.0; sections[last].segment + 1];
    for i in 1..sections.len() {
        let step = distance(sections[i - 1].position, sections[i].position);
        if sections[i].segment == sections[i - 1].segment {
            along[i] = along[i - 1] + step;
        } else {
            lengths[sections[i - 1].segment] = along[i - 1] + step;
        }
    }
    lengths[sections[last].segment] = along[last];
    let mut phase = 0.0;
    let mut previous_segment = usize::MAX;
    sections
        .iter()
        .enumerate()
        .map(|(i, section)| {
            let segment = section.segment;
            if segment != previous_segment {
                phase = 0.0;
                previous_segment = segment;
            } else if i > 0 {
                let step = distance(sections[i - 1].position, section.position);
                phase += step / (BEND_LENGTH * section.width.max(1.0));
            }
            let length = lengths.get(segment).copied().unwrap_or_default();
            let from_point = if i == last {
                0.0
            } else {
                along[i].min(length - along[i])
            };
            let pin = if section.straight {
                0.0
            } else {
                smoothstep(from_point / (PIN_RAMP * section.width))
            };
            let segment_seed = mix(u64::from(seed), segment as u64);
            let swing = meander * BEND_SWING * section.width * pin * bends(segment_seed, phase);
            let n = normals[i];
            Section {
                position: [
                    section.position[0] + n[0] * swing,
                    section.position[1] + n[1] * swing,
                ],
                ..*section
            }
        })
        .collect()
}

/// The outline around the line of cross-sections, and its mouth: the union of the triangles
/// between neighbouring cross-sections and of the source's point, which stays one clean outline
/// even where a tight bend folds a bank over itself.
fn banked(line: &[Section], seed: u32) -> (MultiPolygon, Option<[Point; 2]>) {
    let normals = normals(line);
    let wobble_seed = |side: u64| mix(u64::from(seed), 0xB4_4E_u64 + side);
    let mut left = Vec::with_capacity(line.len());
    let mut right = Vec::with_capacity(line.len());
    let (mut phase, mut previous, mut from_source) = (0.0, line[0].position, 0.0);
    let taper = SOURCE_TAPER * line[0].width;
    for (section, n) in line.iter().zip(&normals) {
        let step = distance(previous, section.position);
        phase += step / (WOBBLE_LENGTH * section.width.max(1.0));
        from_source += step;
        previous = section.position;
        let narrowing =
            SOURCE_NARROWEST + (1.0 - SOURCE_NARROWEST) * smoothstep(from_source / taper);
        let half = section.width * narrowing / 2.0;
        let side = |s: u64, sign: f64| {
            let reach = half * (1.0 + BANK_WOBBLE * bends(wobble_seed(s), phase));
            [
                section.position[0] + sign * n[0] * reach,
                section.position[1] + sign * n[1] * reach,
            ]
        };
        left.push(side(0, 1.0));
        right.push(side(1, -1.0));
    }
    let mut triangles = Vec::with_capacity(2 * line.len() + 1);
    for i in 1..line.len() {
        triangles.push(triangle(left[i - 1], left[i], right[i - 1]));
        triangles.push(triangle(left[i], right[i], right[i - 1]));
    }
    let source = &line[0];
    let back = normals[0];
    // Upstream from the source: the normal turned a quarter turn back from downstream.
    let reach = source.width * SOURCE_NARROWEST * SOURCE_TIP;
    let tip = [
        source.position[0] + back[1] * reach,
        source.position[1] - back[0] * reach,
    ];
    triangles.push(triangle(left[0], right[0], tip));
    let triangles: Vec<geo::Polygon<f64>> = triangles.into_iter().flatten().collect();
    let outline = land::normalised(unary_union(&triangles));
    let last = line.len() - 1;
    (outline, Some([left[last], right[last]]))
}

/// A counter-clockwise triangle, or `None` when it has no area.
fn triangle(a: Point, b: Point, c: Point) -> Option<geo::Polygon<f64>> {
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
    if area.is_nan() || area.abs() <= 1e-12 {
        return None;
    }
    let ring = if area > 0.0 {
        vec![a, b, c]
    } else {
        vec![a, c, b]
    };
    Some(geo::Polygon::new(LineString::from(ring), Vec::new()))
}

/// The unit normal at each cross-section, pointing to the left bank (the left of a walk
/// downstream on screen, where y grows downward). From the neighbours on both sides, so
/// it turns smoothly.
fn normals(sections: &[Section]) -> Vec<Point> {
    let last = sections.len() - 1;
    let mut previous = [0.0, -1.0];
    (0..sections.len())
        .map(|i| {
            let a = sections[i.saturating_sub(1)].position;
            let b = sections[(i + 1).min(last)].position;
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let length = (dx * dx + dy * dy).sqrt();
            if length > 1e-12 {
                previous = [dy / length, -dx / length];
            }
            previous
        })
        .collect()
}

/// Smooth noise from about -1 to 1 along `phase`: two octaves of value noise, seeded.
fn bends(seed: u64, phase: f64) -> f64 {
    let octave = |salt: u64, x: f64| {
        let cell = x.floor();
        let at = |c: f64| unit(mix(seed ^ salt, c as i64 as u64)) * 2.0 - 1.0;
        let (a, b) = (at(cell), at(cell + 1.0));
        a + (b - a) * smoothstep(x - cell)
    };
    (octave(0, phase) + 0.4 * octave(0x5EED, phase * 2.3 + 0.5)) / 1.4
}

/// 0 at or below 0, 1 at or above 1, and an S-curve between.
fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn mirror(end: Point, neighbour: Point) -> Point {
    [2.0 * end[0] - neighbour[0], 2.0 * end[1] - neighbour[1]]
}

fn distance(a: Point, b: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(widths: [f64; 2]) -> River {
        River::new(vec![
            RiverPoint::new([0.0, 0.0], widths[0]),
            RiverPoint::new([400.0, 0.0], widths[1]),
        ])
    }

    fn area(outline: &MultiPolygon) -> f64 {
        outline
            .0
            .iter()
            .map(|polygon| {
                land::signed_area(&polygon.exterior)
                    + polygon
                        .holes
                        .iter()
                        .map(|hole| land::signed_area(hole))
                        .sum::<f64>()
            })
            .sum()
    }

    #[test]
    fn a_straight_river_is_one_valid_part_about_as_big_as_its_widths() {
        let river = straight([10.0, 30.0]).with_meander(0.0);
        assert_eq!(river.problem(), None);
        assert_eq!(river.outline().0.len(), 1);
        assert_eq!(river.outline().problem(), None);
        // A trapezoid 400 long, 10 to 30 wide, give or take the wobble and the source tip.
        let area = area(river.outline());
        assert!((7_500.0..8_700.0).contains(&area), "area {area}");
    }

    #[test]
    fn it_tapers_from_source_to_mouth() {
        let river = straight([6.0, 40.0]).with_meander(0.0);
        let width_at = |x: f64| {
            let ys: Vec<f64> = river
                .outline()
                .0
                .iter()
                .flat_map(|p| p.exterior.iter())
                .filter(|p| (p[0] - x).abs() < 3.0)
                .map(|p| p[1])
                .collect();
            ys.iter().copied().fold(f64::MIN, f64::max)
                - ys.iter().copied().fold(f64::MAX, f64::min)
        };
        assert!(width_at(20.0) < 12.0);
        assert!(width_at(380.0) > 32.0);
    }

    #[test]
    fn the_mouth_is_square_at_the_last_point_and_left_out_of_the_banks() {
        let river = straight([10.0, 20.0]);
        let [a, b] = river.mouth().unwrap();
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        assert!(
            distance(mid, [400.0, 0.0]) < 20.0 * BANK_WOBBLE,
            "about the last point"
        );
        assert!((distance(a, b) - 20.0).abs() < 20.0 * BANK_WOBBLE * 1.01);
        let banks = river.banks();
        assert_eq!(
            banks.len(),
            1,
            "one line from mouth to mouth round the source"
        );
        let line = &banks[0];
        let ends = [line[0], line[line.len() - 1]];
        let near_corner = |p: Point| distance(p, a).min(distance(p, b)) < 1e-3;
        assert!(
            ends.iter().all(|&p| near_corner(p)),
            "{ends:?} vs {a:?} {b:?}"
        );
        assert!(
            line.windows(2)
                .all(|w| !near_mouth([a, b], w[0], w[1], 0.0))
        );
    }

    #[test]
    fn bends_pass_through_every_point_and_move_with_the_seed_only() {
        let path = vec![
            RiverPoint::new([0.0, 0.0], 8.0),
            RiverPoint::new([300.0, 100.0], 12.0),
            RiverPoint::new([600.0, 0.0], 20.0),
        ];
        let river = River::new(path.clone());
        let other = river.clone().with_seed(7);
        assert_ne!(river.outline(), other.outline());
        assert_eq!(other.path(), &path[..]);
        // The middle point is inside both outlines: the bends are pinned there.
        assert!(river.outline().contains([300.0, 100.0]));
        assert!(other.outline().contains([300.0, 100.0]));
        // And the bends swing off the straight path between points.
        let flat = river.clone().with_meander(0.0);
        assert_ne!(river.outline(), flat.outline());
    }

    #[test]
    fn a_straight_segment_has_no_bends() {
        let mut path = vec![
            RiverPoint::new([0.0, 0.0], 8.0),
            RiverPoint::new([400.0, 0.0], 8.0),
        ];
        let bendy = River::new(path.clone()).with_meander(MAX_MEANDER);
        path[1].straight = true;
        let straight = River::new(path).with_meander(MAX_MEANDER);
        let spread = |river: &River| {
            let ys = river
                .outline()
                .0
                .iter()
                .flat_map(|p| p.exterior.iter().map(|q| q[1]));
            ys.fold(0.0_f64, |most, y| most.max(y.abs()))
        };
        assert!(spread(&straight) < 8.0, "{}", spread(&straight));
        assert!(spread(&bendy) > spread(&straight));
        let json = serde_json::to_value(straight.path()).unwrap();
        assert_eq!(json[1]["straight"], true);
        assert!(json[0].get("straight").is_none(), "false is left out");
    }

    #[test]
    fn the_same_river_always_has_the_same_outline() {
        let make = || {
            River::new(vec![
                RiverPoint::new([10.0, 10.0], 4.0),
                RiverPoint::new([200.0, 60.0], 9.0),
                RiverPoint::new([260.0, 300.0], 25.0),
            ])
            .with_seed(42)
        };
        assert_eq!(make().outline(), make().outline());
    }

    #[test]
    fn a_hairpin_still_makes_one_clean_outline() {
        let river = River::new(vec![
            RiverPoint::new([0.0, 0.0], 30.0),
            RiverPoint::new([200.0, 0.0], 30.0),
            RiverPoint::new([0.0, 10.0], 30.0),
        ])
        .with_meander(MAX_MEANDER);
        assert_eq!(river.problem(), None);
        assert_eq!(river.outline().problem(), None);
        assert!(area(river.outline()) > 0.0);
    }

    #[test]
    fn problems_are_named() {
        let one = River::new(vec![RiverPoint::new([0.0, 0.0], 5.0)]);
        assert!(one.problem().unwrap().contains("two points"));
        let same = River::new(vec![RiverPoint::new([1.0, 1.0], 5.0); 3]);
        assert!(same.problem().unwrap().contains("one place"));
        let thin = straight([0.1, 5.0]);
        assert!(thin.problem().unwrap().contains("width"));
        let wild = straight([5.0, 5.0]).with_meander(3.0);
        assert!(wild.problem().unwrap().contains("bending"));
        let lost = River::new(vec![
            RiverPoint::new([f64::NAN, 0.0], 5.0),
            RiverPoint::new([1.0, 0.0], 5.0),
        ]);
        assert!(lost.problem().unwrap().contains("not finite"));
        assert!(lost.outline().is_empty());
    }

    #[test]
    fn it_round_trips_without_its_outline() {
        let river = straight([5.0, 15.0]).with_seed(3).with_meander(0.8);
        let json = serde_json::to_value(&river).unwrap();
        assert!(json.get("outline").is_none());
        assert_eq!(json["seed"], 3);
        let back: River = serde_json::from_value(json).unwrap();
        assert_eq!(back, river);
    }

    #[test]
    fn a_river_saved_without_optional_fields_gets_the_defaults() {
        let back: River = serde_json::from_str(
            r#"{"path":[{"position":[0,0],"width":4},{"position":[50,0],"width":8}]}"#,
        )
        .unwrap();
        assert_eq!(back.meander(), River::DEFAULT_MEANDER);
        assert_eq!(back.fill(), Lake::DEFAULT_FILL);
        assert_eq!(back.problem(), None);
    }

    fn island_with_a_lake() -> Terrain {
        use crate::geometry::Polygon;
        use crate::node::Landmass;
        use crate::{Canvas, Command, DistanceUnit, Id, Map, RoughnessCache, Scale};
        let mut map = Map::new(Canvas::new(
            1000,
            1000,
            Scale::new(1.0, DistanceUnit::Miles),
        ));
        let layer = Id::new();
        let square = |min: f64, max: f64| {
            MultiPolygon(vec![Polygon {
                exterior: vec![[min, min], [max, min], [max, max], [min, max]],
                holes: Vec::new(),
            }])
        };
        let commands = [
            Command::CreateLayer {
                id: layer,
                parent: None,
                index: None,
                name: "Land".into(),
            },
            Command::CreateLandmass {
                id: Id::new(),
                parent: layer,
                index: None,
                geometry: square(0.0, 500.0),
                fill: Some(Landmass::DEFAULT_FILL),
            },
            Command::CreateLake {
                id: Id::new(),
                parent: layer,
                index: None,
                geometry: square(300.0, 400.0),
                fill: None,
            },
        ];
        for command in &commands {
            map.execute(command).unwrap();
        }
        Terrain::of(&map, &mut RoughnessCache::new())
    }

    #[test]
    fn a_path_into_a_lake_ends_just_past_its_shore() {
        let terrain = island_with_a_lake();
        let path = [
            RiverPoint::new([100.0, 350.0], 4.0),
            RiverPoint::new([200.0, 350.0], 10.0),
            RiverPoint::new([350.0, 350.0], 20.0),
        ];
        let cut = ending_at_shore(&path, &terrain);
        assert_eq!(cut.len(), 3);
        let end = cut[2];
        // The shore is at x = 300, square across the River's path.
        let expected = 300.0 + SHORE_OVERSHOOT;
        assert!(
            (end.position[0] - expected).abs() < 0.01,
            "{:?}",
            end.position
        );
        let width = 10.0 + 10.0 * (expected - 200.0) / 150.0;
        assert!((end.width - width).abs() < 0.01, "{}", end.width);
    }

    #[test]
    fn a_slanting_path_ends_once_both_banks_are_in_the_water() {
        let terrain = island_with_a_lake();
        // Heading down and right into the Lake's top-left corner region at 45 degrees.
        let path = [
            RiverPoint::new([200.0, 250.0], 20.0),
            RiverPoint::new([280.0, 330.0], 20.0),
            RiverPoint::new([340.0, 390.0], 20.0),
        ];
        let cut = ending_at_shore(&path, &terrain);
        let end = cut[cut.len() - 1];
        let reach = 10.0 * std::f64::consts::FRAC_1_SQRT_2;
        // The banks run 7.07 either side of the centre; the later one meets x = 300 or
        // y = 300 only once the centre is that far past the first shore crossing.
        assert!(end.position[0] > 300.0 + reach - 0.5, "{:?}", end.position);
        assert!(end.position[1] > 300.0 + reach - 0.5, "{:?}", end.position);
    }

    #[test]
    fn a_path_that_leaves_land_early_drops_the_points_beyond() {
        let terrain = island_with_a_lake();
        let path = [
            RiverPoint::new([100.0, 100.0], 4.0),
            RiverPoint::new([600.0, 100.0], 10.0),
            RiverPoint::new([800.0, 100.0], 20.0),
        ];
        let cut = ending_at_shore(&path, &terrain);
        assert_eq!(cut.len(), 2);
        assert!(
            (cut[1].position[0] - 500.0).abs() < 2.0,
            "{:?}",
            cut[1].position
        );
    }

    #[test]
    fn a_path_on_land_or_starting_at_sea_is_left_alone() {
        let terrain = island_with_a_lake();
        let inland = [
            RiverPoint::new([50.0, 50.0], 4.0),
            RiverPoint::new([250.0, 50.0], 10.0),
        ];
        assert_eq!(ending_at_shore(&inland, &terrain), inland.to_vec());
        let at_sea = [
            RiverPoint::new([700.0, 700.0], 4.0),
            RiverPoint::new([900.0, 900.0], 10.0),
        ];
        assert_eq!(ending_at_shore(&at_sea, &terrain), at_sea.to_vec());
    }

    #[test]
    fn a_very_long_river_stays_within_the_section_budget() {
        let river = straight([0.5, 0.5]).with_path(vec![
            RiverPoint::new([0.0, 0.0], 0.5),
            RiverPoint::new([1.0e6, 0.0], 0.5),
        ]);
        let points: usize = river.outline().0.iter().map(|p| p.exterior.len()).sum();
        assert!(points < 3 * MAX_SAMPLES, "{points} points");
    }
}
