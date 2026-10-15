//! The starter template: a pre-filled Map that every new Map starts from.
//!
//! A template is a data file (`templates/starter.json` in the assets folder) in the shape of
//! [`MapView`], plus a `format_version` naming the Map file format its shape follows. Starting
//! a Map from it replaces the template's Canvas with the one the user chose.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::asset::assets_dir;
use crate::file::{FORMAT_VERSION, FormatVersion};
use crate::id::Id;
use crate::map::{Canvas, InvariantError, Map, MapView};

/// The starter template's file, inside the assets folder.
pub const STARTER_TEMPLATE: &str = "templates/starter.json";

#[derive(Debug, Error)]
pub enum TemplateError {
    #[error("couldn't read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the template is not valid: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("the template's format_version “{0}” is not `major.minor`")]
    BadVersion(String),
    #[error(
        "the template uses Map format {found}, but this version reads {}.x",
        FORMAT_VERSION.major
    )]
    UnsupportedVersion { found: FormatVersion },
    #[error("the template is not a valid Map: {0}")]
    Invalid(#[from] InvariantError),
}

/// The Layers new Objects go into while no Layer is active, so a new Map is ready to paint
/// land, scatter Stamps and add Labels with no setup. A Layer the Map no longer has is ignored.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Deserialize)]
pub struct DefaultLayers {
    pub land: Option<Id>,
    pub assets: Option<Id>,
    pub labels: Option<Id>,
}

#[derive(Deserialize)]
struct TemplateFile {
    format_version: String,
    #[serde(default)]
    default_layers: DefaultLayers,
    #[serde(flatten)]
    view: MapView,
}

/// A pre-filled Map to start new Maps from.
#[derive(Clone, PartialEq, Debug)]
pub struct MapTemplate {
    view: MapView,
    default_layers: DefaultLayers,
}

impl MapTemplate {
    /// Reads a template from JSON text. A minor version newer than ours is read, since minor
    /// bumps only add optional fields; a different major is refused.
    pub fn parse(text: &str) -> Result<Self, TemplateError> {
        let file: TemplateFile = serde_json::from_str(text)?;
        let found: FormatVersion = file
            .format_version
            .parse()
            .map_err(|()| TemplateError::BadVersion(file.format_version.clone()))?;
        if found.major != FORMAT_VERSION.major {
            return Err(TemplateError::UnsupportedVersion { found });
        }
        // Checked now, so a broken template fails when loaded and not when a Map is made.
        Map::from_view(file.view.clone())?;
        Ok(Self {
            view: file.view,
            default_layers: file.default_layers,
        })
    }

    /// The Layers new Objects go into at first.
    pub fn default_layers(&self) -> DefaultLayers {
        self.default_layers
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, TemplateError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| TemplateError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text)
    }

    /// The starter template installed with the app.
    pub fn starter() -> Result<Self, TemplateError> {
        Self::load(assets_dir().join(STARTER_TEMPLATE))
    }

    /// A new Map from the template with the given Canvas.
    pub fn new_map(&self, canvas: Canvas) -> Map {
        let view = MapView {
            canvas,
            ..self.view.clone()
        };
        Map::from_view(view).expect("the template was checked when loaded")
    }
}
