//! Templates: pre-filled Maps that new Maps start from.
//!
//! A template is a data file (`templates/*.json` in the assets folder; `starter.json` is the
//! default) in the shape of [`MapView`], plus a display `name` and a `format_version` naming
//! the Map file format its shape follows. Starting a Map from it replaces the template's
//! Canvas size and scale with the ones the user chose; the template's background stays, and the
//! Map remembers the template's name.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::asset::{TrashError, assets_dir, user_data_dir};
use crate::background::Background;
use crate::file::{FORMAT_VERSION, FormatVersion};
use crate::id::Id;
use crate::map::{Canvas, InvariantError, Map, MapDefaults, MapView, Subtree};

/// The starter template's file, inside the assets folder.
const STARTER_TEMPLATE: &str = "templates/starter.json";

/// The folder of templates, inside the assets folder.
const TEMPLATES_DIR: &str = "templates";

/// The folder of user templates, inside the user data folder.
const USER_TEMPLATES_DIR: &str = "templates";

/// The starter template's id: its file name without the extension.
const STARTER_TEMPLATE_ID: &str = "starter";

/// Why a template could not be loaded.
#[derive(Debug, Error)]
pub enum TemplateError {
    /// The template file could not be read.
    #[error("couldn't read {path}: {source}")]
    Read {
        /// The file that could not be read.
        path: PathBuf,
        /// Why it failed.
        source: std::io::Error,
    },
    /// The file is not valid JSON for a template.
    #[error("the template is not valid: {0}")]
    Parse(#[from] serde_json::Error),
    /// The `format_version` is not `major.minor`.
    #[error("the template's format_version “{0}” is not `major.minor`")]
    BadVersion(String),
    /// The template uses a newer Map format than this version reads.
    #[error(
        "the template uses Map format {found}, but this version reads {}.x",
        FORMAT_VERSION.major
    )]
    UnsupportedVersion {
        /// The template's format version.
        found: FormatVersion,
    },
    /// The template does not describe a valid Map.
    #[error("the template is not a valid Map: {0}")]
    Invalid(#[from] InvariantError),
    /// The platform reports no per-user data folder.
    #[error("this system has no folder for user templates")]
    NoUserFolder,
    /// The template could not be written.
    #[error("couldn't save the template: {0}")]
    Write(std::io::Error),
    /// The template ships with the app, so it can't be deleted.
    #[error("“{0}” is built into the app and cannot be deleted")]
    BuiltIn(String),
    /// Moving the template to the Trash failed.
    #[error("cannot move the template to the Trash: {0}")]
    Trash(TrashError),
}

/// The Layers new Objects go into while no Layer is active, so a new Map is ready to paint
/// land, scatter Stamps and add Labels with no setup. A Layer the Map no longer has is ignored.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
pub struct DefaultLayers {
    /// Where new Landmasses and Lakes go.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub land: Option<Id>,
    /// Where new Stamps go.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<Id>,
    /// Where new Labels go.
    #[serde(skip_serializing_if = "Option::is_none")]
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

/// What [`MapTemplate::to_json`] writes: the file's fields, borrowed.
#[derive(Serialize)]
struct TemplateOut<'a> {
    format_version: String,
    name: &'a str,
    default_layers: DefaultLayers,
    #[serde(flatten)]
    view: &'a MapView,
}

/// A pre-filled Map to start new Maps from.
#[derive(Clone, PartialEq, Debug)]
pub struct MapTemplate {
    /// Where the file was read from or saved to, if it has one.
    path: Option<PathBuf>,
    /// Whether it was loaded from the user templates folder.
    user: bool,
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
            path: None,
            user: false,
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

    /// Reads a template from the file at `path`.
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
        template.path = Some(path.to_path_buf());
        Ok(template)
    }

    /// The starter template installed with the app.
    pub fn starter() -> Result<Self, TemplateError> {
        Self::load(assets_dir().join(STARTER_TEMPLATE))
    }

    /// Every template: the built-in ones (the starter first, then the rest by file name), then
    /// the user's by file name. A template that can't be loaded is returned among the problems
    /// and left out of the list.
    pub fn load_all() -> (Vec<Self>, Vec<TemplateError>) {
        Self::load_all_with_user(Self::user_dir().as_deref())
    }

    /// Like [`load_all`](Self::load_all), with the user templates read from `user`.
    pub fn load_all_with_user(user: Option<&Path>) -> (Vec<Self>, Vec<TemplateError>) {
        Self::load_all_from(&assets_dir().join(TEMPLATES_DIR), user)
    }

    /// Like [`load_all`](Self::load_all), from the given folders. A user folder that doesn't
    /// exist yet is not a problem.
    pub fn load_all_from(built_in: &Path, user: Option<&Path>) -> (Vec<Self>, Vec<TemplateError>) {
        let (mut templates, mut problems) = Self::load_dir(built_in);
        if let Some(dir) = user.filter(|dir| dir.is_dir()) {
            let (mut mine, more) = Self::load_dir(dir);
            for template in &mut mine {
                template.user = true;
            }
            templates.append(&mut mine);
            problems.extend(more);
        }
        (templates, problems)
    }

    /// The per-user templates folder, such as `~/.local/share/starscript-map-maker/templates`
    /// on Linux. `None` when the platform reports no home folder.
    pub fn user_dir() -> Option<PathBuf> {
        user_data_dir().map(|dir| dir.join(USER_TEMPLATES_DIR))
    }

    /// Whether the template ships with the app. Built-in templates can't be deleted.
    pub fn is_built_in(&self) -> bool {
        !self.user
    }

    /// The template's file, when it has one.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// A template with `name` that gives new Maps `map`'s background, shore and lake styles,
    /// Label styles and Layers (without their Objects). A Map made from it takes the Canvas
    /// size and scale chosen then.
    pub fn from_map(name: &str, map: &Map, default_layers: DefaultLayers) -> Self {
        let mut view = map.view();
        view.canvas.template = None;
        view.embedded = Default::default();
        view.export_settings = Default::default();
        view.layers = view.layers.into_iter().map(layers_only).collect();
        let keep = |id: Option<Id>| id.filter(|&id| map.contains(id));
        Self {
            path: None,
            user: true,
            id: String::new(),
            name: name.trim().to_owned(),
            view,
            default_layers: DefaultLayers {
                land: keep(default_layers.land),
                assets: keep(default_layers.assets),
                labels: keep(default_layers.labels),
            },
        }
    }

    /// The template as JSON text, in the shape [`parse`](Self::parse) reads.
    pub fn to_json(&self) -> String {
        let out = TemplateOut {
            format_version: FORMAT_VERSION.to_string(),
            name: &self.name,
            default_layers: self.default_layers,
            view: &self.view,
        };
        // Plain data with string keys, so serialising cannot fail.
        serde_json::to_string_pretty(&out).unwrap_or_default()
    }

    /// Writes the template into `dir` (made if missing) as a new file named after it, never
    /// replacing one that is there. Returns the template as loaded from that file.
    pub fn save_to(&self, dir: &Path) -> Result<Self, TemplateError> {
        std::fs::create_dir_all(dir).map_err(TemplateError::Write)?;
        let stem = file_stem(&self.name);
        let mut n = 1;
        let (id, path) = loop {
            let id = if n == 1 {
                stem.clone()
            } else {
                format!("{stem}-{n}")
            };
            let path = dir.join(format!("{id}.json"));
            if !path.exists() {
                break (id, path);
            }
            n += 1;
        };
        std::fs::write(&path, self.to_json()).map_err(TemplateError::Write)?;
        let mut saved = self.clone();
        saved.id = id;
        saved.path = Some(path);
        saved.user = true;
        Ok(saved)
    }

    /// Writes the template into the user templates folder (see [`save_to`](Self::save_to)).
    pub fn save_user(&self) -> Result<Self, TemplateError> {
        let dir = Self::user_dir().ok_or(TemplateError::NoUserFolder)?;
        self.save_to(&dir)
    }

    /// Deletes a user template by handing its file to `trash` (which should move it to the
    /// Trash). Refuses built-in templates, whatever the caller's UI allowed.
    pub fn delete(
        &self,
        trash: impl FnOnce(&Path) -> Result<(), TrashError>,
    ) -> Result<(), TemplateError> {
        match (&self.path, self.user) {
            (Some(path), true) => trash(path).map_err(TemplateError::Trash),
            _ => Err(TemplateError::BuiltIn(self.name.clone())),
        }
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
    #[allow(clippy::expect_used, reason = "the template was checked when loaded")]
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

/// `subtree` with every Object removed, keeping the Layers.
fn layers_only(mut subtree: Subtree) -> Subtree {
    subtree.children = subtree
        .children
        .into_iter()
        .filter(|child| child.node.is_layer())
        .map(layers_only)
        .collect();
    subtree
}

/// A file name for a template called `name`: lower case letters and digits joined by dashes.
fn file_stem(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        "template".to_owned()
    } else {
        words.join("-")
    }
}
