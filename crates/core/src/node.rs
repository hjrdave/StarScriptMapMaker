//! The data held by each node of the Map tree. Tree structure (parent and children) lives in
//! [`Map`](crate::Map), not here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::asset::AssetRef;
use crate::color::Color;
use crate::extra::Extra;
use crate::geometry::{MultiPolygon, Point};
use crate::id::Id;
use crate::label::Label;
use crate::roughness::Roughness;
use crate::shore::{ShoreKind, ShoreStyle};

/// A Layer or an Object.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    /// A Layer.
    Layer(Layer),
    /// An Object.
    Object(Object),
}

impl Node {
    /// Whether this is a Layer.
    pub fn is_layer(&self) -> bool {
        matches!(self, Node::Layer(_))
    }

    /// Whether this node is locked itself, not counting a locked Layer above it.
    pub fn locked(&self) -> bool {
        match self {
            Node::Layer(layer) => layer.locked,
            Node::Object(object) => object.locked,
        }
    }

    pub(crate) fn set_link(&mut self, link: Option<Id>) {
        match self {
            Node::Layer(layer) => layer.link = link,
            Node::Object(object) => object.link = link,
        }
    }

    /// The Link set this node belongs to, if any.
    pub fn link(&self) -> Option<Id> {
        match self {
            Node::Layer(layer) => layer.link,
            Node::Object(object) => object.link,
        }
    }
}

/// A container of Objects and other Layers. A Group is just a nested Layer.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Layer {
    /// The Layer's name.
    pub name: String,
    /// Whether the Layer and what it holds are drawn.
    pub visible: bool,
    /// Whether the Layer refuses Commands, along with everything inside it.
    pub locked: bool,
    /// From 0 (transparent) to 1 (opaque).
    pub opacity: f32,
    /// The Landmass this Layer is clipped to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Id>,
    /// Draws the Stamps directly inside by Anchor height, lower in front (ADR 0003).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub depth_sort: bool,
    /// The Link set this Layer belongs to: Layers and Objects with the same id move and
    /// transform together. A set of one links nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Id>,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Layer {
    /// A visible, unlocked, fully opaque Layer called `name`, with nothing set.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            clip: None,
            depth_sort: false,
            link: None,
            extra: Extra::new(),
        }
    }
}

/// A single thing placed on a Map.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Object {
    /// A name the user gave it; `None` shows the kind's default name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether the Object refuses Commands.
    pub locked: bool,
    /// Whether the Object belongs to the Canvas (and is clipped at its edge) or sits off it.
    /// Changed only by dragging, never by resizing the Canvas.
    #[serde(default, skip_serializing_if = "Membership::is_default")]
    pub membership: Membership,
    /// What the Object is, with that kind's own settings.
    pub kind: ObjectKind,
    /// The Link set this Object belongs to, as on a [`Layer`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Id>,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Object {
    /// An unnamed, unlocked Object of `kind` that belongs to the Canvas.
    pub fn new(kind: ObjectKind) -> Self {
        Self {
            name: None,
            locked: false,
            membership: Membership::default(),
            kind,
            link: None,
            extra: Extra::new(),
        }
    }
}

/// Whether an Object belongs to the Canvas or sits off it. A member is clipped at the Canvas edge
/// and exported; an off-Canvas Object shows in full around the Canvas and is not exported.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Membership {
    /// Belongs to the Canvas. Files saved before membership existed get this.
    #[default]
    Canvas,
    /// Sits off the Canvas. Saved as `pasteboard`, the name files and journals already use.
    #[serde(rename = "pasteboard")]
    OffCanvas,
}

impl Membership {
    fn is_default(&self) -> bool {
        *self == Membership::default()
    }
}

/// The v1 Object kinds. Later tickets fill in their fields.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectKind {
    /// A filled region of land.
    Landmass(Landmass),
    /// A closed region of fresh water.
    Lake(Lake),
    /// One placed Asset.
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
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Landmass {
    /// The fill colour a new Landmass gets.
    pub const DEFAULT_FILL: Color = Color::rgb(0xd8, 0xc4, 0x9a);

    /// A Landmass with outline `geometry` and the default fill, Roughness and Shore style.
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
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Lake {
    /// The fill colour a new Lake gets.
    pub const DEFAULT_FILL: Color = Color::rgb(0x6f, 0xa8, 0xc8);

    /// A Lake with outline `geometry` and the default fill, Roughness and Shore style.
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
    /// The Asset this Stamp places.
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
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Stamp {
    /// A Stamp of `asset` with its Anchor at `position`, at normal size, upright and opaque.
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

/// How the lines of a Label line up with each other and with its position.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// Lines start at the Label's left edge.
    Left,
    /// Lines are centred on the Label's position.
    #[default]
    Centre,
    /// Lines end at the Label's right edge.
    Right,
}

/// A font family and style. v1 offers the bundled fonts only (ADR 0006); characters a family
/// lacks fall back to Noto Serif.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FontFace {
    /// The font family name.
    pub family: String,
    /// The CSS weight, from 1 to 1000; 400 is regular and 700 bold.
    pub weight: u16,
    /// Whether the face is italic.
    pub italic: bool,
    /// Fields from a newer file format that this version keeps without reading.
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
    /// The halo's colour.
    pub color: Color,
    /// Fields from a newer file format that this version keeps without reading.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_canvas_keeps_its_saved_name() {
        let json = serde_json::to_string(&Membership::OffCanvas).unwrap();
        assert_eq!(json, "\"pasteboard\"");
        let back: Membership = serde_json::from_str("\"pasteboard\"").unwrap();
        assert_eq!(back, Membership::OffCanvas);
    }
}
