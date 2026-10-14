//! wgpu Canvas rendering and PNG export. Never depends on egui; the app hosts the result.
//!
//! The Canvas always renders into an offscreen, 4x-multisampled target ([`CanvasRenderer`]).
//! The editor then copies that target into its own frame ([`Presenter`]), and export and
//! golden tests read it back ([`render_to_image`]), so the screen and export share one path.

mod canvas;
mod color;
mod gpu;
mod ocean;
mod offscreen;
mod present;
mod scene;
pub mod text;
mod vector;
mod view;

pub use canvas::{Backdrop, CANVAS_FORMAT, CanvasRenderer, CanvasScene, Content};
pub use color::Color;
pub use gpu::{GpuCapabilities, Headless, HeadlessError};
pub use offscreen::{RgbaImage, render_to_image};
pub use present::Presenter;
pub use scene::MapScene;
pub use text::{Fonts, LabelLayout};
pub use vector::{FillRule, LyonRenderer, Paint, VectorGroup, VectorRenderer, VectorShape};
pub use view::View;

/// The wgpu version this crate is built against, for callers that share a device with it.
pub use wgpu;
