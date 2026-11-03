//! The header row of a category: a chevron, the Layers icon, the name and a plus that makes a
//! Layer of that category.

use ssmap_core::Category;

use super::*;

/// The tooltip of a category's plus.
pub(crate) fn new_layer_label(category: Category) -> &'static str {
    match category {
        Category::Labels => "New Label Layer",
        Category::Assets => "New Asset Layer",
        Category::Land => "New Land Layer",
        Category::Backdrop => "New Backdrop Layer",
    }
}

impl LayerTree {
    /// Draws the header of `category`, which holds `layers` Layers.
    pub(super) fn category_row(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        category: Category,
        layers: usize,
        actions: &mut Vec<Action>,
    ) {
        let _ = editor;
        let palette = theme::palette(ui.ctx());
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click(),
        );
        if response.hovered() {
            ui.painter().rect_filled(rect, 2.0, palette.tab_hover);
        }
        let collapsed = self.collapsed_categories.contains(&category);
        // The plus sits at the far right; the rest of the row is the header.
        let plus = egui::Rect::from_min_size(
            egui::pos2(
                rect.right() - 4.0 - ROW_BUTTON,
                rect.center().y - ROW_BUTTON / 2.0,
            ),
            egui::Vec2::splat(ROW_BUTTON),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    rect.min,
                    egui::pos2(plus.left() - 4.0, rect.bottom()),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 4.0;
        child.add_space(4.0);
        let (chevron, name) = if collapsed {
            (Icon::ChevronRight, "Expand")
        } else {
            (Icon::ChevronDown, "Collapse")
        };
        if icons::icon_button_with(
            &mut child,
            chevron,
            &format!("{name} {}", category.name()),
            ROW_BUTTON,
            ROW_GLYPH,
            Some(palette.text_tertiary),
        )
        .clicked()
        {
            actions.push(Action::ToggleCategory(category));
        }
        let (icon_rect, _) =
            child.allocate_exact_size(egui::Vec2::splat(icons::GRID), egui::Sense::hover());
        icons::paint(
            &child,
            icon_rect,
            Icon::LayerGroup,
            icons::GRID,
            palette.text_secondary,
        );
        child.add(
            egui::Label::new(
                egui::RichText::new(category.name())
                    .size(theme::BODY)
                    .strong()
                    .color(palette.text_primary),
            )
            .selectable(false),
        );
        // A collapsed category says how many Layers it is holding.
        if collapsed {
            child.add(
                egui::Label::new(
                    egui::RichText::new(layers.to_string())
                        .size(theme::CAPTION)
                        .color(palette.text_tertiary),
                )
                .selectable(false),
            );
        }
        // A double click on the name folds it too.
        if response.double_clicked() {
            actions.push(Action::ToggleCategory(category));
        }
        let label = new_layer_label(category);
        let plus_response = ui
            .put(plus, |ui: &mut egui::Ui| {
                icons::icon_button_with(
                    ui,
                    Icon::Add,
                    label,
                    ROW_BUTTON,
                    ROW_GLYPH,
                    Some(palette.text_secondary),
                )
            })
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if plus_response.clicked() {
            actions.push(Action::NewLayer(category));
        }
        response.context_menu(|ui| {
            if ui.button(label).clicked() {
                actions.push(Action::NewLayer(category));
                ui.close();
            }
        });
    }
}
