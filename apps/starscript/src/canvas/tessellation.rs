//! Tessellates the Canvas content on a thread of its own after a Map opens, so Objects appear
//! as they complete instead of the first frame waiting for all of them.

use std::thread::JoinHandle;

use ssmap_render::{Content, TessellationCache};

pub(crate) struct BackgroundTessellation {
    /// The renderer's cache; `None` without a wgpu device.
    cache: Option<TessellationCache>,
    /// Set when a Map opens; cleared once everything is ready and no outline is still to land.
    armed: bool,
}

impl BackgroundTessellation {
    /// Armed, since the Map the app starts with has just opened.
    pub(crate) fn new(cache: Option<TessellationCache>) -> Self {
        Self { cache, armed: true }
    }

    /// Called each frame with the content about to be drawn. While armed, starts a worker
    /// whenever Objects are not ready and none is running, which includes the content rebuilt
    /// when a roughened outline lands (`outlines_running` says more may follow). `repaint` is
    /// called from the worker as each Object lands and when it ends. Frames while a worker
    /// runs leave out Objects that are not ready; once disarmed, a frame tessellates whatever
    /// it needs on the spot, as edits, picking and export always do.
    pub(crate) fn update(
        &mut self,
        content: &Content,
        outlines_running: bool,
        repaint: impl Fn() + Send + 'static,
    ) -> Option<JoinHandle<()>> {
        let cache = self.cache.as_ref().filter(|_| self.armed)?;
        if cache.is_building() {
            return None;
        }
        if cache.pending(content.groups()) == 0 {
            self.armed = outlines_running;
            return None;
        }
        let rebuild = cache.rebuild(content.groups());
        Some(std::thread::spawn(move || rebuild.run(repaint)))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ssmap_core::{Canvas, Command, DistanceUnit, Id, Map, MultiPolygon, Polygon, Scale};

    use super::*;
    use crate::editor::Editor;

    fn content(objects: usize) -> Content {
        let mut map = Map::new(Canvas::new(
            1000,
            1000,
            Scale::new(1.0, DistanceUnit::Miles),
        ));
        let layer = Id::new();
        map.execute(&Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        })
        .unwrap();
        for i in 0..objects {
            let min = 20.0 * i as f64;
            map.execute(&Command::CreateLandmass {
                id: Id::new(),
                parent: layer,
                index: None,
                geometry: MultiPolygon(vec![Polygon {
                    exterior: vec![
                        [min, min],
                        [min + 10.0, min],
                        [min + 10.0, min + 10.0],
                        [min, min + 10.0],
                    ],
                    holes: Vec::new(),
                }]),
                fill: None,
            })
            .unwrap();
        }
        Editor::new(map).scene_content()
    }

    #[test]
    fn opening_a_map_tessellates_its_objects_one_by_one() {
        let content = content(3);
        let cache = TessellationCache::new();
        let mut tessellation = BackgroundTessellation::new(Some(cache.clone()));
        let landed = Arc::new(AtomicUsize::new(0));
        let repaint = {
            let landed = landed.clone();
            move || {
                landed.fetch_add(1, Ordering::SeqCst);
            }
        };
        assert!(cache.pending(content.groups()) > 0);
        let worker = tessellation
            .update(&content, false, repaint.clone())
            .expect("a worker starts for the new Map");
        // Started before it returns, so the frame being built already leaves gaps.
        worker.join().unwrap();
        assert_eq!(cache.pending(content.groups()), 0);
        assert!(!cache.is_building());
        // A repaint per Object, and one at the end.
        assert_eq!(landed.load(Ordering::SeqCst), cache.len() + 1);
        assert!(cache.len() >= 3);
        // Nothing left to do: no second worker, and the start-up is over.
        assert!(
            tessellation
                .update(&content, false, repaint.clone())
                .is_none()
        );
        assert!(!tessellation.armed);
        // Edits after that are built on the spot, not by a worker.
        assert!(
            tessellation
                .update(&self::content(4), false, repaint)
                .is_none()
        );
    }

    #[test]
    fn finished_work_is_not_started_again_and_a_landing_outline_is() {
        let first = content(2);
        let cache = TessellationCache::new();
        let mut tessellation = BackgroundTessellation::new(Some(cache.clone()));
        tessellation
            .update(&first, true, || {})
            .unwrap()
            .join()
            .unwrap();
        // Outlines are still being computed, so it stays armed but starts nothing.
        assert!(tessellation.update(&first, true, || {}).is_none());
        assert!(tessellation.armed);
        // A rebuilt content with an Object that is not ready starts a worker again.
        let next = self::content(3);
        let worker = tessellation.update(&next, false, || {}).expect("restarted");
        worker.join().unwrap();
        assert_eq!(cache.pending(next.groups()), 0);
    }

    #[test]
    fn without_a_gpu_nothing_starts() {
        let mut tessellation = BackgroundTessellation::new(None);
        assert!(tessellation.update(&content(2), false, || {}).is_none());
    }

    #[test]
    fn each_tab_keeps_its_own_cache_and_switching_back_tessellates_nothing() {
        let first = content(3);
        let second = content(2);
        let (first_cache, second_cache) = (TessellationCache::new(), TessellationCache::new());
        let mut first_tab = BackgroundTessellation::new(Some(first_cache.clone()));
        let mut second_tab = BackgroundTessellation::new(Some(second_cache.clone()));
        first_tab
            .update(&first, false, || {})
            .unwrap()
            .join()
            .unwrap();
        let built = first_cache.generation();
        let objects = first_cache.len();
        assert!(second_cache.is_empty(), "the other tab's cache is separate");

        // The second tab is shown and tessellates its own Map.
        second_tab
            .update(&second, false, || {})
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(first_cache.generation(), built);

        // Back to the first tab: nothing is pending, no worker starts, and no Object is
        // tessellated again (the cache's generation counts every Object that lands).
        assert_eq!(first_cache.pending(first.groups()), 0);
        assert!(first_tab.update(&first, false, || {}).is_none());
        assert_eq!(first_cache.generation(), built);
        assert_eq!(first_cache.len(), objects);
        assert!(!first_cache.same_as(&second_cache));
        assert!(first_cache.same_as(&first_cache.clone()));
    }
}
