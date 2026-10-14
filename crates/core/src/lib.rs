//! Map model, Commands, geometry and persistence.
//!
//! This crate must never depend on egui or wgpu (ADR 0001); CI enforces it.

mod color;
mod command;
mod extra;
pub mod file;
mod geometry;
mod history;
mod id;
mod map;
mod node;
mod op;

pub use color::Color;
pub use command::{Command, CommandError};
pub use extra::Extra;
pub use geometry::{MultiPolygon, Point, Polygon};
pub use history::{DEFAULT_BUDGET_BYTES, ExecuteError, History, HistoryError, Selection, UndoStep};
pub use id::Id;
pub use map::{Canvas, DistanceUnit, InvariantError, Map, MapView, Scale, Subtree};
pub use node::{
    Align, FontFace, Halo, Label, Lake, Landmass, Layer, Node, Object, ObjectKind, Stamp,
};
pub use op::{Op, OpError, Property};
