//! The Asset picker: the installed Asset packs, filtered by pack, category and a search over
//! names and tags, as a grid of thumbnails. It sits in the Stamp tool
//! sections of Properties; choosing an Asset sets `Editor::selected_asset`.

use std::collections::HashMap;

use eframe::egui;
use ssmap_core::asset::{Asset, AssetRef, Pack, PackLibrary};
use ssmap_render::SpriteStore;

use crate::editor::Editor;
use crate::theme;

/// Thumbnails per row.
const COLUMNS: usize = 4;
/// The grid scrolls past this height, in points.
const GRID_MAX_HEIGHT: f32 = 240.0;

#[derive(Default)]
pub struct AssetBrowser {
    /// The pack shown, or every pack.
    pack: Option<String>,
    /// Set once the pack flagged `default = true` has been selected, which happens once.
    default_applied: bool,
    /// The category shown, or every category.
    category: Option<String>,
    search: String,
    /// `None` records art that could not be loaded.
    thumbnails: HashMap<AssetRef, Option<ThumbArt>>,
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

/// A thumbnail's texture, with whether its art is light enough to need the off-white tile.
/// Worked out once, when the texture is made.
#[derive(Clone)]
pub(crate) struct ThumbArt {
    pub texture: egui::TextureHandle,
    pub light: bool,
}

/// Mean luminance (0 to 1) above which art counts as white or very light.
const LIGHT_ART_LUMINANCE: f64 = 0.8;

/// Whether the art is white or very light: its mean Rec. 709 luminance, weighted by alpha so
/// transparent pixels don't count, is above [`LIGHT_ART_LUMINANCE`]. `rgba` is straight alpha.
fn is_light_art(rgba: &[u8]) -> bool {
    let (mut sum, mut weight) = (0.0_f64, 0.0_f64);
    for px in rgba.chunks_exact(4) {
        let alpha = f64::from(px[3]) / 255.0;
        let luma =
            0.2126 * f64::from(px[0]) + 0.7152 * f64::from(px[1]) + 0.0722 * f64::from(px[2]);
        sum += luma / 255.0 * alpha;
        weight += alpha;
    }
    weight > 0.0 && sum / weight > LIGHT_ART_LUMINANCE
}

/// The id of the pack the browser starts on: the one flagged `default = true`, or none.
fn default_pack(library: &PackLibrary) -> Option<String> {
    library.default_pack().map(|pack| pack.id.clone())
}

/// The distinct categories of `packs`, sorted.
pub(crate) fn categories<'a>(packs: impl Iterator<Item = &'a Pack>) -> Vec<&'a str> {
    let mut categories: Vec<&str> = packs
        .flat_map(|pack| &pack.assets)
        .map(|asset| asset.category.as_str())
        .collect();
    categories.sort_unstable();
    categories.dedup();
    categories
}

/// What a thumbnail says to a screen reader (and on hover): the name and reference, the
/// size of its Variant group and whether its pack must be credited.
pub(crate) fn thumbnail_label(
    reference: &AssetRef,
    asset: &Asset,
    variants: usize,
    attribution: bool,
) -> String {
    let mut label = format!("{} ({reference})", asset.name);
    if variants > 1 {
        label.push_str(&format!(", {variants} variants"));
    }
    if attribution {
        label.push_str(", attribution required");
    }
    label
}

impl AssetBrowser {
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let Some(library) = editor.sprites().map(|sprites| sprites.library().clone()) else {
            ui.weak("No Asset packs are loaded.");
            return;
        };
        if !self.default_applied {
            self.default_applied = true;
            self.pack = default_pack(&library);
        }
        if self
            .pack
            .as_ref()
            .is_some_and(|id| library.pack(id).is_none())
        {
            self.pack = None;
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search names and tags")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            egui::ComboBox::from_id_salt("asset pack")
                .width(width)
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
                .width(width)
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
        });
        ui.add_space(theme::SPACING[1]);

        let mut shown_assets = Vec::new();
        for pack in library
            .packs()
            .iter()
            .filter(|pack| self.pack.as_ref().is_none_or(|id| *id == pack.id))
        {
            for asset in pack
                .assets
                .iter()
                .filter(|asset| matches(asset, self.category.as_deref(), &self.search))
            {
                if let Ok(reference) = AssetRef::new(pack.id.clone(), asset.id.clone()) {
                    shown_assets.push((pack, asset, reference));
                }
            }
        }
        let mut chosen = None;
        if shown_assets.is_empty() {
            self.empty_state(ui);
        } else {
            let gap = ui.spacing().item_spacing.x;
            let side = ((ui.available_width() - gap * (COLUMNS - 1) as f32) / COLUMNS as f32)
                .floor()
                .max(24.0);
            egui::ScrollArea::vertical()
                .id_salt("asset grid")
                .max_height(GRID_MAX_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for row in shown_assets.chunks(COLUMNS) {
                        ui.horizontal(|ui| {
                            for (pack, asset, reference) in row {
                                let selected = editor.selected_asset.as_ref() == Some(reference);
                                let variants = asset
                                    .variant_group
                                    .as_deref()
                                    .map_or(0, |group| pack.variant_group(group).count());
                                if self.thumbnail(
                                    ui,
                                    editor,
                                    reference,
                                    asset,
                                    Thumbnail {
                                        side,
                                        selected,
                                        variants,
                                        attribution: pack.attribution_required,
                                    },
                                ) {
                                    chosen = Some(reference.clone());
                                }
                            }
                        });
                    }
                });
        }
        if let Some(reference) = chosen {
            editor.selected_asset = Some(reference);
        }
        self.picked(ui, editor, &library);
    }

    /// "No Assets match “…”" with the two ways out.
    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let palette = theme::palette(ui.ctx());
        let text = if self.search.trim().is_empty() {
            "No Assets in this selection".to_owned()
        } else {
            format!("No Assets match “{}”", self.search.trim())
        };
        ui.add_space(theme::SPACING[2]);
        ui.label(
            egui::RichText::new(text)
                .size(theme::BODY)
                .color(palette.text_secondary),
        );
        ui.horizontal(|ui| {
            if ui.button("Search all categories").clicked() {
                self.category = None;
                self.pack = None;
            }
            if ui.button("Clear search").clicked() {
                self.search.clear();
            }
        });
    }

    /// Under the grid: the picked Asset's pack · author · licence, and the attribution
    /// notice when its pack needs one.
    fn picked(&self, ui: &mut egui::Ui, editor: &Editor, library: &PackLibrary) {
        let Some(reference) = &editor.selected_asset else {
            return;
        };
        let palette = theme::palette(ui.ctx());
        ui.add_space(theme::SPACING[1]);
        let Some((pack, asset)) = library.asset(reference) else {
            ui.label(
                egui::RichText::new("Pack missing")
                    .size(theme::CAPTION)
                    .color(palette.warn),
            );
            ui.label(
                egui::RichText::new(reference.to_string())
                    .size(theme::CAPTION)
                    .color(palette.text_tertiary),
            );
            return;
        };
        ui.label(
            egui::RichText::new(&asset.name)
                .size(theme::BODY)
                .color(palette.text_primary),
        );
        ui.label(
            egui::RichText::new(format!(
                "{} · {} · {}",
                pack.name, asset.author, asset.licence
            ))
            .size(theme::CAPTION)
            .color(palette.text_tertiary),
        );
        if pack.attribution_required {
            ui.label(
                egui::RichText::new("Attribution required: this pack is credited on export.")
                    .size(theme::CAPTION)
                    .color(palette.gold_text),
            );
        }
    }

    /// One Asset as a thumbnail button showing its art. Returns whether it was clicked.
    fn thumbnail(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        reference: &AssetRef,
        asset: &Asset,
        look: Thumbnail,
    ) -> bool {
        let texture = self
            .thumbnails
            .entry(reference.clone())
            .or_insert_with(|| load_art(ui.ctx(), editor.sprites()?, reference))
            .clone();
        let response = paint_thumbnail(ui, texture, reference, asset, look);
        // Dropped on a Map in any window, it is placed there.
        if response.dragged() {
            response.dnd_set_drag_payload(reference.clone());
        }
        response.clicked()
    }
}

/// Makes the texture for an installed Asset's art, `None` when it cannot be loaded.
pub(crate) fn load_art(
    ctx: &egui::Context,
    sprites: &SpriteStore,
    reference: &AssetRef,
) -> Option<ThumbArt> {
    let sprite = sprites.wait_for_installed(reference)?;
    let art = &sprite.art;
    let pixels = art.unpremultiplied();
    let light = is_light_art(&pixels);
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [art.width as usize, art.height as usize],
        &pixels,
    );
    let texture = ctx.load_texture(reference.to_string(), image, egui::TextureOptions::LINEAR);
    Some(ThumbArt { texture, light })
}

/// Draws one Asset as a square thumbnail tile and returns its response.
pub(crate) fn paint_thumbnail(
    ui: &mut egui::Ui,
    texture: Option<ThumbArt>,
    reference: &AssetRef,
    asset: &Asset,
    look: Thumbnail,
) -> egui::Response {
    let palette = theme::palette(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(look.side), egui::Sense::click_and_drag());
    let label = thumbnail_label(reference, asset, look.variants, look.attribution);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            look.selected,
            &label,
        )
    });
    let painter = ui.painter();
    let fill = match &texture {
        Some(art) if art.light => palette.thumb_bg_light_art,
        _ => palette.thumb_bg,
    };
    let stroke = if look.selected {
        egui::Stroke::new(1.5, palette.gold)
    } else if response.hovered() {
        egui::Stroke::new(1.0, palette.border_strong)
    } else {
        egui::Stroke::new(1.0, palette.border_subtle)
    };
    painter.rect(
        rect,
        theme::RADIUS_FIELD,
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    let inner = rect.shrink(5.0);
    match texture {
        Some(ThumbArt { texture, .. }) => {
            let size = texture.size_vec2();
            let scale = (inner.width() / size.x).min(inner.height() / size.y);
            let art = egui::Rect::from_center_size(inner.center(), size * scale);
            painter.image(
                texture.id(),
                art,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            painter.text(
                inner.center(),
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(theme::HEADING),
                palette.text_tertiary,
            );
        }
    }
    if look.variants > 1 {
        let galley = painter.layout_no_wrap(
            look.variants.to_string(),
            egui::FontId::proportional(theme::CAPTION),
            palette.text_secondary,
        );
        let size = galley.size() + egui::vec2(6.0, 2.0);
        let chip =
            egui::Rect::from_min_size(rect.right_bottom() - size - egui::vec2(2.0, 2.0), size);
        painter.rect_filled(chip, theme::RADIUS_CHIP, palette.bg_panel);
        painter.galley(
            chip.min + egui::vec2(3.0, 1.0),
            galley,
            palette.text_secondary,
        );
    }
    if look.attribution {
        painter.circle_filled(rect.right_top() + egui::vec2(-7.0, 7.0), 3.0, palette.gold);
    }
    response.on_hover_text(label)
}

/// How one thumbnail is drawn.
#[derive(Clone, Copy)]
pub(crate) struct Thumbnail {
    pub side: f32,
    /// Armed: the Asset the tool will place.
    pub selected: bool,
    /// The size of its Variant group, 0 when it has none.
    pub variants: usize,
    /// Its pack must be credited on export: a gold dot.
    pub attribution: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::asset::{ArtFormat, PackSource, parse_manifest};

    fn pack(id: &str, flags: &str) -> Pack {
        let text = format!(
            "[pack]\nid = \"{id}\"\nname = \"{id}\"\nlicence = \"CC0-1.0\"\ncredits = \"-\"\n{flags}"
        );
        parse_manifest(
            std::path::Path::new("p/pack.toml"),
            &text,
            PackSource::BuiltIn,
        )
        .unwrap()
    }

    #[test]
    fn the_flagged_pack_is_selected_and_without_one_nothing_is() {
        let mut library = PackLibrary::default();
        let mut problems = Vec::new();
        library.add(pack("first", ""), &mut problems);
        assert_eq!(default_pack(&library), None);
        library.add(pack("second", "default = true\n"), &mut problems);
        assert_eq!(default_pack(&library).as_deref(), Some("second"));
    }

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
    fn light_art_is_told_from_dark_art() {
        let white = [255, 255, 255, 255].repeat(4);
        let black = [0, 0, 0, 255].repeat(4);
        assert!(is_light_art(&white));
        assert!(!is_light_art(&black));
        assert!(!is_light_art(&[]), "no pixels");
        // Transparent pixels are ignored, whatever colour they carry.
        let mut mixed = [255, 255, 255, 0].repeat(8);
        mixed.extend([0, 0, 0, 255].repeat(2));
        assert!(!is_light_art(&mixed));
        let mut pale = [255, 255, 255, 0].repeat(8);
        pale.extend([240, 240, 240, 255].repeat(2));
        assert!(is_light_art(&pale));
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
