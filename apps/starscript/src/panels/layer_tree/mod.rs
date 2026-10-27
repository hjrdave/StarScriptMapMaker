//! The layer tree: the Map's tree shown top of the Stacking order first, with selection,
//! rename, hide, lock, collapse and drag-and-drop. Every change goes through a Command.
//! Rows are virtualised, so a Layer holding 100k Objects only lays out what is on screen.

use std::collections::HashSet;

use eframe::egui;
use ssmap_core::{Command, Id, Layer, Map, Node};

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
pub(crate) const DEPTH_SORT_HINT: &str = "Depth sort is on for this Layer: Stamps are listed in the order they draw, by Anchor height. Move a Stamp to re-sort it.";

mod model;
mod row;
use model::kind_icon;
pub(crate) use model::{
    DropTarget, Row, RowState, counts, footer_text, is_depth_sorted_stamp, landmass_name,
    landmasses, move_command, node_name, rows, selected_landmass,
};
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(crate) struct LayerTree {
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
}

/// How a click on a row changes the Selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pick {
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
pub(crate) fn delete_message(name: &str, inside: usize) -> String {
    let items = if inside == 1 { "item" } else { "items" };
    format!("Delete the Group \"{name}\"? Its {inside} {items} will be deleted too.")
}

impl LayerTree {
    /// `footer_shown` shows the Object and Layer counts in the bar under the list. The bar
    /// always holds the collapse control; returns true when it was clicked.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        footer_shown: bool,
    ) -> bool {
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
