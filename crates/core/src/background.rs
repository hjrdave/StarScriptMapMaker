//! The Map's background: the water colour and the procedural texture drawn over it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::extra::Extra;

/// The look of the Canvas behind the Objects. The Map keeps its own copy, so changing a
/// template file later never changes an existing Map.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Background {
    /// The flat colour under the texture.
    pub water: Color,
    /// The mottling over the water.
    #[serde(default)]
    pub texture: BackgroundTexture,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The mottling over the water: noise that lightens and darkens it. Fields left out take
/// their defaults.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BackgroundTexture {
    /// How strongly the texture shows, from 0 to 1. At 0 the background is the flat colour.
    pub strength: f32,
    /// Stretches the noise: 1 is the default size, 2 makes the mottling twice as large.
    pub scale: f32,
    /// The colour the noise lightens towards.
    pub light: Color,
    /// The colour the noise darkens towards.
    pub dark: Color,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for Background {
    fn default() -> Self {
        Self {
            water: Color::rgb(0x3d, 0x6e, 0x8c),
            texture: BackgroundTexture::default(),
            extra: Extra::new(),
        }
    }
}

impl Default for BackgroundTexture {
    fn default() -> Self {
        Self {
            strength: 0.3,
            scale: 1.0,
            light: Color::WHITE,
            dark: Color::BLACK,
            extra: Extra::new(),
        }
    }
}

impl Background {
    /// Whether this is the default look, which files leave out.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
