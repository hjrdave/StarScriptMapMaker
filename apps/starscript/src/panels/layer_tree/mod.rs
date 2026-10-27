//! The layer tree: the Map's tree shown top of the Stacking order first, with selection,
//! rename, hide, lock, collapse and drag-and-drop. Every change goes through a Command.
//! Rows are virtualised, so a Layer holding 100k Objects only lays out what is on screen.

use std::collections::HashSet;

use eframe::egui;
use ssmap_core::{Command, Id, Map, Node};

use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::theme::{self, Palette, metrics};

const ROW_HEIGHT: f32 = metrics::LAYER_ROW;
const INDENT: f32 = 14.0;
/// Side of the chevron, eye and lock buttons in a row.
const ROW_BUTTON: f32 = 18.0;
/// Glyph size inside a row.
const ROW_GLYPH: f32 = 14.0;
pub(crate) const FOOTER_HEIGHT: f32 = 24.0;

/// Why a Depth-sorted Layer's Stamps can't be reordered by hand.
pub const DEPTH_SORT_HINT: &str = "Depth sort is on for this Layer: Stamps are listed in the order they draw, by Anchor height. Move a Stamp to re-sort it.";

mod model;
use model::kind_icon;
pub use model::{
    DropTarget, Row, RowState, counts, footer_text, is_depth_sorted_stamp, landmass_name,
    landmasses, move_command, node_name, rows, selected_landmass,
};
#[cfg(test)]
mod tests;

#[derive(Default)]
pub struct LayerTree {
    collapsed: HashSet<Id>,
    /// The row being renamed and its text so far.
    renaming: Option<(Id, String)>,
    /// Rows cached by Map revision and collapse state, so huge trees aren't walked every frame.
    cache: Option<(u64, u64, Vec<Row>)>,
    /// Object and Layer counts by Map revision, for the same reason.
    counts: Option<(u64, (usize, usize))>,
    collapse_generation: u64,
    /// A delete waiting on the confirmation: the Group it names and how many nodes are inside.
    confirm_delete: Option<(Id, usize)>,
    /// The row a Shift-click range grows from: the last row clicked without Shift.
    anchor: Option<Id>,
    /// Set by the menu's Paste; the shell pastes, since it knows where the Canvas is looking.
    pub paste_requested: bool,
    /// A row is being dragged this frame; the app takes it to learn which window a Layer is
    /// dragged out of.
    pub dragging: bool,
}

/// How a click on a row changes the Selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pick {
    /// Selects only this row.
    Only,
    /// ⌘-click: adds the row, or removes it if it was selected.
    Toggle,
    /// Shift-click: every visible row from the anchor to this one, across Groups.
    Range,
}

/// The ids of the visible `rows` from `from` to `to`, whichever comes first.
fn range(rows: &[Row], from: Id, to: Id) -> Vec<Id> {
    let position = |id| rows.iter().position(|row| row.id == id);
    match (position(from), position(to)) {
        (Some(a), Some(b)) => rows[a.min(b)..=a.max(b)].iter().map(|r| r.id).collect(),
        _ => vec![to],
    }
}

enum Action {
    Select(Id, Pick),
    /// Pastes what Copy or Cut took.
    Paste,
    Run(Command),
    Toggle(Id),
    StartRename(Id),
    /// Duplicates the row, or the whole selection if the row is part of it.
    Duplicate(Id),
    /// Copies the row, or the whole selection if the row is part of it.
    Copy(Id),
    /// Cuts the row, or the whole selection if the row is part of it.
    Cut(Id),
    /// Deletes the row, or the whole selection if the row is part of it.
    Delete(Id),
    /// Wraps the selection in a Group.
    Group,
    /// Dissolves the selected Groups.
    Ungroup,
    /// Links the selection so it moves and transforms together.
    Link,
    /// Takes the selection out of its Link sets.
    Unlink,
    FinishRename,
    CancelRename,
}

/// The first selected Layer that holds something, if any: deleting it takes its contents too.
fn filled_group(map: &Map, selection: &[Id]) -> Option<Id> {
    selection
        .iter()
        .copied()
        .find(|&id| map.node(id).is_some_and(Node::is_layer) && !map.children(Some(id)).is_empty())
}

/// How many nodes sit inside the selected Layers, at any depth.
fn contents(map: &Map, selection: &[Id]) -> usize {
    let mut pending: Vec<Id> = selection.to_vec();
    let mut count = 0;
    while let Some(id) = pending.pop() {
        let children = map.children(Some(id));
        count += children.len();
        pending.extend(children);
    }
    count
}

/// The confirmation text for deleting the Group `name` and the `inside` nodes it holds.
pub fn delete_message(name: &str, inside: usize) -> String {
    let items = if inside == 1 { "item" } else { "items" };
    format!("Delete the Group \"{name}\"? Its {inside} {items} will be deleted too.")
}

impl LayerTree {
    /// `footer_shown` shows the Object and Layer counts in the bar under the list. The bar
    /// always holds the collapse control; returns true when it was clicked.
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor, footer_shown: bool) -> bool {
        let revision = editor.map().revision();
        let rows = match self.cache.take() {
            Some((r, g, rows)) if r == revision && g == self.collapse_generation => rows,
            _ => rows(editor.map(), &self.collapsed),
        };
        let (objects, layers) = match self.counts {
            Some((r, counts)) if r == revision => counts,
            _ => {
                let counts = counts(editor.map());
                self.counts = Some((revision, counts));
                counts
            }
        };
        if rows.is_empty() {
            ui.weak("No Layers yet. Use New Layer to add one.");
        }

        let mut actions = Vec::new();
        ui.spacing_mut().item_spacing.y = 0.0;
        let list_height = (ui.available_height() - FOOTER_HEIGHT).max(ROW_HEIGHT);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .max_height(list_height)
            .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for row in &rows[range] {
                    self.row(ui, editor, *row, &mut actions);
                }
            });
        let text = footer_shown.then(|| footer_text(objects, layers));
        let collapse_clicked = footer(ui, text.as_deref());

        for action in actions {
            match action {
                Action::Select(id, pick) => {
                    let selection = match pick {
                        Pick::Only => {
                            self.anchor = Some(id);
                            vec![id]
                        }
                        Pick::Toggle => {
                            self.anchor = Some(id);
                            let mut selection = editor.selection().to_vec();
                            if selection.contains(&id) {
                                selection.retain(|&s| s != id);
                            } else {
                                selection.push(id);
                            }
                            selection
                        }
                        Pick::Range => {
                            let from = self.anchor.filter(|&a| editor.map().contains(a));
                            self.anchor.get_or_insert(id);
                            range(&rows, from.unwrap_or(id), id)
                        }
                    };
                    editor.select(selection);
                }
                Action::Paste => self.paste_requested = true,
                Action::Run(command) => {
                    let _ = editor.run(command);
                }
                Action::Toggle(id) => {
                    if !self.collapsed.remove(&id) {
                        self.collapsed.insert(id);
                    }
                    self.collapse_generation += 1;
                }
                Action::Duplicate(id) => {
                    if !editor.is_selected(id) {
                        editor.select(vec![id]);
                    }
                    editor.duplicate_selection();
                }
                Action::Copy(id) => {
                    if !editor.is_selected(id) {
                        editor.select(vec![id]);
                    }
                    editor.copy_selection();
                }
                Action::Cut(id) => {
                    if !editor.is_selected(id) {
                        editor.select(vec![id]);
                    }
                    editor.cut_selection();
                }
                Action::Delete(id) => {
                    if !editor.is_selected(id) {
                        editor.select(vec![id]);
                    }
                    match filled_group(editor.map(), editor.selection()) {
                        Some(group) => {
                            let inside = contents(editor.map(), editor.selection());
                            self.confirm_delete = Some((group, inside));
                        }
                        None => editor.delete_selection(),
                    }
                }
                Action::Group => {
                    editor.group_selection();
                }
                Action::Ungroup => {
                    editor.ungroup_selection();
                }
                Action::Link => {
                    editor.link_selection();
                }
                Action::Unlink => {
                    editor.unlink_selection();
                }
                Action::StartRename(id) => {
                    self.renaming = Some((id, node_name(editor.map(), id)));
                }
                Action::FinishRename => {
                    if let Some((id, name)) = self.renaming.take()
                        && name != node_name(editor.map(), id)
                    {
                        let command = match editor.map().node(id) {
                            Some(Node::Object(_)) => Command::RenameObject { id, name },
                            _ => Command::RenameLayer { id, name },
                        };
                        let _ = editor.run(command);
                    }
                }
                Action::CancelRename => self.renaming = None,
            }
        }
        self.cache = Some((revision, self.collapse_generation, rows));
        self.confirm_delete_dialog(ui.ctx(), editor);
        collapse_clicked
    }

    /// The confirmation before deleting a Group that holds something: it names the Group and
    /// says the contents go too.
    fn confirm_delete_dialog(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let Some((group, inside)) = self.confirm_delete else {
            return;
        };
        let mut choice = None;
        egui::Window::new("Delete Group")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(delete_message(&node_name(editor.map(), group), inside));
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            choice = Some(false);
        }
        match choice {
            Some(true) => {
                self.confirm_delete = None;
                if editor.map().contains(group) {
                    editor.delete_selection();
                }
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }

    fn row(&mut self, ui: &mut egui::Ui, editor: &Editor, row: Row, actions: &mut Vec<Action>) {
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

        let layer = match node {
            Node::Layer(layer) => Some(layer),
            Node::Object(_) => None,
        };

        // Trailing indicators are laid out right to left first, so the name knows how much
        // room is left.
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
        // No indicator where the drop would be refused, such as onto a locked Group.
        if let Some(dragged) = response.dnd_hover_payload::<Id>()
            && let Some(target) = drop_target(ui, rect, row.id, layer.is_some())
            && let Some(command) = move_command(map, *dragged, target)
        {
            paint_drop_indicator(ui, palette, rect, target);
            if response.dnd_release_payload::<Id>().is_some() {
                actions.push(Action::Run(command));
            }
        }
    }
}

/// A read-only glyph with a tooltip; `label` is its accessible name too.
fn indicator(ui: &mut egui::Ui, icon: Icon, tint: egui::Color32, label: &str) {
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(ROW_BUTTON), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
    icons::paint(ui, rect, icon, ROW_GLYPH, tint);
    response.on_hover_text(label);
}

/// The bar under the list: the Object and Layer counts (when `text` is given) and, at its
/// end, the collapse control. Returns true when that was clicked.
fn footer(ui: &mut egui::Ui, text: Option<&str>) -> bool {
    let palette = theme::palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), FOOTER_HEIGHT),
        egui::Sense::hover(),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        egui::Stroke::new(1.0, palette.border_subtle),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.add_space(theme::SPACING[3]);
    if let Some(text) = text {
        child.label(
            egui::RichText::new(text)
                .size(theme::CAPTION)
                .color(palette.text_tertiary),
        );
    }
    child
        .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(theme::SPACING[2]);
            crate::shell::collapse_button(ui, false).clicked()
        })
        .inner
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
