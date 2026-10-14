//! The Asset browser: the installed Asset packs, filtered by pack, category and a search over
//! names and tags. Choosing an Asset arms the Stamp tool with it.

use std::collections::HashMap;

use eframe::egui;
use ssmap_core::asset::{Asset, AssetRef, Pack};

use crate::editor::Editor;
use crate::tools::ToolId;

/// A thumbnail's side, in points.
const THUMBNAIL: f32 = 56.0;

#[derive(Default)]
pub struct AssetBrowser {
    /// The pack shown, or every pack.
    pack: Option<String>,
    /// The category shown, or every category.
    category: Option<String>,
    search: String,
    /// `None` records art that could not be loaded.
    thumbnails: HashMap<AssetRef, Option<egui::TextureHandle>>,
}

/// Whether `asset` passes the filters: `category` exactly, and every word of `search` found
/// in its name, id, category or tags, ignoring case.
pub fn matches(asset: &Asset, category: Option<&str>, search: &str) -> bool {
    if category.is_some_and(|category| asset.category != category) {
        return false;
    }
    let haystack = [&asset.name, &asset.id, &asset.category]
        .into_iter()
        .chain(&asset.tags)
        .map(|text| text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    search
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

/// The distinct categories of `packs`, sorted.
fn categories<'a>(packs: impl Iterator<Item = &'a Pack>) -> Vec<&'a str> {
    let mut categories: Vec<&str> = packs
        .flat_map(|pack| &pack.assets)
        .map(|asset| asset.category.as_str())
        .collect();
    categories.sort_unstable();
    categories.dedup();
    categories
}

impl AssetBrowser {
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let Some(library) = editor.sprites().map(|sprites| sprites.library().clone()) else {
            ui.weak("No Asset packs are loaded.");
            return;
        };
        if self
            .pack
            .as_ref()
            .is_some_and(|id| library.pack(id).is_none())
        {
            self.pack = None;
        }
        ui.horizontal(|ui| {
            ui.strong("Assets");
            egui::ComboBox::from_id_salt("asset pack")
                .selected_text(self.pack.as_deref().unwrap_or("All packs"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.pack, None, "All packs");
                    for pack in library.packs() {
                        ui.selectable_value(&mut self.pack, Some(pack.id.clone()), &pack.name);
                    }
                });
            let shown = || {
                library
                    .packs()
                    .iter()
                    .filter(|pack| self.pack.as_ref().is_none_or(|id| *id == pack.id))
            };
            let all = categories(shown());
            if self
                .category
                .as_deref()
                .is_some_and(|category| !all.contains(&category))
            {
                self.category = None;
            }
            egui::ComboBox::from_id_salt("asset category")
                .selected_text(self.category.as_deref().unwrap_or("All categories"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.category, None, "All categories");
                    for category in all {
                        ui.selectable_value(
                            &mut self.category,
                            Some(category.to_owned()),
                            category,
                        );
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("Search names and tags")
                    .desired_width(180.0),
            );
        });
        ui.separator();
        let mut chosen = None;
        let (shown_pack, shown_category, search) = (
            self.pack.clone(),
            self.category.clone(),
            self.search.clone(),
        );
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for pack in library
                    .packs()
                    .iter()
                    .filter(|pack| shown_pack.as_ref().is_none_or(|id| *id == pack.id))
                {
                    for asset in pack
                        .assets
                        .iter()
                        .filter(|asset| matches(asset, shown_category.as_deref(), &search))
                    {
                        let Ok(reference) = AssetRef::new(pack.id.clone(), asset.id.clone()) else {
                            continue;
                        };
                        let selected = editor.selected_asset.as_ref() == Some(&reference);
                        if self.thumbnail_button(ui, editor, &reference, asset, selected) {
                            chosen = Some(reference);
                        }
                    }
                }
            });
        });
        if let Some(reference) = chosen {
            editor.selected_asset = Some(reference);
            editor.active_tool = ToolId::Stamp;
        }
    }

    /// One Asset as a button showing its art. Returns whether it was clicked.
    fn thumbnail_button(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        reference: &AssetRef,
        asset: &Asset,
        selected: bool,
    ) -> bool {
        let texture = self
            .thumbnails
            .entry(reference.clone())
            .or_insert_with(|| {
                let sprite = editor.sprites()?.get(reference)?;
                let art = &sprite.art;
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [art.width as usize, art.height as usize],
                    &art.unpremultiplied(),
                );
                Some(ui.ctx().load_texture(
                    reference.to_string(),
                    image,
                    egui::TextureOptions::LINEAR,
                ))
            })
            .clone();
        let size = egui::Vec2::splat(THUMBNAIL);
        let response = match texture {
            Some(texture) => ui.add(
                egui::Button::image(
                    egui::Image::new(&texture)
                        .fit_to_exact_size(size)
                        .maintain_aspect_ratio(true),
                )
                .selected(selected),
            ),
            None => ui.add_sized(size, egui::Button::new(&asset.name).selected(selected)),
        };
        response
            .on_hover_text(format!("{} ({reference})", asset.name))
            .clicked()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::asset::ArtFormat;

    fn asset(name: &str, category: &str, tags: &[&str]) -> Asset {
        Asset {
            id: name.to_lowercase().replace(' ', "-"),
            name: name.into(),
            category: category.into(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            file: "art.svg".into(),
            format: ArtFormat::Svg,
            anchor: [0.5, 1.0],
            width: 64.0,
            silhouette: None,
            variant_group: None,
            author: "A".into(),
            source_url: None,
            licence: "CC0-1.0".into(),
            modified: "2026-10-14".into(),
        }
    }

    #[test]
    fn filters_by_category_and_every_search_word() {
        let mountain = asset("Snowy Mountain", "mountains", &["peak", "cold"]);
        assert!(matches(&mountain, None, ""));
        assert!(matches(&mountain, Some("mountains"), "snow"));
        assert!(matches(&mountain, None, "COLD peak"), "any order, any case");
        assert!(!matches(&mountain, Some("forests"), ""));
        assert!(!matches(&mountain, None, "peak desert"));
    }
}
