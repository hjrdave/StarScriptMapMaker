//! The Map file behind the editor, if it has been saved or opened, and whether the Map has
//! changed since.

use std::path::{Path, PathBuf};

use ssmap_core::Map;
use ssmap_core::file::{FileError, MapFile};

/// File extension of Map files.
pub const EXTENSION: &str = "ssmap";

#[derive(Default)]
pub struct Document {
    file: Option<MapFile>,
    /// [`Map::revision`] when last saved or opened. A new Map starts unchanged at 0.
    saved_revision: u64,
}

impl Document {
    /// Opens a Map file, returning it with its Map.
    pub fn open(path: &Path) -> Result<(Self, Map), FileError> {
        let (file, map) = MapFile::open(path)?;
        let document = Self {
            saved_revision: file.saved_revision(),
            file: Some(file),
        };
        Ok((document, map))
    }

    pub fn path(&self) -> Option<&Path> {
        self.file.as_ref().map(MapFile::path)
    }

    /// The file name, or "Untitled" before the first save.
    pub fn name(&self) -> String {
        self.path()
            .and_then(Path::file_name)
            .map_or_else(|| "Untitled".into(), |name| name.to_string_lossy().into())
    }

    pub fn has_unsaved_changes(&self, map: &Map) -> bool {
        map.revision() != self.saved_revision
    }

    /// Saves to the open file. Fails with `None` before the first save, which needs a path.
    pub fn save(&mut self, map: &Map, preview: Option<&[u8]>) -> Option<Result<(), FileError>> {
        let file = self.file.as_mut()?;
        Some(file.save(map, preview).map(|_| {
            self.saved_revision = map.revision();
        }))
    }

    /// Saves a copy at `path` that becomes the open file.
    pub fn save_as(
        &mut self,
        path: &Path,
        map: &Map,
        preview: Option<&[u8]>,
    ) -> Result<(), FileError> {
        match &mut self.file {
            Some(file) => {
                file.save_as(path, map, preview)?;
            }
            None => {
                self.file = Some(MapFile::create(path, map, preview)?.0);
            }
        }
        self.saved_revision = map.revision();
        Ok(())
    }
}

/// Adds the Map file extension unless the path already ends in it.
pub fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|ext| ext == EXTENSION) {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".");
        name.push(EXTENSION);
        name.into()
    }
}
