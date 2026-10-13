//! The data held by each node of the Map tree. Tree structure (parent and children) lives in
//! [`Map`](crate::Map), not here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::geometry::MultiPolygon;

/// A Layer or an Object.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    Layer(Layer),
    Object(Object),
}

impl Node {
    pub fn is_layer(&self) -> bool {
        matches!(self, Node::Layer(_))
    }

    pub fn locked(&self) -> bool {
        match self {
            Node::Layer(layer) => layer.locked,
            Node::Object(object) => object.locked,
        }
    }
}

/// A container of Objects and other Layers. A Group is just a nested Layer.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    /// From 0 (transparent) to 1 (opaque).
    pub opacity: f32,
}

impl Layer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visible: true,
            locked: false,
            opacity: 1.0,
        }
    }
}

/// A single thing placed on a Map.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Object {
    pub locked: bool,
    pub kind: ObjectKind,
}

impl Object {
    pub fn new(kind: ObjectKind) -> Self {
        Self {
            locked: false,
            kind,
        }
    }
}

/// The v1 Object kinds. Later tickets fill in their fields.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectKind {
    Landmass(Landmass),
    Lake(Lake),
    Stamp(Stamp),
    Label(Label),
}

impl ObjectKind {
    /// The outline of kinds that have one.
    pub fn geometry(&self) -> Option<&MultiPolygon> {
        match self {
            ObjectKind::Landmass(landmass) => Some(&landmass.geometry),
            ObjectKind::Lake(lake) => Some(&lake.geometry),
            ObjectKind::Stamp(_) | ObjectKind::Label(_) => None,
        }
    }

    pub(crate) fn geometry_mut(&mut self) -> Option<&mut MultiPolygon> {
        match self {
            ObjectKind::Landmass(landmass) => Some(&mut landmass.geometry),
            ObjectKind::Lake(lake) => Some(&mut lake.geometry),
            ObjectKind::Stamp(_) | ObjectKind::Label(_) => None,
        }
    }
}

/// A filled region of land.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Landmass {
    pub geometry: MultiPolygon,
}

/// A closed region of fresh water drawn above land.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Lake {
    pub geometry: MultiPolygon,
}

/// One placed Asset. Fields arrive with the Stamps ticket.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Stamp {}

/// Text placed on the Map. Fields arrive with the Labels ticket.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Label {}
