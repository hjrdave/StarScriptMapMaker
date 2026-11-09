//! Fill style: how a Landmass's fill colour is dressed (ADR 0015). A flat fill is one colour;
//! grain adds fine speckle to it. The grain is not stored as pixels: the renderer works it out
//! from the Canvas position, so it looks the same at any zoom and in export.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most grain strength there is: the fill's brightness varies by up to this fraction.
pub const MAX_GRAIN_AMOUNT: f64 = 1.0;

/// The smallest grain size, in Canvas units. Anything finer would be lost between samples.
pub const MIN_GRAIN_SIZE: f64 = 0.25;

/// The largest grain size, in Canvas units.
pub const MAX_GRAIN_SIZE: f64 = 16.0;

/// Fine speckle laid over a fill colour.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Grain {
    /// How much each speck lightens or darkens the fill, from 0 (none) to 1 (the full range).
    pub amount: f64,
    /// The width of one speck, in Canvas units.
    pub size: f64,
}

impl Grain {
    /// A light grain, for a new Landmass that is switched to grain.
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

/// How a Landmass's fill colour is drawn.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FillStyle {
    /// The fill colour, as it is.
    #[default]
    Flat,
    /// The fill colour with fine speckle over it.
    Grain(Grain),
}

impl FillStyle {
    /// Whether this is the plain colour. Flat is the default, so it is left out of the file.
    pub fn is_flat(&self) -> bool {
        matches!(self, FillStyle::Flat)
    }

    /// The grain, if the style has any.
    pub fn grain(&self) -> Option<Grain> {
        match self {
            FillStyle::Flat => None,
            FillStyle::Grain(grain) => Some(*grain),
        }
    }

    /// Why this style cannot be used, if it cannot.
    pub fn problem(&self) -> Option<String> {
        self.grain().and_then(|grain| grain.problem())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_is_the_default_and_has_no_grain() {
        assert_eq!(FillStyle::default(), FillStyle::Flat);
        assert!(FillStyle::Flat.is_flat());
        assert_eq!(FillStyle::Flat.grain(), None);
    }

    #[test]
    fn grain_reports_its_settings() {
        let style = FillStyle::Grain(Grain::DEFAULT);
        assert!(!style.is_flat());
        assert_eq!(style.grain(), Some(Grain::DEFAULT));
        assert_eq!(style.problem(), None);
    }

    #[test]
    fn out_of_range_grain_is_a_problem() {
        let grain = |amount, size| FillStyle::Grain(Grain { amount, size });
        assert!(grain(-0.1, 2.0).problem().is_some());
        assert!(grain(1.5, 2.0).problem().is_some());
        assert!(grain(f64::NAN, 2.0).problem().is_some());
        assert!(grain(0.5, 0.0).problem().is_some());
        assert!(grain(0.5, 100.0).problem().is_some());
        assert!(grain(0.5, f64::INFINITY).problem().is_some());
        assert_eq!(grain(0.0, MIN_GRAIN_SIZE).problem(), None);
        assert_eq!(grain(MAX_GRAIN_AMOUNT, MAX_GRAIN_SIZE).problem(), None);
    }

    #[test]
    fn it_round_trips_through_json() {
        for style in [FillStyle::Flat, FillStyle::Grain(Grain::DEFAULT)] {
            let json = serde_json::to_string(&style).unwrap();
            assert_eq!(serde_json::from_str::<FillStyle>(&json).unwrap(), style);
        }
    }
}
