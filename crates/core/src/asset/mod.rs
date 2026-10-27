//! Asset packs: the `pack.toml` format, its validation, and finding packs on disk (ADR 0003).
//!
//! Nothing here touches pixels. Rasterising art, Silhouettes and atlases live in `render`.
//!
//! An Asset pack is a folder holding a hand-editable `pack.toml` and image files:
//!
//! ```toml
//! [pack]
//! id = "kenney-cartography"          # lowercase letters, digits, `-` and `_`
//! name = "Kenney Cartography"
//! licence = "CC0-1.0"                # the default for every Asset
//! licence_url = "https://creativecommons.org/publicdomain/zero/1.0/"  # optional
//! credits = "Art by Kenney (kenney.nl)."
//! attribution_required = false       # optional; true means exports must credit it
//! default = false                    # optional; true makes the Asset browser select this pack
//! author = "Kenney"                  # optional default for every Asset
//! source_url = "https://kenney.nl/"  # optional default for every Asset
//! modified = "2024-05-01"            # optional default for every Asset
//!
//! [[asset]]
//! id = "mountain-1"
//! name = "Mountain"
//! category = "mountains"
//! tags = ["peak", "rocky"]           # optional
//! file = "art/mountain-1.svg"        # SVG or PNG, relative to the pack folder
//! anchor = [0.5, 0.92]               # the ground point, as fractions of the art's width and height
//! width = 64.0                       # default width in Canvas units; height follows the art
//! silhouette = "silhouettes/mountain-1.png"  # optional
//! variant_group = "mountains"        # optional
//! author = "Kenney"                  # these four fall back to the [pack] values
//! source_url = "https://kenney.nl/assets/cartography-pack"
//! licence = "CC0-1.0"
//! modified = "2024-05-01"
//! ```
//!
//! Assets are referenced as `pack-id/asset-id` ([`AssetRef`]).
#![allow(
    missing_docs,
    reason = "still to document; remove this once the file is done (#351)"
)]

mod credits;
mod install;
mod library;
mod manage;
mod manifest;

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use credits::{AttributedAsset, PackCredit, credit_lines};
pub use install::{PACK_MANIFEST, assets_dir};
pub use library::{LoadProblem, PackLibrary, PackRoots};
pub use manage::{DeleteError, InstallError, InstallProgress, TrashError};
pub use manifest::{ManifestError, ManifestErrors, parse_manifest, read_pack};

/// One Asset pack, as read from its manifest.
#[derive(Clone, PartialEq, Debug)]
pub struct Pack {
    pub id: String,
    pub name: String,
    /// The folder holding `pack.toml`. Asset files are relative to it.
    pub dir: PathBuf,
    pub licence: String,
    /// Where the licence text can be read.
    pub licence_url: Option<String>,
    /// Free-form credits text shown wherever the pack is credited.
    pub credits: String,
    /// Whether a Map using this pack's Assets must credit it on export.
    pub attribution_required: bool,
    /// Whether the Asset browser selects this pack. At most one pack should be flagged.
    pub default: bool,
    pub source: PackSource,
    /// In manifest order.
    pub assets: Vec<Asset>,
}

impl Pack {
    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.id == id)
    }

    /// The Assets in `group`, in manifest order.
    pub fn variant_group<'a>(&'a self, group: &'a str) -> impl Iterator<Item = &'a Asset> + 'a {
        self.assets
            .iter()
            .filter(move |asset| asset.variant_group.as_deref() == Some(group))
    }

    /// The absolute path of a file named in the manifest.
    pub fn path(&self, file: &std::path::Path) -> PathBuf {
        self.dir.join(file)
    }
}

/// Where a pack was found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PackSource {
    /// Installed with the app (the repo's `assets/packs` in development).
    BuiltIn,
    /// The per-user packs folder. Unsupported in v1, but loaded.
    User,
}

/// One piece of map art.
#[derive(Clone, PartialEq, Debug)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub category: String,
    pub tags: Vec<String>,
    /// Relative to the pack folder.
    pub file: PathBuf,
    pub format: ArtFormat,
    /// The ground point, as fractions of the art's width and height from its top-left.
    pub anchor: [f64; 2],
    /// Default width in Canvas units. The height follows the art's aspect ratio.
    pub width: f64,
    /// Relative to the pack folder.
    pub silhouette: Option<PathBuf>,
    pub variant_group: Option<String>,
    pub author: String,
    pub source_url: Option<String>,
    pub licence: String,
    /// `YYYY-MM-DD`.
    pub modified: String,
}

/// How an Asset's art is stored, from its file extension.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtFormat {
    /// Rasterised at whatever scale is needed.
    Svg,
    /// Used at its native resolution.
    Png,
}

impl ArtFormat {
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "svg" => Some(ArtFormat::Svg),
            "png" => Some(ArtFormat::Png),
            _ => None,
        }
    }
}

/// A reference to one Asset, written `pack-id/asset-id`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct AssetRef {
    pub pack: String,
    pub asset: String,
}

impl AssetRef {
    /// Fails unless both ids are valid ([`is_valid_id`]).
    pub fn new(pack: impl Into<String>, asset: impl Into<String>) -> Result<Self, AssetRefError> {
        let (pack, asset) = (pack.into(), asset.into());
        if !is_valid_id(&pack) || !is_valid_id(&asset) {
            return Err(AssetRefError(format!("{pack}/{asset}")));
        }
        Ok(Self { pack, asset })
    }
}

impl fmt::Display for AssetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.pack, self.asset)
    }
}

/// A string that is not a valid `pack-id/asset-id`.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
#[error("`{0}` is not a valid Asset reference; expected `pack-id/asset-id`")]
pub struct AssetRefError(String);

impl FromStr for AssetRef {
    type Err = AssetRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (pack, asset) = s.split_once('/').ok_or_else(|| AssetRefError(s.into()))?;
        AssetRef::new(pack, asset).map_err(|_| AssetRefError(s.into()))
    }
}

impl Serialize for AssetRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AssetRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl schemars::JsonSchema for AssetRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "AssetRef".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[a-z0-9][a-z0-9_-]*/[a-z0-9][a-z0-9_-]*$",
        })
    }
}

/// Pack and Asset ids: a lowercase letter or digit, then lowercase letters, digits, `-` and `_`.
pub(crate) fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_refs_round_trip_through_text() {
        let reference: AssetRef = "kenney/mountain-1".parse().unwrap();
        assert_eq!(reference, AssetRef::new("kenney", "mountain-1").unwrap());
        assert_eq!(reference.to_string(), "kenney/mountain-1");
        let json = serde_json::to_string(&reference).unwrap();
        assert_eq!(json, "\"kenney/mountain-1\"");
        assert_eq!(serde_json::from_str::<AssetRef>(&json).unwrap(), reference);
    }

    #[test]
    fn malformed_asset_refs_are_rejected() {
        for bad in [
            "",
            "kenney",
            "kenney/",
            "/mountain",
            "a/b/c",
            "Kenney/m",
            "k/m 1",
        ] {
            assert!(bad.parse::<AssetRef>().is_err(), "{bad:?} parsed");
        }
    }
}
