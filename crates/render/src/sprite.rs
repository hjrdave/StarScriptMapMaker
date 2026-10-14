//! Stamp art loaded from the installed packs: rasterised line art, its Silhouette, and the
//! size the Asset is placed at. The renderer uploads these to the atlas; picking tests
//! points against the Silhouette.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ssmap_core::SpriteShape;
use ssmap_core::asset::{AssetRef, PackLibrary};

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
    /// `None` records an Asset that is missing or whose art failed to load.
    sprites: Mutex<HashMap<AssetRef, Option<Arc<Sprite>>>>,
}

impl SpriteStore {
    pub fn new(library: Arc<PackLibrary>) -> Self {
        Self {
            inner: Arc::new(Inner {
                library,
                sprites: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn library(&self) -> &Arc<PackLibrary> {
        &self.inner.library
    }

    /// The sprite of `asset`, or `None` if no installed pack has it or its art is unreadable.
    pub fn get(&self, asset: &AssetRef) -> Option<Arc<Sprite>> {
        let mut sprites = self.inner.sprites.lock().expect("sprite cache lock");
        sprites
            .entry(asset.clone())
            .or_insert_with(|| self.load(asset).map(Arc::new))
            .clone()
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
        let width = (entry.width * PIXELS_PER_UNIT).round().max(1.0) as u32;
        let raster = art.rasterise(width);
        let silhouette = match &entry.silhouette {
            Some(file) => {
                let outline = Art::load(&pack.path(file)).ok()?;
                outline.rasterise(raster.width)
            }
            None => silhouette::generate(&raster, SILHOUETTE_GAP),
        };
        let aspect = f64::from(raster.height) / f64::from(raster.width);
        Some(Sprite {
            shape: SpriteShape {
                size: [entry.width, entry.width * aspect],
                anchor: entry.anchor,
            },
            art: raster,
            silhouette,
        })
    }
}
