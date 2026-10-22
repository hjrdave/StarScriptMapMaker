//! Map model, Commands, geometry and persistence.
//!
//! This crate must never depend on egui or wgpu (ADR 0001); CI enforces it.

pub mod asset;
mod background;
mod color;
mod command;
mod embedded;
mod export;
mod extra;
pub mod file;
mod geometry;
mod history;
mod id;
mod label;
pub mod land;
mod map;
mod node;
mod op;
mod paste_gaps;
mod pick;
mod roughness;
pub mod scatter;
mod shore;
mod stamp_pick;
mod template;

pub use asset::AssetRef;
pub use background::{Background, BackgroundTexture};
pub use color::{Color, ParseColorError};
pub use command::{Command, CommandError, PlacedStamp};
pub use embedded::{
    Blob, ContentHash, EmbedAsset, EmbedFont, Embedded, EmbeddedAsset, EmbeddedFont, ParseHexError,
};
pub use export::{
    CREDITS_CHUNK_KEYWORD, CreditsOptions, CreditsOutput, ExportRegion, ExportSettings, ExportSize,
    ExportSizeError, MAX_EXPORT_SIDE, SizeMode, WARN_EXPORT_SIDE, credits_file_path,
};
pub use extra::Extra;
pub use geometry::{Affine, MultiPolygon, NodeRef, Point, Polygon, Rect};
pub use history::{DEFAULT_BUDGET_BYTES, ExecuteError, History, HistoryError, Selection, UndoStep};
pub use id::Id;
pub use label::{Label, LabelStyle, LabelStyles, ResolvedLabel};
pub use map::{Canvas, DistanceUnit, InvariantError, Map, MapDefaults, MapView, Scale, Subtree};
pub use node::{
    Align, FontFace, Halo, Lake, Landmass, Layer, Membership, Node, Object, ObjectKind, Stamp,
};
pub use op::{Op, OpError, Property};
pub use paste_gaps::PasteGaps;
pub use pick::PickIndex;
pub use roughness::{Rebuild, Roughness, RoughnessCache, roughen};
pub use shore::{MAX_RIPPLES, ShoreKind, ShoreOutline, ShoreRipples, ShoreShading, ShoreStyle};
pub use stamp_pick::{SpriteShape, StampIndex};
pub use template::{DefaultLayers, MapTemplate, TemplateError};
