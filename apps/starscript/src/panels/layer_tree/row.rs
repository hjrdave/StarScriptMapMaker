//! Drawing one row of the tree: its trailing indicators, name, context menu and drop target.

use super::*;

impl LayerTree {
    /// The chevron, kind icon and name (or the rename box) of a row, in the room left of its
    /// trailing indicators, which start at `trailing_left`.
    fn row_label(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        paint: &RowPaint<'_>,
        trailing_left: f32,
        actions: &mut Vec<Action>,
    ) {
        let RowPaint {
            row,
            rect,
            state,
            selected,
            ..
        } = *paint;
        let layer = paint.layer();
        let map = editor.map();
        let palette = theme::palette(ui.ctx());
        let node = paint.node;
        let left = egui::Rect::from_min_max(
            rect.min,
            egui::pos2((trailing_left - 4.0).max(rect.left()), rect.bottom()),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(left)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 4.0;
        child.add_space(4.0 + INDENT * row.depth as f32);
        let has_children = !map.children(Some(row.id)).is_empty();
        if layer.is_some() && has_children {
            let collapsed = self.collapsed.contains(&row.id);
            let (icon, name) = if collapsed {
                (Icon::ChevronRight, "Expand")
            } else {
                (Icon::ChevronDown, "Collapse")
            };
            if icons::icon_button_with(
                &mut child,
                icon,
                name,
                ROW_BUTTON,
                ROW_GLYPH,
                Some(palette.text_tertiary),
            )
            .clicked()
            {
                actions.push(Action::Toggle(row.id));
            }
        } else {
            child.add_space(ROW_BUTTON);
        }
        let (kind_rect, _) =
            child.allocate_exact_size(egui::Vec2::splat(icons::GRID), egui::Sense::hover());
        let kind_tint = if selected {
            palette.gold_text
        } else if state.dim() {
            palette.text_tertiary
        } else {
            palette.text_secondary
        };
        icons::paint(&child, kind_rect, kind_icon(node), icons::GRID, kind_tint);

        match &mut self.renaming {
            Some((id, text)) if *id == row.id => {
                let edit = egui::Frame::new()
                    .fill(palette.bg_field)
                    .stroke(egui::Stroke::new(1.0, palette.gold))
                    .corner_radius(theme::RADIUS_FIELD)
                    .inner_margin(egui::Margin::symmetric(6, 0))
                    .show(&mut child, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(text)
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY),
                        )
                    })
                    .inner;
                if !edit.has_focus() && !edit.lost_focus() {
                    edit.request_focus();
                }
                if child.input(|i| i.key_pressed(egui::Key::Escape)) {
                    actions.push(Action::CancelRename);
                } else if edit.lost_focus() {
                    actions.push(Action::FinishRename);
                }
            }
            _ => {
                let color = if state.dim() {
                    palette.text_tertiary
                } else if selected {
                    palette.text_primary
                } else {
                    palette.text_secondary
                };
                let mut text = egui::RichText::new(node_name(map, row.id))
                    .size(theme::BODY)
                    .color(color);
                if state.italic() {
                    text = text.italics();
                }
                child.add(egui::Label::new(text).selectable(false).truncate());
            }
        }
    }

    pub(super) fn row(
        &mut self,
        ui: &mut egui::Ui,
        editor: &Editor,
        row: Row,
        actions: &mut Vec<Action>,
    ) {
        let map = &editor.map();
        let Some(node) = map.node(row.id) else { return };
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click_and_drag(),
        );
        let palette = theme::palette(ui.ctx());
        let selected = editor.is_selected(row.id);
        let state = RowState::of(map, row.id);
        let hovering = ui.rect_contains_pointer(rect);
        if selected {
            ui.painter().rect_filled(rect, 2.0, palette.gold_subtle);
            ui.painter().rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(1.0, palette.gold_ring),
                egui::StrokeKind::Inside,
            );
        } else if hovering {
            ui.painter().rect_filled(rect, 2.0, palette.tab_hover);
        }

        let paint = RowPaint {
            row,
            node,
            rect,
            state,
            selected,
            hovering,
        };
        let trailing_left = trailing_indicators(ui, editor, &paint, actions);
        self.row_label(ui, editor, &paint, trailing_left, actions);

        if response.clicked() {
            let (command, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            let pick = if shift {
                Pick::Range
            } else if command {
                Pick::Toggle
            } else {
                Pick::Only
            };
            actions.push(Action::Select(row.id, pick));
        }
        if response.double_clicked() {
            actions.push(Action::StartRename(row.id));
        }
        // A Depth-sorted Layer orders its own Stamps; they can still be dragged to another Layer.
        if response.dragged() {
            response.dnd_set_drag_payload(row.id);
            self.dragging = true;
        }
        if is_depth_sorted_stamp(map, row.id) {
            response.clone().on_hover_text(DEPTH_SORT_HINT);
        }
        row_context_menu(&response, editor, &paint, actions);
        // No indicator where the drop would be refused, such as onto a locked Group.
        if let Some(dragged) = response.dnd_hover_payload::<Id>()
            && let Some(target) = drop_target(ui, rect, row.id, paint.layer().is_some())
            && let Some(command) = move_command(map, *dragged, target)
        {
            paint_drop_indicator(ui, palette, rect, target);
            if response.dnd_release_payload::<Id>().is_some() {
                actions.push(Action::Run(command));
            }
        }
    }
}

/// Everything one row needs to draw itself.
#[derive(Clone, Copy)]
struct RowPaint<'a> {
    row: Row,
    node: &'a Node,
    rect: egui::Rect,
    state: RowState,
    selected: bool,
    hovering: bool,
}

impl RowPaint<'_> {
    /// The row's Layer, or `None` for an Object.
    fn layer(&self) -> Option<&Layer> {
        match self.node {
            Node::Layer(layer) => Some(layer),
            Node::Object(_) => None,
        }
    }
}

/// Lays out the lock, Link, eye, active Layer, opacity, Depth sort and Clip indicators at the
/// right of a row, right to left, and returns the x where they start.
fn trailing_indicators(
    ui: &mut egui::Ui,
    editor: &Editor,
    paint: &RowPaint<'_>,
    actions: &mut Vec<Action>,
) -> f32 {
    let RowPaint {
        row,
        rect,
        state,
        hovering,
        ..
    } = *paint;
    let layer = paint.layer();
    let map = editor.map();
    let palette = theme::palette(ui.ctx());
    // Laid out first, so the name knows how much room is left.
    let mut trailing = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(4.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    trailing.spacing_mut().item_spacing.x = 2.0;
    let mut trailing_left = rect.right();
    // Lock: always when locked, otherwise on hover.
    if state.locked_by_parent {
        // Shown so the inherited lock is visible, but it can't be toggled on the child.
        let (r, lock) =
            trailing.allocate_exact_size(egui::Vec2::splat(ROW_BUTTON), egui::Sense::click());
        let label = "Locked by parent Layer";
        lock.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, false, label));
        icons::paint(
            &trailing,
            r,
            Icon::LockDashed,
            ROW_GLYPH,
            palette.text_tertiary,
        );
        lock.on_hover_text(label);
        trailing_left = trailing.min_rect().left();
    } else if state.locked || hovering {
        let (icon, label, tint) = if state.locked {
            (Icon::LockFilled, "Unlock", palette.text_primary)
        } else {
            (Icon::Lock, "Lock", palette.text_tertiary)
        };
        if icons::icon_button_with(
            &mut trailing,
            icon,
            label,
            ROW_BUTTON,
            ROW_GLYPH,
            Some(tint),
        )
        .clicked()
        {
            actions.push(Action::Run(Command::SetLocked {
                id: row.id,
                locked: !state.locked,
            }));
        }
        trailing_left = trailing.min_rect().left();
    }
    if map.is_linked(row.id) {
        indicator(&mut trailing, Icon::Link, palette.text_secondary, "Linked");
        trailing_left = trailing.min_rect().left();
    }
    if let Some(layer) = layer {
        // Eye: always when hidden, otherwise on hover.
        if state.hidden || hovering {
            let (icon, label, tint) = if layer.visible {
                (Icon::Visible, "Hide", palette.text_tertiary)
            } else {
                (Icon::Hidden, "Show", palette.text_primary)
            };
            if icons::icon_button_with(
                &mut trailing,
                icon,
                label,
                ROW_BUTTON,
                ROW_GLYPH,
                Some(tint),
            )
            .clicked()
            {
                actions.push(Action::Run(Command::SetVisible {
                    id: row.id,
                    visible: !layer.visible,
                }));
            }
            trailing_left = trailing.min_rect().left();
        }
        if editor.active_layer() == Some(row.id) {
            indicator(
                &mut trailing,
                Icon::ActiveLayer,
                palette.gold,
                "Active Layer (new Objects go here)",
            );
            trailing_left = trailing.min_rect().left();
        }
        let percent = (layer.opacity * 100.0).round();
        if percent != 100.0 {
            let text = format!("{percent:.0}%");
            trailing
                .add(
                    egui::Label::new(
                        egui::RichText::new(&text)
                            .size(theme::CAPTION)
                            .color(palette.text_secondary),
                    )
                    .selectable(false)
                    .sense(egui::Sense::hover()),
                )
                .on_hover_text(format!("Opacity {text}"));
            trailing_left = trailing.min_rect().left();
        }
        if layer.depth_sort {
            indicator(
                &mut trailing,
                Icon::DepthSort,
                palette.text_secondary,
                "Depth-sorted",
            );
            trailing_left = trailing.min_rect().left();
        }
        if layer.clip.is_some() {
            indicator(
                &mut trailing,
                Icon::Clip,
                palette.clip,
                "Clipped to Landmass",
            );
            trailing_left = trailing.min_rect().left();
        }
    }
    trailing_left
}

/// The right-click menu of a row.
fn row_context_menu(
    response: &egui::Response,
    editor: &Editor,
    paint: &RowPaint<'_>,
    actions: &mut Vec<Action>,
) {
    let RowPaint {
        row,
        state,
        selected,
        ..
    } = *paint;
    let layer = paint.layer();
    let map = editor.map();
    response.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            actions.push(Action::StartRename(row.id));
            ui.close();
        }
        let duplicate = if layer.is_some() {
            "Duplicate Layer"
        } else {
            "Duplicate"
        };
        if ui.button(duplicate).clicked() {
            actions.push(Action::Duplicate(row.id));
            ui.close();
        }
        if ui.button("Group").clicked() {
            if !selected {
                actions.push(Action::Select(row.id, Pick::Only));
            }
            actions.push(Action::Group);
            ui.close();
        }
        if layer.is_some() && ui.button("Ungroup").clicked() {
            if !selected {
                actions.push(Action::Select(row.id, Pick::Only));
            }
            actions.push(Action::Ungroup);
            ui.close();
        }
        if ui
            .add_enabled(selected && editor.can_link(), egui::Button::new("Link"))
            .on_disabled_hover_text("Select two or more layers to link")
            .clicked()
        {
            actions.push(Action::Link);
            ui.close();
        }
        if ui
            .add_enabled(map.is_linked(row.id), egui::Button::new("Unlink"))
            .on_disabled_hover_text("This layer is not linked")
            .clicked()
        {
            if !selected {
                actions.push(Action::Select(row.id, Pick::Only));
            }
            actions.push(Action::Unlink);
            ui.close();
        }
        if ui.button("Copy").clicked() {
            actions.push(Action::Copy(row.id));
            ui.close();
        }
        if ui.button("Cut").clicked() {
            actions.push(Action::Cut(row.id));
            ui.close();
        }
        if ui
            .add_enabled(editor.has_clipboard(), egui::Button::new("Paste"))
            .on_disabled_hover_text("Copy or cut something first")
            .clicked()
        {
            if !selected {
                actions.push(Action::Select(row.id, Pick::Only));
            }
            actions.push(Action::Paste);
            ui.close();
        }
        if !state.locked_by_parent {
            let lock = if state.locked { "Unlock" } else { "Lock" };
            if ui.button(lock).clicked() {
                actions.push(Action::Run(Command::SetLocked {
                    id: row.id,
                    locked: !state.locked,
                }));
                ui.close();
            }
        }
        if let Some(layer) = layer {
            let hide = if layer.visible { "Hide" } else { "Show" };
            if ui.button(hide).clicked() {
                actions.push(Action::Run(Command::SetVisible {
                    id: row.id,
                    visible: !layer.visible,
                }));
                ui.close();
            }
        }
        if ui.button("Delete").clicked() {
            actions.push(Action::Delete(row.id));
            ui.close();
        }
        if let Some(layer) = layer {
            let selected = selected_landmass(map, editor.selection());
            if ui
                .add_enabled(
                    selected.is_some(),
                    egui::Button::new("Clip to selected Landmass"),
                )
                .on_disabled_hover_text("Select one Landmass first")
                .clicked()
            {
                actions.push(Action::Run(Command::SetClip {
                    id: row.id,
                    target: selected,
                }));
                ui.close();
            }
            if ui
                .add_enabled(layer.clip.is_some(), egui::Button::new("Clear Clip"))
                .clicked()
            {
                actions.push(Action::Run(Command::SetClip {
                    id: row.id,
                    target: None,
                }));
                ui.close();
            }
        }
    });
}

/// A read-only glyph with a tooltip; `label` is its accessible name too.
fn indicator(ui: &mut egui::Ui, icon: Icon, tint: egui::Color32, label: &str) {
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(ROW_BUTTON), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
    icons::paint(ui, rect, icon, ROW_GLYPH, tint);
    response.on_hover_text(label);
}

/// Top quarter drops above, bottom quarter below; the middle drops into a Layer, or splits
/// in half for an Object.
fn drop_target(ui: &egui::Ui, rect: egui::Rect, id: Id, is_layer: bool) -> Option<DropTarget> {
    let pointer = ui.ctx().pointer_interact_pos()?;
    let t = (pointer.y - rect.top()) / rect.height();
    Some(if is_layer {
        match t {
            t if t < 0.25 => DropTarget::Above(id),
            t if t > 0.75 => DropTarget::Below(id),
            _ => DropTarget::Into(id),
        }
    } else if t < 0.5 {
        DropTarget::Above(id)
    } else {
        DropTarget::Below(id)
    })
}

/// A gold line with a ring at its start between rows; a gold ring round the row to drop into.
fn paint_drop_indicator(ui: &egui::Ui, palette: &Palette, rect: egui::Rect, target: DropTarget) {
    let stroke = egui::Stroke::new(2.0, palette.gold);
    let painter = ui.painter();
    let line = |y: f32| {
        painter.hline((rect.left() + 20.0)..=rect.right(), y, stroke);
        painter.circle(
            egui::pos2(rect.left() + 18.0, y),
            4.0,
            palette.bg_panel,
            stroke,
        );
    };
    match target {
        DropTarget::Above(_) => line(rect.top()),
        DropTarget::Below(_) => line(rect.bottom()),
        DropTarget::Into(_) => {
            painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
        }
    }
}
