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
mod present;
pub mod raster;
pub mod silhouette;
mod vector;
mod view;

pub use canvas::{Backdrop, CANVAS_FORMAT, CanvasRenderer, CanvasScene};
pub use color::Color;
pub use gpu::{GpuCapabilities, Headless, HeadlessError};
pub use offscreen::{RgbaImage, read_texture, render_to_image};
pub use present::Presenter;
pub use vector::{FillPath, LyonRenderer, VectorRenderer};
pub use view::View;

/// The wgpu version this crate is built against, for callers that share a device with it.
pub use wgpu;
