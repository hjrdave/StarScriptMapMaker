//! Templates: pre-filled Maps that new Maps start from.
//!
//! A template is a data file (`templates/*.json` in the assets folder; `starter.json` is the
//! default) in the shape of [`MapView`], plus a display `name` and a `format_version` naming
//! the Map file format its shape follows. Starting a Map from it replaces the template's
//! Canvas size and scale with the ones the user chose; the template's background stays, and the
//! Map remembers the template's name.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::asset::assets_dir;
use crate::background::Background;
use crate::file::{FORMAT_VERSION, FormatVersion};
use crate::id::Id;
use crate::map::{Canvas, InvariantError, Map, MapDefaults, MapView};

/// The starter template's file, inside the assets folder.
pub const STARTER_TEMPLATE: &str = "templates/starter.json";

/// The folder of templates, inside the assets folder.
const TEMPLATES_DIR: &str = "templates";

/// The starter template's id: its file name without the extension.
const STARTER_TEMPLATE_ID: &str = "starter";

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
    /// The name the template is listed under.
    name: String,
    #[serde(default)]
    default_layers: DefaultLayers,
    #[serde(flatten)]
    view: MapView,
}

/// A pre-filled Map to start new Maps from.
#[derive(Clone, PartialEq, Debug)]
pub struct MapTemplate {
    id: String,
    name: String,
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
            id: String::new(),
            name: file.name,
            view: file.view,
            default_layers: file.default_layers,
        })
    }

    /// The Layers new Objects go into at first.
    pub fn default_layers(&self) -> DefaultLayers {
        self.default_layers
    }

    /// The template's file name without its extension, which names it in code. Empty for a
    /// template read from text.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The name the template is listed under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The background new Maps from this template get.
    pub fn background(&self) -> &Background {
        &self.view.canvas.background
    }

    /// The Map-wide defaults new Maps from this template get.
    pub fn defaults(&self) -> &MapDefaults {
        &self.view.defaults
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, TemplateError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| TemplateError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut template = Self::parse(&text)?;
        if let Some(stem) = path.file_stem() {
            template.id = stem.to_string_lossy().into_owned();
        }
        Ok(template)
    }

    /// The starter template installed with the app.
    pub fn starter() -> Result<Self, TemplateError> {
        Self::load(assets_dir().join(STARTER_TEMPLATE))
    }

    /// Every template in the assets folder: the starter first, then the rest by file name. A
    /// template that can't be loaded is returned among the problems and left out of the list.
    pub fn load_all() -> (Vec<Self>, Vec<TemplateError>) {
        Self::load_dir(&assets_dir().join(TEMPLATES_DIR))
    }

    /// Every `.json` template in `dir`, in the order [`load_all`](Self::load_all) lists them.
    pub fn load_dir(dir: &Path) -> (Vec<Self>, Vec<TemplateError>) {
        let mut templates = Vec::new();
        let mut problems = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(source) => {
                let path = dir.to_path_buf();
                return (templates, vec![TemplateError::Read { path, source }]);
            }
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        for path in paths {
            match Self::load(&path) {
                Ok(template) => templates.push(template),
                Err(err) => problems.push(err),
            }
        }
        // A stable sort, so the rest stay in file-name order.
        templates.sort_by_key(|template| template.id != STARTER_TEMPLATE_ID);
        (templates, problems)
    }

    /// A new Map from the template with the given Canvas, which takes on the template's
    /// background.
    pub fn new_map(&self, mut canvas: Canvas) -> Map {
        canvas.background = self.view.canvas.background.clone();
        canvas.template = Some(self.name.clone());
        let view = MapView {
            canvas,
            ..self.view.clone()
        };
        Map::from_view(view).expect("the template was checked when loaded")
    }
}
