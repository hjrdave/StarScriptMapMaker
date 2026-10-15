//! Progressive rebuild of derived data after a Map opens (#90): roughened outlines are computed
//! on a background thread while the Canvas draws the stored outlines in their place.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use ssmap_core::{Map, RoughnessCache};

/// The roughness cache, shared between the editor and the background thread.
#[derive(Clone, Default)]
pub struct SharedRoughness(Arc<Mutex<RoughnessCache>>);

impl SharedRoughness {
    pub fn lock(&self) -> MutexGuard<'_, RoughnessCache> {
        // A panic in another holder leaves the cache usable: entries are only ever whole.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A running background computation of every outline that is not ready. Dropping it stops the
/// thread after its current outline.
pub struct OutlineRebuild {
    cancel: Arc<AtomicBool>,
    remaining: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
}

impl OutlineRebuild {
    /// Starts computing the outlines of `map` that `cache` lacks, calling `repaint` as each one
    /// completes.
    pub fn start(map: &Map, cache: &SharedRoughness, repaint: impl Fn() + Send + 'static) -> Self {
        let jobs = cache.lock().pending(map);
        let cancel = Arc::new(AtomicBool::new(false));
        let remaining = Arc::new(AtomicUsize::new(jobs.len()));
        let completed = Arc::new(AtomicUsize::new(0));
        let rebuild = Self {
            cancel: cancel.clone(),
            remaining: remaining.clone(),
            completed: completed.clone(),
        };
        if !jobs.is_empty() {
            let cache = cache.clone();
            thread::spawn(move || {
                {
                    // Whatever ends the loop, the rebuild stops counting as running.
                    let _finish = Finish(remaining);
                    for job in jobs {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let done = job.run();
                        cache.lock().accept(done);
                        completed.fetch_add(1, Ordering::Release);
                        repaint();
                    }
                }
                // The frame for the last outline may have run while the rebuild still counted as
                // running; this one sees it finished.
                repaint();
            });
        }
        rebuild
    }

    /// Whether some outline is still being computed.
    pub fn running(&self) -> bool {
        self.remaining.load(Ordering::Acquire) > 0
    }

    /// How many outlines have completed so far; changes whenever the Canvas should redraw.
    pub fn completed(&self) -> usize {
        self.completed.load(Ordering::Acquire)
    }
}

/// Zeroes the count of outlines left when the thread ends, normally or by panic or cancel.
struct Finish(Arc<AtomicUsize>);

impl Drop for Finish {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Release);
    }
}

impl Drop for OutlineRebuild {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{
        Canvas, DistanceUnit, Id, Landmass, Layer, MapDefaults, MultiPolygon, Node, Object,
        ObjectKind, Op, Polygon, Roughness, Scale, Subtree, roughen,
    };
    use std::time::{Duration, Instant};

    fn island(x: f64) -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: vec![[x, 0.0], [x + 300.0, 0.0], [x + 300.0, 300.0], [x, 300.0]],
            holes: vec![],
        }])
    }

    fn map_with_islands(count: usize) -> (Map, Vec<Id>) {
        let mut map = Map::new(Canvas::new(
            4096,
            3072,
            Scale::new(0.25, DistanceUnit::Miles),
        ));
        let rough = Roughness::new(8.0, 32.0, 3);
        map.apply(Op::SetDefaults {
            defaults: MapDefaults {
                roughness: rough,
                ..Default::default()
            },
        })
        .unwrap();
        let insert = |map: &mut Map, parent: Option<Id>, node: Node| {
            let id = Id::new();
            let index = map.children(parent).len();
            let subtree = Subtree {
                id,
                node,
                children: Vec::new(),
            };
            map.apply(Op::Insert {
                parent,
                index,
                subtree,
            })
            .unwrap();
            id
        };
        let layer = insert(&mut map, None, Node::Layer(Layer::new("Land")));
        let ids = (0..count)
            .map(|i| {
                let kind = ObjectKind::Landmass(Landmass::new(island(i as f64 * 400.0)));
                insert(&mut map, Some(layer), Node::Object(Object::new(kind)))
            })
            .collect();
        (map, ids)
    }

    #[test]
    fn background_result_matches_a_synchronous_build() {
        let (map, ids) = map_with_islands(6);
        let shared = SharedRoughness::default();
        let rebuild = OutlineRebuild::start(&map, &shared, || {});
        let start = Instant::now();
        while rebuild.running() {
            assert!(start.elapsed() < Duration::from_secs(30), "rebuild hung");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(rebuild.completed(), 6);
        let mut sync = RoughnessCache::new();
        let mut background = shared.lock();
        for &id in &ids {
            assert!(background.is_ready(&map, id));
            assert_eq!(
                background.outline(&map, id).unwrap(),
                sync.outline(&map, id).unwrap()
            );
        }
        assert_eq!(background.computed(), 0, "nothing recomputed on the caller");
        let expected = roughen(&island(0.0), &Roughness::new(8.0, 32.0, 3));
        assert_eq!(*background.outline(&map, ids[0]).unwrap(), expected);
    }
}
