//! Map model, Commands, geometry and persistence.
//!
//! This crate must never depend on egui or wgpu (ADR 0001); CI enforces it.

mod geometry;
mod id;
mod map;
mod node;
mod op;

pub use geometry::{MultiPolygon, Point, Polygon};
pub use id::Id;
pub use map::{Canvas, DistanceUnit, InvariantError, Map, MapView, Scale, Subtree};
pub use node::{Label, Lake, Landmass, Layer, Node, Object, ObjectKind, Stamp};
pub use op::{Op, OpError, Property};
