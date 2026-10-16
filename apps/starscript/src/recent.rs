//! Recently opened Map files, newest first, with thumbnails read from each file's preview.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;

/// How many recent files are remembered.
pub const MAX_RECENT: usize = 10;
const STORAGE_KEY: &str = "recent_files";
/// Thumbnail height in the menu, in points.
const THUMBNAIL_HEIGHT: f32 = 48.0;

#[derive(Default)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
    /// `None` when the file has no preview or can't be read.
    thumbnails: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

impl RecentFiles {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let paths: Vec<PathBuf> = storage
            .and_then(|storage| storage.get_string(STORAGE_KEY))
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self {
            paths: paths.into_iter().take(MAX_RECENT).collect(),
            thumbnails: HashMap::new(),
        }
    }

    pub fn store(&self, storage: &mut dyn eframe::Storage) {
        let json = serde_json::to_string(&self.paths).expect("paths serialise");
        storage.set_string(STORAGE_KEY, json);
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Puts `path` first, and forgets its thumbnail so the next look reads the new preview.
    pub fn add(&mut self, path: &Path) {
        self.paths.retain(|known| known != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX_RECENT);
        self.thumbnails.remove(path);
    }

    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|known| known != path);
        self.thumbnails.remove(path);
    }

    /// The entries of the Open Recent menu. Returns the path the user picked.
    pub fn menu(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
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
