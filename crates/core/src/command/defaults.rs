//! The Commands that change the Map-wide defaults and the Canvas background.

use crate::background::Background;
use crate::fill::FillStyle;
use crate::map::{Map, MapDefaults};
use crate::op::Op;
use crate::roughness::Roughness;
use crate::shore::{ShoreKind, ShoreStyle};

use super::CommandError;
use super::land::valid_roughness;

/// Sets the Map-wide Shore style of every Landmass, or every Lake, without its own.
pub(super) fn set_default_shore(
    map: &Map,
    kind: ShoreKind,
    shore: &ShoreStyle,
) -> Result<Vec<Op>, CommandError> {
    if let Some(problem) = shore.problem() {
        return Err(CommandError::InvalidShore(problem));
    }
    let mut defaults = map.defaults().clone();
    match kind {
        ShoreKind::Landmass => defaults.shore = shore.clone(),
        ShoreKind::Lake => defaults.lake_shore = shore.clone(),
    }
    Ok(vec![Op::SetDefaults { defaults }])
}

/// Sets the Map-wide Fill style of every Landmass without its own.
pub(super) fn set_default_fill_style(
    map: &Map,
    style: &FillStyle,
) -> Result<Vec<Op>, CommandError> {
    if let Some(problem) = style.problem() {
        return Err(CommandError::InvalidFillStyle(problem));
    }
    Ok(vec![Op::SetDefaults {
        defaults: MapDefaults {
            fill_style: style.clone(),
            ..map.defaults().clone()
        },
    }])
}

/// Replaces the Canvas's background. The texture strength must be 0 to 1 and its scale above 0.
pub(super) fn set_background(background: &Background) -> Result<Vec<Op>, CommandError> {
    let texture = &background.texture;
    if !(0.0..=1.0).contains(&texture.strength)
        || !texture.scale.is_finite()
        || texture.scale <= 0.0
    {
        return Err(CommandError::InvalidBackground);
    }
    Ok(vec![Op::SetBackground {
        background: background.clone(),
    }])
}

/// Sets the Roughness of every Landmass and Lake without its own.
pub(super) fn set_default_roughness(
    map: &Map,
    roughness: &Roughness,
) -> Result<Vec<Op>, CommandError> {
    valid_roughness(roughness)?;
    Ok(vec![Op::SetDefaults {
        defaults: MapDefaults {
            roughness: roughness.clone(),
            ..map.defaults().clone()
        },
    }])
}
