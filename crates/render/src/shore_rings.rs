//! The rings ripples and shading follow, offset from roughened outlines. Offsetting a rough
//! outline is slow, so rings are kept between builds, and the editor can have them computed
//! on a thread of its own while it draws the rings it had (see [`RingWorker`]).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use ssmap_core::{Affine, Id, MultiPolygon, ShoreStyle, land};

/// How many bands approximate the inward glow.
pub(crate) const SHADING_BANDS: u32 = 6;

/// How far out each ripple line runs, nearest first. Empty when ripples draw nothing.
pub(crate) fn ripple_distances(shore: &ShoreStyle) -> Vec<f64> {
    let ripples = &shore.ripples;
    if ripples.count == 0 || ripples.width <= 0.0 {
        return Vec::new();
    }
    (1..=ripples.count)
        .map(|k| ripples.spacing * f64::from(k))
        .collect()
}

/// How far in each shading band runs (negative), nearest the coast first. Empty when shading
/// draws nothing.
pub(crate) fn shading_distances(shore: &ShoreStyle) -> Vec<f64> {
    let shading = &shore.shading;
    if shading.width <= 0.0 || shading.color.a == 0 {
        return Vec::new();
    }
    (0..SHADING_BANDS)
        .map(|i| -(f64::from(i) + 0.5) / f64::from(SHADING_BANDS) * shading.width)
        .collect()
}

/// `outline` offset by `distance`, grown from the furthest of `kept` on the same side that
/// falls short of it: kept rings are smoother, so far quicker to offset, and growing by `a`
/// then `b` is growing by `a + b`.
fn grow(outline: &MultiPolygon, kept: &[OffsetRing], distance: f64) -> MultiPolygon {
    let (base, base_distance) = kept
        .iter()
        .filter(|kept| {
            kept.distance.signum() == distance.signum() && kept.distance.abs() < distance.abs()
        })
        .max_by(|a, b| a.distance.abs().total_cmp(&b.distance.abs()))
        .map_or((outline, 0.0), |kept| (&*kept.ring, kept.distance));
    land::offset(base, distance - base_distance)
}

/// Every ring of `distances` around `outline`, nearest first on each side so each grows from
/// the one before.
fn grow_all(outline: &MultiPolygon, distances: &[f64]) -> Vec<OffsetRing> {
    let mut order = distances.to_vec();
    order.sort_by(|a, b| a.abs().total_cmp(&b.abs()));
    let mut rings: Vec<OffsetRing> = Vec::with_capacity(order.len());
    for distance in order {
        if rings.iter().any(|kept| same(kept.distance, distance)) {
            continue;
        }
        let ring = Arc::new(grow(outline, &rings, distance));
        rings.push(OffsetRing {
            distance,
            ring,
            used: false,
        });
    }
    rings
}

fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

/// The offset rings of one Object's roughened outline, kept between builds.
struct ShoreRings {
    /// The outline `rings` were made from.
    outline: Arc<MultiPolygon>,
    rings: Vec<OffsetRing>,
    /// Rings of an earlier outline, drawn in place of rings still being computed.
    stale: Vec<OffsetRing>,
    /// Whether the build in progress has drawn the Object.
    drawn: bool,
    /// Counts the changes to what [`ShoreRings::ring`] returns.
    serial: u64,
}

#[derive(Clone)]
struct OffsetRing {
    distance: f64,
    ring: Arc<MultiPolygon>,
    /// Whether the build in progress has drawn it.
    used: bool,
}

impl ShoreRings {
    fn new(outline: &Arc<MultiPolygon>) -> Self {
        Self {
            outline: outline.clone(),
            rings: Vec::new(),
            stale: Vec::new(),
            drawn: false,
            serial: 0,
        }
    }

    /// Makes the rings fit `outline`: moved along with it if it is the old outline moved,
    /// otherwise set aside as stale.
    fn follow(&mut self, outline: &Arc<MultiPolygon>) {
        if Arc::ptr_eq(&self.outline, outline) {
            return;
        }
        match self.outline.translation_to(outline) {
            Some([0.0, 0.0]) => {}
            Some(offset) => {
                let shift = Affine::translate(offset);
                for kept in self.rings.iter_mut().chain(&mut self.stale) {
                    kept.ring = Arc::new(kept.ring.transformed(&shift));
                }
            }
            None => {
                // Rings still being computed keep showing the last complete set.
                if !self.rings.is_empty() {
                    self.stale = std::mem::take(&mut self.rings);
                }
            }
        }
        self.outline = outline.clone();
        self.serial += 1;
    }

    fn has_all(&self, distances: &[f64]) -> bool {
        distances
            .iter()
            .all(|&d| self.rings.iter().any(|kept| same(kept.distance, d)))
    }

    fn install(&mut self, rings: Vec<OffsetRing>) {
        self.serial += 1;
        for ring in rings {
            if !self
                .rings
                .iter()
                .any(|kept| same(kept.distance, ring.distance))
            {
                self.rings.push(ring);
            }
        }
    }

    /// The ring at `distance`: the current one, or while that is being computed the stale ring
    /// nearest it on the same side. `None` when there is neither.
    fn ring(&mut self, distance: f64) -> Option<Arc<MultiPolygon>> {
        if let Some(kept) = self
            .rings
            .iter_mut()
            .find(|kept| same(kept.distance, distance))
        {
            kept.used = true;
            return Some(kept.ring.clone());
        }
        self.stale
            .iter()
            .filter(|kept| kept.distance.signum() == distance.signum())
            .min_by(|a, b| {
                (a.distance - distance)
                    .abs()
                    .total_cmp(&(b.distance - distance).abs())
            })
            .map(|kept| kept.ring.clone())
    }

    /// Forgets the rings the finished build did not draw, and says whether the Object is still
    /// drawn at all.
    fn end_build(&mut self) -> bool {
        self.rings.retain(|kept| kept.used);
        for kept in &mut self.rings {
            kept.used = false;
        }
        std::mem::take(&mut self.drawn)
    }
}

/// Every Object's kept rings, and the worker computing new ones, if there is one.
#[derive(Default)]
pub(crate) struct RingStore {
    objects: HashMap<Id, ShoreRings>,
    worker: Option<RingWorker>,
}

impl RingStore {
    /// Computes rings on a thread of its own from now on. `repaint` is called from it as each
    /// Object's rings land.
    pub(crate) fn compute_in_background(&mut self, repaint: impl Fn() + Send + Sync + 'static) {
        self.worker = Some(RingWorker::spawn(Box::new(repaint)));
    }

    /// Changes whenever background rings land, which is when what was drawn is out of date.
    pub(crate) fn generation(&self) -> u64 {
        self.worker.as_ref().map_or(0, RingWorker::generation)
    }

    /// Makes sure `id` has every ring `shore` draws around `outline`. Without a worker, or when
    /// `complete`, missing rings are computed now; otherwise they are queued, and until they
    /// land the Object draws the rings it had.
    pub(crate) fn prepare(
        &mut self,
        id: Id,
        outline: &Arc<MultiPolygon>,
        shore: &ShoreStyle,
        complete: bool,
    ) {
        let entry = self
            .objects
            .entry(id)
            .or_insert_with(|| ShoreRings::new(outline));
        entry.follow(outline);
        entry.drawn = true;
        let mut distances = ripple_distances(shore);
        distances.extend(shading_distances(shore));
        if entry.has_all(&distances) {
            return;
        }
        if let Some(worker) = &self.worker {
            match worker.take(id, outline) {
                Some(Landed::Current(rings)) => {
                    entry.install(rings);
                    if entry.has_all(&distances) {
                        entry.stale.clear();
                        return;
                    }
                }
                // Rings of an outline since replaced are still nearer than older ones.
                Some(Landed::Earlier(rings)) => entry.stale = rings,
                None => {}
            }
            if !complete {
                worker.request(id, outline, distances);
                return;
            }
        }
        let missing: Vec<f64> = distances
            .iter()
            .copied()
            .filter(|&d| !entry.rings.iter().any(|kept| same(kept.distance, d)))
            .collect();
        entry.serial += 1;
        for distance in missing {
            let ring = Arc::new(grow(outline, &entry.rings, distance));
            entry.rings.push(OffsetRing {
                distance,
                ring,
                used: false,
            });
        }
        entry.stale.clear();
    }

    /// The ring at `distance` around `id`'s outline, after [`RingStore::prepare`]. `None` while
    /// it is being computed and no earlier ring can stand in.
    pub(crate) fn ring(&mut self, id: Id, distance: f64) -> Option<Arc<MultiPolygon>> {
        self.objects.get_mut(&id)?.ring(distance)
    }

    /// Changes whenever the rings [`RingStore::ring`] returns for `id` do. Rings that landed in
    /// the background count only once [`RingStore::prepare`] has picked them up.
    pub(crate) fn serial(&self, id: Id) -> u64 {
        self.objects.get(&id).map_or(0, |entry| entry.serial)
    }

    /// Ends a build: forgets the rings of Objects and distances it did not draw.
    pub(crate) fn end_build(&mut self) {
        self.objects.retain(|_, entry| entry.end_build());
        if let Some(worker) = &self.worker {
            worker.retain(|id| self.objects.contains_key(&id));
        }
    }

    pub(crate) fn clear(&mut self) {
        self.objects.clear();
        if let Some(worker) = &self.worker {
            worker.retain(|_| false);
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.objects.len()
    }
}

struct Job {
    outline: Arc<MultiPolygon>,
    distances: Vec<f64>,
}

/// Rings the worker finished, for the outline now drawn or an earlier one.
enum Landed {
    Current(Vec<OffsetRing>),
    Earlier(Vec<OffsetRing>),
}

struct Done {
    outline: Arc<MultiPolygon>,
    rings: Vec<OffsetRing>,
}

#[derive(Default)]
struct Queue {
    /// The newest request for each Object; an older one still waiting is replaced.
    waiting: HashMap<Id, Job>,
    /// The request being computed.
    running: Option<(Id, Arc<MultiPolygon>)>,
    done: HashMap<Id, Done>,
    stop: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    generation: AtomicU64,
    repaint: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A thread computing offset rings, newest request per Object first served. Dropping it stops
/// the thread once its current job is done.
struct RingWorker {
    shared: Arc<Shared>,
}

impl RingWorker {
    #[allow(clippy::expect_used, reason = "the shore ring thread starts")]
    fn spawn(repaint: Box<dyn Fn() + Send + Sync>) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
            repaint,
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("shore rings".into())
            .spawn(move || run(&worker))
            .expect("the shore ring thread starts");
        Self { shared }
    }

    fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    /// Asks for the rings at `distances` around `outline`, unless they are already asked for.
    fn request(&self, id: Id, outline: &Arc<MultiPolygon>, distances: Vec<f64>) {
        let mut queue = self.shared.queue();
        let asked = |job_outline: &Arc<MultiPolygon>, job_distances: &[f64]| {
            Arc::ptr_eq(job_outline, outline)
                && distances
                    .iter()
                    .all(|&d| job_distances.iter().any(|&j| same(j, d)))
        };
        if queue
            .waiting
            .get(&id)
            .is_some_and(|job| asked(&job.outline, &job.distances))
        {
            return;
        }
        if queue
            .running
            .as_ref()
            .is_some_and(|(running, job_outline)| {
                *running == id && Arc::ptr_eq(job_outline, outline)
            })
            && !queue.waiting.contains_key(&id)
        {
            return;
        }
        queue.waiting.insert(
            id,
            Job {
                outline: outline.clone(),
                distances,
            },
        );
        self.shared.wake.notify_one();
    }

    /// The finished rings of `id`, moved along if `outline` is theirs moved.
    fn take(&self, id: Id, outline: &Arc<MultiPolygon>) -> Option<Landed> {
        let done = self.shared.queue().done.remove(&id)?;
        if Arc::ptr_eq(&done.outline, outline) {
            return Some(Landed::Current(done.rings));
        }
        let Some(offset) = done.outline.translation_to(outline) else {
            return Some(Landed::Earlier(done.rings));
        };
        let shift = Affine::translate(offset);
        let rings = done
            .rings
            .into_iter()
            .map(|kept| OffsetRing {
                ring: Arc::new(kept.ring.transformed(&shift)),
                ..kept
            })
            .collect();
        Some(Landed::Current(rings))
    }

    /// Drops requests and results for Objects `keep` turns down.
    fn retain(&self, keep: impl Fn(Id) -> bool) {
        let mut queue = self.shared.queue();
        queue.waiting.retain(|&id, _| keep(id));
        queue.done.retain(|&id, _| keep(id));
    }
}

impl Drop for RingWorker {
    fn drop(&mut self) {
        self.shared.queue().stop = true;
        self.shared.wake.notify_all();
    }
}

fn run(shared: &Shared) {
    loop {
        let (id, job) = {
            let mut queue = shared.queue();
            loop {
                if queue.stop {
                    return;
                }
                if let Some(&id) = queue.waiting.keys().next()
                    && let Some(job) = queue.waiting.remove(&id)
                {
                    queue.running = Some((id, job.outline.clone()));
                    break (id, job);
                }
                queue = shared
                    .wake
                    .wait(queue)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let rings = grow_all(&job.outline, &job.distances);
        {
            let mut queue = shared.queue();
            queue.running = None;
            // Delivered even when a newer outline is waiting: the editor draws them in place of
            // older rings until the newer ones land.
            queue.done.insert(
                id,
                Done {
                    outline: job.outline,
                    rings,
                },
            );
        }
        shared.generation.fetch_add(1, Ordering::SeqCst);
        (shared.repaint)();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use ssmap_core::{Polygon, Roughness, roughen};

    use super::*;

    fn rough_island(seed: u32) -> Arc<MultiPolygon> {
        let square = MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [400.0, 0.0], [400.0, 400.0], [0.0, 400.0]],
            holes: Vec::new(),
        }]);
        Arc::new(roughen(&square, &Roughness::new(8.0, 16.0, seed)))
    }

    fn shore() -> ShoreStyle {
        ShoreStyle::landmass_default()
    }

    fn distances() -> Vec<f64> {
        let mut all = ripple_distances(&shore());
        all.extend(shading_distances(&shore()));
        all
    }

    /// A store computing in the background, and what it sends as rings land.
    fn background() -> (RingStore, mpsc::Receiver<()>) {
        let (sender, landed) = mpsc::channel();
        let sender = Mutex::new(sender);
        let mut store = RingStore::default();
        store.compute_in_background(move || {
            let _ = sender.lock().unwrap().send(());
        });
        (store, landed)
    }

    fn wait(landed: &mpsc::Receiver<()>) {
        landed
            .recv_timeout(Duration::from_secs(30))
            .expect("the rings land");
    }

    #[test]
    fn rings_are_kept_until_the_outline_changes() {
        let mut store = RingStore::default();
        let id = Id::new();
        let outline = rough_island(3);
        store.prepare(id, &outline, &shore(), false);
        let ring = store.ring(id, 12.0).expect("computed at once");
        store.prepare(id, &outline, &shore(), false);
        assert!(Arc::ptr_eq(&ring, &store.ring(id, 12.0).unwrap()));

        let mut reshaped = (*outline).clone();
        reshaped.0[0].exterior[5][1] -= 40.0;
        let reshaped = Arc::new(reshaped);
        store.prepare(id, &reshaped, &shore(), false);
        let fresh = store.ring(id, 12.0).unwrap();
        assert!(!Arc::ptr_eq(&ring, &fresh));
        assert_eq!(*fresh, land::offset(&reshaped, 12.0));
    }

    #[test]
    fn a_moved_outline_moves_its_kept_rings() {
        let mut store = RingStore::default();
        let id = Id::new();
        let outline = rough_island(3);
        store.prepare(id, &outline, &shore(), false);
        let ring = store.ring(id, -4.5).unwrap();
        let shift = Affine::translate([30.0, -12.5]);
        let moved = Arc::new(outline.transformed(&shift));
        store.prepare(id, &moved, &shore(), false);
        assert_eq!(*store.ring(id, -4.5).unwrap(), ring.transformed(&shift));
    }

    #[test]
    fn a_ring_grown_from_a_kept_ring_matches_one_grown_at_once() {
        let outline = rough_island(3);
        let chained = grow_all(&outline, &[12.0, 24.0]);
        let direct = land::offset(&outline, 24.0);
        let (a, b) = (chained[1].ring.bounds().unwrap(), direct.bounds().unwrap());
        for (p, q) in [(a.min, b.min), (a.max, b.max)] {
            assert!((p[0] - q[0]).abs() < 1.0 && (p[1] - q[1]).abs() < 1.0);
        }
    }

    #[test]
    fn objects_a_build_does_not_draw_are_dropped() {
        let mut store = RingStore::default();
        let id = Id::new();
        store.prepare(id, &rough_island(3), &shore(), false);
        store.ring(id, 12.0);
        store.end_build();
        assert_eq!(store.len(), 1);
        assert_eq!(store.objects[&id].rings.len(), 1, "undrawn distances go");
        store.end_build();
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn background_rings_land_and_stale_ones_stand_in_meanwhile() {
        let (mut store, landed) = background();
        let id = Id::new();
        let first = rough_island(3);
        store.prepare(id, &first, &shore(), false);
        assert!(store.ring(id, 12.0).is_none(), "nothing to draw yet");
        wait(&landed);
        assert_eq!(store.generation(), 1);
        store.prepare(id, &first, &shore(), false);
        let old = store.ring(id, 12.0).expect("landed");
        assert!(store.objects[&id].has_all(&distances()));

        // A new outline draws the old rings until its own land.
        let second = rough_island(4);
        store.prepare(id, &second, &shore(), false);
        assert!(Arc::ptr_eq(&store.ring(id, 12.0).unwrap(), &old));
        wait(&landed);
        store.prepare(id, &second, &shore(), false);
        let new = store.ring(id, 12.0).unwrap();
        assert_eq!(*new, *grow_all(&second, &[12.0])[0].ring);
        assert!(store.objects[&id].stale.is_empty());
    }

    #[test]
    fn a_complete_build_never_waits_on_the_worker() {
        let (mut store, _landed) = background();
        let id = Id::new();
        let outline = rough_island(3);
        store.prepare(id, &outline, &shore(), true);
        assert!(store.objects[&id].has_all(&distances()));
        assert_eq!(*store.ring(id, 12.0).unwrap(), land::offset(&outline, 12.0));
    }
}
