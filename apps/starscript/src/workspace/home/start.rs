//! A new tab's start page: New Map, Open… and Asset packs buttons over a search field, with the
//! Recents below it. Focusing the search field opens a results panel over the Recents.
//!
//! Which Maps the panel shows and which text of a name is highlighted are plain functions,
//! tested without a window. [`Home::start_page`] draws them.

use std::time::SystemTime;

use eframe::egui::{self, Color32, Rect, text::LayoutJob, vec2};

use super::{Action, Data, Entry, Home, edited_ago, size_label};
use crate::theme;

/// How many Maps the search panel lists.
const RESULTS: usize = 5;
/// How many Maps the Recents list shows.
const RECENTS: usize = 5;
const CONTENT_WIDTH: f32 = 560.0;
const ROW_HEIGHT: f32 = 52.0;
const THUMB: egui::Vec2 = vec2(64.0, 40.0);

/// The Maps the search panel lists: the first [`RESULTS`] of `known` (the app's order, newest
/// first) whose name contains `query`, ignoring case; with no query, simply the first ones.
pub(super) fn results<'a>(known: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let query = query.trim().to_lowercase();
    known
        .iter()
        .filter(|entry| query.is_empty() || entry.name.to_lowercase().contains(&query))
        .take(RESULTS)
        .collect()
}

/// The byte range of `name` that matches `query` (ignoring case), if any.
pub(super) fn match_range(name: &str, query: &str) -> Option<std::ops::Range<usize>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return None;
    }
    // Compare char by char, so a char whose lowercase is longer cannot shift the offsets.
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let wanted: Vec<char> = query.chars().collect();
    (0..chars.len()).find_map(|start| {
        let mut at = start;
        for want in &wanted {
            let (_, have) = chars.get(at)?;
            let mut lower = have.to_lowercase();
            if lower.next() != Some(*want) || lower.next().is_some() {
                return None;
            }
            at += 1;
        }
        let end = chars.get(at).map_or(name.len(), |(i, _)| *i);
        Some(chars.get(start)?.0..end)
    })
}

fn name_job(name: &str, query: &str, size: f32, normal: Color32, marked: Color32) -> LayoutJob {
    let font = egui::FontId::new(size, theme::ui_family());
    let mut job = LayoutJob::default();
    let mut add = |text: &str, color: Color32| {
        if !text.is_empty() {
            job.append(
                text,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    ..egui::TextFormat::default()
                },
            );
        }
    };
    match match_range(name, query) {
        Some(range) => {
            add(&name[..range.start], normal);
            add(&name[range.clone()], marked);
            add(&name[range.end..], normal);
        }
        None => add(name, normal),
    }
    job.wrap = egui::text::TextWrapping {
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
        ..egui::text::TextWrapping::default()
    };
    job
}

impl Home {
    /// Draws a new tab's start page and returns what the user did.
    pub(crate) fn start_page(&mut self, ui: &mut egui::Ui, data: &Data<'_>) -> Vec<Action> {
        let mut out = Vec::new();
        let p = theme::palette(ui.ctx());
        let now = SystemTime::now();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(p.bg_window))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.start_body(ui, data, now, &mut out));
            });
        out
    }

    fn start_body(
        &mut self,
        ui: &mut egui::Ui,
        data: &Data<'_>,
        now: SystemTime,
        out: &mut Vec<Action>,
    ) {
        let p = theme::palette(ui.ctx());
        let width = CONTENT_WIDTH.min(ui.available_width() - 48.0).max(240.0);
        let left = ui.max_rect().center().x - width / 2.0;
        ui.add_space(48.0);
        let top = ui.cursor().min.y;
        let mut column = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    egui::pos2(left, top),
                    egui::pos2(left + width, f32::INFINITY),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        let ui = &mut column;

        ui.horizontal(|ui| {
            let new_map =
                egui::Button::new(egui::RichText::new("New Map").color(p.bg_window).strong())
                    .fill(p.gold);
            if ui.add(new_map).clicked() {
                out.push(Action::NewMap);
            }
            if ui.button("Open…").clicked() {
                out.push(Action::OpenDialog);
            }
            if ui.button("Asset packs").clicked() {
                out.push(Action::ShowPacks);
            }
        });
        ui.add_space(12.0);

        let field = ui.add(
            egui::TextEdit::singleline(&mut self.start_query)
                .hint_text("Search maps")
                .desired_width(width),
        );
        if field.gained_focus() || field.changed() {
            self.start_search_open = true;
        }
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.start_search_open = false;
            field.surrender_focus();
        }
        let found = results(data.known, &self.start_query);
        if enter && let Some(entry) = found.first() {
            open_entry(entry, out);
            self.start_search_open = false;
        }
        // Clicking a result takes focus from the field first, so the panel stays while the
        // pointer is over it.
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over_panel = self
            .start_panel
            .zip(pointer)
            .is_some_and(|(rect, at)| rect.contains(at));
        if !field.has_focus() && !over_panel {
            self.start_search_open = false;
        }

        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Recents")
                    .size(theme::HEADING)
                    .color(p.text_primary)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.link("Browse all maps").clicked() {
                    out.push(Action::ShowHome);
                }
            });
        });
        ui.add_space(4.0);
        if data.known.is_empty() {
            ui.label(
                egui::RichText::new("No Maps yet. Create one or open a .ssmap file.")
                    .color(p.text_secondary),
            );
        }
        for entry in data.known.iter().take(RECENTS) {
            self.map_row(ui, entry, "", now, out);
        }

        self.start_panel = if self.start_search_open {
            Some(self.search_panel(ui, field.rect, &found, now, out))
        } else {
            None
        };
    }

    /// The results over the Recents. Returns the panel's rectangle.
    fn search_panel(
        &self,
        ui: &mut egui::Ui,
        field: Rect,
        found: &[&Entry],
        now: SystemTime,
        out: &mut Vec<Action>,
    ) -> Rect {
        let p = theme::palette(ui.ctx());
        let area = egui::Area::new(egui::Id::new("start_search_results"))
            .order(egui::Order::Foreground)
            .fixed_pos(field.left_bottom() + vec2(0.0, 4.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(p.bg_raised)
                    .stroke(egui::Stroke::new(1.0, p.border_strong))
                    .corner_radius(theme::RADIUS_POPOVER)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(field.width() - 12.0);
                        if found.is_empty() {
                            ui.label(egui::RichText::new("No Maps match.").color(p.text_secondary));
                        }
                        for entry in found {
                            self.map_row(ui, entry, &self.start_query, now, out);
                        }
                    });
            });
        area.response.rect
    }

    /// One Map: thumbnail, name (with the text matching `query` marked), Canvas size and when
    /// it was edited. A click opens it.
    fn map_row(
        &self,
        ui: &mut egui::Ui,
        entry: &Entry,
        query: &str,
        now: SystemTime,
        out: &mut Vec<Action>,
    ) {
        let p = theme::palette(ui.ctx());
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::click());
        let label = format!("Open {}", entry.name);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, theme::RADIUS_FIELD, p.hover_fill);
        }
        let thumb = Rect::from_min_size(
            egui::pos2(rect.min.x + 6.0, rect.center().y - THUMB.y / 2.0),
            THUMB,
        );
        ui.painter().rect_filled(thumb, 3.0, p.bg_canvas);
        let texture = entry
            .path
            .as_deref()
            .and_then(|path| self.thumbnails.get(path));
        if let Some(texture) = texture {
            let size = texture.size_vec2();
            let scale = ((thumb.width() - 4.0) / size.x).min((thumb.height() - 4.0) / size.y);
            ui.painter().image(
                texture.id(),
                Rect::from_center_size(thumb.center(), size * scale),
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let text_x = thumb.max.x + 12.0;
        let text_width = (rect.max.x - text_x - 6.0).max(0.0);
        let mut name = name_job(&entry.name, query, theme::BODY, p.text_primary, p.gold_text);
        name.wrap.max_width = text_width;
        let galley = ui.painter().layout_job(name);
        ui.painter()
            .galley(egui::pos2(text_x, rect.min.y + 9.0), galley, p.text_primary);
        let mut parts = Vec::new();
        if entry.missing {
            parts.push("File missing".to_owned());
        } else {
            parts.extend(entry.canvas.map(size_label));
            parts.extend(entry.modified.map(|then| edited_ago(now, then)));
        }
        let color = if entry.missing {
            p.danger
        } else {
            p.text_tertiary
        };
        let mut meta = name_job(&parts.join(" · "), "", theme::CAPTION, color, color);
        meta.wrap.max_width = text_width;
        let galley = ui.painter().layout_job(meta);
        ui.painter()
            .galley(egui::pos2(text_x, rect.min.y + 29.0), galley, color);
        if response.clicked() {
            open_entry(entry, out);
        }
    }
}

/// Opens the Map in a tab, or shows its tab. A missing file does nothing.
fn open_entry(entry: &Entry, out: &mut Vec<Action>) {
    if entry.missing {
        return;
    }
    match (entry.tab, &entry.path) {
        (Some(tab), _) => out.push(Action::Focus(tab)),
        (None, Some(path)) => out.push(Action::Open(path.clone())),
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str) -> Entry {
        Entry {
            path: Some(PathBuf::from(format!("/maps/{name}.ssmap"))),
            name: name.into(),
            modified: None,
            canvas: None,
            tab: None,
            missing: false,
        }
    }

    fn names(found: &[&Entry]) -> Vec<String> {
        found.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn no_query_lists_the_five_newest() {
        let known: Vec<Entry> = (0..8).map(|i| entry(&format!("Map {i}"))).collect();
        let found = results(&known, "  ");
        assert_eq!(names(&found), ["Map 0", "Map 1", "Map 2", "Map 3", "Map 4"]);
    }

    #[test]
    fn a_query_keeps_only_matching_names_ignoring_case() {
        let known = vec![
            entry("Eldoria"),
            entry("The Shattered Coast"),
            entry("Bald Hills"),
        ];
        assert_eq!(names(&results(&known, "ALD")), ["Bald Hills"]);
        assert_eq!(names(&results(&known, "ld")), ["Eldoria", "Bald Hills"]);
        assert!(results(&known, "zzz").is_empty());
    }

    #[test]
    fn the_match_is_marked_in_the_name() {
        assert_eq!(match_range("Eldoria", "ld"), Some(1..3));
        assert_eq!(match_range("Bald Hills", "ALD"), Some(1..4));
        assert_eq!(match_range("Eldoria", ""), None);
        assert_eq!(match_range("Eldoria", "x"), None);
        // A multi-byte char before the match keeps the offsets on char boundaries.
        assert_eq!(match_range("Ünder", "nd"), Some(2..4));
    }
}
