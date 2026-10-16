//! The credits screen: every loaded Asset pack and bundled font, with who made it and under
//! what licence.

use eframe::egui;

use crate::editor::Editor;
use crate::theme;
use ssmap_core::asset::{PackCredit, PackLibrary};
use ssmap_fonts::FAMILIES;

/// One bundled font family's entry.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FontCredit {
    pub family: String,
    /// The copyright line of its OFL notice.
    pub copyright: String,
    /// The full OFL text.
    pub licence: String,
}

/// What the credits screen shows, built from the loaded packs and bundled fonts.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CreditsPage {
    pub packs: Vec<PackCredit>,
    pub fonts: Vec<FontCredit>,
}

impl CreditsPage {
    pub fn new(library: &PackLibrary) -> Self {
        let fonts = FAMILIES
            .iter()
            .map(|family| FontCredit {
                family: family.name.into(),
                copyright: family
                    .licence
                    .lines()
                    .map(str::trim)
                    .find(|line| line.starts_with("Copyright"))
                    .unwrap_or_default()
                    .into(),
                licence: family.licence.into(),
            })
            .collect();
        Self {
            packs: library.credits(),
            fonts,
        }
    }
}

/// The credits window, opened from the Help menu.
pub struct Credits {
    pub open: bool,
    page: CreditsPage,
}

impl Credits {
    pub fn new(library: &PackLibrary) -> Self {
        Self {
            open: false,
            page: CreditsPage::new(library),
        }
    }

    pub fn page(&self) -> &CreditsPage {
        &self.page
    }

    pub fn show(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let page = &self.page;
        egui::Window::new("Credits")
            .open(&mut self.open)
            .default_width(480.0)
            .vscroll(true)
            .show(ctx, |ui| {
                let palette = theme::palette(ui.ctx());
                ui.label(
                    egui::RichText::new("Credits")
                        .size(theme::TITLE)
                        .color(palette.text_primary)
                        .strong(),
                );
                ui.add_space(6.0);
                map_credits(ui, editor);
                section_heading(ui, "Asset packs");
                if page.packs.is_empty() {
                    ui.label("No Asset packs are loaded.");
                }
                for pack in &page.packs {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if pack.attribution_required {
                            attribution_dot(ui);
                        }
                        ui.strong(&pack.name);
                    });
                    ui.label(&pack.credits);
                    ui.label(format!("Licence: {}", pack.licence));
                    if let Some(url) = &pack.licence_url {
                        ui.hyperlink(url);
                    }
                    if !pack.authors.is_empty() {
                        ui.label(format!("By {}", pack.authors.join(", ")));
                    }
                    for source in &pack.sources {
                        ui.hyperlink(source);
                    }
                    if pack.attribution_required {
                        ui.label(
                            egui::RichText::new(
                                "Attribution is required when you export a Map using it.",
                            )
                            .color(palette.gold_text),
                        );
                    }
                }
                ui.add_space(8.0);
                section_heading(ui, "Fonts");
                for font in &page.fonts {
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new(&font.family)
                        .id_salt(("font_credit", &font.family))
                        .show(ui, |ui| {
                            ui.label(&font.copyright);
                            ui.label("Licensed under the SIL Open Font License, Version 1.1.");
                            ui.label(egui::RichText::new(&font.licence).small());
                        });
                }
            });
    }
}

fn section_heading(ui: &mut egui::Ui, text: &str) {
    let palette = theme::palette(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .size(theme::HEADING)
            .color(palette.text_secondary)
            .strong(),
    );
    ui.separator();
}

/// The gold dot that marks a pack whose Assets ask to be credited, as in the Asset picker.
fn attribution_dot(ui: &mut egui::Ui) {
    let palette = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.0, palette.gold);
    response.on_hover_text("Attribution required");
}

/// The credit lines this Map owes, with actions to copy them or add them to the Map.
fn map_credits(ui: &mut egui::Ui, editor: &mut Editor) {
    let lines = editor.credit_lines();
    if lines.is_empty() {
        return;
    }
    ui.heading("This Map");
    ui.label("Its Assets ask to be credited when you share it:");
    for line in &lines {
        ui.label(line);
    }
    ui.horizontal(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(lines.join("\n"));
            editor.info("Copied the credits");
        }
        if ui.button("Add as Label").clicked() {
            editor.add_credits_label();
        }
    });
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::asset::{PackRoots, PackSource, parse_manifest};
    use std::path::Path;

    #[test]
    fn the_page_lists_every_loaded_pack_and_bundled_font() {
        let manifest = "[pack]\nid = \"icons\"\nname = \"Icons\"\nlicence = \"CC-BY-3.0\"\n\
                        credits = \"Icons made by Lorc.\"\nattribution_required = true\n";
        let pack =
            parse_manifest(Path::new("icons/pack.toml"), manifest, PackSource::BuiltIn).unwrap();
        let mut library = PackLibrary::default();
        library.add(pack, &mut Vec::new());

        let page = CreditsPage::new(&library);
        assert_eq!(page.packs.len(), 1);
        assert_eq!(page.packs[0].name, "Icons");
        assert!(page.packs[0].attribution_required);

        let families: Vec<&str> = page.fonts.iter().map(|f| f.family.as_str()).collect();
        let expected: Vec<&str> = FAMILIES.iter().map(|f| f.name).collect();
        assert_eq!(families, expected);
        for font in &page.fonts {
            assert!(
                font.copyright.starts_with("Copyright"),
                "{} has no copyright line",
                font.family
            );
            assert!(font.licence.contains("SIL Open Font License"));
        }
    }

    #[test]
    fn an_empty_library_still_credits_the_fonts() {
        let (library, _) = PackLibrary::load(&PackRoots {
            built_in: "does-not-exist".into(),
            user: None,
        });
        let page = CreditsPage::new(&library);
        assert!(page.packs.is_empty());
        assert_eq!(page.fonts.len(), FAMILIES.len());
    }
}
