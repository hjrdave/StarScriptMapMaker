//! Tiled offscreen rendering for export (ADR 0007): the image is drawn in tiles no bigger than
//! the device's texture limit, each with an overlap margin that is cropped away, and handed to
//! the caller one row at a time, so the whole image never exists in memory.

use ssmap_core::{ExportSettings, ExportSize, ExportSizeError, Map, RoughnessCache};
use thiserror::Error;

use crate::canvas::{CanvasRenderer, CanvasScene};
use crate::offscreen::render_to_image;
use crate::scene::MapScene;
use crate::view::View;

/// Pixels rendered around each tile and then cropped, wide enough for halos, outlines and
/// ripples that reach across a tile edge.
pub const TILE_MARGIN: u32 = 64;

/// Where an export stands, for the export dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportStatus {
    /// Waiting for derived data (Roughness outlines, rasterised SVGs, tessellation) to finish
    /// rebuilding. Rendering starts when it has.
    WaitingForDerivedData,
    /// `done` of `total` tiles are rendered.
    Rendering { done: u32, total: u32 },
}

#[derive(Debug, Error)]
pub enum TiledExportError<E: std::error::Error + 'static> {
    #[error(transparent)]
    Size(#[from] ExportSizeError),
    #[error("could not write a row: {0}")]
    Sink(#[source] E),
}

/// A region of the output image, in pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// How an image of `size` is cut into tiles for a device whose textures are at most `max_side`
/// pixels wide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TilePlan {
    pub size: [u32; 2],
    /// The overlap margin on every side of a rendered tile.
    pub margin: u32,
    /// The most output pixels a tile covers, per axis.
    pub core: [u32; 2],
}

impl TilePlan {
    pub fn new(size: [u32; 2], max_side: u32) -> Self {
        // A quarter of the limit at most, so the tile core keeps at least half of it.
        let margin = TILE_MARGIN.min(max_side.saturating_sub(1) / 4);
        let core = size.map(|side| side.min(max_side - 2 * margin).max(1));
        Self { size, margin, core }
    }

    /// The size every tile is rendered at, margin included.
    pub fn render_size(&self) -> [u32; 2] {
        self.core.map(|side| side + 2 * self.margin)
    }

    pub fn columns(&self) -> u32 {
        self.size[0].div_ceil(self.core[0])
    }

    pub fn rows(&self) -> u32 {
        self.size[1].div_ceil(self.core[1])
    }

    pub fn tile(&self, column: u32, row: u32) -> Tile {
        let (x, y) = (column * self.core[0], row * self.core[1]);
        Tile {
            x,
            y,
            width: self.core[0].min(self.size[0] - x),
            height: self.core[1].min(self.size[1] - y),
        }
    }
}

/// One export of a Map: its settings, the derived data behind it and the renderer to draw it.
pub struct TiledExport<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub renderer: &'a mut CanvasRenderer,
    pub scene: &'a mut MapScene,
    pub map: &'a Map,
    pub roughness: &'a mut RoughnessCache,
    pub settings: &'a ExportSettings,
    /// Overrides the device's texture limit, which tests use to force many small tiles.
    pub max_tile_side: Option<u32>,
}

impl TiledExport<'_> {
    /// Waits for derived data to finish rebuilding, then renders the export tile by tile and
    /// calls `row` with each row of 8-bit sRGB RGBA, top to bottom, `width * 4` bytes long.
    /// `status` is told what the export is doing.
    ///
    /// Hidden Layers are left out and locked ones drawn. Editor visuals are never drawn: the
    /// scene is the Map's own content over the ocean.
    pub fn run<E: std::error::Error + 'static>(
        self,
        mut status: impl FnMut(ExportStatus),
        mut row: impl FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<ExportSize, TiledExportError<E>> {
        let output = self.settings.output_size(&self.map.canvas)?;

        status(ExportStatus::WaitingForDerivedData);
        self.roughness.wait_until_ready();
        // Content built while outlines were placeholders must not be reused.
        self.scene.invalidate();
        let content = self.scene.content(self.map, self.roughness);
        self.renderer.wait_until_ready(&content);

        let mut scene = CanvasScene::new(&self.map.canvas).with_content(content);
        // A region reaching past the Canvas shows water, not the editor's pasteboard.
        scene.pasteboard = scene.backdrop.water;
        let region = self.settings.region_rect(&self.map.canvas);
        let zoom = f64::from(output.width) / region.width();
        let max_side = self
            .max_tile_side
            .unwrap_or(self.renderer.capabilities().limits.max_texture_dimension_2d);
        let plan = TilePlan::new([output.width, output.height], max_side);
        let (columns, rows) = (plan.columns(), plan.rows());
        let total = columns * rows;
        let stride = output.width as usize * 4;
        let mut band = Vec::new();
        let mut done = 0;
        status(ExportStatus::Rendering { done, total });
        for tile_row in 0..rows {
            let height = plan.tile(0, tile_row).height as usize;
            band.clear();
            band.resize(stride * height, 0);
            for column in 0..columns {
                let tile = plan.tile(column, tile_row);
                let view = View {
                    origin: [
                        region.min[0] + (f64::from(tile.x) - f64::from(plan.margin)) / zoom,
                        region.min[1] + (f64::from(tile.y) - f64::from(plan.margin)) / zoom,
                    ],
                    zoom,
                    size: plan.render_size(),
                };
                let image = render_to_image(self.device, self.queue, self.renderer, &scene, &view);
                // Crop the margin away, copying the tile's rows into the band.
                let source_stride = image.width as usize * 4;
                let margin = plan.margin as usize * 4;
                let len = tile.width as usize * 4;
                for line in 0..tile.height as usize {
                    let from = (line + plan.margin as usize) * source_stride + margin;
                    let to = line * stride + tile.x as usize * 4;
                    band[to..to + len].copy_from_slice(&image.pixels[from..from + len]);
                }
                done += 1;
                status(ExportStatus::Rendering { done, total });
            }
            for line in band.chunks_exact(stride) {
                row(line).map_err(TiledExportError::Sink)?;
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_images_are_one_tile() {
        let plan = TilePlan::new([500, 300], 2048);
        assert_eq!((plan.columns(), plan.rows()), (1, 1));
        assert_eq!(plan.tile(0, 0).width, 500);
        assert_eq!(plan.render_size(), [500 + 128, 300 + 128]);
    }

    #[test]
    fn tiles_cover_the_image_exactly_and_fit_the_limit() {
        for (size, limit) in [
            ([5000, 3000], 2048),
            ([32768, 32768], 2048),
            ([1, 9999], 512),
        ] {
            let plan = TilePlan::new(size, limit);
            assert!(plan.render_size().iter().all(|&side| side <= limit));
            let mut covered = 0u64;
            for row in 0..plan.rows() {
                for column in 0..plan.columns() {
                    let tile = plan.tile(column, row);
                    assert!(tile.x + tile.width <= size[0] && tile.y + tile.height <= size[1]);
                    covered += u64::from(tile.width) * u64::from(tile.height);
                }
            }
            assert_eq!(covered, u64::from(size[0]) * u64::from(size[1]));
        }
    }

    #[test]
    fn a_tiny_limit_still_makes_progress() {
        let plan = TilePlan::new([100, 100], 8);
        assert!(plan.render_size().iter().all(|&side| side <= 8));
        assert!(plan.core.iter().all(|&side| side >= 1));
    }
}
