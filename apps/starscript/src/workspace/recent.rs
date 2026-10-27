//! Recently opened Map files, newest first, with thumbnails read from each file's preview.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;

/// How many recent files are remembered.
const MAX_RECENT: usize = 10;
const STORAGE_KEY: &str = "recent_files";
/// Thumbnail height in the menu, in points.
const THUMBNAIL_HEIGHT: f32 = 48.0;

#[derive(Default)]
pub(crate) struct RecentFiles {
    paths: Vec<PathBuf>,
    /// `None` when the file has no preview or can't be read.
    thumbnails: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

impl RecentFiles {
    pub(crate) fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let paths: Vec<PathBuf> = storage
            .and_then(|storage| storage.get_string(STORAGE_KEY))
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self {
            paths: paths.into_iter().take(MAX_RECENT).collect(),
            thumbnails: HashMap::new(),
        }
    }

    pub(crate) fn store(&self, storage: &mut dyn eframe::Storage) {
        // Paths that aren't valid UTF-8 can't be serialised, so they aren't remembered.
        let storable: Vec<&Path> = self
            .paths
            .iter()
            .map(PathBuf::as_path)
            .filter(|path| path.to_str().is_some())
            .collect();
        match serde_json::to_string(&storable) {
            Ok(json) => storage.set_string(STORAGE_KEY, json),
            Err(err) => eprintln!("could not store recent files: {err}"),
        }
    }

    pub(crate) fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Puts `path` first, and forgets its thumbnail so the next look reads the new preview.
    pub(crate) fn add(&mut self, path: &Path) {
        self.paths.retain(|known| known != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX_RECENT);
        self.thumbnails.remove(path);
    }

    pub(crate) fn remove(&mut self, path: &Path) {
        self.paths.retain(|known| known != path);
        self.thumbnails.remove(path);
    }

    /// The entries of the Open Recent menu. Returns the path the user picked.
    pub(crate) fn menu(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        if self.paths.is_empty() {
            ui.weak("No recent files");
            return None;
        }
        let mut picked = None;
        for path in self.paths.clone() {
            let thumbnail = self
                .thumbnails
                .entry(path.clone())
                .or_insert_with(|| load_thumbnail(ui.ctx(), &path));
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into(),
            );
            let response = ui
                .horizontal(|ui| {
                    match thumbnail {
                        Some(texture) => {
                            let size = texture.size_vec2();
                            let width = THUMBNAIL_HEIGHT * size.x / size.y.max(1.0);
                            ui.image((texture.id(), egui::vec2(width, THUMBNAIL_HEIGHT)));
                        }
                        None => {
                            ui.allocate_exact_size(
                                egui::vec2(THUMBNAIL_HEIGHT, THUMBNAIL_HEIGHT),
                                egui::Sense::hover(),
                            );
                        }
                    }
                    ui.button(name)
                })
                .inner
                .on_hover_text(path.display().to_string());
            if response.clicked() {
                picked = Some(path);
            }
        }
        picked
    }
}

fn load_thumbnail(ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
    let png = ssmap_core::file::read_preview(path).ok()??;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(
        format!("recent:{}", path.display()),
        color,
        egui::TextureOptions::LINEAR,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[cfg(unix)]
    #[test]
    fn store_skips_non_utf8_paths_without_panicking() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let mut recent = RecentFiles::default();
        recent.add(Path::new("/maps/ok.ssmap"));
        recent.add(Path::new(OsStr::from_bytes(b"/maps/bad\xff.ssmap")));
        let mut storage = MemoryStorage::default();
        recent.store(&mut storage);

        let loaded = RecentFiles::load(Some(&storage));
        assert_eq!(loaded.paths(), [PathBuf::from("/maps/ok.ssmap")]);
    }

    #[test]
    fn newest_first_without_duplicates_and_capped() {
        let mut recent = RecentFiles::default();
        for i in 0..12 {
            recent.add(Path::new(&format!("/maps/{i}.ssmap")));
        }
        recent.add(Path::new("/maps/5.ssmap"));
        assert_eq!(recent.paths().len(), MAX_RECENT);
        assert_eq!(recent.paths()[0], Path::new("/maps/5.ssmap"));
        assert_eq!(recent.paths()[1], Path::new("/maps/11.ssmap"));
        assert_eq!(
            recent
                .paths()
                .iter()
                .filter(|p| p.ends_with("5.ssmap"))
                .count(),
            1
        );
    }
}
