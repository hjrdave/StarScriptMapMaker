//! PNG export settings and the size maths behind them (ADR 0007).
//!
//! Only the settings live here; rendering and UI come later. The settings are stored in the
//! Map (ADR 0005) with the file name only, never an absolute path.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::extra::Extra;
use crate::geometry::Rect;
use crate::map::Canvas;

/// The largest export side in pixels.
pub const MAX_EXPORT_SIDE: u32 = 32_768;
/// Sides above this many pixels still export, but the user is warned.
pub const WARN_EXPORT_SIDE: u32 = 16_384;

/// How the pixel size of an export is chosen.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SizeMode {
    /// A multiple of the region's nominal 1x pixel size (1x, 2x, 4x or custom).
    Scale {
        /// The multiple of the region's nominal size.
        factor: f64,
    },
    /// A target width in pixels; the height follows the region's aspect ratio.
    Width {
        /// The width in pixels.
        pixels: u32,
    },
    /// A print size: a width in inches at `dpi`; the height follows the region's aspect ratio.
    Print {
        /// The print width in inches.
        width_inches: f64,
        /// Dots per inch.
        dpi: u32,
    },
}

/// The part of the Canvas to export.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExportRegion {
    /// The whole Canvas.
    Canvas,
    /// A rectangle in Canvas units.
    Custom {
        /// The rectangle to export.
        rect: Rect,
    },
}

/// What to do about the credits of credit-required Assets (ADR 0003).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreditsOptions {
    /// Write `<name>.credits.txt` beside the PNG.
    pub text_file: bool,
    /// Write the credits to a PNG text chunk.
    pub png_chunk: bool,
}

impl Default for CreditsOptions {
    fn default() -> Self {
        Self {
            text_file: true,
            png_chunk: true,
        }
    }
}

/// The PNG text chunk keyword the credits are written under.
pub const CREDITS_CHUNK_KEYWORD: &str = "Credits";

/// What an export writes for its credits.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CreditsOutput {
    /// The text of `<name>.credits.txt`.
    pub file: Option<String>,
    /// The text of the PNG text chunk.
    pub chunk: Option<String>,
}

impl CreditsOptions {
    /// What to write for `lines`. A Map with no credit lines gets neither output.
    pub fn output(&self, lines: &[String]) -> CreditsOutput {
        if lines.is_empty() {
            return CreditsOutput {
                file: None,
                chunk: None,
            };
        }
        let mut text = lines.join("\n");
        text.push('\n');
        CreditsOutput {
            file: self.text_file.then(|| text.clone()),
            chunk: self.png_chunk.then_some(text),
        }
    }
}

/// `<name>.credits.txt` beside the PNG at `png`.
pub fn credits_file_path(png: &Path) -> PathBuf {
    let mut name = png.file_stem().unwrap_or_default().to_owned();
    name.push(".credits.txt");
    png.with_file_name(name)
}

/// The last export settings of a Map.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExportSettings {
    /// How the pixel size is chosen.
    pub size: SizeMode,
    /// The part of the Canvas to export.
    pub region: ExportRegion,
    /// Skip the Canvas background fill.
    pub transparent_background: bool,
    /// What to do about the credits of credit-required Assets.
    pub credits: CreditsOptions,
    /// The file name only, with no directory. `None` until the first export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// Fields from a newer file format that this version keeps without reading.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            size: SizeMode::Scale { factor: 1.0 },
            region: ExportRegion::Canvas,
            transparent_background: false,
            credits: CreditsOptions::default(),
            file_name: None,
            extra: Extra::new(),
        }
    }
}

/// The pixel size an export comes out at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExportSize {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The DPI to write in the PNG, when a print size was chosen.
    pub dpi: Option<u32>,
    /// A side is above [`WARN_EXPORT_SIDE`].
    pub warn_large: bool,
}

/// Why an export size could not be worked out.
#[derive(Clone, Copy, PartialEq, Debug, Error)]
pub enum ExportSizeError {
    /// The export region has no area.
    #[error("the export region is empty")]
    EmptyRegion,
    /// The chosen size is not a positive number.
    #[error("the export size must be a positive number")]
    InvalidSize,
    /// A side comes out above [`MAX_EXPORT_SIDE`].
    #[error("a {width} × {height} px export is over the {MAX_EXPORT_SIDE}px limit per side")]
    TooLarge {
        /// The width the export would have, in pixels.
        width: u64,
        /// The height the export would have, in pixels.
        height: u64,
    },
}

impl ExportSettings {
    /// These are the settings of a Map nobody has exported yet.
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The region in Canvas units.
    pub fn region_rect(&self, canvas: &Canvas) -> Rect {
        match &self.region {
            ExportRegion::Canvas => canvas.rect(),
            ExportRegion::Custom { rect } => *rect,
        }
    }

    /// The pixel size of the export, or why it can't be made.
    pub fn output_size(&self, canvas: &Canvas) -> Result<ExportSize, ExportSizeError> {
        let region = self.region_rect(canvas);
        let (w, h) = (region.width(), region.height());
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
            return Err(ExportSizeError::EmptyRegion);
        }
        let positive = |v: f64| v.is_finite() && v > 0.0;
        let (width, height, dpi) = match self.size {
            SizeMode::Scale { factor } => {
                if !positive(factor) {
                    return Err(ExportSizeError::InvalidSize);
                }
                (w * factor, h * factor, None)
            }
            SizeMode::Width { pixels } => {
                if pixels == 0 {
                    return Err(ExportSizeError::InvalidSize);
                }
                (f64::from(pixels), f64::from(pixels) * h / w, None)
            }
            SizeMode::Print { width_inches, dpi } => {
                if !positive(width_inches) || dpi == 0 {
                    return Err(ExportSizeError::InvalidSize);
                }
                let width = width_inches * f64::from(dpi);
                (width, width * h / w, Some(dpi))
            }
        };
        let (width, height) = (width.round(), height.round());
        if width < 1.0 || height < 1.0 {
            return Err(ExportSizeError::InvalidSize);
        }
        let max = f64::from(MAX_EXPORT_SIDE);
        if width > max || height > max {
            // Saturating casts keep the error finite for absurd inputs.
            return Err(ExportSizeError::TooLarge {
                width: width as u64,
                height: height as u64,
            });
        }
        let (width, height) = (width as u32, height as u32);
        Ok(ExportSize {
            width,
            height,
            dpi,
            warn_large: width.max(height) > WARN_EXPORT_SIDE,
        })
    }
}
