//! The Asset packs page on Home: every installed pack as a card, and an opened pack's Assets
//! in a thumbnail grid with search and category filtering, like the Asset picker.
//!
//! What a card says of a pack is plain data and functions here, tested without a window.
//! [`PackPage::show`] draws it.

use std::collections::HashMap;

use eframe::egui::{self, Rect, vec2};
use ssmap_core::asset::{Asset, AssetRef, Pack};
use ssmap_render::SpriteStore;

use crate::asset_browser::{self, ThumbArt, Thumbnail};
use crate::theme;

/// Cards per row on the list.
const CARD_COLUMNS: usize = 3;
/// Thumbnails per row inside an opened pack.
const GRID_COLUMNS: usize = 6;
/// How many of a pack's Assets its card previews.
const PREVIEW_COUNT: usize = 4;
const PREVIEW_HEIGHT: f32 = 84.0;
const CARD_FOOTER: f32 = 92.0;

/// What the user did on the page this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Open the Credits & licences view.
    Credits,
}

/// "Built in" or "Added by you".
pub fn origin_label(pack: &Pack) -> &'static str {
    if pack.is_built_in() {
        "Built in"
    } else {
        "Added by you"
    }
}

/// "1 Asset" or "12 Assets".
pub fn asset_count_label(pack: &Pack) -> String {
    match pack.asset_count() {
        1 => "1 Asset".into(),
        n => format!("{n} Assets"),
    }
}

/// The authors of a pack's Assets, sorted and without repeats: "A", "A and B" or "A, B and 3
/// more". "Unknown author" when none is given.
pub fn author_label(pack: &Pack) -> String {
    let mut authors: Vec<&str> = pack
        .assets
        .iter()
        .map(|asset| asset.author.trim())
        .filter(|author| !author.is_empty())
        .collect();
    authors.sort_unstable_by_key(|author| author.to_lowercase());
    authors.dedup();
    match authors.as_slice() {
        [] => "Unknown author".into(),
        [one] => (*one).into(),
        [a, b] => format!("{a} and {b}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

/// The Assets of `pack` that pass the filters, in manifest order.
pub fn filtered<'a>(pack: &'a Pack, category: Option<&str>, search: &str) -> Vec<&'a Asset> {
    pack.assets
        .iter()
        .filter(|asset| asset_browser::matches(asset, category, search))
        .collect()
}

/// The page's state: the opened pack, its filters and the thumbnails made so far.
#[derive(Default)]
pub struct PackPage {
    /// The id of the opened pack, or the list of packs.
    open: Option<String>,
    category: Option<String>,
    search: String,
    /// `None` records art that could not be loaded.
    thumbnails: HashMap<AssetRef, Option<ThumbArt>>,
}

impl PackPage {
    /// Draws the page and returns what the user did.
    pub fn show(&mut self, ui: &mut egui::Ui, sprites: Option<&SpriteStore>) -> Vec<Action> {
        let mut actions = Vec::new();
        let p = theme::palette(ui.ctx());
        let Some(sprites) = sprites else {
            ui.label(egui::RichText::new("No Asset packs are loaded.").color(p.text_secondary));
            return actions;
        };
        let library = sprites.library().clone();
        // A pack that is gone closes.
        if self
            .open
            .as_ref()
            .is_some_and(|id| library.pack(id).is_none())
        {
            self.open = None;
        }
        egui::Frame::NONE
            .inner_margin(egui::Margin {
                left: 32,
                right: 32,
                top: 24,
                bottom: 0,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.open.is_some() && ui.button("‹ All packs").clicked() {
                        self.open = None;
                        self.category = None;
                        self.search.clear();
                    }
                    let title = self
                        .open
                        .as_ref()
                        .and_then(|id| library.pack(id))
                        .map_or("Asset packs", |pack| pack.name.as_str());
                    ui.label(
                        egui::RichText::new(title)
                            .size(20.0)
                            .color(p.text_primary)
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Credits & licences").clicked() {
                            actions.push(Action::Credits);
                        }
                    });
                });
            });
        match self.open.clone().and_then(|id| library.pack(&id).cloned()) {
            Some(pack) => self.pack_detail(ui, sprites, &pack, &mut actions),
            None => self.pack_list(ui, sprites, library.packs()),
        }
        actions
    }

    fn pack_list(&mut self, ui: &mut egui::Ui, sprites: &SpriteStore, packs: &[Pack]) {
        let p = theme::palette(ui.ctx());
        egui::ScrollArea::vertical()
            .id_salt("pack list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(24, 16))
                    .show(ui, |ui| {
                        if packs.is_empty() {
                            ui.label(
                                egui::RichText::new("No Asset packs are installed.")
                                    .color(p.text_secondary),
                            );
                            return;
                        }
                        let gap = 8.0;
                        let width = ((ui.available_width() - gap * (CARD_COLUMNS as f32 - 1.0))
                            / CARD_COLUMNS as f32)
                            .floor()
                            .max(200.0);
                        ui.spacing_mut().item_spacing = vec2(gap, gap);
                        for row in packs.chunks(CARD_COLUMNS) {
                            ui.horizontal_top(|ui| {
                                for pack in row {
                                    if self.pack_card(ui, sprites, pack, width) {
                                        self.open = Some(pack.id.clone());
                                    }
                                }
                            });
                        }
                    });
            });
    }

    /// One pack as a card. Returns whether it was clicked.
    fn pack_card(
        &mut self,
        ui: &mut egui::Ui,
        sprites: &SpriteStore,
        pack: &Pack,
        width: f32,
    ) -> bool {
        let p = theme::palette(ui.ctx());
        let size = vec2(width, PREVIEW_HEIGHT + 1.0 + CARD_FOOTER);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let label = format!(
            "Open {}, {}, {}",
            pack.name,
            origin_label(pack),
            asset_count_label(pack)
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        let preview = Rect::from_min_size(rect.min, vec2(width, PREVIEW_HEIGHT));
        let radius = theme::RADIUS_FIELD;
        let painter = ui.painter().clone();
        painter.rect_filled(rect, radius, p.bg_raised);
        painter.rect_filled(
            preview,
            egui::CornerRadius {
                nw: radius,
                ne: radius,
                sw: 0,
                se: 0,
            },
            p.bg_canvas,
        );
        // A few of the pack's Assets, side by side.
        let tile = PREVIEW_HEIGHT - 16.0;
        let shown: Vec<&Asset> = pack.assets.iter().take(PREVIEW_COUNT).collect();
        let strip = tile * shown.len() as f32 + 6.0 * shown.len().saturating_sub(1) as f32;
        let mut x = preview.center().x - strip / 2.0;
        for asset in shown {
            let tile_rect =
                Rect::from_min_size(egui::pos2(x, preview.min.y + 8.0), egui::Vec2::splat(tile));
            x += tile + 6.0;
            let Ok(reference) = AssetRef::new(pack.id.clone(), asset.id.clone()) else {
                continue;
            };
            let art = self
                .thumbnails
                .entry(reference.clone())
                .or_insert_with(|| asset_browser::load_art(ui.ctx(), sprites, &reference))
                .clone();
            let fill = match &art {
                Some(art) if art.light => p.thumb_bg_light_art,
                _ => p.thumb_bg,
            };
            painter.rect_filled(tile_rect, radius, fill);
            if let Some(art) = art {
                let size = art.texture.size_vec2();
                let scale = ((tile - 8.0) / size.x.max(1.0)).min((tile - 8.0) / size.y.max(1.0));
                painter.image(
                    art.texture.id(),
                    Rect::from_center_size(tile_rect.center(), size * scale),
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }
        painter.hline(
            rect.x_range(),
            preview.max.y + 0.5,
            egui::Stroke::new(1.0, p.border_subtle),
        );
        painter.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(
                1.0,
                if response.hovered() {
                    p.border_strong
                } else {
                    p.border_subtle
                },
            ),
            egui::StrokeKind::Inside,
        );
        let left = rect.min.x + 10.0;
        let text_width = width - 20.0;
        let top = preview.max.y + 10.0;
        text_line(
            ui,
            egui::pos2(left, top),
            text_width,
            &pack.name,
            theme::BODY,
            p.text_primary,
        );
        text_line(
            ui,
            egui::pos2(left, top + 18.0),
            text_width,
            &format!("{} · {}", author_label(pack), pack.licence),
            theme::CAPTION,
            p.text_tertiary,
        );
        text_line(
            ui,
            egui::pos2(left, top + 34.0),
            text_width,
            &asset_count_label(pack),
            theme::CAPTION,
            p.text_secondary,
        );
        // Badges: where the pack came from, and whether it must be credited.
        let mut badge_x = left;
        let badge_y = top + 54.0;
        let mut badges = vec![(origin_label(pack), p.text_secondary)];
        if pack.attribution_required {
            badges.push(("Attribution required", p.gold_text));
        }
        for (text, color) in badges {
            let galley = painter.layout_no_wrap(
                text.to_owned(),
                egui::FontId::new(theme::CAPTION, theme::ui_family()),
                color,
            );
            let chip = Rect::from_min_size(
                egui::pos2(badge_x, badge_y),
                galley.size() + vec2(10.0, 4.0),
            );
            painter.rect_filled(chip, theme::RADIUS_CHIP, p.bg_panel);
            painter.galley(chip.min + vec2(5.0, 2.0), galley, color);
            badge_x = chip.max.x + 6.0;
        }
        response.clicked()
    }

    fn pack_detail(
        &mut self,
        ui: &mut egui::Ui,
        sprites: &SpriteStore,
        pack: &Pack,
        actions: &mut Vec<Action>,
    ) {
        let p = theme::palette(ui.ctx());
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(32, 8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} · {} · {} · {}",
                        origin_label(pack),
                        author_label(pack),
                        pack.licence,
                        asset_count_label(pack)
                    ))
                    .size(theme::BODY)
                    .color(p.text_secondary),
                );
                if pack.attribution_required {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(
                                "Attribution required: this pack is credited on export.",
                            )
                            .size(theme::CAPTION)
                            .color(p.gold_text),
                        );
                        if ui.link("Credits & licences").clicked() {
                            actions.push(Action::Credits);
                        }
                    });
                }
                ui.add_space(theme::SPACING[3]);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Search names and tags")
                            .desired_width(240.0),
                    );
                    let all = asset_browser::categories(std::iter::once(pack));
                    if self
                        .category
                        .as_deref()
                        .is_some_and(|category| !all.contains(&category))
                    {
                        self.category = None;
                    }
                    egui::ComboBox::from_id_salt("pack category")
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
            });
        let assets = filtered(pack, self.category.as_deref(), &self.search);
        if assets.is_empty() {
            egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(32, 8))
                .show(ui, |ui| {
                    let text = if self.search.trim().is_empty() {
                        "No Assets in this selection".to_owned()
                    } else {
                        format!("No Assets match “{}”", self.search.trim())
                    };
                    ui.label(egui::RichText::new(text).color(p.text_secondary));
                    if ui.button("Clear search").clicked() {
                        self.search.clear();
                        self.category = None;
                    }
                });
            return;
        }
        let gap = ui.spacing().item_spacing.x;
        let side = ((ui.available_width() - 64.0 - gap * (GRID_COLUMNS - 1) as f32)
            / GRID_COLUMNS as f32)
            .floor()
            .clamp(40.0, 120.0);
        let row_height = side + ui.spacing().item_spacing.y;
        let rows = assets.len().div_ceil(GRID_COLUMNS);
        egui::ScrollArea::vertical()
            .id_salt("pack grid")
            .auto_shrink([false, false])
            .show_rows(ui, row_height, rows, |ui, range| {
                for row in assets
                    .chunks(GRID_COLUMNS)
                    .skip(range.start)
                    .take(range.len())
                {
                    ui.horizontal(|ui| {
                        ui.add_space(32.0);
                        for asset in row {
                            let Ok(reference) = AssetRef::new(pack.id.clone(), asset.id.clone())
                            else {
                                continue;
                            };
                            let art = self
                                .thumbnails
                                .entry(reference.clone())
                                .or_insert_with(|| {
                                    asset_browser::load_art(ui.ctx(), sprites, &reference)
                                })
                                .clone();
                            let variants = asset
                                .variant_group
                                .as_deref()
                                .map_or(0, |group| pack.variant_group(group).count());
                            asset_browser::paint_thumbnail(
                                ui,
                                art,
                                &reference,
                                asset,
                                Thumbnail {
                                    side,
                                    selected: false,
                                    variants,
                                    attribution: pack.attribution_required,
                                },
                            );
                        }
                    });
                }
            });
    }
}

/// One line of text, cut with an ellipsis at `width`.
fn text_line(
    ui: &egui::Ui,
    at: egui::Pos2,
    width: f32,
    text: &str,
    size: f32,
    color: egui::Color32,
) {
    let mut job = egui::text::LayoutJob::simple_singleline(
        text.to_owned(),
        egui::FontId::new(size, theme::ui_family()),
        color,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: width.max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(at, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::asset::{ArtFormat, PackSource, parse_manifest};
    use std::path::Path;

    fn asset(id: &str, category: &str, author: &str) -> Asset {
        Asset {
            id: id.into(),
            name: id.into(),
            category: category.into(),
            tags: vec![],
            file: "art.svg".into(),
            format: ArtFormat::Svg,
            anchor: [0.5, 1.0],
            width: 64.0,
            silhouette: None,
            variant_group: None,
            author: author.into(),
            source_url: None,
            licence: "CC0-1.0".into(),
            modified: "2026-10-14".into(),
        }
    }

    fn pack(source: PackSource, assets: Vec<Asset>) -> Pack {
        let mut pack = parse_manifest(
            Path::new("p/pack.toml"),
            "[pack]\nid = \"p\"\nname = \"P\"\nlicence = \"CC0-1.0\"\ncredits = \"-\"\n",
            source,
        )
        .unwrap();
        pack.assets = assets;
        pack
    }

    #[test]
    fn a_pack_is_built_in_or_added_by_where_it_was_found() {
        assert_eq!(origin_label(&pack(PackSource::BuiltIn, vec![])), "Built in");
        assert_eq!(
            origin_label(&pack(PackSource::User, vec![])),
            "Added by you"
        );
    }

    #[test]
    fn counts_read_as_singular_or_plural() {
        let one = pack(PackSource::User, vec![asset("a", "x", "A")]);
        assert_eq!(asset_count_label(&one), "1 Asset");
        let two = pack(
            PackSource::User,
            vec![asset("a", "x", "A"), asset("b", "x", "A")],
        );
        assert_eq!(asset_count_label(&two), "2 Assets");
        assert_eq!(
            asset_count_label(&pack(PackSource::User, vec![])),
            "0 Assets"
        );
    }

    #[test]
    fn authors_are_sorted_without_repeats_and_shortened() {
        let by = |names: &[&str]| {
            pack(
                PackSource::User,
                names.iter().map(|n| asset("a", "x", n)).collect(),
            )
        };
        assert_eq!(author_label(&by(&[])), "Unknown author");
        assert_eq!(author_label(&by(&["Zed", "Zed"])), "Zed");
        assert_eq!(author_label(&by(&["Zed", "Amy"])), "Amy and Zed");
        assert_eq!(
            author_label(&by(&["Zed", "Amy", "Bob", "Cy", "Di"])),
            "Amy, Bob and 3 more"
        );
    }

    #[test]
    fn filtering_keeps_manifest_order() {
        let pack = pack(
            PackSource::User,
            vec![
                asset("oak", "trees", "A"),
                asset("hill", "terrain", "A"),
                asset("pine", "trees", "A"),
            ],
        );
        let ids = |category, search| {
            filtered(&pack, category, search)
                .iter()
                .map(|asset| asset.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(None, ""), ["oak", "hill", "pine"]);
        assert_eq!(ids(Some("trees"), ""), ["oak", "pine"]);
        assert_eq!(ids(None, "hill"), ["hill"]);
        assert!(ids(Some("terrain"), "oak").is_empty());
    }
}
