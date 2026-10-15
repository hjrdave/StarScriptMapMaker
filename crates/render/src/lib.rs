//! wgpu Canvas rendering and PNG export. Never depends on egui; the app hosts the result.
//!
//! The Canvas always renders into an offscreen, 4x-multisampled target ([`CanvasRenderer`]).
//! The editor then copies that target into its own frame ([`Presenter`]), and export and
//! golden tests read it back ([`render_to_image`]), so the screen and export share one path.
//!
//! Asset art is rasterised on the CPU ([`raster`]), given Silhouettes ([`silhouette`]) and
//! packed into paged textures ([`atlas`]).

pub mod atlas;
mod canvas;
mod color;
mod gpu;
mod ocean;
mod offscreen;
mod png_writer;
mod present;
pub mod raster;
mod scene;
pub mod silhouette;
pub mod sprite;
mod stamp;
pub mod text;
mod vector;
mod view;

pub use canvas::{
    Backdrop, CANVAS_FORMAT, CanvasRenderer, CanvasScene, ClipScope, Content, Quality,
};
pub use color::Color;
pub use gpu::{GpuCapabilities, Headless, HeadlessError, Suitability};
pub use offscreen::{RgbaImage, read_texture, render_to_image};
pub use png_writer::{PngMeta, PngWriteError, PngWriter};
pub use present::Presenter;
pub use scene::MapScene;
pub use sprite::{Sprite, SpriteStore};
pub use stamp::{PlacedStamp, StampDraw};
pub use text::{Fonts, LabelLayout};
pub use vector::{
    FillRule, LyonRenderer, ObjectRef, Paint, TessellationCache, TessellationRebuild, VectorGroup,
    VectorRenderer, VectorShape,
};
pub use view::View;

/// The wgpu version this crate is built against, for callers that share a device with it.
pub use wgpu;
