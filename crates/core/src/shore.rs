//! Shore style: how a coastline is dressed (ADR 0002). Effects are drawn on the roughened
//! outline: an outline, ripple lines stepping outward over the water, and shading inward from
//! the coast. A Map-wide default exists for Landmasses and another for Lakes; either kind can
//! override its default.
#![allow(
    missing_docs,
    reason = "still to document; remove this once the file is done (#351)"
)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::extra::Extra;

/// At most this many ripple lines, to bound the geometry they cost.
pub const MAX_RIPPLES: u32 = 12;

/// A line along the coast. A width of zero draws nothing.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShoreOutline {
    /// In Canvas units.
    pub width: f64,
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

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShoreStyle {
    pub outline: ShoreOutline,
    pub ripples: ShoreRipples,
    pub shading: ShoreShading,
    #[serde(flatten)]
    pub extra: Extra,
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
            extra: Extra::new(),
        }
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

/// The two kinds of Object that have a Shore style, and so two Map-wide defaults.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShoreKind {
    Landmass,
    Lake,
}
