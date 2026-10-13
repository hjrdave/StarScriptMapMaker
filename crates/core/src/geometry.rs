//! Outline geometry. Landmass and Lake outlines are `f64` polygon rings (ADR 0002).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A point in Canvas units, as `[x, y]`.
pub type Point = [f64; 2];

/// One part of a region: an outer boundary and optional holes. Rings are implicitly closed.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Polygon {
    pub exterior: Vec<Point>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<Point>>,
}

/// A region made of zero or more Polygons.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct MultiPolygon(pub Vec<Polygon>);
