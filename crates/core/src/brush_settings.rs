//! The brush settings a Map remembers: what each mode of the Land Brush, the Lake Brush and the
//! Paint tool was last set to. Like the export settings they are a preference kept with the
//! Map, not Map content, and are written outside Commands (ADR 0020).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::extra::Extra;

/// The brush diameter every brush mode starts with, in Canvas units.
pub const DEFAULT_BRUSH_SIZE: f64 = 48.0;

/// The Land Brush's starting Irregularity: enough that strokes look natural, not round.
pub const DEFAULT_BRUSH_IRREGULARITY: f64 = 0.15;

/// What one Land Brush mode (Paint or Erase) keeps.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LandBrushSettings {
    /// The brush diameter, in Canvas units.
    pub size: f64,
    /// How far the brush pushes a stroke's outline out and in, as a share of its radius.
    pub irregularity: f64,
}

impl Default for LandBrushSettings {
    fn default() -> Self {
        Self {
            size: DEFAULT_BRUSH_SIZE,
            irregularity: DEFAULT_BRUSH_IRREGULARITY,
        }
    }
}

/// What one Paint tool mode (Paint or Erase) keeps: the settings a stroke in that mode takes.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PaintBrushSettings {
    /// The brush diameter, in Canvas units.
    pub size: f64,
    /// How soft the brush edge is, from 0 (hard) to 1.
    pub softness: f64,
    /// How strong a stroke is, from 0 to 1.
    pub opacity: f64,
}

impl Default for PaintBrushSettings {
    fn default() -> Self {
        Self {
            size: DEFAULT_BRUSH_SIZE,
            softness: 0.3,
            opacity: 1.0,
        }
    }
}

/// The settings of every brush mode, saved with the Map. Files saved before they existed get
/// the defaults, and so does any field a file leaves out.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrushSettings {
    /// The Land Brush's Paint mode.
    pub land_paint: LandBrushSettings,
    /// The Land Brush's Erase mode.
    pub land_erase: LandBrushSettings,
    /// The Lake Brush's Paint mode diameter, in Canvas units.
    pub water_paint_size: f64,
    /// The Lake Brush's Erase mode diameter, in Canvas units.
    pub water_erase_size: f64,
    /// The Paint tool's colour. Only its Paint mode uses it.
    pub paint_color: Color,
    /// The Paint tool's Paint mode.
    pub painting: PaintBrushSettings,
    /// The Paint tool's Erase mode.
    pub erasing: PaintBrushSettings,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            land_paint: LandBrushSettings::default(),
            land_erase: LandBrushSettings::default(),
            water_paint_size: DEFAULT_BRUSH_SIZE,
            water_erase_size: DEFAULT_BRUSH_SIZE,
            paint_color: Color::rgb(0xc2, 0x4d, 0x3a),
            painting: PaintBrushSettings::default(),
            erasing: PaintBrushSettings::default(),
            extra: Extra::new(),
        }
    }
}

impl BrushSettings {
    /// These are the settings of a Map whose brushes nobody has changed.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_object_fills_the_rest_with_defaults() {
        let settings: BrushSettings =
            serde_json::from_str(r#"{"land_erase":{"size":120.0}}"#).expect("parses");
        assert_eq!(settings.land_erase.size, 120.0);
        assert_eq!(settings.land_erase.irregularity, DEFAULT_BRUSH_IRREGULARITY);
        assert_eq!(settings.land_paint, LandBrushSettings::default());
        assert_eq!(settings.painting, PaintBrushSettings::default());
    }

    #[test]
    fn unknown_fields_round_trip() {
        let settings: BrushSettings =
            serde_json::from_str(r#"{"pencil":{"size":3}}"#).expect("parses");
        assert!(!settings.is_default());
        let json = serde_json::to_value(&settings).expect("serialises");
        assert_eq!(json["pencil"]["size"], 3);
    }
}
