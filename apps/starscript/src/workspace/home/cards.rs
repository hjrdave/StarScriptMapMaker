//! The painting of Home's left rail and Map cards.

use std::time::SystemTime;

use eframe::egui::{self, Color32, Rect, vec2};

use super::{Action, Entry, edited_ago, size_label};
use crate::file_actions;
use crate::icons::{self, Icon};
use crate::theme;

const ROW_HEIGHT: f32 = 28.0;

pub(super) fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(theme::CAPTION)
            .color(theme::palette(ui.ctx()).text_tertiary),
    );
    ui.add_space(2.0);
}

pub(super) fn rail_row(
    ui: &mut egui::Ui,
    selected: bool,
    label: &str,
    count: Option<usize>,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    paint_rail_row(ui, rect, selected, response.hovered(), label, count);
    response
}

fn paint_rail_row(
    ui: &egui::Ui,
    rect: Rect,
    selected: bool,
    hovered: bool,
    label: &str,
    count: Option<usize>,
) {
    let p = theme::palette(ui.ctx());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, theme::RADIUS_BUTTON, p.gold_subtle);
    } else if hovered {
        painter.rect_filled(rect, theme::RADIUS_BUTTON, p.hover_fill);
    }
    let color = if selected {
        p.gold_text
    } else {
        p.text_secondary
    };
    painter.text(
        egui::pos2(rect.min.x + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::new(theme::BODY, theme::ui_family()),
        color,
    );
    if let Some(count) = count {
        painter.text(
            egui::pos2(rect.max.x - 10.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::new(theme::CAPTION, theme::ui_family()),
            p.text_tertiary,
        );
    }
}

/// One line of text, cut with an ellipsis at `width`.
fn line(ui: &egui::Ui, at: egui::Pos2, width: f32, text: &str, size: f32, color: Color32) {
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

const FOOTER_HEIGHT: f32 = 52.0;

fn thumb_height(width: f32) -> f32 {
    // The preview is 16:10, as in the mockup.
    (width * 0.625).round()
}

fn card_size(width: f32) -> egui::Vec2 {
    vec2(width, thumb_height(width) + 1.0 + FOOTER_HEIGHT)
}

pub(super) fn new_map_card(ui: &mut egui::Ui, width: f32, out: &mut Vec<Action>) {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(card_size(width), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            "New Map: blank or from template",
        )
    });
    let preview = Rect::from_min_size(rect.min, vec2(width, thumb_height(width)));
    let stroke = egui::Stroke::new(
        1.0,
        if response.hovered() {
            p.gold
        } else {
            p.border_strong
        },
    );
    let corners = [
        preview.left_top(),
        preview.right_top(),
        preview.right_bottom(),
        preview.left_bottom(),
        preview.left_top(),
    ];
    ui.painter()
        .extend(egui::Shape::dashed_line(&corners, stroke, 4.0, 3.0));
    icons::paint(
        ui,
        Rect::from_center_size(preview.center() - vec2(0.0, 12.0), vec2(24.0, 24.0)),
        Icon::Add,
        24.0,
        p.gold,
    );
    ui.painter().text(
        preview.center() + vec2(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        "New Map",
        egui::FontId::new(theme::BODY, theme::ui_family()),
        p.text_secondary,
    );
    let below = preview.max.y + 10.0;
    line(
        ui,
        egui::pos2(rect.min.x + 2.0, below),
        width - 4.0,
        "Blank or from template",
        theme::BODY,
        p.text_primary,
    );
    line(
        ui,
        egui::pos2(rect.min.x + 2.0, below + 18.0),
        width - 4.0,
        NEW_MAP_SHORTCUT,
        theme::CAPTION,
        p.text_tertiary,
    );
    if response.clicked() {
        out.push(Action::NewMap);
    }
}

/// How the New Map shortcut reads.
const NEW_MAP_SHORTCUT: &str = if cfg!(target_os = "macos") {
    "⌘N"
} else {
    "Ctrl+N"
};

pub(super) fn map_card(
    ui: &mut egui::Ui,
    width: f32,
    entry: &Entry,
    texture: Option<&egui::TextureHandle>,
    now: SystemTime,
    out: &mut Vec<Action>,
) {
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(card_size(width), egui::Sense::click());
    let label = format!("Open {}", entry.name);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    let preview = Rect::from_min_size(rect.min, vec2(width, thumb_height(width)));
    let radius = theme::RADIUS_FIELD;
    let more_id = response.id.with("more");
    let menu_open = egui::Popup::is_id_open(ui.ctx(), more_id.with("popup"))
        || egui::Popup::is_id_open(ui.ctx(), response.id.with("context"));
    let hovered = ui.rect_contains_pointer(rect) || menu_open;
    let painter = ui.painter();
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
    match texture {
        Some(texture) => {
            // Shown at about 2×: the stored preview is up to 512 px on its longer side.
            let size = texture.size_vec2();
            let scale = ((preview.width() - 16.0) / size.x).min((preview.height() - 16.0) / size.y);
            let image = Rect::from_center_size(preview.center(), size * scale);
            painter.image(
                texture.id(),
                image,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            let text = if entry.missing {
                "File missing"
            } else {
                "No preview"
            };
            painter.text(
                preview.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::new(theme::CAPTION, theme::ui_family()),
                if entry.missing {
                    p.danger
                } else {
                    p.text_tertiary
                },
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
        egui::Stroke::new(1.0, if hovered { p.gold } else { p.border_subtle }),
        egui::StrokeKind::Inside,
    );

    // The footer: file-type icon, name, and when it was edited.
    let footer_top = preview.max.y + 1.0;
    let text_x = rect.min.x + 14.0;
    let text_width = (rect.max.x - text_x - 36.0).max(0.0);
    line(
        ui,
        egui::pos2(text_x, footer_top + 8.0),
        text_width,
        &entry.name,
        theme::BODY,
        p.text_primary,
    );
    let meta = if entry.missing {
        "Missing".to_owned()
    } else {
        entry
            .modified
            .map(|then| edited_ago(now, then))
            .unwrap_or_default()
    };
    line(
        ui,
        egui::pos2(text_x, footer_top + 28.0),
        text_width,
        &meta,
        theme::CAPTION,
        if entry.missing {
            p.danger
        } else {
            p.text_tertiary
        },
    );

    let mut removed = false;
    if let (true, Some(path)) = (entry.missing, &entry.path) {
        let button = Rect::from_center_size(preview.center() + vec2(0.0, 26.0), vec2(130.0, 24.0));
        if ui
            .put(button, egui::Button::new("Remove from list"))
            .clicked()
        {
            out.push(Action::Remove(path.clone()));
            removed = true;
        }
    }
    // The "…" button, shown while the card is hovered or its menu is open.
    if entry.path.is_some() {
        let button = Rect::from_center_size(
            egui::pos2(rect.max.x - 22.0, footer_top + FOOTER_HEIGHT / 2.0),
            vec2(24.0, 24.0),
        );
        let more = ui.interact(button, more_id, egui::Sense::click());
        more.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "More actions")
        });
        if hovered || more.has_focus() {
            if more.hovered() {
                ui.painter()
                    .rect_filled(button, theme::RADIUS_BUTTON, p.hover_fill);
            }
            icons::paint(ui, button, Icon::More, 16.0, p.text_secondary);
        }
        egui::Popup::menu(&more)
            .id(more_id.with("popup"))
            .show(|ui| card_menu(ui, entry, out));
        removed |= more.clicked();
    }
    response.context_menu(|ui| card_menu(ui, entry, out));

    // No tooltip over an open menu, where it would cover the items.
    let menu_open =
        egui::Popup::is_id_open(ui.ctx(), more_id.with("popup")) || response.context_menu_opened();
    if !menu_open {
        let mut parts = Vec::new();
        if let Some(canvas) = entry.canvas {
            parts.push(size_label(canvas));
        }
        if entry.is_open() {
            parts.push("Open in a tab".to_owned());
        }
        if !parts.is_empty() {
            response.clone().on_hover_text(parts.join(" · "));
        }
    }
    if response.clicked() && !removed && !entry.missing {
        match (entry.tab, &entry.path) {
            (Some(tab), _) => out.push(Action::Focus(tab)),
            (None, Some(path)) => out.push(Action::Open(path.clone())),
            (None, None) => {}
        }
    }
}

/// The items of a card's "…" and right-click menus.
fn card_menu(ui: &mut egui::Ui, entry: &Entry, out: &mut Vec<Action>) {
    let Some(path) = &entry.path else {
        return;
    };
    if !entry.missing {
        let open = if entry.is_open() { "Show tab" } else { "Open" };
        if ui.button(open).clicked() {
            match entry.tab {
                Some(tab) => out.push(Action::Focus(tab)),
                None => out.push(Action::Open(path.clone())),
            }
            ui.close();
        }
    }
    if ui
        .add_enabled(
            !entry.missing,
            egui::Button::new(file_actions::REVEAL_LABEL),
        )
        .clicked()
    {
        out.push(Action::Reveal(path.clone()));
        ui.close();
    }
    ui.separator();
    if entry.missing {
        if ui.button("Remove from list").clicked() {
            out.push(Action::Remove(path.clone()));
            ui.close();
        }
    } else if ui
        .button(egui::RichText::new("Delete…").color(theme::palette(ui.ctx()).danger))
        .clicked()
    {
        out.push(Action::Delete(path.clone()));
        ui.close();
    }
}
