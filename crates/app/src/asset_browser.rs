//! The Asset browser: every Asset of every loaded pack, filtered by pack, category and text
//! (name, category or tags). Choosing one arms the Stamp tool with it.

use eframe::egui;
use ssmap_core::AssetRef;
use ssmap_core::asset::{Asset, Pack};

use crate::editor::Editor;
use crate::tools::ToolId;

/// Whether `pack` is the one shown, or every pack is.
fn in_pack(shown: &Option<String>, pack: &Pack) -> bool {
    shown.as_ref().is_none_or(|id| *id == pack.id)
}

#[derive(Default)]
pub struct AssetBrowser {
    search: String,
    /// The pack shown, or every pack.
    pack: Option<String>,
    /// The category shown, or every category.
    category: Option<String>,
}

impl AssetBrowser {
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        ui.heading("Assets");
        ui.separator();
        let library = editor.library.clone();
        if library.packs().is_empty() {
            ui.weak("No Asset packs are installed.");
            return;
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            let pack_name = self
                .pack
                .as_deref()
                .and_then(|id| library.pack(id))
                .map_or("All packs", |pack| pack.name.as_str());
            egui::ComboBox::from_id_salt("asset_pack")
                .selected_text(pack_name)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.pack, None, "All packs");
                    for pack in library.packs() {
                        ui.selectable_value(&mut self.pack, Some(pack.id.clone()), &pack.name);
                    }
                });
            let mut categories: Vec<&str> = library
                .packs()
                .iter()
                .filter(|pack| in_pack(&self.pack, pack))
                .flat_map(|pack| pack.assets.iter().map(|asset| asset.category.as_str()))
                .collect();
            categories.sort_unstable();
            categories.dedup();
            if self
                .category
                .as_deref()
                .is_some_and(|category| !categories.contains(&category))
            {
                self.category = None;
            }
            egui::ComboBox::from_id_salt("asset_category")
                .selected_text(self.category.as_deref().unwrap_or("All categories"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.category, None, "All categories");
                    for category in categories {
                        ui.selectable_value(
                            &mut self.category,
                            Some(category.to_owned()),
                            category,
                        );
                    }
                });
        });
        ui.separator();
        let search = self.search.trim().to_lowercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for pack in library
                .packs()
                .iter()
                .filter(|pack| in_pack(&self.pack, pack))
            {
                let shown: Vec<&Asset> = pack
                    .assets
                    .iter()
                    .filter(|asset| self.matches(asset, &search))
                    .collect();
                if shown.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(&pack.name)
                    .default_open(true)
                    .show(ui, |ui| {
                        for asset in shown {
                            self.row(ui, editor, pack, asset);
                        }
                    });
            }
        });
    }

    fn matches(&self, asset: &Asset, search: &str) -> bool {
        self.category
            .as_ref()
            .is_none_or(|category| *category == asset.category)
            && (search.is_empty()
                || asset.name.to_lowercase().contains(search)
                || asset.category.to_lowercase().contains(search)
                || asset
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(search)))
    }

    fn row(&self, ui: &mut egui::Ui, editor: &mut Editor, pack: &Pack, asset: &Asset) {
        let Ok(reference) = AssetRef::new(&pack.id, &asset.id) else {
            return;
        };
        let selected = editor.selected_asset.as_ref() == Some(&reference);
        let response = ui
            .selectable_label(selected, format!("{}  ·  {}", asset.name, asset.category))
            .on_hover_text(format!(
                "{reference}\nBy {} ({})\n{}",
                asset.author,
                asset.licence,
                asset.tags.join(", ")
            ));
        if response.clicked() {
            editor.selected_asset = Some(reference);
            editor.active_tool = ToolId::Stamp;
        }
    }
}
