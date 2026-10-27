//! The Map file behind the editor, if it has been saved or opened, and whether the Map has
//! changed since.

use std::path::{Path, PathBuf};

use ssmap_core::file::{FileError, MapFile};
use ssmap_core::{ExportSettings, Map};
use uuid::Uuid;

/// File extension of Map files.
pub(crate) const EXTENSION: &str = "ssmap";

#[derive(Default)]
pub struct Document {
    file: Option<MapFile>,
    /// [`Map::revision`] when last saved or opened. A new Map starts unchanged at 0.
    saved_revision: u64,
    /// The export settings when last saved or opened. They are written outside Commands (ADR
    /// 0011) and so leave the revision alone, so they are compared separately.
    saved_export: ExportSettings,
}

impl Document {
    /// Opens a Map file, returning it with its Map.
    pub fn open(path: &Path) -> Result<(Self, Map), FileError> {
        let (file, map) = MapFile::open(path)?;
        let document = Self {
            saved_revision: file.saved_revision(),
            saved_export: map.export_settings.clone(),
            file: Some(file),
        };
        Ok((document, map))
    }

    /// A Map never saved, recovered after a crash: it has unsaved changes until saved.
    pub fn recovered() -> Self {
        Self {
            file: None,
            saved_revision: u64::MAX,
            saved_export: ExportSettings::default(),
        }
    }

    /// The open file's save generation, which its Recovery journal records.
    pub fn save_generation(&self) -> Option<Uuid> {
        self.file.as_ref().map(MapFile::save_generation)
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
        map.revision() != self.saved_revision || map.export_settings != self.saved_export
    }

    /// Saves to the open file. Fails with `None` before the first save, which needs a path.
    pub fn save(&mut self, map: &Map, preview: Option<&[u8]>) -> Option<Result<(), FileError>> {
        let file = self.file.as_mut()?;
        Some(file.save(map, preview).map(|_| {
            self.saved_revision = map.revision();
            self.saved_export = map.export_settings.clone();
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
        self.saved_export = map.export_settings.clone();
        Ok(())
    }
}

/// Adds the Map file extension unless the path already ends in it.
pub(crate) fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|ext| ext == EXTENSION) {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".");
        name.push(EXTENSION);
        name.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Canvas, DistanceUnit, Scale};

    fn map() -> Map {
        Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)))
    }

    #[test]
    fn changing_the_export_settings_marks_the_map_unsaved_until_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.ssmap");
        let mut map = map();
        let mut document = Document::default();
        document.save_as(&path, &map, None).unwrap();
        assert!(!document.has_unsaved_changes(&map));

        map.export_settings.transparent_background = true;
        assert!(document.has_unsaved_changes(&map));

        document.save(&map, None).unwrap().unwrap();
        assert!(!document.has_unsaved_changes(&map));
        let (reopened, loaded) = Document::open(&path).unwrap();
        assert_eq!(loaded.export_settings, map.export_settings);
        assert!(!reopened.has_unsaved_changes(&loaded));
    }

    #[test]
    fn setting_the_export_settings_back_clears_the_unsaved_mark() {
        let mut map = map();
        let document = Document::default();
        map.export_settings.transparent_background = true;
        assert!(document.has_unsaved_changes(&map));
        map.export_settings = ExportSettings::default();
        assert!(!document.has_unsaved_changes(&map));
    }
}
