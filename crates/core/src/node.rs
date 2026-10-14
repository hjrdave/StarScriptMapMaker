//! The data held by each node of the Map tree. Tree structure (parent and children) lives in
//! [`Map`](crate::Map), not here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::extra::Extra;
use crate::geometry::MultiPolygon;
use crate::id::Id;
use crate::roughness::Roughness;
use crate::shore::{ShoreKind, ShoreStyle};

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
    /// The Landmass this Layer is clipped to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Id>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Layer {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            clip: None,
            extra: Extra::new(),
        }
    }
}

/// A single thing placed on a Map.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Object {
    pub locked: bool,
    pub kind: ObjectKind,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Object {
    pub fn new(kind: ObjectKind) -> Self {
        Self {
            locked: false,
            kind,
            extra: Extra::new(),
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

    /// The fill colour of kinds that have an outline.
    pub fn fill(&self) -> Option<Color> {
        match self {
            ObjectKind::Landmass(landmass) => Some(landmass.fill),
            ObjectKind::Lake(lake) => Some(lake.fill),
            ObjectKind::Stamp(_) | ObjectKind::Label(_) => None,
        }
    }

    pub(crate) fn fill_mut(&mut self) -> Option<&mut Color> {
        match self {
            ObjectKind::Landmass(landmass) => Some(&mut landmass.fill),
            ObjectKind::Lake(lake) => Some(&mut lake.fill),
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

    /// The Roughness override of kinds that have an outline, if set.
    pub fn roughness(&self) -> Option<&Roughness> {
        match self {
            ObjectKind::Landmass(landmass) => landmass.roughness.as_ref(),
            ObjectKind::Lake(lake) => lake.roughness.as_ref(),
            ObjectKind::Stamp(_) | ObjectKind::Label(_) => None,
        }
    }

    pub(crate) fn roughness_mut(&mut self) -> Option<&mut Option<Roughness>> {
        match self {
            ObjectKind::Landmass(landmass) => Some(&mut landmass.roughness),
            ObjectKind::Lake(lake) => Some(&mut lake.roughness),
            ObjectKind::Stamp(_) | ObjectKind::Label(_) => None,
        }
    }

    /// A copy with an empty outline, for storing the outline separately.
    /// The Shore style override of kinds that have an outline, if set.
    pub fn shore(&self) -> Option<&ShoreStyle> {
        match self {
            ObjectKind::Landmass(landmass) => landmass.shore.as_ref(),
            ObjectKind::Lake(lake) => lake.shore.as_ref(),
            ObjectKind::Stamp(_) | ObjectKind::Label(_) => None,
        }
    }

    /// Which Map-wide Shore style default this kind uses, for kinds that have an outline.
    pub fn shore_kind(&self) -> Option<ShoreKind> {
        match self {
            ObjectKind::Landmass(_) => Some(ShoreKind::Landmass),
            ObjectKind::Lake(_) => Some(ShoreKind::Lake),
            ObjectKind::Stamp(_) | ObjectKind::Label(_) => None,
        }
    }

    pub(crate) fn shore_mut(&mut self) -> Option<&mut Option<ShoreStyle>> {
        match self {
            ObjectKind::Landmass(landmass) => Some(&mut landmass.shore),
            ObjectKind::Lake(lake) => Some(&mut lake.shore),
            ObjectKind::Stamp(_) | ObjectKind::Label(_) => None,
        }
    }

    pub(crate) fn without_geometry(&self) -> ObjectKind {
        match self {
            ObjectKind::Landmass(landmass) => ObjectKind::Landmass(Landmass {
                geometry: MultiPolygon::default(),
                fill: landmass.fill,
                roughness: landmass.roughness.clone(),
                shore: landmass.shore.clone(),
                extra: landmass.extra.clone(),
            }),
            ObjectKind::Lake(lake) => ObjectKind::Lake(Lake {
                geometry: MultiPolygon::default(),
                fill: lake.fill,
                roughness: lake.roughness.clone(),
                shore: lake.shore.clone(),
                extra: lake.extra.clone(),
            }),
            ObjectKind::Stamp(_) | ObjectKind::Label(_) => self.clone(),
        }
    }

    /// The `kind` tag, as stored in the Map file.
    pub fn name(&self) -> &'static str {
        match self {
            ObjectKind::Landmass(_) => "landmass",
            ObjectKind::Lake(_) => "lake",
            ObjectKind::Stamp(_) => "stamp",
            ObjectKind::Label(_) => "label",
        }
    }
}

/// A filled region of land.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Landmass {
    /// Stored apart from the other fields in the Map file, hence the default.
    #[serde(default)]
    pub geometry: MultiPolygon,
    /// Files saved before fills existed get the default.
    #[serde(default = "default_landmass_fill")]
    pub fill: Color,
    /// Overrides the Map-wide default Roughness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<Roughness>,
    /// Overrides the Map-wide default Shore style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shore: Option<ShoreStyle>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Landmass {
    pub const DEFAULT_FILL: Color = Color::rgb(0xd8, 0xc4, 0x9a);

    pub fn new(geometry: MultiPolygon) -> Self {
        Self {
            geometry,
            fill: Self::DEFAULT_FILL,
            roughness: None,
            shore: None,
            extra: Extra::new(),
        }
    }
}

impl Default for Landmass {
    fn default() -> Self {
        Self::new(MultiPolygon::default())
    }
}

fn default_landmass_fill() -> Color {
    Landmass::DEFAULT_FILL
}

/// A closed region of fresh water drawn above land.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Lake {
    /// Stored apart from the other fields in the Map file, hence the default.
    #[serde(default)]
    pub geometry: MultiPolygon,
    /// Files saved before fills existed get the default.
    #[serde(default = "default_lake_fill")]
    pub fill: Color,
    /// Overrides the Map-wide default Roughness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<Roughness>,
    /// Overrides the Map-wide default Shore style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shore: Option<ShoreStyle>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Lake {
    pub const DEFAULT_FILL: Color = Color::rgb(0x6f, 0xa8, 0xc8);

    pub fn new(geometry: MultiPolygon) -> Self {
        Self {
            geometry,
            fill: Self::DEFAULT_FILL,
            roughness: None,
            shore: None,
            extra: Extra::new(),
        }
    }
}

impl Default for Lake {
    fn default() -> Self {
        Self::new(MultiPolygon::default())
    }
}

fn default_lake_fill() -> Color {
    Lake::DEFAULT_FILL
}

/// One placed Asset. Fields arrive with the Stamps ticket.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Stamp {
    #[serde(flatten)]
    pub extra: Extra,
}

/// Text placed on the Map. Fields arrive with the Labels ticket.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct Label {
    #[serde(flatten)]
    pub extra: Extra,
}
