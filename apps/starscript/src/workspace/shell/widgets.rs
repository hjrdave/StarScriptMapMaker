//! The widgets of the Layers and Properties columns: headers, the history list, the dividers,
//! the collapse buttons and the rails.

use eframe::egui;

use super::{DIVIDER, RailClick, Shell, Side, SidebarSection};
use crate::icons::{self, Icon};
use crate::theme::{self, metrics};

/// A panel's 34 pt header: `title` in the heading style and, on the right, whatever `trailing`
/// adds.
pub fn panel_header(ui: &mut egui::Ui, title: &str, trailing: impl FnOnce(&mut egui::Ui)) {
    let palette = theme::palette(ui.ctx());
    let width = ui.available_width();
    let header = ui.allocate_ui_with_layout(
        egui::vec2(width, metrics::PANEL_HEADER),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add_space(theme::SPACING[3]);
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

/// The undo history as a list: "Original" and then every step, oldest first. `applied` steps
/// are in effect; the rest are dimmed and can be redone. Returns how many steps should be
/// applied after a click on an entry.
pub fn history_list(ui: &mut egui::Ui, steps: &[String], applied: usize) -> Option<usize> {
    let palette = theme::palette(ui.ctx());
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let entries = std::iter::once("Original").chain(steps.iter().map(String::as_str));
            for (position, label) in entries.enumerate() {
                let color = if position > applied {
                    palette.text_secondary
                } else {
                    palette.text_primary
                };
                let text = egui::RichText::new(label).color(color);
                let response = ui.add_sized(
                    [ui.available_width(), metrics::PANEL_HEADER * 0.8],
                    egui::Button::selectable(position == applied, text).frame_when_inactive(false),
                );
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                    && position != applied
                {
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

/// The Layers collapse control: a double chevron that points left while the panel is
/// expanded and is mirrored to point right once `collapsed`. Its place is the bottom of the
/// panel in both states.
pub fn collapse_button(ui: &mut egui::Ui, collapsed: bool) -> egui::Response {
    let label = if collapsed {
        "Expand Layers (⌘⌥1)"
    } else {
        "Collapse Layers (⌘⌥1)"
    };
    icons::icon_button_flipped(ui, Icon::CollapseDouble, label, collapsed)
}

/// The Properties collapse control: the Layers double chevron mirrored, so it points right
/// while the panel is expanded and left once `collapsed`.
pub fn properties_collapse_button(ui: &mut egui::Ui, collapsed: bool) -> egui::Response {
    let label = if collapsed {
        "Expand Properties (⌘⌥2)"
    } else {
        "Collapse Properties (⌘⌥2)"
    };
    icons::icon_button_flipped(ui, Icon::CollapseDouble, label, !collapsed)
}

/// The 40 pt rail Layers collapses to: only the section icons, with the expand control pinned
/// to the bottom. Returns what was clicked, which expands the sidebar.
pub fn layers_rail(ui: &mut egui::Ui, path: &[String], shell: &Shell) -> Option<RailClick> {
    let current = shell.sidebar_section();
    let palette = theme::palette(ui.ctx());
    let rect = ui.max_rect();
    let mut clicked = None;
    let step = icons::GRID + 8.0 + theme::SPACING[1];
    let mut layers_y = rect.top() + metrics::PANEL_HEADER / 2.0;
    let mut count = 0.0;
    for section in SidebarSection::ALL
        .into_iter()
        .filter(|&s| shell.section_available(s))
    {
        let slot = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, layers_y + count * step),
            egui::Vec2::splat(icons::GRID + 8.0),
        );
        let response = ui.interact(
            slot,
            ui.id().with(("sidebar section", section.title())),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, section.label())
        });
        let tint = if section == current || response.hovered() {
            palette.text_primary
        } else {
            palette.text_secondary
        };
        icons::paint(ui, slot, section.icon(), icons::GRID, tint);
        if response
            .on_hover_text(section.label())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            clicked = Some(RailClick::Section(section));
        }
        count += 1.0;
    }
    layers_y += (count - 1.0).max(0.0) * step;
    if !path.is_empty() {
        let text = path.join(" › ");
        let galley = ui.painter().layout_no_wrap(
            text,
            egui::FontId::proportional(theme::BODY),
            palette.text_secondary,
        );
        // Turned a quarter clockwise about its top-left corner, so it reads top to bottom
        // and sits in a column as wide as one line of text.
        let origin = egui::pos2(
            rect.center().x + galley.size().y / 2.0,
            layers_y + icons::GRID + theme::SPACING[3],
        );
        let shape = egui::epaint::TextShape::new(origin, galley, palette.text_secondary)
            .with_angle(std::f32::consts::FRAC_PI_2);
        ui.painter().add(shape);
    }
    let bar = egui::Rect::from_min_max(
        egui::pos2(
            rect.left(),
            rect.bottom() - crate::layer_tree::FOOTER_HEIGHT,
        ),
        rect.right_bottom(),
    );
    let mut bottom =
        ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(
            egui::Layout::top_down(egui::Align::Center).with_main_align(egui::Align::Center),
        ));
    if collapse_button(&mut bottom, true)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        clicked = Some(RailClick::Expand);
    }
    clicked
}

/// The 40 pt icon strip Properties collapses to, or narrows to at the smallest window: the
/// Properties icon with the expand control pinned to the bottom, as on the Layers rail.
/// Returns true when either is clicked.
pub fn properties_rail(ui: &mut egui::Ui) -> bool {
    let palette = theme::palette(ui.ctx());
    let rect = ui.max_rect();
    let mut clicked = false;
    let slot = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + metrics::PANEL_HEADER / 2.0),
        egui::Vec2::splat(icons::GRID + 8.0),
    );
    let response = ui.interact(slot, ui.id().with("properties rail"), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Open Properties (⌘⌥2)")
    });
    let tint = if response.hovered() {
        palette.text_primary
    } else {
        palette.text_secondary
    };
    icons::paint(ui, slot, Icon::Shape, icons::GRID, tint);
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        clicked = true;
    }
    let bar = egui::Rect::from_min_max(
        egui::pos2(
            rect.left(),
            rect.bottom() - crate::layer_tree::FOOTER_HEIGHT,
        ),
        rect.right_bottom(),
    );
    let mut bottom =
        ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(
            egui::Layout::top_down(egui::Align::Center).with_main_align(egui::Align::Center),
        ));
    if properties_collapse_button(&mut bottom, true)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        clicked = true;
    }
    clicked
}
