//! wgpu Canvas rendering and PNG export. Never depends on egui; the app hosts the result.
//!
//! The Canvas always renders into an offscreen, 4x-multisampled target ([`CanvasRenderer`]).
//! The editor then copies that target into its own frame ([`Presenter`]), and export and
//! golden tests read it back ([`render_to_image`]), so the screen and export share one path.
//!
//! Asset art is rasterised on the CPU ([`raster`]), given Silhouettes ([`silhouette`]) and
//! packed into paged textures ([`atlas`]).

// Stays `pub`: the atlas packing test in `tests/golden.rs` is an integration test.
pub mod atlas;
mod canvas;
mod color;
mod export_job;
mod gpu;
mod ocean;
mod offscreen;
mod png_writer;
mod present;
pub mod profile;
pub mod raster;
mod scene;
mod shore_rings;
pub mod silhouette;
pub mod sprite;
mod stamp;
pub mod text;
mod tiled;
mod vector;
mod view;

pub use canvas::{
    Backdrop, BoardArea, CANVAS_FORMAT, CanvasRenderer, CanvasScene, Content, Quality,
};
pub use color::Color;
pub use export_job::{ExportContext, ExportError, ExportJob, ExportStep};
pub use gpu::{GpuCapabilities, Headless, HeadlessError, Suitability};
pub use offscreen::{ReadbackError, RgbaImage, read_texture, render_to_image};
pub use png_writer::{PngMeta, PngWriteError, PngWriter};
pub use present::Presenter;
pub use scene::MapScene;
pub use sprite::{PLACEHOLDER_SIZE, Sprite, SpriteError, SpriteState, SpriteStore, placeholder};
pub use stamp::{PlacedStamp, StampDraw};
pub use text::{Fonts, LabelLayout};
pub use tiled::{ExportStatus, TiledExport, TiledExportError};
pub use vector::{FillRule, Paint, Placement, TessellationCache, TessellationRebuild, VectorGroup};
pub use view::View;

/// The wgpu version this crate is built against, for callers that share a device with it.
pub use wgpu;
