//! Stamp art: rasterised line art, its Silhouette, and the size the Asset is placed at. The
//! renderer uploads these to the atlas; picking tests points against the Silhouette.
//!
//! A Map's embedded copy of an Asset wins over the installed pack's (ADR 0005), so a Map
//! renders the same with its packs missing, removed or updated. Only an Asset the Map does
//! not embed comes from the installed packs.
//!
//! Rasterising an SVG is slow, so [`SpriteStore::poll`] builds sprites on background threads
//! and reports one that is not done yet as [`SpriteState::Pending`], which is not the same as
//! missing and is never cached as such.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, RwLock};

use ssmap_core::asset::{AssetRef, PackLibrary};
use ssmap_core::{Embedded, SpriteShape};

use crate::raster::{Art, ArtError, Raster};
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

/// Where an Asset's sprite stands.
#[derive(Clone)]
pub enum SpriteState {
    /// Being rasterised on a background thread. Ask again later; it is not missing.
    Pending,
    Ready(Arc<Sprite>),
    /// The Asset is unknown or its art is unreadable.
    Missing,
}

/// Loads each Asset's sprite once. Cheap to clone: clones share the cache, so the renderer
/// and picking never load the same Asset twice.
#[derive(Clone)]
pub struct SpriteStore {
    inner: Arc<Inner>,
}

struct Inner {
    /// Swapped whole when packs are installed or deleted.
    library: RwLock<Arc<PackLibrary>>,
    /// Whether [`SpriteStore::poll`] rasterises on a thread of its own rather than inline.
    background: bool,
    state: Mutex<State>,
    /// Signalled when a job ends, for callers waiting on a pending sprite.
    finished: Condvar,
    /// Called each time a raster finishes.
    repaint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

/// Why an Asset has no sprite.
#[derive(Debug, thiserror::Error)]
pub enum SpriteError {
    /// No installed pack or embedded copy has the Asset.
    #[error("Asset {0} is not installed or embedded")]
    Unknown(AssetRef),
    /// The Asset's art or Silhouette is unreadable or invalid.
    #[error("Asset {asset}: {source}")]
    Art {
        asset: AssetRef,
        #[source]
        source: ArtError,
    },
}

/// A sprite being built, or built. `Ready(Err(_))` records an Asset that is missing or whose
/// art failed to load; `Pending` is never that.
#[derive(Clone)]
enum Slot {
    Pending,
    Ready(Result<Arc<Sprite>, Arc<SpriteError>>),
}

type Cache = HashMap<AssetRef, Slot>;

/// Builds a sprite from copies of its inputs, so it never reads state that is being edited.
type Job = Box<dyn FnOnce() -> Result<Sprite, SpriteError> + Send>;

struct State {
    /// The open Map's embedded Assets, which win over the library.
    embedded: Embedded,
    /// Counts changes to `embedded`, so what was uploaded from older art can be dropped.
    generation: u64,
    /// Counts swaps of the library, so installed art built from an older one is dropped.
    library_generation: u64,
    /// Counts finished jobs, so a renderer can tell new art has landed.
    completed: u64,
    embedded_sprites: Cache,
    installed_sprites: Cache,
}

/// Which cache a sprite belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Embedded,
    Installed,
}

impl State {
    fn cache(&mut self, origin: Origin) -> &mut Cache {
        match origin {
            Origin::Embedded => &mut self.embedded_sprites,
            Origin::Installed => &mut self.installed_sprites,
        }
    }

    fn origin(&self, asset: &AssetRef) -> Origin {
        if self.embedded.assets.contains_key(asset) {
            Origin::Embedded
        } else {
            Origin::Installed
        }
    }
}

impl SpriteStore {
    /// A store that rasterises on background threads.
    pub fn new(library: Arc<PackLibrary>) -> Self {
        Self::with(library, true)
    }

    /// A store whose [`SpriteStore::poll`] rasterises before it returns, for frames that
    /// must be complete the first time, such as exports and golden images.
    pub fn synchronous(library: Arc<PackLibrary>) -> Self {
        Self::with(library, false)
    }

    fn with(library: Arc<PackLibrary>, background: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                library: RwLock::new(library),
                background,
                state: Mutex::new(State {
                    embedded: Embedded::default(),
                    generation: 0,
                    library_generation: 0,
                    completed: 0,
                    embedded_sprites: HashMap::new(),
                    installed_sprites: HashMap::new(),
                }),
                finished: Condvar::new(),
                repaint: Mutex::new(None),
            }),
        }
    }

    /// The installed packs as of now. A later [`SpriteStore::set_library`] does not change the
    /// returned library.
    pub fn library(&self) -> Arc<PackLibrary> {
        self.inner.library.read().expect("library lock").clone()
    }

    /// Swaps the installed packs, for after one is installed or deleted. Installed art is
    /// forgotten and built again on demand; the open Map's embedded art is kept.
    /// [`SpriteStore::generation`] moves on, so renderers upload again.
    pub fn set_library(&self, library: Arc<PackLibrary>) {
        let mut state = self.state();
        *self.inner.library.write().expect("library lock") = library;
        // Jobs still running for the old library see the generation move and drop their result.
        state.installed_sprites.clear();
        state.library_generation += 1;
        drop(state);
        self.inner.finished.notify_all();
        let repaint = self.inner.repaint.lock().expect("repaint lock").clone();
        if let Some(repaint) = repaint {
            repaint();
        }
    }

    /// Sets what to call each time a raster finishes, from the thread that built it.
    pub fn set_repaint(&self, repaint: impl Fn() + Send + Sync + 'static) {
        *self.inner.repaint.lock().expect("repaint lock") = Some(Arc::new(repaint));
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
        state.embedded = embedded.clone();
        // Jobs still running for the old art see the generation move and drop their result.
        state.embedded_sprites.clear();
        state.generation += 1;
        drop(state);
        self.inner.finished.notify_all();
        true
    }

    /// Changes whenever the embedded Assets or the library do. Both counters only increase, so
    /// the sum is only meant for equality checks, not as a count of anything.
    pub fn generation(&self) -> u64 {
        let state = self.state();
        state.generation + state.library_generation
    }

    /// How many rasters have finished. A renderer that skipped a pending sprite draws again
    /// when this moves.
    pub fn completed(&self) -> u64 {
        self.state().completed
    }

    /// The sprite of `asset` as the open Map draws it, without waiting: its embedded copy if
    /// it has one, otherwise the installed pack's. A sprite not yet built starts building on
    /// a background thread and reads as [`SpriteState::Pending`] until it lands.
    pub fn poll(&self, asset: &AssetRef) -> SpriteState {
        let (slot, claimed) = {
            let mut state = self.state();
            let origin = state.origin(asset);
            self.claim(&mut state, asset, origin)
        };
        if let Some(claimed) = claimed {
            if self.inner.background {
                let store = self.clone();
                std::thread::spawn(move || claimed.finish(&store));
                return SpriteState::Pending;
            }
            claimed.finish(self);
            return self.poll(asset);
        }
        match slot {
            Slot::Pending => SpriteState::Pending,
            Slot::Ready(Ok(sprite)) => SpriteState::Ready(sprite),
            Slot::Ready(Err(_)) => SpriteState::Missing,
        }
    }

    /// Why `asset` has no sprite, once that is known: it is unknown, or its art failed to
    /// load. `None` while it is pending or if it loaded.
    pub fn error(&self, asset: &AssetRef) -> Option<Arc<SpriteError>> {
        let mut state = self.state();
        let origin = state.origin(asset);
        match state.cache(origin).get(asset) {
            Some(Slot::Ready(Err(error))) => Some(error.clone()),
            _ => None,
        }
    }

    /// Starts building the sprites of `assets` in the background, so they are ready by the
    /// time they are drawn.
    pub fn prefetch<'a>(&self, assets: impl IntoIterator<Item = &'a AssetRef>) {
        for asset in assets {
            self.poll(asset);
        }
    }

    /// How many sprites are being built right now.
    pub fn pending(&self) -> usize {
        let state = self.state();
        [&state.embedded_sprites, &state.installed_sprites]
            .into_iter()
            .flat_map(|cache| cache.values())
            .filter(|slot| matches!(slot, Slot::Pending))
            .count()
    }

    /// Blocks until no sprite is being built, so a frame drawn afterwards has no placeholders.
    /// Export calls this after [`SpriteStore::prefetch`]ing the Assets it draws.
    pub fn wait_until_ready(&self) {
        let mut state = self.state();
        loop {
            let pending = [&state.embedded_sprites, &state.installed_sprites]
                .into_iter()
                .flat_map(|cache| cache.values())
                .any(|slot| matches!(slot, Slot::Pending));
            if !pending {
                return;
            }
            // The lock is held from the look to the wait, so the wake-up is not missed.
            state = self.inner.finished.wait(state).expect("sprite cache lock");
        }
    }

    /// The sprite of `asset` as the open Map draws it, waiting for it if it is being built.
    /// `None` if neither a copy nor an installed Asset exists or the art is unreadable.
    pub fn wait_for(&self, asset: &AssetRef) -> Option<Arc<Sprite>> {
        self.wait_for_origin(asset, false)
    }

    /// The sprite of `asset` from the installed packs only, as the Asset browser shows it.
    /// Blocks until it is built.
    pub fn wait_for_installed(&self, asset: &AssetRef) -> Option<Arc<Sprite>> {
        self.wait_for_origin(asset, true)
    }

    fn wait_for_origin(&self, asset: &AssetRef, installed_only: bool) -> Option<Arc<Sprite>> {
        loop {
            let mut state = self.state();
            let origin = if installed_only {
                Origin::Installed
            } else {
                state.origin(asset)
            };
            let (slot, claimed) = self.claim(&mut state, asset, origin);
            if let Some(claimed) = claimed {
                drop(state);
                claimed.finish(self);
                continue;
            }
            match slot {
                Slot::Ready(sprite) => return sprite.ok(),
                // Another thread is building it: sleep until a job ends, then look again.
                // The lock is held from the look to the wait, so the wake-up is not missed.
                Slot::Pending => drop(self.inner.finished.wait(state).expect("sprite cache lock")),
            }
        }
    }

    /// Looks `asset` up in the cache of `origin`. If it was never asked for, marks it pending
    /// and returns the job that builds it.
    fn claim(
        &self,
        state: &mut State,
        asset: &AssetRef,
        origin: Origin,
    ) -> (Slot, Option<Claimed>) {
        if let Some(slot) = state.cache(origin).get(asset) {
            return (slot.clone(), None);
        }
        let job: Job = match origin {
            Origin::Embedded => embedded_job(&state.embedded, asset),
            Origin::Installed => {
                let library = self.library();
                let asset = asset.clone();
                Box::new(move || load_installed(&library, &asset))
            }
        };
        state.cache(origin).insert(asset.clone(), Slot::Pending);
        let claimed = Claimed {
            asset: asset.clone(),
            origin,
            generation: state.generation,
            library_generation: state.library_generation,
            job,
        };
        (Slot::Pending, Some(claimed))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().expect("sprite cache lock")
    }

    pub fn shape(&self, asset: &AssetRef) -> Option<SpriteShape> {
        self.wait_for(asset).map(|sprite| sprite.shape)
    }

    /// Whether `uv` is inside the Silhouette of `asset`.
    pub fn contains(&self, asset: &AssetRef, uv: [f64; 2]) -> bool {
        self.wait_for(asset)
            .is_some_and(|sprite| sprite.contains(uv))
    }
}

/// A sprite marked pending whose job has not yet run.
struct Claimed {
    asset: AssetRef,
    origin: Origin,
    /// The embedded generation the job was started under.
    generation: u64,
    /// The library generation the job was started under.
    library_generation: u64,
    job: Job,
}

impl Claimed {
    /// Builds the sprite, files it, wakes anyone waiting and asks for a repaint. The result
    /// is dropped if the embedded Assets changed meanwhile.
    fn finish(self, store: &SpriteStore) {
        let sprite = (self.job)().map(Arc::new).map_err(Arc::new);
        if let Err(error) = &sprite
            && !matches!(**error, SpriteError::Unknown(_))
        {
            eprintln!("sprite: {error}");
        }
        {
            let mut state = store.state();
            state.completed += 1;
            let current = match self.origin {
                Origin::Embedded => state.generation == self.generation,
                Origin::Installed => state.library_generation == self.library_generation,
            };
            if current {
                state
                    .cache(self.origin)
                    .insert(self.asset, Slot::Ready(sprite));
            }
        }
        store.inner.finished.notify_all();
        let repaint = store.inner.repaint.lock().expect("repaint lock").clone();
        if let Some(repaint) = repaint {
            repaint();
        }
    }
}

/// The size, in Canvas units, of the box drawn for a Stamp whose Asset is not installed.
pub const PLACEHOLDER_SIZE: [f64; 2] = [64.0, 40.0];
/// The box's border, in pixels.
const PLACEHOLDER_BORDER: u32 = 2;

/// The plain box a Stamp shows when its Asset is not installed, centred on the Anchor. It
/// carries no name: the app writes that over it.
pub fn placeholder() -> Sprite {
    let width = (PLACEHOLDER_SIZE[0] * PIXELS_PER_UNIT) as u32;
    let height = (PLACEHOLDER_SIZE[1] * PIXELS_PER_UNIT) as u32;
    let mut art = Raster::new(width, height);
    let mut silhouette = Raster::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            let edge = x < PLACEHOLDER_BORDER
                || y < PLACEHOLDER_BORDER
                || x >= width - PLACEHOLDER_BORDER
                || y >= height - PLACEHOLDER_BORDER;
            // Premultiplied: a translucent grey fill inside an opaque dark border.
            let pixel = if edge {
                [90, 90, 90, 255]
            } else {
                [32, 32, 32, 90]
            };
            art.pixels[i..i + 4].copy_from_slice(&pixel);
            silhouette.pixels[i..i + 4].copy_from_slice(&[255; 4]);
        }
    }
    Sprite {
        art,
        silhouette,
        shape: SpriteShape {
            size: PLACEHOLDER_SIZE,
            anchor: [0.5, 0.5],
        },
    }
}

fn load_installed(library: &PackLibrary, asset: &AssetRef) -> Result<Sprite, SpriteError> {
    let (pack, entry) = library
        .asset(asset)
        .ok_or_else(|| SpriteError::Unknown(asset.clone()))?;
    let failed = |source| SpriteError::Art {
        asset: asset.clone(),
        source,
    };
    let art = Art::load(&pack.path(&entry.file)).map_err(failed)?;
    let outline = match &entry.silhouette {
        Some(file) => Some(Art::load(&pack.path(file)).map_err(failed)?),
        None => None,
    };
    Ok(build(&art, outline.as_ref(), entry.width, entry.anchor))
}

/// A job over copies of the Asset's record and files.
fn embedded_job(embedded: &Embedded, asset: &AssetRef) -> Job {
    let record = embedded.assets.get(asset).cloned();
    let art = embedded.art(asset).cloned();
    let silhouette = embedded.silhouette(asset).cloned();
    let asset = asset.clone();
    Box::new(move || {
        let unknown = || SpriteError::Unknown(asset.clone());
        let record = record.ok_or_else(unknown)?;
        let art = art.ok_or_else(unknown)?;
        // Embedded art has no file; the error names the Asset instead.
        let failed = |message| SpriteError::Art {
            asset: asset.clone(),
            source: ArtError {
                path: PathBuf::from("<embedded>"),
                message,
            },
        };
        let art = Art::from_bytes(&art).map_err(failed)?;
        let outline = match &silhouette {
            Some(blob) => Some(Art::from_bytes(blob).map_err(failed)?),
            None => None,
        };
        Ok(build(&art, outline.as_ref(), record.width, record.anchor))
    })
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
