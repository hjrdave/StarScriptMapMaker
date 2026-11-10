//! Shore style: how a coastline is dressed (ADR 0002). Effects are drawn on the roughened
//! outline: an outline, ripple lines stepping outward over the water, shading inward from
//! the coast, and (ADR 0016) a glow outward over the water and short horizontal lines. A Map-wide default exists for Landmasses and another for Lakes; either kind can
//! override its default.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::extra::Extra;

/// At most this many ripple lines, to bound the geometry they cost.
pub const MAX_RIPPLES: u32 = 12;

/// The least gap between rows of water lines, in Canvas units, to bound the geometry they cost.
pub const MIN_LINE_SPACING: f64 = 1.0;

/// The most a glow or a water line can reach out from the coast, in Canvas units.
pub const MAX_REACH: f64 = 500.0;

/// A line along the coast. A width of zero draws nothing.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShoreOutline {
    /// In Canvas units.
    pub width: f64,
    /// The line's colour.
    pub color: Color,
}

/// Lines stepping outward from the coast over the water.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShoreRipples {
    /// How many lines. Zero draws nothing.
    pub count: u32,
    /// The gap between neighbouring lines, and between the coast and the first, in Canvas units.
    pub spacing: f64,
    /// The width of each line, in Canvas units.
    pub width: f64,
    /// The lines' colour.
    pub color: Color,
    /// How much each line fades relative to the one before it, from 0 (none) to 1 (the last
    /// line is invisible).
    pub fade: f64,
}

/// Shading that glows inward from the coast.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShoreShading {
    /// How far in the glow reaches, in Canvas units. Zero draws nothing.
    pub width: f64,
    /// The colour at the coast; its alpha is the strength there, fading to nothing inward.
    pub color: Color,
}

/// A tinted glow that fades outward from the coast over the water. A width of zero draws
/// nothing.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShoreGlow {
    /// How far out the glow reaches, in Canvas units.
    pub width: f64,
    /// The colour at the coast; its alpha is the strength there, fading to nothing outward.
    pub color: Color,
}

impl Default for ShoreGlow {
    /// Off, with the colour a switched-on glow starts with.
    fn default() -> Self {
        Self {
            width: 0.0,
            color: Color::rgba(0x9f, 0xd0, 0xcc, 0x78),
        }
    }
}

/// Short horizontal lines on the water beside the coast, in rows. Where the coast runs up and
/// down they are long; where it runs along they hardly show. A length of zero draws nothing.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShoreLines {
    /// The gap between rows, in Canvas units.
    pub spacing: f64,
    /// How far out a line runs from the coast, in Canvas units. It stops short of other land.
    pub length: f64,
    /// The width of each line, in Canvas units.
    pub width: f64,
    /// The lines' colour; they fade out along their length.
    pub color: Color,
    /// How much each line thins towards its far end, from 0 (even width) to 1 (a point).
    /// Files saved before it existed have 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub taper: f64,
    /// How uneven the lines are in length, weight and gaps, from 0 (evenly spaced, all the
    /// same) to 1 (very hand-drawn). Files saved before it existed have 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub variation: f64,
    /// Picks which lines are uneven and how. The same seed gives the same lines.
    #[serde(default, skip_serializing_if = "is_zero_seed")]
    pub seed: u32,
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

fn is_zero_seed(value: &u32) -> bool {
    *value == 0
}

impl Default for ShoreLines {
    /// Off, with the values a switched-on set of lines starts with.
    fn default() -> Self {
        Self {
            spacing: 5.0,
            length: 0.0,
            width: 1.0,
            color: Color::rgba(0x86, 0xb8, 0xb4, 0xa0),
            taper: 0.0,
            variation: 0.0,
            seed: 0,
        }
    }
}

/// How the coast of a Landmass or Lake is drawn: outline, ripples, inner shading, and the
/// outer glow and water lines.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShoreStyle {
    /// The line along the coast.
    pub outline: ShoreOutline,
    /// The lines stepping outward over the water.
    pub ripples: ShoreRipples,
    /// The shading inward from the coast.
    pub shading: ShoreShading,
    /// The glow outward over the water. Files saved before it existed have none.
    #[serde(default, skip_serializing_if = "is_default_glow")]
    pub glow: ShoreGlow,
    /// The short horizontal lines on the water. Files saved before they existed have none.
    #[serde(default, skip_serializing_if = "is_default_lines")]
    pub lines: ShoreLines,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

fn is_default_glow(glow: &ShoreGlow) -> bool {
    *glow == ShoreGlow::default()
}

fn is_default_lines(lines: &ShoreLines) -> bool {
    *lines == ShoreLines::default()
}

impl ShoreStyle {
    /// The Map-wide default for Landmasses: a thin outline, three soft ripples and a warm
    /// inner glow.
    pub fn landmass_default() -> Self {
        Self {
            outline: ShoreOutline {
                width: 1.5,
                color: Color::rgb(0x5a, 0x46, 0x28),
            },
            ripples: ShoreRipples {
                count: 3,
                spacing: 12.0,
                width: 1.5,
                color: Color::rgba(0xff, 0xff, 0xff, 0x90),
                fade: 0.6,
            },
            shading: ShoreShading {
                width: 18.0,
                color: Color::rgba(0x8a, 0x6a, 0x38, 0x70),
            },
            glow: ShoreGlow::default(),
            lines: ShoreLines::default(),
            extra: Extra::new(),
        }
    }

    /// The Map-wide default for Lakes: an outline and a little shading, no ripples.
    pub fn lake_default() -> Self {
        Self {
            outline: ShoreOutline {
                width: 1.0,
                color: Color::rgb(0x3a, 0x6a, 0x8a),
            },
            ripples: ShoreRipples {
                count: 0,
                ..Self::landmass_default().ripples
            },
            shading: ShoreShading {
                width: 8.0,
                color: Color::rgba(0x2a, 0x5a, 0x7a, 0x60),
            },
            glow: ShoreGlow::default(),
            lines: ShoreLines::default(),
            extra: Extra::new(),
        }
    }

    /// No effects at all: a bare fill.
    pub fn plain() -> Self {
        let none = Self::landmass_default();
        Self {
            outline: ShoreOutline {
                width: 0.0,
                ..none.outline
            },
            ripples: ShoreRipples {
                count: 0,
                ..none.ripples
            },
            shading: ShoreShading {
                width: 0.0,
                ..none.shading
            },
            glow: ShoreGlow::default(),
            lines: ShoreLines::default(),
            extra: Extra::new(),
        }
    }

    /// A soft look for Landmasses: a thin dark outline, a pale glow over the water and short
    /// horizontal lines beside the coast, with no ripples and a little inner shade.
    pub fn soft_glow() -> Self {
        let base = Self::landmass_default();
        Self {
            outline: ShoreOutline {
                width: 1.0,
                color: Color::rgb(0x4a, 0x3a, 0x2a),
            },
            ripples: ShoreRipples {
                count: 0,
                ..base.ripples
            },
            shading: ShoreShading {
                width: 12.0,
                color: Color::rgba(0x8a, 0x6a, 0x38, 0x50),
            },
            glow: ShoreGlow {
                width: 26.0,
                ..ShoreGlow::default()
            },
            lines: ShoreLines {
                length: 44.0,
                ..ShoreLines::default()
            },
            extra: Extra::new(),
        }
    }

    /// The named presets offered for a kind of Object, in the order a picker lists them.
    /// Landmass-only looks (those with water lines) are not offered for Lakes.
    pub fn presets(kind: ShoreKind) -> Vec<(&'static str, Self)> {
        match kind {
            ShoreKind::Landmass => vec![
                ("Default", Self::landmass_default()),
                ("Soft glow", Self::soft_glow()),
            ],
            ShoreKind::Lake => vec![("Default", Self::lake_default())],
        }
    }

    /// The name of the preset for `kind` whose fields equal this style's, if any. Fields from
    /// a newer file format are not compared.
    pub fn preset_name(&self, kind: ShoreKind) -> Option<&'static str> {
        let mut plain = self.clone();
        plain.extra = Extra::new();
        Self::presets(kind)
            .into_iter()
            .find_map(|(name, preset)| (preset == plain).then_some(name))
    }

    /// Why these settings cannot be used, if they cannot.
    pub fn problem(&self) -> Option<String> {
        let finite_from_zero = |name: &str, value: f64| {
            (!(value.is_finite() && value >= 0.0))
                .then(|| format!("{name} must be 0 or more, not {value}"))
        };
        finite_from_zero("the outline width", self.outline.width)
            .or_else(|| finite_from_zero("the ripple width", self.ripples.width))
            .or_else(|| finite_from_zero("the shading width", self.shading.width))
            .or_else(|| {
                (!(self.glow.width.is_finite() && (0.0..=MAX_REACH).contains(&self.glow.width)))
                    .then(|| {
                        format!(
                            "the glow width must be from 0 to {MAX_REACH}, not {}",
                            self.glow.width
                        )
                    })
            })
            .or_else(|| self.lines.problem())
            .or_else(|| {
                (self.ripples.count > MAX_RIPPLES)
                    .then(|| format!("there can be at most {MAX_RIPPLES} ripples"))
            })
            .or_else(|| {
                (self.ripples.count > 0
                    && !(self.ripples.spacing.is_finite() && self.ripples.spacing > 0.0))
                    .then(|| {
                        format!(
                            "the ripple spacing must be above 0, not {}",
                            self.ripples.spacing
                        )
                    })
            })
            .or_else(|| {
                (!(0.0..=1.0).contains(&self.ripples.fade)).then(|| {
                    format!(
                        "the ripple fade must be from 0 to 1, not {}",
                        self.ripples.fade
                    )
                })
            })
    }
}

impl ShoreLines {
    /// Why these settings cannot be used, if they cannot.
    pub fn problem(&self) -> Option<String> {
        let reach = |name: &str, value: f64, max: f64| {
            (!(value.is_finite() && (0.0..=max).contains(&value)))
                .then(|| format!("the line {name} must be from 0 to {max}, not {value}"))
        };
        let unit = |name: &str, value: f64| {
            (!(value.is_finite() && (0.0..=1.0).contains(&value)))
                .then(|| format!("the line {name} must be from 0 to 1, not {value}"))
        };
        reach("length", self.length, MAX_REACH)
            .or_else(|| reach("width", self.width, 50.0))
            .or_else(|| unit("taper", self.taper))
            .or_else(|| unit("variation", self.variation))
            .or_else(|| {
                (!(self.spacing.is_finite()
                    && (MIN_LINE_SPACING..=MAX_REACH).contains(&self.spacing)))
                .then(|| {
                    format!(
                        "the line spacing must be from {MIN_LINE_SPACING} to {MAX_REACH}, not {}",
                        self.spacing
                    )
                })
            })
    }
}

/// The two kinds of Object that have a Shore style, and so two Map-wide defaults.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShoreKind {
    /// The Shore style of Landmasses.
    Landmass,
    /// The Shore style of Lakes.
    Lake,
}
