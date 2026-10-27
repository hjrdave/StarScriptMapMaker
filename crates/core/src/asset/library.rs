//! Finding and loading every pack.

use std::fmt;
use std::path::{Path, PathBuf};

use super::install::{PACK_MANIFEST, built_in_packs_dir, user_packs_dir};
use super::manifest::{ManifestErrors, read_pack};
use super::{Asset, AssetRef, Pack, PackSource};

/// The folders searched for packs. Each holds one folder per pack.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PackRoots {
    /// The folder of packs that ship with the app.
    pub built_in: PathBuf,
    /// The per-user packs folder, if there is one.
    pub user: Option<PathBuf>,
}

impl PackRoots {
    /// The installed app's built-in packs and the per-user packs folder.
    pub fn installed() -> Self {
        Self {
            built_in: built_in_packs_dir(),
            user: user_packs_dir(),
        }
    }
}

/// A pack that could not be loaded. The other packs still load.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LoadProblem {
    /// A pack's manifest has errors.
    Manifest(ManifestErrors),
    /// A pack folder could not be listed.
    Unreadable {
        /// The folder that could not be listed.
        dir: PathBuf,
        /// Why it could not be listed.
        message: String,
    },
    /// Two packs share an id. Built-in packs win over user packs, then the earlier folder name.
    DuplicateId {
        /// The id both packs use.
        id: String,
        /// The folder of the pack that loaded.
        kept: PathBuf,
        /// The folder of the pack that did not.
        skipped: PathBuf,
    },
}

impl fmt::Display for LoadProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadProblem::Manifest(errors) => write!(f, "{errors}"),
            LoadProblem::Unreadable { dir, message } => {
                write!(f, "{}: cannot be read: {message}", dir.display())
            }
            LoadProblem::DuplicateId { id, kept, skipped } => write!(
                f,
                "{}: pack id `{id}` is already used by {}; this pack is skipped",
                skipped.join(PACK_MANIFEST).display(),
                kept.join(PACK_MANIFEST).display()
            ),
        }
    }
}

impl std::error::Error for LoadProblem {}

/// Every loaded pack.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PackLibrary {
    packs: Vec<Pack>,
}

impl PackLibrary {
    /// Loads every pack under `roots`: built-in packs first, each root in folder-name order.
    /// A missing root is simply empty, and folders without a `pack.toml` are ignored.
    pub fn load(roots: &PackRoots) -> (Self, Vec<LoadProblem>) {
        let mut library = Self::default();
        let mut problems = Vec::new();
        let sources = std::iter::once((roots.built_in.as_path(), PackSource::BuiltIn))
            .chain(roots.user.as_deref().map(|dir| (dir, PackSource::User)));
        for (root, source) in sources {
            for dir in pack_dirs(root, &mut problems) {
                match read_pack(&dir, source) {
                    Ok(pack) => library.add(pack, &mut problems),
                    Err(errors) => problems.push(LoadProblem::Manifest(errors)),
                }
            }
        }
        (library, problems)
    }

    /// Adds a pack, unless one with its id is already loaded.
    pub fn add(&mut self, pack: Pack, problems: &mut Vec<LoadProblem>) {
        if let Some(existing) = self.pack(&pack.id) {
            problems.push(LoadProblem::DuplicateId {
                id: pack.id.clone(),
                kept: existing.dir.clone(),
                skipped: pack.dir,
            });
            return;
        }
        self.packs.push(pack);
    }

    /// Adds a freshly installed pack. The caller has already checked its id is free.
    pub(super) fn add_installed(&mut self, pack: Pack) {
        self.packs.push(pack);
    }

    /// Removes and returns the pack with this id.
    pub(super) fn remove(&mut self, id: &str) -> Option<Pack> {
        let index = self.packs.iter().position(|pack| pack.id == id)?;
        Some(self.packs.remove(index))
    }

    /// Every loaded pack, in load order.
    pub fn packs(&self) -> &[Pack] {
        &self.packs
    }

    /// The pack with id `id`, if one is loaded.
    pub fn pack(&self, id: &str) -> Option<&Pack> {
        self.packs.iter().find(|pack| pack.id == id)
    }

    /// The pack the Asset browser selects: the first one flagged `default = true`, if any.
    pub fn default_pack(&self) -> Option<&Pack> {
        self.packs.iter().find(|pack| pack.default)
    }

    /// The pack and Asset that `reference` names, if both are loaded.
    pub fn asset(&self, reference: &AssetRef) -> Option<(&Pack, &Asset)> {
        let pack = self.pack(&reference.pack)?;
        Some((pack, pack.asset(&reference.asset)?))
    }
}

/// The folders under `root` that hold a manifest, sorted by name.
fn pack_dirs(root: &Path, problems: &mut Vec<LoadProblem>) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            problems.push(LoadProblem::Unreadable {
                dir: root.to_path_buf(),
                message: err.to_string(),
            });
            return Vec::new();
        }
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(PACK_MANIFEST).is_file())
        .collect();
    dirs.sort();
    dirs
}
