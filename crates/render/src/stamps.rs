//! Stamp art on the GPU and under the cursor (ADR 0003): rasterising Assets into the atlas,
//! and finding the Stamp at a point with an rstar broad phase and a pixel-accurate Silhouette
//! hit test.

use std::collections::HashMap;
use std::sync::Arc;

use rstar::primitives::{GeomWithData, Rectangle};
use rstar::{AABB, RTree};
use ssmap_core::asset::PackLibrary;
use ssmap_core::{AssetRef, Id, Map, Node, ObjectKind, Stamp};

use crate::atlas::{Atlas, AtlasEntry, page_size_for};
use crate::raster::{Art, Raster};
use crate::silhouette;

/// Pixels rasterised per Canvas unit of an SVG Asset's default width, so art stays crisp up
/// to about this much zoom.
const RASTER_DENSITY: f32 = 4.0;
const MIN_RASTER_WIDTH: u32 = 32;
const MAX_RASTER_WIDTH: u32 = 1024;
/// The widest break in an outline, in pixels, that a generated Silhouette still treats as closed.
const SILHOUETTE_GAP: u32 = 2;
/// Alpha at or above which a Silhouette pixel counts as under the cursor.
const HIT_ALPHA: u8 = 128;

/// What picking needs to know about an Asset, without any GPU state.
#[derive(Clone, Debug)]
pub struct StampShape {
    /// The art's size in Canvas units at scale 1.
    pub size: [f32; 2],
    /// The ground point, as fractions of the art's width and height from its top-left.
    pub anchor: [f32; 2],
    /// The Silhouette, or the art itself when it has none: an opaque pixel is hit.
    pub mask: Arc<Raster>,
}

impl StampShape {
    /// Whether `point` (Canvas units) lies on the Stamp's mask.
    pub fn hits(&self, stamp: &Stamp, point: [f64; 2]) -> bool {
        let (dx, dy) = (point[0] - stamp.position[0], point[1] - stamp.position[1]);
        let (sin, cos) = stamp.rotation.to_radians().sin_cos();
        // Undo the clockwise rotation, then the flip.
        let (mut x, y) = (dx * cos + dy * sin, -dx * sin + dy * cos);
        if stamp.flip {
            x = -x;
        }
        let u = f64::from(self.anchor[0]) + x / (f64::from(self.size[0]) * stamp.scale);
        let v = f64::from(self.anchor[1]) + y / (f64::from(self.size[1]) * stamp.scale);
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            return false;
        }
        let px = ((u * f64::from(self.mask.width)) as u32).min(self.mask.width - 1);
        let py = ((v * f64::from(self.mask.height)) as u32).min(self.mask.height - 1);
        self.mask.pixel(px, py)[3] >= HIT_ALPHA
    }

    /// The Canvas-space corners of the Stamp's quad, in order around it.
    pub fn corners(&self, stamp: &Stamp) -> [[f64; 2]; 4] {
        let (sin, cos) = stamp.rotation.to_radians().sin_cos();
        let flip = if stamp.flip { -1.0 } else { 1.0 };
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|[cx, cy]| {
            let x = (cx - f64::from(self.anchor[0])) * f64::from(self.size[0]) * stamp.scale * flip;
            let y = (cy - f64::from(self.anchor[1])) * f64::from(self.size[1]) * stamp.scale;
            [
                stamp.position[0] + x * cos - y * sin,
                stamp.position[1] + x * sin + y * cos,
            ]
        })
    }
}

/// An Asset ready to draw: where its art and Silhouette sit in the atlas, and its shape.
#[derive(Clone, Debug)]
pub struct StampSprite {
    pub art: AtlasEntry,
    /// A blank entry when the Asset has no Silhouette to fill.
    pub silhouette: AtlasEntry,
    /// `[u0, v0, u1, v1]` of each on its page.
    pub art_uv: [f32; 4],
    pub silhouette_uv: [f32; 4],
    pub shape: StampShape,
}

/// Why an Asset has no sprite.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum SpriteError {
    #[error("{0} is not in any loaded pack")]
    Missing(AssetRef),
    #[error("{asset}: {message}")]
    Unusable { asset: AssetRef, message: String },
}

/// The atlas holding every Asset a Map has drawn, filled on demand.
pub struct StampAtlas {
    atlas: Atlas,
    blank: Option<AtlasEntry>,
    /// Failed Assets are remembered, so a missing pack costs one lookup per frame, not a reload.
    sprites: HashMap<AssetRef, Result<StampSprite, SpriteError>>,
}

impl StampAtlas {
    pub fn new(limits: &wgpu::Limits) -> Self {
        Self {
            atlas: Atlas::new(page_size_for(limits)),
            blank: None,
            sprites: HashMap::new(),
        }
    }

    pub fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    /// The sprite for `asset`, rasterising it into the atlas on first use.
    pub fn sprite(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &PackLibrary,
        asset: &AssetRef,
    ) -> Result<&StampSprite, &SpriteError> {
        if !self.sprites.contains_key(asset) {
            let built = self.build(device, queue, library, asset);
            self.sprites.insert(asset.clone(), built);
        }
        self.sprites[asset].as_ref()
    }

    /// The sprite for `asset` if it has been loaded already.
    pub fn loaded(&self, asset: &AssetRef) -> Option<&StampSprite> {
        self.sprites.get(asset)?.as_ref().ok()
    }

    /// Forgets failures, so Assets from a pack that has since appeared load on the next draw.
    pub fn retry_failed(&mut self) {
        self.sprites.retain(|_, sprite| sprite.is_ok());
    }

    fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &PackLibrary,
        asset: &AssetRef,
    ) -> Result<StampSprite, SpriteError> {
        let (pack, entry) = library
            .asset(asset)
            .ok_or_else(|| SpriteError::Missing(asset.clone()))?;
        let unusable = |message: String| SpriteError::Unusable {
            asset: asset.clone(),
            message,
        };
        let art = Art::load(&pack.path(&entry.file)).map_err(|err| unusable(err.to_string()))?;
        let [art_width, art_height] = art.size();
        let width = ((entry.width as f32 * RASTER_DENSITY).ceil() as u32)
            .clamp(MIN_RASTER_WIDTH, MAX_RASTER_WIDTH);
        let raster = art.rasterise(width);
        let mask = match &entry.silhouette {
            Some(file) => Art::load(&pack.path(file))
                .map_err(|err| unusable(err.to_string()))?
                .rasterise(width),
            None => silhouette::generate(&raster, SILHOUETTE_GAP),
        };
        let blank = self
            .blank_entry(device, queue)
            .map_err(|err| unusable(err.to_string()))?;
        let art_entry = self
            .atlas
            .insert(device, queue, &raster)
            .map_err(|err| unusable(err.to_string()))?;
        let silhouette = if entry.silhouette.is_some() || mask.pixels.iter().any(|&p| p != 0) {
            self.atlas
                .insert(device, queue, &mask)
                .map_err(|err| unusable(err.to_string()))?
        } else {
            blank
        };
        let size_width = entry.width as f32;
        let page_size = self.atlas.page_size();
        Ok(StampSprite {
            art: art_entry,
            silhouette,
            art_uv: art_entry.uv(page_size),
            silhouette_uv: silhouette.uv(page_size),
            shape: StampShape {
                size: [size_width, size_width * art_height / art_width],
                anchor: [entry.anchor[0] as f32, entry.anchor[1] as f32],
                mask: Arc::new(mask),
            },
        })
    }

    fn blank_entry(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<AtlasEntry, crate::atlas::AtlasError> {
        if let Some(blank) = self.blank {
            return Ok(blank);
        }
        let blank = self.atlas.insert(device, queue, &Raster::new(1, 1))?;
        self.blank = Some(blank);
        Ok(blank)
    }
}

type Indexed = GeomWithData<Rectangle<[f64; 2]>, Entry>;

#[derive(Clone)]
struct Entry {
    id: Id,
    /// Position in the draw order; higher draws above.
    rank: usize,
    stamp: Stamp,
    shape: StampShape,
}

/// A spatial index of the Map's visible, unlocked Stamps. It is a snapshot: rebuild it when
/// [`StampIndex::is_current`] says the Map has changed.
pub struct StampIndex {
    revision: u64,
    tree: RTree<Indexed>,
}

impl StampIndex {
    /// Indexes the Stamps of `map` whose shape `shape_of` knows.
    pub fn build<'a>(map: &Map, shape_of: impl Fn(&AssetRef) -> Option<&'a StampShape>) -> Self {
        let entries = map
            .draw_order()
            .into_iter()
            .enumerate()
            .filter_map(|(rank, id)| {
                let Some(Node::Object(object)) = map.node(id) else {
                    return None;
                };
                let ObjectKind::Stamp(stamp) = &object.kind else {
                    return None;
                };
                if map.is_locked(id) {
                    return None;
                }
                let shape = shape_of(&stamp.asset)?;
                let corners = shape.corners(stamp);
                let (mut min, mut max) = (corners[0], corners[0]);
                for corner in corners {
                    min = [min[0].min(corner[0]), min[1].min(corner[1])];
                    max = [max[0].max(corner[0]), max[1].max(corner[1])];
                }
                Some(GeomWithData::new(
                    Rectangle::from_corners(min, max),
                    Entry {
                        id,
                        rank,
                        stamp: stamp.clone(),
                        shape: shape.clone(),
                    },
                ))
            })
            .collect();
        Self {
            revision: map.revision(),
            tree: RTree::bulk_load(entries),
        }
    }

    pub fn is_current(&self, map: &Map) -> bool {
        self.revision == map.revision()
    }

    /// The topmost Stamp whose Silhouette covers `point`, with its draw-order rank, which
    /// compares with the rank [`ssmap_core::PickIndex::pick_ranked`] gives.
    pub fn pick(&self, point: [f64; 2]) -> Option<(Id, usize)> {
        self.tree
            .locate_all_at_point(&point)
            .filter(|c| c.data.shape.hits(&c.data.stamp, point))
            .max_by_key(|c| c.data.rank)
            .map(|c| (c.data.id, c.data.rank))
    }

    /// Stamps whose bounds meet the rectangle, first lowest.
    pub fn intersecting(&self, min: [f64; 2], max: [f64; 2]) -> Vec<Id> {
        let mut found: Vec<&Entry> = self
            .tree
            .locate_in_envelope_intersecting(&AABB::from_corners(min, max))
            .map(|c| &c.data)
            .collect();
        found.sort_by_key(|entry| entry.rank);
        found.into_iter().map(|entry| entry.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10x10 mask whose left half is hit.
    fn shape() -> StampShape {
        let mut mask = Raster::new(10, 10);
        for y in 0..10 {
            for x in 0..5 {
                let i = ((y * 10 + x) * 4) as usize;
                mask.pixels[i..i + 4].copy_from_slice(&[255; 4]);
            }
        }
        StampShape {
            size: [100.0, 100.0],
            anchor: [0.5, 1.0],
            mask: Arc::new(mask),
        }
    }

    fn stamp() -> Stamp {
        Stamp {
            position: [200.0, 200.0],
            ..Stamp::new("test/box".parse().unwrap())
        }
    }

    #[test]
    fn hits_follow_the_mask_and_the_anchor() {
        let (shape, stamp) = (shape(), stamp());
        // The anchor is bottom centre: the quad spans x 150..250, y 100..200.
        assert!(shape.hits(&stamp, [160.0, 150.0]), "left half");
        assert!(!shape.hits(&stamp, [240.0, 150.0]), "right half is empty");
        assert!(!shape.hits(&stamp, [160.0, 210.0]), "below the anchor");
    }

    #[test]
    fn flipping_mirrors_the_mask_about_the_anchor() {
        let (shape, stamp) = (
            shape(),
            Stamp {
                flip: true,
                ..stamp()
            },
        );
        assert!(!shape.hits(&stamp, [160.0, 150.0]));
        assert!(shape.hits(&stamp, [240.0, 150.0]));
    }

    #[test]
    fn rotation_and_scale_pivot_on_the_anchor() {
        let shape = shape();
        // Rotated a quarter turn clockwise, the quad lies to the right of the anchor and its
        // left half is the upper one.
        let stamp = Stamp {
            rotation: 90.0,
            ..stamp()
        };
        assert!(shape.hits(&stamp, [250.0, 170.0]));
        assert!(!shape.hits(&stamp, [250.0, 230.0]));
        let big = Stamp {
            scale: 2.0,
            ..self::stamp()
        };
        assert!(shape.hits(&big, [110.0, 20.0]), "twice as far out");
        assert!(!shape.hits(&self::stamp(), [110.0, 20.0]));
    }

    #[test]
    fn the_index_picks_the_topmost_hit() {
        use ssmap_core::{Canvas, Command, DistanceUnit, Scale};
        let mut map = Map::new(Canvas::new(500, 500, Scale::new(1.0, DistanceUnit::Miles)));
        let layer = Id::new();
        map.execute(&Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Terrain".into(),
        })
        .unwrap();
        let ids: Vec<Id> = (0..2).map(|_| Id::new()).collect();
        for (&id, x) in ids.iter().zip([200.0, 210.0]) {
            map.execute(&Command::CreateStamp {
                id,
                parent: layer,
                index: None,
                stamp: Stamp {
                    position: [x, 200.0],
                    ..Stamp::new("test/box".parse().unwrap())
                },
            })
            .unwrap();
        }
        let shape = shape();
        let index = StampIndex::build(&map, |_| Some(&shape));
        assert!(index.is_current(&map));
        assert_eq!(
            index.pick([155.0, 150.0]).map(|(id, _)| id),
            Some(ids[0]),
            "only the first reaches"
        );
        assert_eq!(
            index.pick([175.0, 150.0]).map(|(id, _)| id),
            Some(ids[1]),
            "the later one is above"
        );
        assert_eq!(index.pick([20.0, 20.0]), None);
        assert_eq!(index.intersecting([0.0, 0.0], [500.0, 500.0]), ids);
    }
}
