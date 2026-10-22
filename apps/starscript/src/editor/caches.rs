//! What the Editor derives from the Map and keeps for reuse: roughened outlines, the indexes
//! picking runs on, and the Stamp art they depend on.

use ssmap_core::{Id, Map, PickIndex, RoughnessCache, StampIndex};
use ssmap_render::SpriteStore;

/// The pick indexes, the Roughness cache and the sprite store, with the generations that tell
/// when each has gone stale.
pub(super) struct Caches {
    /// Rebuilt lazily once the Map has changed.
    pick_index: Option<PickIndex>,
    /// Roughened outlines, shared by the screen and picking so they agree.
    roughness: RoughnessCache,
    /// The cache's generation when the Canvas content and picking were last built.
    outlines_seen: u64,
    /// Stamp art, shared with the Canvas renderer. Absent in headless tests.
    sprites: Option<SpriteStore>,
    stamp_index: Option<StampIndex>,
    /// The store's generation when `stamp_index` was last reset.
    sprites_generation: u64,
}

impl Caches {
    pub(super) fn new() -> Self {
        Self {
            pick_index: None,
            roughness: RoughnessCache::new(),
            outlines_seen: 0,
            sprites: None,
            stamp_index: None,
            sprites_generation: 0,
        }
    }

    pub(super) fn sprites(&self) -> Option<&SpriteStore> {
        self.sprites.as_ref()
    }

    pub(super) fn set_sprites(&mut self, sprites: SpriteStore) {
        self.sprites = Some(sprites);
        self.stamp_index = None;
    }

    pub(super) fn roughness(&mut self) -> &mut RoughnessCache {
        &mut self.roughness
    }

    /// Points the sprite store at the Assets `map` embeds, which win over the installed packs'.
    pub(super) fn sync_embedded(&mut self, map: &Map) {
        if let Some(sprites) = &self.sprites {
            sprites.set_embedded(map.embedded());
            // The library can also change under us, when a pack is installed or deleted.
            let generation = sprites.generation();
            if generation != self.sprites_generation {
                self.sprites_generation = generation;
                self.stamp_index = None;
            }
        }
    }

    /// Drops the pick index once background outlines have landed, and says whether they had,
    /// so the caller can drop what it built from placeholder outlines too.
    pub(super) fn sync_background_outlines(&mut self) -> bool {
        let generation = self.roughness.generation();
        if generation == self.outlines_seen {
            return false;
        }
        self.outlines_seen = generation;
        self.pick_index = None;
        true
    }

    /// Drops roughened outlines of Objects no longer in `map`.
    pub(super) fn retain(&mut self, map: &Map) {
        self.roughness.retain(map);
    }

    /// Computes every roughened outline of `map` on a thread of its own.
    pub(super) fn start_background_roughness(
        &mut self,
        map: &Map,
        repaint: impl Fn() + Send + 'static,
    ) {
        let rebuild = self.roughness.rebuild(map);
        if rebuild.is_empty() {
            return;
        }
        std::thread::spawn(move || rebuild.run(repaint));
    }

    pub(super) fn outlines_rebuilding(&self) -> bool {
        self.roughness.is_rebuilding()
    }

    /// The topmost unlocked Landmass or Lake of `map` drawn at `point`.
    pub(super) fn pick_outline(&mut self, map: &Map, point: [f64; 2]) -> Option<Id> {
        let index = match &mut self.pick_index {
            Some(index) if index.is_current(map) => index,
            slot => slot.insert(PickIndex::build(map, &mut self.roughness)),
        };
        index.pick(point)
    }

    /// The topmost unlocked Stamp of `map` whose Silhouette is under `point`.
    pub(super) fn pick_stamp(&mut self, map: &Map, point: [f64; 2]) -> Option<Id> {
        let sprites = self.sprites.as_ref()?;
        let index = match &mut self.stamp_index {
            Some(index) if index.is_current(map) => index,
            slot => slot.insert(StampIndex::build(map, |asset| sprites.shape(asset))),
        };
        index.pick(point, |asset, uv| sprites.contains(asset, uv))
    }
}
