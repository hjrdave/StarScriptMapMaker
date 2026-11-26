//! The widgets of the Layers and Properties columns: headers, the history list, the dividers,
//! the collapse buttons and the rails.

use eframe::egui;

use super::{DIVIDER, Shell, Side, SidebarSection, history_icon};
use crate::icons::{self, Icon};
use crate::theme::{self, metrics};

/// A panel's 34 pt header: `title` in the heading style and, on the right, whatever `trailing`
/// adds.
pub fn panel_header(ui: &mut egui::Ui, title: &str, trailing: impl FnOnce(&mut egui::Ui)) {
    header(ui, None, title, trailing);
}

/// [`panel_header`] with an icon before the title, painted in `tint` (a tool's icon and name).
pub fn icon_header(
    ui: &mut egui::Ui,
    icon: Icon,
    tint: egui::Color32,
    title: &str,
    trailing: impl FnOnce(&mut egui::Ui),
) {
    header(ui, Some((icon, tint)), title, trailing);
}

/// A small rounded label at a header's right edge, such as the tool's mode. `danger` paints it
/// in the danger colour instead of the gold accent. Named "Mode: <text>" for assistive tools.
pub fn mode_chip(ui: &mut egui::Ui, text: &str, danger: bool) {
    let palette = theme::palette(ui.ctx());
    let (fill, ink) = if danger {
        (palette.danger.gamma_multiply(0.18), palette.danger)
    } else {
        (palette.gold_subtle, palette.gold_text)
    };
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(theme::CAPTION),
        ink,
    );
    let size = galley.size() + egui::vec2(12.0, 6.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("Mode: {text}"))
    });
    ui.painter().rect_filled(rect, theme::RADIUS_CHIP, fill);
    ui.painter()
        .galley(rect.min + egui::vec2(6.0, 3.0), galley, ink);
}

fn header(
    ui: &mut egui::Ui,
    lead: Option<(Icon, egui::Color32)>,
    title: &str,
    trailing: impl FnOnce(&mut egui::Ui),
) {
    let palette = theme::palette(ui.ctx());
    let width = ui.available_width();
    let header = ui.allocate_ui_with_layout(
        egui::vec2(width, metrics::PANEL_HEADER),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add_space(theme::SPACING[3]);
            if let Some((icon, tint)) = lead {
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(icons::GRID), egui::Sense::hover());
                icons::paint(ui, rect, icon, icons::GRID, tint);
            }
            ui.label(
                egui::RichText::new(title)
                    .size(theme::HEADING)
                    .strong()
                    .color(palette.text_primary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(theme::SPACING[2]);
                trailing(ui);
            });
        },
    );
    let rect = header.response.rect;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0, palette.border_subtle),
    );
}

/// Height of one history row.
const HISTORY_ROW: f32 = metrics::PANEL_HEADER * 0.8;

/// The undo history as a list: "Original" and then every step, oldest first, each with its
/// icon and its name on the left. `applied` steps are in effect; the current one is gold and
/// the rest are dimmed and can be redone. A hovered step shows "Step here" at its right edge.
/// Returns how many steps should be applied after a click on an entry.
pub fn history_list(ui: &mut egui::Ui, steps: &[String], applied: usize) -> Option<usize> {
    let palette = theme::palette(ui.ctx());
    let mut clicked = None;
    // The list follows the current step when it moves, and otherwise leaves the scroll alone.
    let seen_id = ui.id().with("history applied");
    let moved = ui.data_mut(|data| data.get_temp::<usize>(seen_id)) != Some(applied);
    ui.data_mut(|data| data.insert_temp(seen_id, applied));
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let entries = std::iter::once("Original").chain(steps.iter().map(String::as_str));
            for (position, label) in entries.enumerate() {
                let current = position == applied;
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), HISTORY_ROW),
                    egui::Sense::click(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, label)
                });
                if current && moved {
                    response.scroll_to_me(None);
                }
                let hovered = response.hovered() || response.has_focus();
                if current {
                    ui.painter()
                        .rect_filled(rect, theme::RADIUS_BUTTON, palette.hover_fill);
                } else if hovered {
                    ui.painter().rect_filled(
                        rect,
                        theme::RADIUS_BUTTON,
                        palette.hover_fill.gamma_multiply(0.5),
                    );
                }
                let color = if current {
                    palette.gold_text
                } else if position > applied {
                    palette.text_secondary
                } else {
                    palette.text_primary
                };
                let icon_box = egui::Rect::from_center_size(
                    egui::pos2(
                        rect.left() + theme::SPACING[3] + icons::GRID / 2.0,
                        rect.center().y,
                    ),
                    egui::Vec2::splat(icons::GRID),
                );
                match history_icon::step_icon(label) {
                    Some(icon) => icons::paint(ui, icon_box, icon, icons::GRID, color),
                    None => {
                        ui.painter().circle_filled(icon_box.center(), 2.0, color);
                    }
                }
                ui.painter().text(
                    egui::pos2(icon_box.right() + theme::SPACING[2], rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(theme::BODY),
                    color,
                );
                if hovered && !current {
                    ui.painter().text(
                        egui::pos2(rect.right() - theme::SPACING[3], rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        "Step here",
                        egui::FontId::proportional(theme::CAPTION),
                        palette.text_tertiary,
                    );
                }
                let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                let response = if current {
                    response
                } else {
                    response.on_hover_text("Step here")
                };
                if response.clicked() && !current {
                    clicked = Some(position);
                }
            }
        });
    clicked
}

/// Drags the strip on a column's inner edge, resizing it; a double click resets the width.
/// `panel` is the column's rect.
pub fn divider(ui: &mut egui::Ui, shell: &mut Shell, side: Side, panel: egui::Rect) {
    let x = match side {
        Side::Left => panel.right(),
        Side::Right => panel.left(),
    };
    let strip =
        egui::Rect::from_x_y_ranges((x - DIVIDER / 2.0)..=(x + DIVIDER / 2.0), panel.y_range());
    let id = ui.id().with(("shell divider", matches!(side, Side::Left)));
    let response = ui
        .interact(strip, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            match side {
                Side::Left => "Layers divider",
                Side::Right => "Properties divider",
            },
        )
    });
    if response.double_clicked() {
        shell.reset_width(side);
    } else if response.dragged() {
        let delta = response.drag_delta().x;
        let width = shell.width(side);
        shell.set_width(
            side,
            match side {
                Side::Left => width + delta,
                Side::Right => width - delta,
            },
        );
    }
    let palette = theme::palette(ui.ctx());
    let color = if response.hovered() || response.dragged() {
        palette.gold
    } else {
        palette.border_subtle
    };
    ui.painter()
        .vline(x, panel.y_range(), egui::Stroke::new(1.0, color));
}

/// The size a rail icon is drawn at; it has its own crisp atlas.
const RAIL_ICON: f32 = icons::RAIL_GRID;
/// Size of the rounded tile behind a rail icon.
const RAIL_TILE: egui::Vec2 = egui::vec2(40.0, 32.0);
/// Gap above the tile, inside the slot.
const RAIL_TILE_TOP: f32 = 2.0;
/// Height of a rail slot: the tile and the caption below it.
const RAIL_SLOT_HEIGHT: f32 = RAIL_TILE_TOP + RAIL_TILE.y + 18.0;
/// The click area: the rail less a margin each side, wide enough for the longest caption.
const RAIL_SLOT: egui::Vec2 = egui::vec2(metrics::LAYERS_PANEL_RAIL - 8.0, RAIL_SLOT_HEIGHT);

/// How a rail slot looks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RailState {
    Active,
    Hover,
    Idle,
}

impl RailState {
    fn new(active: bool, hovered: bool) -> Self {
        if active {
            Self::Active
        } else if hovered {
            Self::Hover
        } else {
            Self::Idle
        }
    }
}

/// The tile behind a slot's icon.
fn rail_tile_rect(slot: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(
        egui::pos2(
            slot.center().x,
            slot.top() + RAIL_TILE_TOP + RAIL_TILE.y / 2.0,
        ),
        RAIL_TILE,
    )
}

/// A rail slot: the tile (gold when active, grey on hover) with the icon on it, and the caption
/// below the tile.
fn paint_rail_slot(ui: &egui::Ui, slot: egui::Rect, icon: Icon, caption: &str, state: RailState) {
    let palette = theme::palette(ui.ctx());
    let tile = rail_tile_rect(slot);
    let radius = egui::CornerRadius::same(theme::RADIUS_POPOVER);
    let (icon_tint, caption_tint, caption_font) = match state {
        RailState::Active => {
            ui.painter().rect_filled(tile, radius, palette.gold_subtle);
            ui.painter().rect_stroke(
                tile,
                radius,
                egui::Stroke::new(1.0, palette.gold_ring),
                egui::StrokeKind::Inside,
            );
            (
                palette.gold_text,
                palette.text_primary,
                theme::medium_font(ui.ctx(), theme::CAPTION),
            )
        }
        RailState::Hover => {
            ui.painter().rect_filled(tile, radius, palette.hover_fill);
            (
                palette.text_primary,
                palette.text_primary,
                egui::FontId::proportional(theme::CAPTION),
            )
        }
        RailState::Idle => (
            palette.text_secondary,
            palette.text_tertiary,
            egui::FontId::proportional(theme::CAPTION),
        ),
    };
    let icon_area = egui::Rect::from_center_size(tile.center(), egui::Vec2::splat(RAIL_ICON));
    icons::paint(ui, icon_area, icon, RAIL_ICON, icon_tint);
    ui.painter().text(
        egui::pos2(slot.center().x, slot.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        caption,
        caption_font,
        caption_tint,
    );
}

/// The rail of the left sidebar, always on screen: the section icons, with the open section's
/// on a gold tile. Returns the section whose icon was clicked.
pub fn layers_rail(ui: &mut egui::Ui, shell: &Shell) -> Option<SidebarSection> {
    let current = shell.sidebar_section();
    let rect = ui.max_rect();
    let mut clicked = None;
    let step = RAIL_SLOT_HEIGHT + theme::SPACING[1];
    let layers_y = rect.top() + metrics::PANEL_HEADER / 2.0 + RAIL_SLOT_HEIGHT / 2.0 - 12.0;
    let mut count = 0.0;
    for section in SidebarSection::ALL {
        let slot = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, layers_y + count * step),
            RAIL_SLOT,
        );
        let response = ui.interact(
            slot,
            ui.id().with(("sidebar section", section.title())),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, section.label())
        });
        let open = section == current && shell.panel_open();
        let state = RailState::new(open, response.hovered());
        paint_rail_slot(ui, slot, section.icon(), section.title(), state);
        if response
            .on_hover_text(section.label())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            clicked = Some(section);
        }
        count += 1.0;
    }
    clicked
}

/// The icon strip Properties narrows to at the smallest window: the Properties icon, which
/// opens the floating inspector. Returns true when it is clicked.
pub fn properties_rail(ui: &mut egui::Ui) -> bool {
    let rect = ui.max_rect();
    let mut clicked = false;
    let slot = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + metrics::PANEL_HEADER / 2.0),
        RAIL_SLOT,
    );
    let response = ui.interact(slot, ui.id().with("properties rail"), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Open Properties (⌘⌥2)")
    });
    let state = RailState::new(false, response.hovered());
    paint_rail_slot(ui, slot, Icon::Shape, "Properties", state);
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        clicked = true;
    }
    clicked
}

#[cfg(test)]
mod rail_tests {
    use super::*;

    #[test]
    fn state_prefers_active_over_hover() {
        assert_eq!(RailState::new(true, true), RailState::Active);
        assert_eq!(RailState::new(false, true), RailState::Hover);
        assert_eq!(RailState::new(false, false), RailState::Idle);
    }

    #[test]
    fn tile_sits_inside_the_slot_above_the_caption() {
        let slot = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), RAIL_SLOT);
        let tile = rail_tile_rect(slot);
        assert!(slot.contains_rect(tile));
        assert_eq!(tile.size(), RAIL_TILE);
        assert!(slot.bottom() - tile.bottom() >= 14.0);
    }
}
