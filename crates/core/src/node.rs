//! The data held by each node of the Map tree. Tree structure (parent and children) lives in
//! [`Map`](crate::Map), not here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::asset::AssetRef;
use crate::color::Color;
use crate::extra::Extra;
use crate::geometry::{MultiPolygon, Point};
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
    /// Draws the Stamps directly inside by Anchor height, lower in front (ADR 0003).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub depth_sort: bool,
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
            depth_sort: false,
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
    /// Boxed: a Label carries far more fields than other kinds, and Maps hold many nodes.
    Label(Box<Label>),
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

/// One placed Asset (ADR 0003). It stores the concrete Asset it got, never a Variant group,
/// so editing a pack never reshuffles an existing Map. Scale, rotation and flip all pivot on
/// the Asset's Anchor.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Stamp {
    pub asset: AssetRef,
    /// Where the Anchor sits, in Canvas units.
    pub position: Point,
    /// A uniform multiple of the Asset's default size.
    pub scale: f64,
    /// Degrees clockwise about the Anchor.
    pub rotation: f64,
    /// Mirrors the art left to right about the Anchor, before rotating.
    pub flip: bool,
    /// Multiplies the line art's colour. `None` leaves the art as drawn.
    pub tint: Option<Color>,
    /// The Silhouette's fill, drawn under the line art. `None` uses the land colour.
    pub fill: Option<Color>,
    /// From 0 (transparent) to 1 (opaque).
    pub opacity: f32,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Stamp {
    pub fn new(asset: AssetRef, position: Point) -> Self {
        Self {
            asset,
            position,
            ..Self::default()
        }
    }
}

impl Default for Stamp {
    /// A Stamp of the placeholder Asset `unset/unset`, which no pack provides.
    fn default() -> Self {
        Self {
            asset: AssetRef {
                pack: "unset".into(),
                asset: "unset".into(),
            },
            position: [0.0, 0.0],
            scale: 1.0,
            rotation: 0.0,
            flip: false,
            tint: None,
            fill: None,
            opacity: 1.0,
            extra: Extra::new(),
        }
    }
}

/// Text placed on the Map, sized in Canvas units so it is part of the picture (ADR 0006).
/// It has no geometry blob: its fields are plain JSON in the Map file.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Label {
    /// The text, with explicit line breaks as `\n`.
    pub text: String,
    /// The anchor in Canvas units: the vertical centre of the text block, and its left edge,
    /// centre or right edge according to `align`.
    pub position: Point,
    pub align: Align,
    /// Degrees clockwise about `position`.
    pub rotation: f64,
    pub font: FontFace,
    /// The em size in Canvas units.
    pub size: f64,
    /// Extra space after each character, as a share of `size`.
    pub letter_spacing: f64,
    /// The distance from one baseline to the next, as a multiple of `size`.
    pub line_height: f64,
    /// Shows the text in capitals without changing what is stored.
    pub all_caps: bool,
    pub fill: Color,
    /// From 0 (transparent) to 1 (opaque), for fill and halo alike.
    pub opacity: f32,
    pub halo: Halo,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Label {
    fn default() -> Self {
        Self {
            text: String::new(),
            position: [0.0, 0.0],
            align: Align::Centre,
            rotation: 0.0,
            font: FontFace::default(),
            size: 48.0,
            letter_spacing: 0.0,
            line_height: 1.2,
            all_caps: false,
            fill: Color::rgb(0x2b, 0x21, 0x18),
            opacity: 1.0,
            halo: Halo::default(),
            extra: Extra::new(),
        }
    }
}

impl Label {
    /// The text as drawn: in capitals when `all_caps` is set.
    pub fn display_text(&self) -> String {
        if self.all_caps {
            self.text.to_uppercase()
        } else {
            self.text.clone()
        }
    }
}

/// How the lines of a Label line up with each other and with its position.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    #[default]
    Centre,
    Right,
}

/// A font family and style. v1 offers the bundled fonts only (ADR 0006); characters a family
/// lacks fall back to Noto Serif.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FontFace {
    pub family: String,
    /// The CSS weight, from 1 to 1000; 400 is regular and 700 bold.
    pub weight: u16,
    pub italic: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for FontFace {
    fn default() -> Self {
        Self {
            family: "Cinzel".into(),
            weight: 400,
            italic: false,
            extra: Extra::new(),
        }
    }
}

/// A stroke drawn under a Label's fill, so it reads over busy art.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Halo {
    /// How far the halo reaches past the glyph edges, in Canvas units. 0 draws none.
    pub width: f64,
    pub color: Color,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Halo {
    fn default() -> Self {
        Self {
            width: 0.0,
            color: Color::rgb(0xf4, 0xec, 0xd8),
            extra: Extra::new(),
        }
    }
}
