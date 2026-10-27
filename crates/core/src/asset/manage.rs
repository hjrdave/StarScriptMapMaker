//! Managing installed packs: installing a pack folder, deleting a user pack, and counting how
//! many Stamps in a Map use a pack. Built-in packs are protected here, not only in the UI.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::map::Map;
use crate::node::{Node, ObjectKind};

use super::install::PACK_MANIFEST;
use super::library::{PackLibrary, PackRoots};
use super::manifest::{ManifestErrors, read_pack};
use super::{AssetRef, Pack, PackSource};

/// Why a pack could not be installed.
#[derive(Debug, Error)]
pub enum InstallError {
    /// The platform reports no per-user packs folder.
    #[error("this system has no folder for user packs")]
    NoUserFolder,
    /// The chosen path is not a folder (for example a zip file, which is not supported yet).
    #[error("{} is not a pack folder; only folders can be added for now", .0.display())]
    UnsupportedFormat(PathBuf),
    /// The folder has no `pack.toml`.
    #[error("{} has no {PACK_MANIFEST}", .0.display())]
    MissingManifest(PathBuf),
    /// The manifest is invalid, or names files that are missing.
    #[error("invalid pack: {0}")]
    InvalidManifest(ManifestErrors),
    /// A loaded pack already uses this id.
    #[error("a pack named `{id}` is already installed ({})", existing.display())]
    DuplicateId {
        /// The id the pack has.
        id: String,
        /// The folder of the pack that already uses it.
        existing: PathBuf,
    },
    /// Copying failed. Nothing is left half-installed.
    #[error("cannot install the pack: {0}")]
    Io(#[from] std::io::Error),
}

/// Progress of an install, in bytes of pack files copied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InstallProgress {
    /// The bytes copied so far.
    pub copied: u64,
    /// The bytes to copy in all.
    pub total: u64,
}

/// Why a pack could not be deleted.
#[derive(Debug, Error)]
pub enum DeleteError {
    /// No loaded pack has this id.
    #[error("no pack named `{0}` is installed")]
    NotFound(String),
    /// The pack ships with the app (or lives outside the user packs folder).
    #[error("`{0}` is built into the app and cannot be deleted")]
    BuiltIn(String),
    /// Moving the pack to the Trash failed.
    #[error("cannot move the pack to the Trash: {0}")]
    Trash(TrashError),
}

/// What a `trash` closure passed to [`PackLibrary::delete_pack`] returns on failure.
pub type TrashError = Box<dyn std::error::Error + Send + Sync>;

impl Pack {
    /// Whether the pack ships with the app. Decided by where it was found, never by its manifest.
    pub fn is_built_in(&self) -> bool {
        self.source == PackSource::BuiltIn
    }

    /// How many Assets the pack has.
    pub fn asset_count(&self) -> usize {
        self.assets.len()
    }
}

impl PackLibrary {
    /// How many Stamps in `map` place an Asset of pack `pack_id`, hidden and locked ones included.
    pub fn pack_usage(&self, map: &Map, pack_id: &str) -> usize {
        stamp_refs(map)
            .into_iter()
            .filter(|reference| reference.pack == pack_id)
            .count()
    }

    /// The distinct Assets of pack `pack_id` that `map` places, sorted.
    pub fn pack_assets_used(&self, map: &Map, pack_id: &str) -> Vec<AssetRef> {
        stamp_refs(map)
            .into_iter()
            .filter(|reference| reference.pack == pack_id)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Whether `id` names a loaded pack that may be deleted.
    pub fn can_delete(&self, id: &str, roots: &PackRoots) -> bool {
        self.pack(id).is_some_and(|pack| deletable(pack, roots))
    }

    /// Installs the pack folder `source` into the user packs folder and adds it to the library.
    /// `progress` is called as files are copied. On failure nothing is left behind.
    pub fn install_pack(
        &mut self,
        source: &Path,
        roots: &PackRoots,
        mut progress: impl FnMut(InstallProgress),
    ) -> Result<Pack, InstallError> {
        let user_root = roots.user.as_deref().ok_or(InstallError::NoUserFolder)?;
        if !source.is_dir() {
            return Err(InstallError::UnsupportedFormat(source.to_path_buf()));
        }
        if !source.join(PACK_MANIFEST).is_file() {
            return Err(InstallError::MissingManifest(source.to_path_buf()));
        }
        let candidate =
            read_pack(source, PackSource::User).map_err(InstallError::InvalidManifest)?;
        if let Some(existing) = self.pack(&candidate.id) {
            return Err(InstallError::DuplicateId {
                id: candidate.id,
                existing: existing.dir.clone(),
            });
        }
        let target = user_root.join(&candidate.id);
        if target.exists() {
            return Err(InstallError::DuplicateId {
                id: candidate.id,
                existing: target,
            });
        }
        let mut files = Vec::new();
        collect_files(source, Path::new(""), &mut files)?;
        let total = files.iter().map(|(_, size)| size).sum();
        let mut state = InstallProgress { copied: 0, total };
        progress(state);

        // Copy beside the target, then rename, so a failure never leaves a half-installed pack.
        let staging = user_root.join(format!(".installing-{}", candidate.id));
        let _ = std::fs::remove_dir_all(&staging);
        let copied = (|| {
            std::fs::create_dir_all(&staging)?;
            for (relative, size) in &files {
                let to = staging.join(relative);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(source.join(relative), &to)?;
                state.copied += size;
                progress(state);
            }
            std::fs::rename(&staging, &target)
        })();
        if let Err(err) = copied {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(err.into());
        }
        let installed = read_pack(&target, PackSource::User).map_err(|errors| {
            let _ = std::fs::remove_dir_all(&target);
            InstallError::InvalidManifest(errors)
        })?;
        self.add_installed(installed.clone());
        Ok(installed)
    }

    /// Deletes a user pack by handing its folder to `trash` (which should move it to the Trash),
    /// then removes it from the library. Refuses built-in packs and anything outside the user
    /// packs folder, whatever the caller's UI allowed.
    pub fn delete_pack(
        &mut self,
        id: &str,
        roots: &PackRoots,
        trash: impl FnOnce(&Path) -> Result<(), TrashError>,
    ) -> Result<Pack, DeleteError> {
        let pack = self
            .pack(id)
            .ok_or_else(|| DeleteError::NotFound(id.to_owned()))?;
        if !deletable(pack, roots) {
            return Err(DeleteError::BuiltIn(id.to_owned()));
        }
        trash(&pack.dir).map_err(DeleteError::Trash)?;
        self.remove(id)
            .ok_or_else(|| DeleteError::NotFound(id.to_owned()))
    }
}

/// A pack may be deleted only if it was found in the user packs folder and its folder is a
/// direct child of that folder (compared after resolving links, so `..` and symlinks can't
/// point the delete at a built-in pack).
fn deletable(pack: &Pack, roots: &PackRoots) -> bool {
    if pack.is_built_in() {
        return false;
    }
    let Some(user_root) = roots.user.as_deref() else {
        return false;
    };
    let (Ok(dir), Ok(user_root)) = (pack.dir.canonicalize(), user_root.canonicalize()) else {
        return false;
    };
    if dir.parent() != Some(user_root.as_path()) {
        return false;
    }
    match roots.built_in.canonicalize() {
        Ok(built_in) => !dir.starts_with(built_in),
        Err(_) => true,
    }
}

fn stamp_refs(map: &Map) -> Vec<&AssetRef> {
    map.ids()
        .filter_map(|id| match map.node(id)? {
            Node::Object(object) => match &object.kind {
                ObjectKind::Stamp(stamp) => Some(&stamp.asset),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Regular files under `base/relative`, with sizes. Symlinks are skipped.
fn collect_files(
    base: &Path,
    relative: &Path,
    out: &mut Vec<(PathBuf, u64)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(base.join(relative))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = relative.join(entry.file_name());
        if kind.is_dir() {
            collect_files(base, &path, out)?;
        } else if kind.is_file() {
            out.push((path, entry.metadata()?.len()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io::ErrorKind;

    use super::*;

    #[test]
    fn io_error_is_the_source_of_install_error() {
        let err = InstallError::from(std::io::Error::from(ErrorKind::PermissionDenied));
        let source = err
            .source()
            .and_then(|source| source.downcast_ref::<std::io::Error>());
        assert_eq!(
            source.map(std::io::Error::kind),
            Some(ErrorKind::PermissionDenied)
        );
        assert!(err.to_string().starts_with("cannot install the pack: "));
    }
}
