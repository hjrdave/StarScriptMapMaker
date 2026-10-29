//! Roughness: generated jagged detail on coastlines (ADR 0002).
//!
//! The roughened outline is derived from the stored outline plus [`Roughness`] settings. Render,
//! picking, Clip and export all use it, so it must be the same everywhere: the computation uses
//! only IEEE-754 operations that are correctly rounded on every platform (`+ - * /` and `sqrt`),
//! never `sin`, `exp` or `powf`, and draws its randomness from an integer hash.
//!
//! Each edge of a stored ring is cut into pieces about `scale` long, then each piece is refined by
//! midpoint displacement along the edge's normal, halving the amplitude at every level. Stored
//! vertices never move, so every node stays on the coast. An edge's detail depends only on its two
//! end points and the seed, so moving one node changes only its two edges.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::extra::Extra;
use crate::geometry::{Affine, MultiPolygon, Point, Polygon};
use crate::id::Id;
use crate::map::Map;

/// The finest detail is `scale / 2^MAX_LEVELS`.
const MAX_LEVELS: u32 = 5;

/// No displacement moves a midpoint further than this fraction of its piece's length, which keeps
/// pieces from folding back over themselves.
const MAX_BEND: f64 = 0.35;

/// Edges are cut into at most this many pieces, whatever the scale, to bound the output size.
const MAX_PIECES: f64 = 4096.0;

/// How rough a coastline is.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Roughness {
    /// The largest displacement, in Canvas units. Zero leaves the outline smooth.
    pub amount: f64,
    /// The size of the largest features, in Canvas units.
    pub scale: f64,
    /// Picks one of many equally rough coastlines.
    pub seed: u32,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Roughness {
    /// A Roughness of `amount` and `scale` with `seed`.
    pub fn new(amount: f64, scale: f64, seed: u32) -> Self {
        Self {
            amount,
            scale,
            seed,
            extra: Extra::new(),
        }
    }

    /// Whether these settings leave an outline unchanged.
    fn is_smooth(&self) -> bool {
        !(self.amount > 0.0
            && self.scale > 0.0
            && self.amount.is_finite()
            && self.scale.is_finite())
    }
}

impl Default for Roughness {
    fn default() -> Self {
        Self::new(0.0, 32.0, 0)
    }
}

/// The roughened copy of `outline`. Rings keep their order and orientation; holes stay holes.
pub fn roughen(outline: &MultiPolygon, roughness: &Roughness) -> MultiPolygon {
    if roughness.is_smooth() {
        return outline.clone();
    }
    let origin = origin_of(outline);
    MultiPolygon(
        outline
            .0
            .iter()
            .map(|polygon| Polygon {
                exterior: roughen_ring(&polygon.exterior, roughness, origin),
                holes: polygon
                    .holes
                    .iter()
                    .map(|hole| roughen_ring(hole, roughness, origin))
                    .collect(),
            })
            .collect(),
    )
}

fn roughen_ring(ring: &[Point], roughness: &Roughness, origin: Point) -> Vec<Point> {
    if ring.len() < 3 {
        return ring.to_vec();
    }
    let mut out = Vec::with_capacity(ring.len() * 8);
    for (i, &a) in ring.iter().enumerate() {
        let b = ring[(i + 1) % ring.len()];
        out.push(a);
        roughen_edge(a, b, roughness, origin, &mut out);
    }
    out
}

/// Pushes the new points strictly between `a` and `b`.
fn roughen_edge(a: Point, b: Point, roughness: &Roughness, origin: Point, out: &mut Vec<Point>) {
    let (a, b) = (canonical(a), canonical(b));
    // Work in one direction per edge, whichever way the ring runs, so an edge shared by two rings
    // gets the same detail in both.
    let (key_a, key_b) = (key(a, origin), key(b, origin));
    let reversed = key_b < key_a;
    let (from, to) = if reversed { (b, a) } else { (a, b) };
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let length = (dx * dx + dy * dy).sqrt();
    if !length.is_finite() || length <= 0.0 {
        return;
    }
    let normal = [-dy / length, dx / length];

    let pieces = (length / roughness.scale).ceil().clamp(1.0, MAX_PIECES);
    let piece_length = length / pieces;
    let amplitude = roughness.amount * (piece_length / roughness.scale).min(1.0);

    // Offsets along the normal at evenly spaced positions, from `from` to `to` inclusive.
    let levels = levels_for(piece_length, roughness.scale);
    let per_piece = 1usize << levels;
    let pieces = pieces as usize;
    let mut offsets = vec![0.0; pieces * per_piece + 1];
    let (key_from, key_to) = if reversed {
        (key_b, key_a)
    } else {
        (key_a, key_b)
    };
    let mut hash = mix(
        mix(u64::from(roughness.seed), key_bits(key_from)),
        key_bits(key_to),
    );
    for piece in 0..pieces {
        let base = piece * per_piece;
        let mut step = per_piece;
        let mut level_amplitude = amplitude;
        let mut level_length = piece_length;
        while step > 1 {
            let half = step / 2;
            let limit = level_length * MAX_BEND;
            let mut start = base;
            while start < base + per_piece {
                hash = mix(hash, (start + half) as u64);
                let shift = (unit(hash) - 0.5) * 2.0 * level_amplitude;
                let mid = (offsets[start] + offsets[start + step]) * 0.5;
                offsets[start + half] = mid + shift.clamp(-limit, limit);
                start += step;
            }
            step = half;
            level_amplitude *= 0.5;
            level_length *= 0.5;
        }
    }

    let total = offsets.len() - 1;
    let point = |k: usize| {
        let t = k as f64 / total as f64;
        let offset = offsets[k];
        [
            from[0] + dx * t + normal[0] * offset,
            from[1] + dy * t + normal[1] * offset,
        ]
    };
    if reversed {
        out.extend((1..total).rev().map(point));
    } else {
        out.extend((1..total).map(point));
    }
}

/// How many times a piece is halved: until its parts are no longer than `scale / 2^MAX_LEVELS`.
fn levels_for(piece_length: f64, scale: f64) -> u32 {
    let finest = scale / f64::from(1u32 << MAX_LEVELS);
    let mut levels = 0;
    let mut length = piece_length;
    while levels < MAX_LEVELS && length > finest {
        length *= 0.5;
        levels += 1;
    }
    levels
}

/// Turns `-0.0` into `0.0` so that equal points hash equally.
fn canonical(p: Point) -> Point {
    [p[0] + 0.0, p[1] + 0.0]
}

/// Where the first vertex of `outline` is, which positions are measured from.
fn origin_of(outline: &MultiPolygon) -> Point {
    outline
        .0
        .iter()
        .find_map(|polygon| polygon.exterior.first())
        .map_or([0.0, 0.0], |&p| canonical(p))
}

/// A point's place relative to `origin`, rounded to a millionth of a unit. Detail is seeded from
/// it rather than from absolute coordinates, so moving a whole outline keeps its coastline (the
/// rounding absorbs the float error a move adds).
fn key(p: Point, origin: Point) -> (i64, i64) {
    let q = |v: f64| (v * 1e6).round() as i64;
    (q(p[0] - origin[0]), q(p[1] - origin[1]))
}

fn key_bits(k: (i64, i64)) -> u64 {
    mix(k.0 as u64, k.1 as u64)
}

/// SplitMix64's finaliser over the combined state.
pub(crate) fn mix(state: u64, value: u64) -> u64 {
    let mut z = state
        .wrapping_add(value.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A value in `[0, 1)`, exact in `f64`.
pub(crate) fn unit(hash: u64) -> f64 {
    (hash >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// Roughened outlines of a Map's Landmasses and Lakes, recomputed only for Objects whose version
/// or settings changed since they were last asked for.
///
/// Clones share one store, so a background thread can fill it (see [`RoughnessCache::rebuild`])
/// while the editor keeps drawing. While a rebuild is running, an Object whose outline is not
/// ready yet answers with the outline it last had, if the Object itself has not changed since
/// (only the settings did), and otherwise with its stored, un-roughened outline, instead of
/// computing it. A newer rebuild supersedes an older one, which stops at its next outline.
#[derive(Default, Debug, Clone)]
pub struct RoughnessCache {
    shared: Arc<Shared>,
}

#[derive(Default, Debug)]
struct Shared {
    entries: Mutex<HashMap<Id, CacheEntry>>,
    computed: AtomicU64,
    /// Bumped whenever an outline lands, so callers know what they drew is out of date.
    generation: AtomicU64,
    /// How many rebuilds are still running.
    rebuilding: AtomicUsize,
    /// Counts the rebuilds started, so an older one can tell it has been superseded.
    epoch: AtomicU64,
    /// Held while a rebuild ends, so a waiter never misses the wake-up.
    ended: Mutex<()>,
    ended_signal: Condvar,
}

#[derive(Debug)]
struct CacheEntry {
    version: u64,
    roughness: Roughness,
    /// The stored outline `outline` was made from.
    geometry: MultiPolygon,
    outline: Arc<MultiPolygon>,
}

impl Shared {
    fn entries(&self) -> MutexGuard<'_, HashMap<Id, CacheEntry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn store(
        &self,
        id: Id,
        version: u64,
        roughness: Roughness,
        geometry: MultiPolygon,
        outline: Arc<MultiPolygon>,
    ) {
        self.entries().insert(
            id,
            CacheEntry {
                version,
                roughness,
                geometry,
                outline,
            },
        );
        self.computed.fetch_add(1, Ordering::SeqCst);
    }
}

impl RoughnessCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    fn lookup(&self, id: Id, version: u64, roughness: &Roughness) -> Option<Arc<MultiPolygon>> {
        let entries = self.shared.entries();
        let entry = entries.get(&id)?;
        (entry.version == version && entry.roughness == *roughness).then(|| entry.outline.clone())
    }

    /// The roughened outline of Landmass or Lake `id`, with its own Roughness or the Map-wide
    /// default, or `None` when `id` is not in the Map or has no outline. During a rebuild, an
    /// outline that is not ready yet is the stored one.
    pub fn outline(&mut self, map: &Map, id: Id) -> Option<Arc<MultiPolygon>> {
        let version = map.version(id)?;
        let roughness = map.roughness(id)?;
        let crate::Node::Object(object) = map.node(id)? else {
            return None;
        };
        let geometry = object.kind.geometry()?;
        if let Some(outline) = self.lookup(id, version, roughness) {
            return Some(outline);
        }
        if self.is_rebuilding() {
            return Some(
                self.stale(id, version)
                    .unwrap_or_else(|| Arc::new(geometry.clone())),
            );
        }
        let outline = self
            .moved(id, geometry, roughness)
            .unwrap_or_else(|| Arc::new(roughen(geometry, roughness)));
        self.shared.store(
            id,
            version,
            roughness.clone(),
            geometry.clone(),
            outline.clone(),
        );
        Some(outline)
    }

    /// The outline `id` last had under other settings, if the Object has not changed since.
    fn stale(&self, id: Id, version: u64) -> Option<Arc<MultiPolygon>> {
        let entries = self.shared.entries();
        let entry = entries.get(&id)?;
        (entry.version == version).then(|| entry.outline.clone())
    }

    /// The cached outline of `id` shifted along, when `geometry` is the outline it was made from
    /// moved as a whole under the same settings. Roughness does not depend on position, so a drag
    /// need not roughen the outline again on every frame.
    fn moved(
        &self,
        id: Id,
        geometry: &MultiPolygon,
        roughness: &Roughness,
    ) -> Option<Arc<MultiPolygon>> {
        let entries = self.shared.entries();
        let entry = entries.get(&id)?;
        if entry.roughness != *roughness || roughness.is_smooth() {
            return None;
        }
        let offset = entry.geometry.translation_to(geometry)?;
        Some(Arc::new(
            entry.outline.transformed(&Affine::translate(offset)),
        ))
    }

    /// Whether the roughened outline of `id` is computed for its current version and settings.
    pub fn is_ready(&self, map: &Map, id: Id) -> bool {
        let (Some(version), Some(roughness)) = (map.version(id), map.roughness(id)) else {
            return false;
        };
        self.lookup(id, version, roughness).is_some()
    }

    /// Whether a [`Rebuild`] is still running.
    pub fn is_rebuilding(&self) -> bool {
        self.shared.rebuilding.load(Ordering::SeqCst) > 0
    }

    /// Changes whenever a [`Rebuild`] lands an outline or ends, which is when what was drawn from
    /// placeholders is out of date. Outlines computed on demand do not change it.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    /// Blocks until no [`Rebuild`] is running, so every outline asked for afterwards is the
    /// roughened one, not a stored placeholder. Export calls this before it builds its scene.
    /// A rebuild that is neither run nor dropped on another thread would wait forever.
    pub fn wait_until_ready(&self) {
        let mut guard = self
            .shared
            .ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while self.is_rebuilding() {
            guard = self
                .shared
                .ended_signal
                .wait(guard)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Starts rebuilding every outline of `map` that is not ready. Until the returned
    /// [`Rebuild`] is run or dropped, [`RoughnessCache::outline`] answers not-ready Objects with
    /// their stored outlines. It can be sent to another thread.
    pub fn rebuild(&self, map: &Map) -> Rebuild {
        let mut jobs = Vec::new();
        for id in map.ids() {
            let (Some(version), Some(roughness)) = (map.version(id), map.roughness(id)) else {
                continue;
            };
            let Some(crate::Node::Object(object)) = map.node(id) else {
                continue;
            };
            let Some(geometry) = object.kind.geometry() else {
                continue;
            };
            if self.lookup(id, version, roughness).is_none() {
                jobs.push(Job {
                    id,
                    version,
                    roughness: roughness.clone(),
                    geometry: geometry.clone(),
                });
            }
        }
        self.shared.rebuilding.fetch_add(1, Ordering::SeqCst);
        // A rebuild with nothing to compute leaves an older one running.
        let epoch = if jobs.is_empty() {
            self.shared.epoch.load(Ordering::SeqCst)
        } else {
            self.shared.epoch.fetch_add(1, Ordering::SeqCst) + 1
        };
        Rebuild {
            shared: self.shared.clone(),
            jobs,
            epoch,
        }
    }

    /// Forgets Objects that are no longer in `map`.
    pub fn retain(&mut self, map: &Map) {
        self.shared.entries().retain(|&id, _| map.contains(id));
    }

    /// Empties the cache, as when another Map is opened.
    pub fn clear(&mut self) {
        self.shared.entries().clear();
    }

    /// How many outlines this cache has computed, for tests and profiling.
    pub fn computed(&self) -> u64 {
        self.shared.computed.load(Ordering::SeqCst)
    }
}

#[derive(Debug)]
struct Job {
    id: Id,
    version: u64,
    roughness: Roughness,
    geometry: MultiPolygon,
}

/// Outlines still to compute, from [`RoughnessCache::rebuild`]. Owns its inputs, so it can run
/// on a thread of its own. Dropping it unrun ends the rebuild.
#[derive(Debug)]
pub struct Rebuild {
    shared: Arc<Shared>,
    jobs: Vec<Job>,
    /// Which rebuild this is, counting from the first.
    epoch: u64,
}

impl Rebuild {
    /// How many outlines this will compute.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// Whether there is nothing to compute.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Computes each outline, calling `ready` after each lands in the cache and once more when
    /// the rebuild is over. Computing holds no lock, so drawing is never blocked by it. Stops
    /// early, leaving the rest to it, once a newer rebuild has started.
    pub fn run(mut self, mut ready: impl FnMut()) {
        for job in std::mem::take(&mut self.jobs) {
            if self.shared.epoch.load(Ordering::SeqCst) != self.epoch {
                break;
            }
            let outline = Arc::new(roughen(&job.geometry, &job.roughness));
            self.shared
                .store(job.id, job.version, job.roughness, job.geometry, outline);
            self.shared.generation.fetch_add(1, Ordering::SeqCst);
            ready();
        }
        // Ends the rebuild first, so the last repaint sees it over.
        drop(self);
        ready();
    }
}

impl Drop for Rebuild {
    fn drop(&mut self) {
        {
            let _guard = self
                .shared
                .ended
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            self.shared.rebuilding.fetch_sub(1, Ordering::SeqCst);
        }
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.shared.ended_signal.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(size: f64) -> Vec<Point> {
        vec![[0.0, 0.0], [size, 0.0], [size, size], [0.0, size]]
    }

    fn island() -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: square(400.0),
            holes: vec![vec![
                [100.0, 100.0],
                [100.0, 200.0],
                [200.0, 200.0],
                [200.0, 100.0],
            ]],
        }])
    }

    #[test]
    fn smooth_settings_leave_the_outline_unchanged() {
        let outline = island();
        assert_eq!(roughen(&outline, &Roughness::new(0.0, 32.0, 7)), outline);
        assert_eq!(roughen(&outline, &Roughness::new(5.0, 0.0, 7)), outline);
        assert_eq!(
            roughen(&outline, &Roughness::new(f64::NAN, 32.0, 7)),
            outline
        );
    }

    #[test]
    fn stored_vertices_stay_in_order() {
        let outline = island();
        let rough = roughen(&outline, &Roughness::new(8.0, 32.0, 1));
        let ring = &rough.0[0].exterior;
        assert!(ring.len() > 4 * 8);
        let positions: Vec<usize> = square(400.0)
            .iter()
            .map(|v| {
                ring.iter()
                    .position(|p| p == v)
                    .expect("stored vertex kept")
            })
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(rough.0[0].holes.len(), 1);
    }

    #[test]
    fn displacement_stays_within_the_amount() {
        let amount = 6.0;
        let rough = roughen(
            &MultiPolygon(vec![Polygon {
                exterior: square(1000.0),
                holes: vec![],
            }]),
            &Roughness::new(amount, 50.0, 3),
        );
        // Each level adds at most its amplitude, which halves, so the total stays under 2x.
        for p in &rough.0[0].exterior {
            let distance = p[0]
                .abs()
                .min((p[0] - 1000.0).abs())
                .min(p[1].abs())
                .min((p[1] - 1000.0).abs());
            assert!(
                distance <= 2.0 * amount,
                "{p:?} is {distance} from the square"
            );
        }
    }

    #[test]
    fn seeds_give_different_coastlines() {
        let outline = island();
        let a = roughen(&outline, &Roughness::new(8.0, 32.0, 1));
        let b = roughen(&outline, &Roughness::new(8.0, 32.0, 2));
        assert_ne!(a, b);
        assert_eq!(a, roughen(&outline, &Roughness::new(8.0, 32.0, 1)));
    }

    #[test]
    fn moving_a_node_changes_only_its_edges() {
        let settings = Roughness::new(8.0, 32.0, 9);
        let ring = vec![
            [0.0, 0.0],
            [300.0, 0.0],
            [300.0, 300.0],
            [150.0, 400.0],
            [0.0, 300.0],
        ];
        let mut moved = ring.clone();
        moved[3] = [150.0, 450.0];
        let before = roughen_ring(&ring, &settings, [0.0, 0.0]);
        let after = roughen_ring(&moved, &settings, [0.0, 0.0]);
        // Up to and including [300, 300], the edges before the moved node are untouched.
        let end = before.iter().position(|p| *p == [300.0, 300.0]).unwrap();
        assert_eq!(before[..=end], after[..=end]);
    }

    #[test]
    fn waiting_returns_once_the_rebuild_ends() {
        let cache = RoughnessCache::new();
        // Nothing rebuilding: no wait.
        cache.wait_until_ready();
        let map = Map::new(crate::Canvas::new(
            100,
            100,
            crate::Scale::new(1.0, crate::DistanceUnit::Miles),
        ));
        let rebuild = cache.rebuild(&map);
        assert!(cache.is_rebuilding());
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            rebuild.run(|| {});
        });
        cache.wait_until_ready();
        assert!(!cache.is_rebuilding());
        worker.join().unwrap();
    }

    #[test]
    fn an_edge_gets_the_same_detail_in_either_direction() {
        let settings = Roughness::new(8.0, 32.0, 4);
        let (a, b) = ([10.0, 20.0], [310.0, 95.0]);
        let mut forward = Vec::new();
        roughen_edge(a, b, &settings, [0.0, 0.0], &mut forward);
        let mut backward = Vec::new();
        roughen_edge(b, a, &settings, [0.0, 0.0], &mut backward);
        backward.reverse();
        assert_eq!(forward, backward);
    }

    #[test]
    fn moving_an_outline_keeps_its_coastline() {
        let settings = Roughness::new(8.0, 32.0, 5);
        let outline = island();
        let offset = [123.37, -81.91];
        let moved = outline.transformed(&Affine::translate(offset));
        let before = roughen(&outline, &settings);
        let after = roughen(&moved, &settings);
        let shifted = before.transformed(&Affine::translate(offset));
        let (a, b) = (&shifted.0[0].exterior, &after.0[0].exterior);
        assert_eq!(a.len(), b.len());
        for (p, q) in a.iter().zip(b) {
            assert!((p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6);
        }
    }

    #[test]
    fn a_pure_move_reuses_the_cached_outline_without_roughening() {
        let settings = Roughness::new(8.0, 32.0, 5);
        let outline = island();
        let cache = RoughnessCache::new();
        let id = Id::default();
        let rough = Arc::new(roughen(&outline, &settings));
        cache
            .shared
            .store(id, 1, settings.clone(), outline.clone(), rough.clone());

        // A whole move takes the `moved` path: the cached outline, shifted, no `roughen`.
        let offset = [40.5, -12.25];
        let moved = outline.transformed(&Affine::translate(offset));
        let reused = cache.moved(id, &moved, &settings).expect("translation");
        assert_eq!(*reused, rough.transformed(&Affine::translate(offset)));

        // A reshaped outline, or other settings, must be roughened afresh.
        let mut bent = moved.clone();
        bent.0[0].exterior[2][0] += 1.0;
        assert!(cache.moved(id, &bent, &settings).is_none());
        assert!(
            cache
                .moved(id, &moved, &Roughness::new(9.0, 32.0, 5))
                .is_none()
        );
    }
}
