//! Labels and the named Label styles they follow (ADR 0006).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::extra::Extra;
use crate::geometry::Point;
use crate::id::Id;
use crate::node::{Align, FontFace, Halo};

/// Text placed on the Map, sized in Canvas units so it is part of the picture (ADR 0006).
/// It has no geometry blob: its fields are plain JSON in the Map file.
///
/// Typography and colour come from the Label style it names; each `Option` field overrides
/// that style, and `None` follows it. Resolve a Label with [`Label::resolve`].
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Label {
    /// The text, with explicit line breaks as `\n`.
    pub text: String,
    /// The anchor in Canvas units: the vertical centre of the text block, and its left edge,
    /// centre or right edge according to `align`.
    pub position: Point,
    /// How the lines line up with each other and with `position`.
    pub align: Align,
    /// Degrees clockwise about `position`.
    pub rotation: f64,
    /// The Label style it follows. An unknown id is read as the default style.
    pub style: Id,
    /// Overrides the style's font face; `None` follows the style.
    pub font: Option<FontFace>,
    /// The em size in Canvas units.
    pub size: Option<f64>,
    /// Extra space after each character, as a share of `size`.
    pub letter_spacing: Option<f64>,
    /// The distance from one baseline to the next, as a multiple of `size`.
    pub line_height: Option<f64>,
    /// Bends each line along an arc: from -1 to 1, where the sign is the direction (positive
    /// arches up, negative sags down) and 1 sweeps half a circle.
    pub arch: Option<f64>,
    /// Shows the text in capitals without changing what is stored.
    pub all_caps: Option<bool>,
    /// Overrides the style's fill colour; `None` follows the style.
    pub fill: Option<Color>,
    /// From 0 (transparent) to 1 (opaque), for fill and halo alike.
    pub opacity: Option<f32>,
    /// Overrides the style's halo; `None` follows the style.
    pub halo: Option<Halo>,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Label {
    /// Empty text that follows the default Label style.
    fn default() -> Self {
        Self {
            text: String::new(),
            position: [0.0, 0.0],
            align: Align::Centre,
            rotation: 0.0,
            style: LabelStyle::DEFAULT_ID,
            font: None,
            size: None,
            letter_spacing: None,
            line_height: None,
            arch: None,
            all_caps: None,
            fill: None,
            opacity: None,
            halo: None,
            extra: Extra::new(),
        }
    }
}

impl Label {
    /// The settings the Label is drawn with: its overrides, then its style's values.
    pub fn resolve(&self, styles: &LabelStyles) -> ResolvedLabel {
        let style = styles
            .get(self.style)
            .unwrap_or_else(|| styles.default_style());
        ResolvedLabel {
            text: self.text.clone(),
            position: self.position,
            align: self.align,
            rotation: self.rotation,
            font: self.font.clone().unwrap_or_else(|| style.font.clone()),
            size: self.size.unwrap_or(style.size),
            letter_spacing: self.letter_spacing.unwrap_or(style.letter_spacing),
            line_height: self.line_height.unwrap_or(style.line_height),
            arch: self.arch.unwrap_or(style.arch),
            all_caps: self.all_caps.unwrap_or(style.all_caps),
            fill: self.fill.unwrap_or(style.fill),
            opacity: self.opacity.unwrap_or(style.opacity),
            halo: self.halo.clone().unwrap_or_else(|| style.halo.clone()),
        }
    }

    /// The Label that results from editing its resolved settings: a setting that differs from
    /// what the Label resolved to becomes an override, and any other keeps following the style
    /// (or its existing override).
    pub fn with_resolved(&self, styles: &LabelStyles, edited: &ResolvedLabel) -> Label {
        let was = self.resolve(styles);
        fn pick<T: PartialEq + Clone>(own: &Option<T>, was: &T, edited: &T) -> Option<T> {
            if edited == was {
                own.clone()
            } else {
                Some(edited.clone())
            }
        }
        Label {
            text: edited.text.clone(),
            position: edited.position,
            align: edited.align,
            rotation: edited.rotation,
            style: self.style,
            font: pick(&self.font, &was.font, &edited.font),
            size: pick(&self.size, &was.size, &edited.size),
            letter_spacing: pick(
                &self.letter_spacing,
                &was.letter_spacing,
                &edited.letter_spacing,
            ),
            line_height: pick(&self.line_height, &was.line_height, &edited.line_height),
            arch: pick(&self.arch, &was.arch, &edited.arch),
            all_caps: pick(&self.all_caps, &was.all_caps, &edited.all_caps),
            fill: pick(&self.fill, &was.fill, &edited.fill),
            opacity: pick(&self.opacity, &was.opacity, &edited.opacity),
            halo: pick(&self.halo, &was.halo, &edited.halo),
            extra: self.extra.clone(),
        }
    }

    /// Copies the values of `style` into the settings the Label does not override, so it
    /// looks the same without the style.
    pub(crate) fn bake(&mut self, style: &LabelStyle) {
        self.font.get_or_insert_with(|| style.font.clone());
        self.size.get_or_insert(style.size);
        self.letter_spacing.get_or_insert(style.letter_spacing);
        self.line_height.get_or_insert(style.line_height);
        self.arch.get_or_insert(style.arch);
        self.all_caps.get_or_insert(style.all_caps);
        self.fill.get_or_insert(style.fill);
        self.opacity.get_or_insert(style.opacity);
        self.halo.get_or_insert_with(|| style.halo.clone());
    }

    /// Why the Label's own values can't be drawn, if they can't. Settings it follows from its
    /// style were checked with the style.
    pub fn problem(&self) -> Option<&'static str> {
        if self.text.is_empty() {
            return Some("the text is empty");
        }
        if !self.position.iter().all(|v| v.is_finite()) || !self.rotation.is_finite() {
            return Some("the position and rotation must be finite");
        }
        typography_problem(
            self.size,
            self.letter_spacing,
            self.line_height,
            self.arch,
            self.font.as_ref().map(|font| font.weight),
            self.halo.as_ref().map(|halo| halo.width),
            self.opacity,
        )
    }
}

/// A Label with its style applied: every setting the renderer needs, none optional.
#[derive(Clone, PartialEq, Debug)]
pub struct ResolvedLabel {
    /// The text, with explicit line breaks as `\n`.
    pub text: String,
    /// The anchor in Canvas units, as on [`Label`].
    pub position: Point,
    /// How the lines line up with each other and with `position`.
    pub align: Align,
    /// Degrees clockwise about `position`.
    pub rotation: f64,
    /// The font face.
    pub font: FontFace,
    /// The em size in Canvas units.
    pub size: f64,
    /// Extra space after each character, as a share of `size`.
    pub letter_spacing: f64,
    /// The distance from one baseline to the next, as a multiple of `size`.
    pub line_height: f64,
    /// How far each line bends along an arc, from -1 to 1.
    pub arch: f64,
    /// Whether the text shows in capitals.
    pub all_caps: bool,
    /// The text colour.
    pub fill: Color,
    /// From 0 (transparent) to 1 (opaque), for fill and halo alike.
    pub opacity: f32,
    /// The halo drawn under the text.
    pub halo: Halo,
}

impl Default for ResolvedLabel {
    /// Empty text in the default Label style.
    fn default() -> Self {
        Label::default().resolve(&LabelStyles::default())
    }
}

impl ResolvedLabel {
    /// The text as drawn: in capitals when `all_caps` is set.
    pub fn display_text(&self) -> String {
        if self.all_caps {
            self.text.to_uppercase()
        } else {
            self.text.clone()
        }
    }
}

/// A named set of typography and colour settings stored in the Map (ADR 0006).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LabelStyle {
    /// The style's id, which Labels name to follow it.
    pub id: Id,
    /// The style's name, such as "Realm". Unique within a Map.
    pub name: String,
    /// The font face.
    pub font: FontFace,
    /// The em size in Canvas units.
    pub size: f64,
    /// Extra space after each character, as a share of `size`.
    pub letter_spacing: f64,
    /// The distance from one baseline to the next, as a multiple of `size`.
    pub line_height: f64,
    /// Bends each line along an arc: from -1 to 1, where the sign is the direction (positive
    /// arches up, negative sags down) and 1 sweeps half a circle.
    pub arch: f64,
    /// Shows the text in capitals without changing what is stored.
    pub all_caps: bool,
    /// The text colour.
    pub fill: Color,
    /// From 0 (transparent) to 1 (opaque), for fill and halo alike.
    pub opacity: f32,
    /// The halo drawn under the text.
    pub halo: Halo,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl LabelStyle {
    /// The id of the default style, which every Map has and none can delete.
    pub const DEFAULT_ID: Id = Id::from_uuid(uuid::Uuid::from_u128(1));

    /// A style named `name` with the default settings.
    pub fn new(id: Id, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            ..Self::default()
        }
    }

    /// Why the style can't be used, if it can't.
    pub fn problem(&self) -> Option<&'static str> {
        if self.name.trim().is_empty() {
            return Some("the name is empty");
        }
        typography_problem(
            Some(self.size),
            Some(self.letter_spacing),
            Some(self.line_height),
            Some(self.arch),
            Some(self.font.weight),
            Some(self.halo.width),
            Some(self.opacity),
        )
    }
}

impl Default for LabelStyle {
    /// The default style, named "Default".
    fn default() -> Self {
        Self {
            id: Self::DEFAULT_ID,
            name: "Default".into(),
            font: FontFace::default(),
            size: 48.0,
            letter_spacing: 0.0,
            line_height: 1.2,
            arch: 0.0,
            all_caps: false,
            fill: Color::rgb(0x2b, 0x21, 0x18),
            opacity: 1.0,
            halo: Halo::default(),
            extra: Extra::new(),
        }
    }
}

/// Why typography values can't be drawn, if they can't. `None` values are not checked.
fn typography_problem(
    size: Option<f64>,
    letter_spacing: Option<f64>,
    line_height: Option<f64>,
    arch: Option<f64>,
    weight: Option<u16>,
    halo_width: Option<f64>,
    opacity: Option<f32>,
) -> Option<&'static str> {
    if size.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
        return Some("the size must be greater than 0");
    }
    if letter_spacing.is_some_and(|v| !v.is_finite()) {
        return Some("the letter spacing must be finite");
    }
    if line_height.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
        return Some("the line height must be greater than 0");
    }
    if arch.is_some_and(|v| !(-1.0..=1.0).contains(&v)) {
        return Some("the arch must be from -1 to 1");
    }
    if weight.is_some_and(|v| !(1..=1000).contains(&v)) {
        return Some("the font weight must be from 1 to 1000");
    }
    if halo_width.is_some_and(|v| !(v.is_finite() && v >= 0.0)) {
        return Some("the halo width must be 0 or more");
    }
    if opacity.is_some_and(|v| !(0.0..=1.0).contains(&v)) {
        return Some("the opacity must be from 0 to 1");
    }
    None
}

/// Every Label style of a Map, in the order the styles panel lists them. The default style
/// is always among them, even in a file that lacks it.
#[derive(Clone, PartialEq, Debug, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct LabelStyles(Vec<LabelStyle>);

impl Default for LabelStyles {
    fn default() -> Self {
        Self(vec![LabelStyle::default()])
    }
}

impl LabelStyles {
    /// Collects `styles`, adding the default style if they lack it.
    pub fn new(mut styles: Vec<LabelStyle>) -> Self {
        if !styles.iter().any(|s| s.id == LabelStyle::DEFAULT_ID) {
            styles.insert(0, LabelStyle::default());
        }
        Self(styles)
    }

    /// The styles, in list order.
    pub fn iter(&self) -> impl Iterator<Item = &LabelStyle> {
        self.0.iter()
    }

    /// How many styles there are.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no styles. Never true, as the default style is always present.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The style with id `id`, if there is one.
    pub fn get(&self, id: Id) -> Option<&LabelStyle> {
        self.0.iter().find(|style| style.id == id)
    }

    /// The default style.
    pub fn default_style(&self) -> &LabelStyle {
        self.get(LabelStyle::DEFAULT_ID)
            .expect("the default style is always present")
    }

    /// The styles a new Map starts with: the default, then Realm, City, Sea and Range (ADR 0006).
    /// Their ids are fixed, so every new Map names them alike.
    pub fn starter() -> Self {
        let style =
            |n: u128, name: &str, family: &str, size: f64, spacing: f64, caps: bool| LabelStyle {
                id: Id::from_uuid(uuid::Uuid::from_u128(n)),
                name: name.into(),
                font: FontFace {
                    family: family.into(),
                    ..FontFace::default()
                },
                size,
                letter_spacing: spacing,
                all_caps: caps,
                ..LabelStyle::default()
            };
        let sea = LabelStyle {
            fill: Color::rgb(0x3a, 0x5a, 0x72),
            ..style(4, "Sea", "IM Fell Double Pica", 56.0, 0.15, false)
        };
        Self::new(vec![
            LabelStyle::default(),
            LabelStyle {
                halo: Halo {
                    width: 6.0,
                    ..Halo::default()
                },
                ..style(2, "Realm", "Cinzel", 120.0, 0.2, true)
            },
            style(3, "City", "Almendra", 40.0, 0.02, false),
            sea,
            style(5, "Range", "Uncial Antiqua", 64.0, 0.3, true),
        ])
    }

    pub(crate) fn to_vec(&self) -> Vec<LabelStyle> {
        self.0.clone()
    }
}

impl<'de> Deserialize<'de> for LabelStyles {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::new(Vec::deserialize(deserializer)?))
    }
}
