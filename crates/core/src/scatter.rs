//! Brush-based placement of many Stamps (spec §4): the scatter brush, the Stamp eraser and
//! Shuffle variant.
//!
//! The randomness is resolved here, from a seed the tool supplies, and the result is one
//! concrete Command that names every Stamp, so replaying a script places identical Stamps
//! (ADR 0004). Land and water are checked at placement time only; a Stamp keeps its place when
//! a Coastline is edited later.

use std::sync::Arc;

use crate::asset::AssetRef;
use crate::command::{Command, PlacedStamp};
use crate::geometry::{MultiPolygon, Point};
use crate::id::Id;
use crate::map::Map;
use crate::node::{Node, Object, ObjectKind, Stamp};
use crate::roughness::RoughnessCache;

/// Where the scatter brush may place Stamps.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Placement {
    /// On land that no Lake covers. The default for terrain.
    #[default]
    Land,
    /// Anywhere that is not land: the ocean, or a Lake.
    Water,
    /// On land, in the ocean or on a Lake alike.
    Anywhere,
}

/// The scatter brush's settings.
#[derive(Clone, PartialEq, Debug)]
pub struct ScatterBrush {
    /// Stamps per 100 by 100 Canvas units under the brush.
    pub density: f64,
    /// The least distance between two Anchors laid by one stroke, in Canvas units.
    pub spacing: f64,
    /// How far a Stamp's scale may stray from the base Stamp's, as a share from 0 to 0.9.
    pub size_jitter: f64,
    /// Mirrors each Stamp or not at random.
    pub random_flip: bool,
    /// Picks each Stamp's Asset at random from the Variant group.
    pub random_variant: bool,
    /// How far a Stamp's rotation may stray from the base Stamp's, in degrees either way.
    pub rotation_jitter: f64,
    /// Where on the Map the brush may place Stamps.
    pub placement: Placement,
}

impl Default for ScatterBrush {
    fn default() -> Self {
        Self {
            density: 8.0,
            spacing: 10.0,
            size_jitter: 0.2,
            random_flip: true,
            random_variant: true,
            rotation_jitter: 0.0,
            placement: Placement::Land,
        }
    }
}

/// The roughened outlines of a Map's Landmasses and Lakes, taken at the moment of placement.
#[derive(Clone, Default, Debug)]
pub struct Terrain {
    land: Vec<Arc<MultiPolygon>>,
    lakes: Vec<Arc<MultiPolygon>>,
}

impl Terrain {
    /// The land and Lakes of `map`, with each outline roughened as it is drawn.
    pub fn of(map: &Map, cache: &mut RoughnessCache) -> Self {
        let mut terrain = Terrain::default();
        for id in map.ids() {
            let Some(Node::Object(Object { kind, .. })) = map.node(id) else {
                continue;
            };
            let target = match kind {
                ObjectKind::Landmass(_) => &mut terrain.land,
                ObjectKind::Lake(_) => &mut terrain.lakes,
                _ => continue,
            };
            if let Some(outline) = cache.outline(map, id) {
                target.push(outline);
            }
        }
        terrain
    }

    /// Whether `point` is land: inside a Landmass and outside every Lake.
    pub(crate) fn is_land(&self, point: Point) -> bool {
        self.land.iter().any(|outline| outline.contains(point))
            && !self.lakes.iter().any(|outline| outline.contains(point))
    }

    /// Whether `point` is somewhere `placement` allows.
    pub fn allows(&self, placement: Placement, point: Point) -> bool {
        match placement {
            Placement::Land => self.is_land(point),
            Placement::Water => !self.is_land(point),
            Placement::Anywhere => true,
        }
    }
}

/// A small deterministic generator (SplitMix64), so a seed always gives the same stroke.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// In `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// In `[-1, 1)`.
    fn signed(&mut self) -> f64 {
        self.unit() * 2.0 - 1.0
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

/// The most Stamps one stroke may place, so a huge brush can't hang the editor.
pub(crate) const MAX_STAMPS_PER_STROKE: usize = 5000;

/// One finished scatter stroke and what it scatters.
#[derive(Clone, Copy, Debug)]
pub struct ScatterStroke<'a> {
    /// The path the brush followed, in Canvas units.
    pub points: &'a [Point],
    /// The brush radius in Canvas units.
    pub radius: f64,
    /// The Layer that receives the Stamps.
    pub parent: Id,
    /// Supplies the Asset, scale, rotation, flip, tint, fill and opacity the jitter varies.
    pub base: &'a Stamp,
    /// The Assets `random_variant` may pick from; empty means only the base Asset.
    pub variants: &'a [AssetRef],
}

/// The Command for one finished scatter stroke, or `None` when nothing could be placed. Each
/// new Stamp takes its id from `new_id`.
pub fn scatter_command(
    brush: &ScatterBrush,
    stroke: ScatterStroke<'_>,
    terrain: &Terrain,
    seed: u64,
    mut new_id: impl FnMut() -> Id,
) -> Option<Command> {
    let ScatterStroke {
        points: stroke,
        radius,
        parent,
        base,
        variants,
    } = stroke;
    if stroke.is_empty() || !(radius.is_finite() && radius > 0.0) || !brush.density.is_finite() {
        return None;
    }
    let mut rng = Rng(seed);
    let lengths: Vec<f64> = stroke.windows(2).map(|w| distance(w[0], w[1])).collect();
    let total: f64 = lengths.iter().sum();
    let area = std::f64::consts::PI * radius * radius + 2.0 * radius * total;
    let attempts = (brush.density.max(0.0) * area / 10_000.0).round() as usize;
    let attempts = attempts.min(MAX_STAMPS_PER_STROKE);
    let spacing = if brush.spacing.is_finite() {
        brush.spacing.max(0.0)
    } else {
        0.0
    };
    let size_jitter = if brush.size_jitter.is_finite() {
        brush.size_jitter.clamp(0.0, 0.9)
    } else {
        0.0
    };
    let rotation_jitter = if brush.rotation_jitter.is_finite() {
        brush.rotation_jitter.abs()
    } else {
        0.0
    };

    let mut placed: Vec<PlacedStamp> = Vec::new();
    for _ in 0..attempts {
        // A point of the stroke, then a point of the disc around it.
        let centre = point_along(stroke, &lengths, total, rng.unit());
        let angle = rng.unit() * std::f64::consts::TAU;
        let reach = radius * rng.unit().sqrt();
        let position = [
            centre[0] + reach * angle.cos(),
            centre[1] + reach * angle.sin(),
        ];
        let size = 1.0 + size_jitter * rng.signed();
        let turn = rotation_jitter * rng.signed();
        let flip = if brush.random_flip {
            rng.unit() < 0.5
        } else {
            base.flip
        };
        let asset = if brush.random_variant && !variants.is_empty() {
            variants[rng.below(variants.len())].clone()
        } else {
            base.asset.clone()
        };

        if !terrain.allows(brush.placement, position) {
            continue;
        }
        if placed
            .iter()
            .any(|other| distance(other.stamp.position, position) < spacing)
        {
            continue;
        }
        placed.push(PlacedStamp {
            id: new_id(),
            stamp: Stamp {
                asset,
                position,
                scale: base.scale * size,
                rotation: base.rotation + turn,
                flip,
                ..base.clone()
            },
        });
    }
    if placed.is_empty() {
        None
    } else {
        Some(Command::PlaceStamps {
            parent,
            stamps: placed,
        })
    }
}

/// The Command for a Stamp eraser stroke: removes Stamps directly inside `layer` whose Anchor
/// is within `radius` of `stroke`. Stamps in other Layers, and locked ones, stay. `None` when
/// the stroke touches none.
pub fn erase_command(map: &Map, layer: Id, stroke: &[Point], radius: f64) -> Option<Command> {
    let ids: Vec<Id> = map
        .children(Some(layer))
        .iter()
        .copied()
        .filter(|&id| !map.is_locked(id))
        .filter(|&id| match map.node(id) {
            Some(Node::Object(Object {
                kind: ObjectKind::Stamp(stamp),
                ..
            })) => near_stroke(stroke, stamp.position, radius),
            _ => false,
        })
        .collect();
    (!ids.is_empty()).then_some(Command::Delete { ids })
}

/// The Commands for Shuffle variant: each selected Stamp, in turn, takes a random Asset from
/// its own Variant group. `group_of` gives the Assets of the group an Asset belongs to (empty
/// when it has none). Stamps that are locked, or have no other variant to take, are left alone.
/// All the Commands belong in one Undo step.
pub fn shuffle_commands(
    map: &Map,
    selection: &[Id],
    group_of: impl Fn(&AssetRef) -> Vec<AssetRef>,
    seed: u64,
) -> Vec<Command> {
    let mut rng = Rng(seed);
    let mut commands = Vec::new();
    for &id in selection {
        let Some(Node::Object(Object {
            kind: ObjectKind::Stamp(stamp),
            ..
        })) = map.node(id)
        else {
            continue;
        };
        if map.is_locked(id) {
            continue;
        }
        let group = group_of(&stamp.asset);
        if group.len() < 2 {
            continue;
        }
        let asset = group[rng.below(group.len())].clone();
        if asset == stamp.asset {
            continue;
        }
        commands.push(Command::SetStamp {
            id,
            stamp: Box::new(Stamp {
                asset,
                ..stamp.clone()
            }),
        });
    }
    commands
}

fn distance(a: Point, b: Point) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The point `t` (from 0 to 1) of the way along the polyline's length.
fn point_along(points: &[Point], lengths: &[f64], total: f64, t: f64) -> Point {
    if total <= 0.0 {
        return points[0];
    }
    let mut left = t * total;
    for (segment, &length) in lengths.iter().enumerate() {
        if left <= length && length > 0.0 {
            let f = left / length;
            let (a, b) = (points[segment], points[segment + 1]);
            return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
        }
        left -= length;
    }
    points[points.len() - 1]
}

/// Whether `point` is within `radius` of the polyline (or of its one point).
fn near_stroke(stroke: &[Point], point: Point, radius: f64) -> bool {
    match stroke {
        [] => false,
        [only] => distance(*only, point) <= radius,
        _ => stroke
            .windows(2)
            .any(|w| distance_to_segment(w[0], w[1], point) <= radius),
    }
}

fn distance_to_segment(a: Point, b: Point, p: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    };
    distance([a[0] + dx * t, a[1] + dy * t], p)
}
