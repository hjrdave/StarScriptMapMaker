//! Stamp art: rasterised line art, its Silhouette, and the size the Asset is placed at. The
//! renderer uploads these to the atlas; picking tests points against the Silhouette.
//!
//! A Map's embedded copy of an Asset wins over the installed pack's (ADR 0005), so a Map
//! renders the same with its packs missing, removed or updated. Only an Asset the Map does
//! not embed comes from the installed packs.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;

use ssmap_core::asset::{AssetRef, PackLibrary};
use ssmap_core::{Embedded, SpriteShape};

use crate::raster::{Art, Raster};
use crate::silhouette;

/// SVG art is rasterised this many pixels per Canvas unit. Re-rasterising for the zoom or
/// export scale is a later refinement; this keeps strokes crisp up to 2x on screen.
const PIXELS_PER_UNIT: f64 = 2.0;
/// The widest break in an outline, in pixels, a generated Silhouette still treats as closed.
const SILHOUETTE_GAP: u32 = 2;
/// Alpha at or above which a Silhouette pixel counts as inside.
const INSIDE_ALPHA: u8 = 128;

/// One Asset's art, ready for the atlas.
pub struct Sprite {
    pub art: Raster,
    /// Opaque inside the art's outline, transparent outside.
    pub silhouette: Raster,
    pub shape: SpriteShape,
}

impl Sprite {
    /// Whether the point at `uv` (fractions of the art's size from its top-left) is inside
    /// the Silhouette.
    pub fn contains(&self, uv: [f64; 2]) -> bool {
        let s = &self.silhouette;
        let x = ((uv[0] * f64::from(s.width)) as u32).min(s.width - 1);
        let y = ((uv[1] * f64::from(s.height)) as u32).min(s.height - 1);
        s.pixel(x, y)[3] >= INSIDE_ALPHA
    }
}

/// Loads each Asset's sprite once. Cheap to clone: clones share the cache, so the renderer
/// and picking never load the same Asset twice.
#[derive(Clone)]
pub struct SpriteStore {
    inner: Arc<Inner>,
}

struct Inner {
    library: Arc<PackLibrary>,
    state: Mutex<State>,
}

/// `None` records an Asset that is missing or whose art failed to load.
type Cache = HashMap<AssetRef, Option<Arc<Sprite>>>;

struct State {
    /// The open Map's embedded Assets, which win over the library.
    embedded: Arc<Embedded>,
    /// Counts changes to `embedded`, so what was uploaded from older art can be dropped.
    generation: u64,
    embedded_sprites: Cache,
    installed_sprites: Cache,
    /// Assets a background thread is loading, or has queued.
    loading: HashSet<AssetRef>,
    /// Counts background loads that finished, so the renderer knows to look again.
    loaded: u64,
    /// Called after each background load, to ask the host for a repaint.
    wake: Option<Wake>,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

impl SpriteStore {
    pub fn new(library: Arc<PackLibrary>) -> Self {
        Self {
            inner: Arc::new(Inner {
                library,
                state: Mutex::new(State {
                    embedded: Arc::new(Embedded::default()),
                    generation: 0,
                    embedded_sprites: HashMap::new(),
                    installed_sprites: HashMap::new(),
                    loading: HashSet::new(),
                    loaded: 0,
                    wake: None,
                }),
            }),
        }
    }

    pub fn library(&self) -> &Arc<PackLibrary> {
        &self.inner.library
    }

    /// Sets the Assets the open Map embeds. Returns whether they changed, in which case
    /// [`SpriteStore::generation`] has moved on.
    pub fn set_embedded(&self, embedded: &Embedded) -> bool {
        let mut state = self.state();
        // Blobs are keyed by content hash, so equal keys mean equal files.
        let same = state.embedded.assets == embedded.assets
            && state.embedded.blobs.keys().eq(embedded.blobs.keys());
        if same {
            return false;
        }
        state.embedded = Arc::new(embedded.clone());
        state.embedded_sprites.clear();
        state.generation += 1;
        true
    }

    /// Changes whenever the embedded Assets do.
    pub fn generation(&self) -> u64 {
        self.state().generation
    }

    /// Called after each background load (see [`load_in_background`](Self::load_in_background)).
    pub fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.state().wake = Some(wake);
    }

    /// Counts finished background loads.
    pub fn loaded(&self) -> u64 {
        self.state().loaded
    }

    /// The sprite of `asset` if it has been loaded already; never loads it.
    pub fn get_if_ready(&self, asset: &AssetRef) -> Option<Arc<Sprite>> {
        let state = self.state();
        let cache = if state.embedded.assets.contains_key(asset) {
            &state.embedded_sprites
        } else {
            &state.installed_sprites
        };
        cache.get(asset).cloned().flatten()
    }

    /// Loads and rasterises the sprites of `assets` that are not loaded yet on background
    /// threads, off the cache lock, and calls the wake function as each finishes.
    pub fn load_in_background(&self, assets: Vec<AssetRef>) {
        let queue: Vec<AssetRef> = {
            let mut state = self.state();
            assets
                .into_iter()
                .filter(|asset| {
                    let cached = if state.embedded.assets.contains_key(asset) {
                        state.embedded_sprites.contains_key(asset)
                    } else {
                        state.installed_sprites.contains_key(asset)
                    };
                    !cached && state.loading.insert(asset.clone())
                })
                .collect()
        };
        if queue.is_empty() {
            return;
        }
        let workers = thread::available_parallelism()
            .map_or(1, |n| n.get().saturating_sub(1).max(1))
            .min(queue.len());
        let queue = Arc::new(Mutex::new(queue));
        for _ in 0..workers {
            let (store, queue) = (self.clone(), queue.clone());
            thread::spawn(move || {
                while let Some(asset) = queue.lock().ok().and_then(|mut q| q.pop()) {
                    store.load_one(&asset);
                }
            });
        }
    }

    fn load_one(&self, asset: &AssetRef) {
        let (embedded, generation) = {
            let state = self.state();
            (state.embedded.clone(), state.generation)
        };
        let is_embedded = embedded.assets.contains_key(asset);
        let sprite = if is_embedded {
            load_embedded(&embedded, asset)
        } else {
            self.load(asset)
        }
        .map(Arc::new);
        let mut state = self.state();
        state.loading.remove(asset);
        // Art embedded since this load began belongs to a Map that is no longer open.
        if state.generation == generation {
            let cache = if is_embedded {
                &mut state.embedded_sprites
            } else {
                &mut state.installed_sprites
            };
            cache.entry(asset.clone()).or_insert(sprite);
        }
        state.loaded += 1;
        let wake = state.wake.clone();
        drop(state);
        if let Some(wake) = wake {
            wake();
        }
    }

    /// The sprite of `asset` as the open Map draws it: its embedded copy if it has one,
    /// otherwise the installed pack's. `None` if neither exists or the art is unreadable.
    pub fn get(&self, asset: &AssetRef) -> Option<Arc<Sprite>> {
        let mut state = self.state();
        let state = &mut *state;
        if state.embedded.assets.contains_key(asset) {
            let embedded = &state.embedded;
            return state
                .embedded_sprites
                .entry(asset.clone())
                .or_insert_with(|| load_embedded(embedded, asset).map(Arc::new))
                .clone();
        }
        self.installed(&mut state.installed_sprites, asset)
    }

    /// The sprite of `asset` from the installed packs only, as the Asset browser shows it.
    pub fn get_installed(&self, asset: &AssetRef) -> Option<Arc<Sprite>> {
        self.installed(&mut self.state().installed_sprites, asset)
    }

    fn installed(&self, cache: &mut Cache, asset: &AssetRef) -> Option<Arc<Sprite>> {
        cache
            .entry(asset.clone())
            .or_insert_with(|| self.load(asset).map(Arc::new))
            .clone()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().expect("sprite cache lock")
    }

    pub fn shape(&self, asset: &AssetRef) -> Option<SpriteShape> {
        self.get(asset).map(|sprite| sprite.shape)
    }

    /// Whether `uv` is inside the Silhouette of `asset`.
    pub fn contains(&self, asset: &AssetRef, uv: [f64; 2]) -> bool {
        self.get(asset).is_some_and(|sprite| sprite.contains(uv))
    }

    fn load(&self, asset: &AssetRef) -> Option<Sprite> {
        let (pack, entry) = self.inner.library.asset(asset)?;
        let art = Art::load(&pack.path(&entry.file)).ok()?;
        let outline = match &entry.silhouette {
            Some(file) => Some(Art::load(&pack.path(file)).ok()?),
            None => None,
        };
        Some(build(&art, outline.as_ref(), entry.width, entry.anchor))
    }
}

fn load_embedded(embedded: &Embedded, asset: &AssetRef) -> Option<Sprite> {
    let record = embedded.assets.get(asset)?;
    let art = Art::from_bytes(embedded.art(asset)?).ok()?;
    let outline = match embedded.silhouette(asset) {
        Some(blob) => Some(Art::from_bytes(blob).ok()?),
        None => None,
    };
    Some(build(&art, outline.as_ref(), record.width, record.anchor))
}

fn build(art: &Art, outline: Option<&Art>, width: f64, anchor: [f64; 2]) -> Sprite {
    let pixels = (width * PIXELS_PER_UNIT).round().max(1.0) as u32;
    let raster = art.rasterise(pixels);
    let silhouette = match outline {
        Some(outline) => outline.rasterise(raster.width),
        None => silhouette::generate(&raster, SILHOUETTE_GAP),
    };
    let aspect = f64::from(raster.height) / f64::from(raster.width);
    Sprite {
        shape: SpriteShape {
            size: [width, width * aspect],
            anchor,
        },
        art: raster,
        silhouette,
    }
}
