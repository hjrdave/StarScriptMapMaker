//! Fill style: how a Landmass's fill colour is dressed (ADR 0015). It can have grain (fine
//! speckle), a mottle (large soft patches of a second colour), both, or neither (a flat
//! fill). Neither is stored as pixels: the renderer works them out from the Canvas position,
//! so they look the same at any zoom and in export.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::asset::AssetRef;
use crate::color::Color;

/// The most grain strength there is: the fill's brightness varies by up to this fraction.
pub const MAX_GRAIN_AMOUNT: f64 = 1.0;

/// The smallest grain size, in Canvas units. Anything finer would be lost between samples.
pub const MIN_GRAIN_SIZE: f64 = 0.25;

/// The largest grain size, in Canvas units.
pub const MAX_GRAIN_SIZE: f64 = 16.0;

/// The smallest mottle scale, in Canvas units.
pub const MIN_MOTTLE_SCALE: f64 = 10.0;

/// The largest mottle scale, in Canvas units.
pub const MAX_MOTTLE_SCALE: f64 = 5000.0;

/// The smallest texture scale, in Canvas units: how wide one tile of the image is.
pub const MIN_TEXTURE_SCALE: f64 = 8.0;

/// The largest texture scale, in Canvas units.
pub const MAX_TEXTURE_SCALE: f64 = 5000.0;

/// Fine speckle laid over a fill colour.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Grain {
    /// How much each speck lightens or darkens the fill, from 0 (none) to 1 (the full range).
    pub amount: f64,
    /// The width of one speck, in Canvas units.
    pub size: f64,
}

impl Grain {
    /// A light grain, for a Landmass that is switched to grain.
    pub const DEFAULT: Grain = Grain {
        amount: 0.25,
        size: 2.0,
    };

    /// Why these settings cannot be used, if they cannot.
    pub fn problem(&self) -> Option<String> {
        if !(self.amount.is_finite() && (0.0..=MAX_GRAIN_AMOUNT).contains(&self.amount)) {
            return Some(format!(
                "the grain amount must be from 0 to {MAX_GRAIN_AMOUNT}, not {}",
                self.amount
            ));
        }
        if !(self.size.is_finite() && (MIN_GRAIN_SIZE..=MAX_GRAIN_SIZE).contains(&self.size)) {
            return Some(format!(
                "the grain size must be from {MIN_GRAIN_SIZE} to {MAX_GRAIN_SIZE}, not {}",
                self.size
            ));
        }
        None
    }
}

/// Large soft patches of a second colour that the fill colour blends into and out of.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Mottle {
    /// The colour of the patches. Its alpha is ignored: the fill's own alpha applies.
    pub color: Color,
    /// How wide a patch is, in Canvas units.
    pub scale: f64,
    /// How far the patches go towards their colour, from 0 (not at all) to 1 (all the way).
    pub contrast: f64,
    /// Picks which patches there are. The same seed gives the same patches.
    pub seed: u32,
}

impl Mottle {
    /// A soft darker tan, for a Landmass that is switched to mottled.
    pub const DEFAULT: Mottle = Mottle {
        color: Color::rgb(0xb0, 0x9a, 0x62),
        scale: 150.0,
        contrast: 0.5,
        seed: 1,
    };

    /// Why these settings cannot be used, if they cannot.
    pub fn problem(&self) -> Option<String> {
        if !(self.scale.is_finite() && (MIN_MOTTLE_SCALE..=MAX_MOTTLE_SCALE).contains(&self.scale))
        {
            return Some(format!(
                "the mottle scale must be from {MIN_MOTTLE_SCALE} to {MAX_MOTTLE_SCALE}, not {}",
                self.scale
            ));
        }
        if !(self.contrast.is_finite() && (0.0..=1.0).contains(&self.contrast)) {
            return Some(format!(
                "the mottle contrast must be from 0 to 1, not {}",
                self.contrast
            ));
        }
        None
    }
}

/// A seamless tiling image (a texture Asset in a pack) blended over the fill colour.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FillTexture {
    /// The texture Asset, written `pack-id/texture-id`.
    pub asset: AssetRef,
    /// How strongly the image shows over the fill colour, from 0 (not at all) to 1 (fully).
    pub amount: f64,
    /// How wide one tile of the image is, in Canvas units.
    pub scale: f64,
}

impl FillTexture {
    /// A texture of `asset` at a middling strength and size.
    pub fn new(asset: AssetRef) -> Self {
        Self {
            asset,
            amount: 0.5,
            scale: 128.0,
        }
    }

    /// Why these settings cannot be used, if they cannot.
    pub fn problem(&self) -> Option<String> {
        if !(self.amount.is_finite() && (0.0..=1.0).contains(&self.amount)) {
            return Some(format!(
                "the texture amount must be from 0 to 1, not {}",
                self.amount
            ));
        }
        if !(self.scale.is_finite()
            && (MIN_TEXTURE_SCALE..=MAX_TEXTURE_SCALE).contains(&self.scale))
        {
            return Some(format!(
                "the texture scale must be from {MIN_TEXTURE_SCALE} to {MAX_TEXTURE_SCALE}, not {}",
                self.scale
            ));
        }
        None
    }
}

/// How a Landmass's fill colour is drawn. With no grain, mottle or texture it is flat.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct FillStyle {
    /// Fine speckle over the fill, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<Grain>,
    /// Large soft patches of a second colour, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mottle: Option<Mottle>,
    /// A tiling texture image blended over the fill colour, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<FillTexture>,
}

impl FillStyle {
    /// The plain colour, with nothing over it.
    pub const FLAT: FillStyle = FillStyle {
        grain: None,
        mottle: None,
        texture: None,
    };

    /// Whether this is the plain colour. Flat is the default, so it is left out of the file.
    pub fn is_flat(&self) -> bool {
        self.grain.is_none() && self.mottle.is_none() && self.texture.is_none()
    }

    /// Why this style cannot be used, if it cannot.
    pub fn problem(&self) -> Option<String> {
        self.grain
            .as_ref()
            .and_then(Grain::problem)
            .or_else(|| self.mottle.as_ref().and_then(Mottle::problem))
            .or_else(|| self.texture.as_ref().and_then(FillTexture::problem))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_is_the_default_and_has_nothing_over_it() {
        assert_eq!(FillStyle::default(), FillStyle::FLAT);
        assert!(FillStyle::FLAT.is_flat());
        assert_eq!(FillStyle::FLAT.grain, None);
        assert_eq!(FillStyle::FLAT.mottle, None);
    }

    #[test]
    fn grain_or_mottle_or_both_is_not_flat() {
        let grain = FillStyle {
            grain: Some(Grain::DEFAULT),
            ..FillStyle::FLAT
        };
        let mottle = FillStyle {
            mottle: Some(Mottle::DEFAULT),
            ..FillStyle::FLAT
        };
        let both = FillStyle {
            grain: Some(Grain::DEFAULT),
            mottle: Some(Mottle::DEFAULT),
            ..FillStyle::FLAT
        };
        for style in [grain, mottle, both] {
            assert!(!style.is_flat());
            assert_eq!(style.problem(), None);
        }
    }

    #[test]
    fn out_of_range_grain_is_a_problem() {
        let grain = |amount, size| Grain { amount, size }.problem();
        assert!(grain(-0.1, 2.0).is_some());
        assert!(grain(1.5, 2.0).is_some());
        assert!(grain(f64::NAN, 2.0).is_some());
        assert!(grain(0.5, 0.0).is_some());
        assert!(grain(0.5, 100.0).is_some());
        assert!(grain(0.5, f64::INFINITY).is_some());
        assert_eq!(grain(0.0, MIN_GRAIN_SIZE), None);
        assert_eq!(grain(MAX_GRAIN_AMOUNT, MAX_GRAIN_SIZE), None);
    }

    #[test]
    fn out_of_range_mottle_is_a_problem() {
        let mottle = |scale, contrast| {
            Mottle {
                scale,
                contrast,
                ..Mottle::DEFAULT
            }
            .problem()
        };
        assert!(mottle(1.0, 0.5).is_some());
        assert!(mottle(9000.0, 0.5).is_some());
        assert!(mottle(f64::NAN, 0.5).is_some());
        assert!(mottle(150.0, -0.1).is_some());
        assert!(mottle(150.0, 1.1).is_some());
        assert!(mottle(150.0, f64::NAN).is_some());
        assert_eq!(mottle(MIN_MOTTLE_SCALE, 0.0), None);
        assert_eq!(mottle(MAX_MOTTLE_SCALE, 1.0), None);
    }

    #[test]
    fn a_style_is_a_problem_if_either_part_is() {
        let bad_grain = Grain {
            amount: 2.0,
            ..Grain::DEFAULT
        };
        let style = FillStyle {
            grain: Some(bad_grain),
            mottle: Some(Mottle::DEFAULT),
            ..FillStyle::FLAT
        };
        assert!(style.problem().is_some());
    }

    fn paper() -> FillTexture {
        FillTexture::new("p/paper".parse().unwrap())
    }

    #[test]
    fn a_texture_is_not_flat_and_is_checked() {
        let texture = |amount, scale| {
            FillTexture {
                amount,
                scale,
                ..paper()
            }
            .problem()
        };
        assert_eq!(texture(0.0, MIN_TEXTURE_SCALE), None);
        assert_eq!(texture(1.0, MAX_TEXTURE_SCALE), None);
        assert!(texture(-0.1, 100.0).is_some());
        assert!(texture(1.1, 100.0).is_some());
        assert!(texture(f64::NAN, 100.0).is_some());
        assert!(texture(0.5, 1.0).is_some());
        assert!(texture(0.5, 1e6).is_some());
        assert!(texture(0.5, f64::INFINITY).is_some());
        let style = FillStyle {
            texture: Some(paper()),
            ..FillStyle::FLAT
        };
        assert!(!style.is_flat());
        assert_eq!(style.problem(), None);
        let bad = FillStyle {
            texture: Some(FillTexture {
                amount: 2.0,
                ..paper()
            }),
            ..FillStyle::FLAT
        };
        assert!(bad.problem().is_some());
    }

    #[test]
    fn it_round_trips_through_json_and_leaves_out_what_is_off() {
        let both = FillStyle {
            grain: Some(Grain::DEFAULT),
            mottle: Some(Mottle::DEFAULT),
            ..FillStyle::FLAT
        };
        let textured = FillStyle {
            texture: Some(paper()),
            ..both.clone()
        };
        for style in [FillStyle::FLAT, both, textured] {
            let json = serde_json::to_string(&style).unwrap();
            assert_eq!(serde_json::from_str::<FillStyle>(&json).unwrap(), style);
        }
        assert_eq!(serde_json::to_string(&FillStyle::FLAT).unwrap(), "{}");
        let grain_only = serde_json::to_value(FillStyle {
            grain: Some(Grain::DEFAULT),
            ..FillStyle::FLAT
        })
        .unwrap();
        assert!(grain_only.get("mottle").is_none());
        assert!(grain_only.get("texture").is_none());
    }
}
