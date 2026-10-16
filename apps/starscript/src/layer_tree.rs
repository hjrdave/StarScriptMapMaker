//! The layer tree: the Map's tree shown top of the Stacking order first, with selection,
//! rename, hide, lock, collapse and drag-and-drop. Every change goes through a Command.
//! Rows are virtualised, so a Layer holding 100k Objects only lays out what is on screen.

use std::collections::HashSet;

use eframe::egui;
use ssmap_core::{Command, Id, Map, Node, Object, ObjectKind};

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

/// One visible row: a node and how deep it sits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Row {
    pub id: Id,
    pub depth: usize,
}

/// Where a dragged row would land, relative to the row under the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropTarget {
    /// Just above the row in the list, which is just above it in the Stacking order.
    Above(Id),
    Below(Id),
    /// On top of everything inside a Layer.
    Into(Id),
}

/// The visible rows, top of the Stacking order first, skipping collapsed Layers' contents.
pub fn rows(map: &Map, collapsed: &HashSet<Id>) -> Vec<Row> {
    fn walk(
        map: &Map,
        parent: Option<Id>,
        depth: usize,
        collapsed: &HashSet<Id>,
        out: &mut Vec<Row>,
    ) {
        let children = match parent {
            Some(layer) => map.ordered_children(layer),
            None => map.children(None).to_vec(),
        };
        for &id in children.iter().rev() {
            out.push(Row { id, depth });
            if map.node(id).is_some_and(Node::is_layer) && !collapsed.contains(&id) {
                walk(map, Some(id), depth + 1, collapsed, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(map, None, 0, collapsed, &mut out);
    out
}

/// The Move Command for dropping `dragged` on `target`, or `None` if it would do nothing
/// or move a Layer into itself.
pub fn move_command(map: &Map, dragged: Id, target: DropTarget) -> Option<Command> {
    let position = |id: Id| {
        map.children(map.parent(id))
            .iter()
            .position(|&sibling| sibling == id)
    };
    let (parent, mut index) = match target {
        DropTarget::Above(row) => (map.parent(row), position(row)? + 1),
        DropTarget::Below(row) => (map.parent(row), position(row)?),
        DropTarget::Into(layer) => (Some(layer), map.children(Some(layer)).len()),
    };
    if let Some(parent) = parent
        && (map.is_ancestor_or_self(dragged, parent) || !map.accepts_children(parent))
    {
        // Into itself, or onto a locked Group (itself or through one above it).
        return None;
    }
    // Move counts the index after the dragged node leaves its old place.
    let old_parent = map.parent(dragged);
    let old_index = position(dragged)?;
    if old_parent == parent && is_depth_sorted_stamp(map, dragged) {
        return None;
    }
    if old_parent == parent && old_index < index {
        index -= 1;
    }
    if old_parent == parent && old_index == index {
        return None;
    }
    Some(Command::Move {
        id: dragged,
        parent,
        index,
    })
}

/// A Stamp inside a Layer with Depth sort on, whose place among its siblings is not its to pick.
pub fn is_depth_sorted_stamp(map: &Map, id: Id) -> bool {
    matches!(
        map.node(id),
        Some(Node::Object(Object {
            kind: ObjectKind::Stamp(_),
            ..
        }))
    ) && matches!(
        map.parent(id).and_then(|p| map.node(p)),
        Some(Node::Layer(layer)) if layer.depth_sort
    )
}

/// Every Landmass, topmost first as the tree lists them.
pub fn landmasses(map: &Map) -> Vec<Id> {
    rows(map, &HashSet::new())
        .into_iter()
        .map(|row| row.id)
        .filter(|&id| is_landmass(map, id))
        .collect()
}

fn is_landmass(map: &Map, id: Id) -> bool {
    matches!(
        map.node(id),
        Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Landmass(_))
    )
}

/// The Landmass a Layer should be clipped to: the one Object `selection` holds, if it is a
/// Landmass.
pub fn selected_landmass(map: &Map, selection: &[Id]) -> Option<Id> {
    match selection {
        &[id] if is_landmass(map, id) => Some(id),
        _ => None,
    }
}

/// A Landmass as a Clip target is named by its place in [`landmasses`], as Landmasses have no
/// names of their own.
pub fn landmass_name(map: &Map, id: Id) -> String {
    match landmasses(map).iter().position(|&l| l == id) {
        Some(index) => format!("Landmass {}", index + 1),
        None => "Missing Landmass".to_owned(),
    }
}

pub fn node_name(map: &Map, id: Id) -> String {
    match map.node(id) {
        Some(Node::Layer(layer)) if layer.name.trim().is_empty() => "Layer".to_owned(),
        Some(Node::Layer(layer)) => layer.name.clone(),
        Some(Node::Object(Object {
            name: Some(name), ..
        })) => name.clone(),
        Some(Node::Object(object)) => match &object.kind {
            ObjectKind::Landmass(_) => "Landmass".to_owned(),
            ObjectKind::Lake(_) => "Lake".to_owned(),
            ObjectKind::Stamp(_) => "Stamp".to_owned(),
            // A Label goes by its first line.
            ObjectKind::Label(label) => label.text.lines().next().unwrap_or("Label").to_owned(),
        },
        None => String::new(),
    }
}

/// How a row looks, apart from selection and hover.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RowState {
    /// A hidden Layer: slashed eye, name italic and dim.
    pub hidden: bool,
    /// Locked itself: filled padlock.
    pub locked: bool,
    /// Locked only through a Layer above: dashed padlock, dim name, not toggleable here.
    pub locked_by_parent: bool,
}

impl RowState {
    pub fn of(map: &Map, id: Id) -> Self {
        let Some(node) = map.node(id) else {
            return Self {
                hidden: false,
                locked: false,
                locked_by_parent: false,
            };
        };
        let locked = node.locked();
        Self {
            hidden: matches!(node, Node::Layer(layer) if !layer.visible),
            locked,
            locked_by_parent: !locked && map.is_locked(id),
        }
    }

    /// Hidden names go italic, so the state is never colour alone.
    pub fn italic(self) -> bool {
        self.hidden
    }

    fn dim(self) -> bool {
        self.hidden || self.locked_by_parent
    }
}

/// How many Objects and Layers the Map holds.
pub fn counts(map: &Map) -> (usize, usize) {
    let layers = map
        .ids()
        .filter(|&id| map.node(id).is_some_and(Node::is_layer))
        .count();
    (map.ids().count() - layers, layers)
}

/// "3 Objects · 2 Layers".
pub fn footer_text(objects: usize, layers: usize) -> String {
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "{} · {}",
        plural(objects, "Object", "Objects"),
        plural(layers, "Layer", "Layers")
    )
}

fn kind_icon(node: &Node) -> Icon {
    match node {
        Node::Layer(_) => Icon::Layer,
        Node::Object(object) => match object.kind {
            ObjectKind::Landmass(_) => Icon::Landmass,
            ObjectKind::Lake(_) => Icon::Lake,
            ObjectKind::Stamp(_) => Icon::Stamp,
            ObjectKind::Label(_) => Icon::LabelKind,
        },
    }
}

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
        let revision = editor.map.revision();
        let rows = match self.cache.take() {
            Some((r, g, rows)) if r == revision && g == self.collapse_generation => rows,
            _ => rows(&editor.map, &self.collapsed),
        };
        let (objects, layers) = match self.counts {
            Some((r, counts)) if r == revision => counts,
            _ => {
                let counts = counts(&editor.map);
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
                            let from = self.anchor.filter(|&a| editor.map.contains(a));
                            self.anchor.get_or_insert(id);
                            range(&rows, from.unwrap_or(id), id)
                        }
                    };
                    editor.select(selection);
                }
                Action::Paste => self.paste_requested = true,
                Action::Run(command) => {
                    editor.run(command);
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
                    match filled_group(&editor.map, editor.selection()) {
                        Some(group) => {
                            let inside = contents(&editor.map, editor.selection());
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
                    self.renaming = Some((id, node_name(&editor.map, id)));
                }
                Action::FinishRename => {
                    if let Some((id, name)) = self.renaming.take()
                        && name != node_name(&editor.map, id)
                    {
                        let command = match editor.map.node(id) {
                            Some(Node::Object(_)) => Command::RenameObject { id, name },
                            _ => Command::RenameLayer { id, name },
                        };
                        editor.run(command);
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
                ui.label(delete_message(&node_name(&editor.map, group), inside));
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
                if editor.map.contains(group) {
                    editor.delete_selection();
                }
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }

    fn row(&mut self, ui: &mut egui::Ui, editor: &Editor, row: Row, actions: &mut Vec<Action>) {
        let map = &editor.map;
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

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Canvas, DistanceUnit, Layer, Op, Scale, Subtree};

    /// Top level, bottom first: A, B, C. B holds b1 (bottom) and b2.
    fn map() -> (Map, [Id; 5]) {
        let mut map = Map::new(Canvas::new(10, 10, Scale::new(1.0, DistanceUnit::Miles)));
        let ids = [Id::new(), Id::new(), Id::new(), Id::new(), Id::new()];
        let [a, b, c, b1, b2] = ids;
        let layer = |id, children| Subtree {
            id,
            node: Node::Layer(Layer::new("")),
            children,
        };
        for (index, subtree) in [
            layer(a, vec![]),
            layer(b, vec![layer(b1, vec![]), layer(b2, vec![])]),
            layer(c, vec![]),
        ]
        .into_iter()
        .enumerate()
        {
            map.apply(Op::Insert {
                parent: None,
                index,
                subtree,
            })
            .unwrap();
        }
        (map, ids)
    }

    #[test]
    fn rows_list_the_top_of_the_stack_first() {
        let (map, [a, b, c, b1, b2]) = map();
        let ids: Vec<(Id, usize)> = rows(&map, &HashSet::new())
            .into_iter()
            .map(|r| (r.id, r.depth))
            .collect();
        assert_eq!(ids, vec![(c, 0), (b, 0), (b2, 1), (b1, 1), (a, 0)]);
        let collapsed = HashSet::from([b]);
        assert_eq!(rows(&map, &collapsed).len(), 3);
    }

    fn drop_on(map: &mut Map, dragged: Id, target: DropTarget) {
        let command = move_command(map, dragged, target).expect("a move");
        map.execute(&command).unwrap();
    }

    #[test]
    fn dropping_above_a_row_puts_it_higher_in_the_stack() {
        let (mut map, [a, b, c, ..]) = map();
        // Drag A (bottom) above C (top).
        drop_on(&mut map, a, DropTarget::Above(c));
        assert_eq!(map.children(None), &[b, c, a]);
        // Drag A back below B.
        drop_on(&mut map, a, DropTarget::Below(b));
        assert_eq!(map.children(None), &[a, b, c]);
    }

    #[test]
    fn dropping_into_a_layer_puts_it_on_top_inside() {
        let (mut map, [a, b, _, b1, b2]) = map();
        drop_on(&mut map, a, DropTarget::Into(b));
        assert_eq!(map.children(Some(b)), &[b1, b2, a]);
        // And out again, beside a nested row.
        drop_on(&mut map, b2, DropTarget::Below(b));
        assert_eq!(map.parent(b2), None);
    }

    #[test]
    fn no_op_and_self_nesting_drops_do_nothing() {
        let (map, [a, b, c, b1, _]) = map();
        assert_eq!(move_command(&map, b, DropTarget::Above(a)), None);
        assert_eq!(move_command(&map, b, DropTarget::Below(c)), None);
        assert_eq!(move_command(&map, b, DropTarget::Into(b)), None);
        assert_eq!(move_command(&map, b, DropTarget::Above(b1)), None);
    }

    #[test]
    fn a_depth_sorted_layer_lists_its_stamps_in_draw_order_and_refuses_reordering() {
        use ssmap_core::{Object, Stamp};
        let (mut map, [a, ..]) = map();
        let stamps = [Id::new(), Id::new()];
        for (index, (id, y)) in stamps.into_iter().zip([30.0, 10.0]).enumerate() {
            let stamp = Stamp::new("p/a".parse().unwrap(), [0.0, y]);
            map.apply(Op::Insert {
                parent: Some(a),
                index,
                subtree: Subtree {
                    id,
                    node: Node::Object(Object::new(ObjectKind::Stamp(stamp))),
                    children: vec![],
                },
            })
            .unwrap();
        }
        let listed = |map: &Map| -> Vec<Id> {
            rows(map, &HashSet::new())
                .into_iter()
                .map(|r| r.id)
                .filter(|id| stamps.contains(id))
                .collect()
        };
        // Top of the list is the top of the draw order: the last tree child.
        assert_eq!(listed(&map), vec![stamps[1], stamps[0]]);
        map.execute(&Command::SetDepthSort {
            id: a,
            depth_sort: true,
        })
        .unwrap();
        assert_eq!(listed(&map), vec![stamps[0], stamps[1]]);
        assert!(is_depth_sorted_stamp(&map, stamps[0]));
        assert_eq!(
            move_command(&map, stamps[0], DropTarget::Above(stamps[1])),
            None
        );
    }

    #[test]
    fn clips_are_set_and_cleared_from_the_selection_and_deleting_the_target_undoes() {
        use crate::editor::Editor;
        use ssmap_core::{MultiPolygon, Polygon};
        let mut editor = Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let layer = editor.new_layer().unwrap();
        let land = Id::new();
        assert!(editor.run(Command::CreateLandmass {
            id: land,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![Polygon {
                exterior: vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [0.0, 50.0]],
                holes: vec![],
            }]),
            fill: None,
        }));
        let art = editor.new_layer().unwrap();
        assert_eq!(selected_landmass(&editor.map, &[layer]), None);
        assert_eq!(selected_landmass(&editor.map, &[land, art]), None);
        let target = selected_landmass(&editor.map, &[land]);
        assert_eq!(target, Some(land));
        assert_eq!(landmasses(&editor.map), vec![land]);
        assert_eq!(landmass_name(&editor.map, land), "Landmass 1");

        let clip = |editor: &Editor| match editor.map.node(art) {
            Some(Node::Layer(layer)) => layer.clip,
            _ => panic!("a Layer"),
        };
        assert!(editor.run(Command::SetClip { id: art, target }));
        assert_eq!(clip(&editor), Some(land));
        assert!(editor.run(Command::SetClip {
            id: art,
            target: None
        }));
        assert_eq!(clip(&editor), None);
        editor.undo();
        assert_eq!(clip(&editor), Some(land));

        editor.select(vec![land]);
        editor.delete_selection();
        assert_eq!(clip(&editor), None, "deleting the target clears the Clip");
        editor.undo();
        assert_eq!(clip(&editor), Some(land), "undo restores both");
        assert!(editor.map.node(land).is_some());
    }

    #[test]
    fn every_tree_action_undoes_and_redoes_cleanly() {
        use crate::editor::Editor;
        let mut editor = Editor::new(Map::new(Canvas::new(
            10,
            10,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let empty = editor.map.view();
        let a = editor.new_layer().unwrap();
        let b = editor.new_layer().unwrap();
        let c = editor.new_layer().unwrap();
        let drag = |editor: &mut Editor, id, target| {
            let command = move_command(&editor.map, id, target).expect("a move");
            assert!(editor.run(command));
        };
        assert!(editor.run(Command::RenameLayer {
            id: a,
            name: "Coast".into()
        }));
        drag(&mut editor, c, DropTarget::Into(a)); // nest
        drag(&mut editor, b, DropTarget::Below(a)); // reorder
        assert!(editor.run(Command::SetVisible {
            id: b,
            visible: false
        }));
        assert!(editor.run(Command::SetLocked {
            id: c,
            locked: true
        }));
        assert!(editor.run(Command::SetLocked {
            id: c,
            locked: false
        }));
        editor.select(vec![b]);
        editor.delete_selection();
        let done = editor.map.view();

        while editor.can_undo() {
            editor.undo();
            editor.map.validate().unwrap();
        }
        assert_eq!(editor.map.view(), empty);
        while editor.can_redo() {
            editor.redo();
        }
        assert_eq!(editor.map.view(), done);
    }

    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    struct Fixture {
        editor: Editor,
        tree: LayerTree,
    }

    fn harness(editor: Editor) -> Harness<'static, Fixture> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(260.0, 400.0))
            .build_ui_state(
                |ui, fixture: &mut Fixture| {
                    theme::install(&ui.ctx().clone());
                    fixture.tree.show(ui, &mut fixture.editor, true);
                },
                Fixture {
                    editor,
                    tree: LayerTree::default(),
                },
            );
        harness.run();
        harness
    }

    fn editor_with_two_layers() -> (Editor, Id, Id) {
        let mut editor = Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let a = editor.new_layer().unwrap();
        let b = editor.new_layer().unwrap();
        (editor, a, b)
    }

    #[test]
    fn a_hidden_row_is_italic_with_the_slashed_eye() {
        let (mut editor, a, b) = editor_with_two_layers();
        assert!(editor.run(Command::SetVisible {
            id: a,
            visible: false
        }));
        assert!(RowState::of(&editor.map, a).italic());
        assert!(!RowState::of(&editor.map, b).italic());
        let harness = harness(editor);
        // Only the hidden row shows its eye (as "Show") without hover.
        harness.get_by_label("Show");
        assert!(harness.query_by_label("Hide").is_none());
    }

    #[test]
    fn a_child_of_a_locked_layer_shows_a_dashed_padlock_that_does_nothing() {
        let (mut editor, a, b) = editor_with_two_layers();
        let command = move_command(&editor.map, b, DropTarget::Into(a)).unwrap();
        assert!(editor.run(command));
        assert!(editor.run(Command::SetLocked {
            id: a,
            locked: true
        }));
        let state = RowState::of(&editor.map, b);
        assert!(state.locked_by_parent && !state.locked);
        assert!(!RowState::of(&editor.map, a).locked_by_parent);

        let revision = editor.map.revision();
        let selection = editor.selection().to_vec();
        let mut harness = harness(editor);
        harness.get_by_label("Unlock"); // the parent's own filled padlock
        harness.get_by_label("Locked by parent Layer").click();
        harness.run();
        let editor = &harness.state().editor;
        assert_eq!(editor.map.revision(), revision);
        assert!(!editor.map.node(b).unwrap().locked());
        assert_eq!(editor.selection(), selection);
    }

    #[test]
    fn the_active_layer_shows_the_target() {
        let (editor, a, _) = editor_with_two_layers();
        let mut harness = harness(editor);
        harness.state_mut().editor.select(vec![a]);
        harness.run();
        // One target, on the one active Layer.
        assert_eq!(
            harness
                .query_all_by_label("Active Layer (new Objects go here)")
                .count(),
            1
        );
    }

    #[test]
    fn indicators_show_clip_depth_sort_and_opacity() {
        let (mut editor, a, _) = editor_with_two_layers();
        assert!(editor.run(Command::SetDepthSort {
            id: a,
            depth_sort: true
        }));
        let harness = harness(editor);
        harness.get_by_label("Depth-sorted");
        assert!(harness.query_by_label("Clipped to Landmass").is_none());
        assert!(harness.query_by_label_contains("%").is_none());
    }

    #[test]
    fn linked_rows_show_the_link_icon_and_link_and_unlink_are_undoable() {
        let (mut editor, a, b) = editor_with_two_layers();
        editor.select(vec![a]);
        assert!(!editor.can_link(), "one row has nothing to link to");
        editor.select(vec![a, b]);
        assert!(editor.can_link() && !editor.can_unlink());
        let plain = editor.map.view();
        assert!(editor.link_selection());
        assert!(editor.map.is_linked(a) && editor.map.is_linked(b));
        assert!(editor.can_unlink());

        let mut harness = harness(editor);
        assert_eq!(harness.query_all_by_label("Linked").count(), 2);

        let editor = &mut harness.state_mut().editor;
        assert!(editor.unlink_selection());
        harness.run();
        assert!(harness.query_by_label("Linked").is_none());

        let editor = &mut harness.state_mut().editor;
        editor.undo();
        harness.run();
        assert_eq!(harness.query_all_by_label("Linked").count(), 2);
        let editor = &mut harness.state_mut().editor;
        editor.undo();
        assert_eq!(editor.map.view(), plain);
        editor.redo();
        assert!(editor.map.is_linked(a));
    }

    #[test]
    fn a_stamp_can_be_renamed_and_an_empty_name_restores_the_default() {
        use ssmap_core::Stamp;
        let (mut editor, a, _) = editor_with_two_layers();
        let id = Id::new();
        let stamp = Stamp::new("p/a".parse().unwrap(), [0.0, 0.0]);
        editor
            .map
            .apply(Op::Insert {
                parent: Some(a),
                index: 0,
                subtree: Subtree {
                    id,
                    node: Node::Object(Object::new(ObjectKind::Stamp(stamp))),
                    children: vec![],
                },
            })
            .unwrap();
        assert_eq!(node_name(&editor.map, id), "Stamp");
        assert!(editor.run(Command::RenameObject {
            id,
            name: "  Old Keep ".into()
        }));
        assert_eq!(node_name(&editor.map, id), "Old Keep");
        assert!(editor.run(Command::RenameObject {
            id,
            name: String::new()
        }));
        assert_eq!(node_name(&editor.map, id), "Stamp");
        editor.undo();
        assert_eq!(node_name(&editor.map, id), "Old Keep");
        // Layers are renamed with their own Command.
        assert!(!editor.run(Command::RenameObject {
            id: a,
            name: "x".into()
        }));
    }

    #[test]
    fn the_footer_counts_objects_and_layers() {
        use ssmap_core::{MultiPolygon, Polygon};
        let (mut editor, a, _) = editor_with_two_layers();
        assert_eq!(footer_text(0, 2), "0 Objects · 2 Layers");
        assert_eq!(footer_text(1, 1), "1 Object · 1 Layer");
        assert!(editor.run(Command::CreateLandmass {
            id: Id::new(),
            parent: a,
            index: None,
            geometry: MultiPolygon(vec![Polygon {
                exterior: vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [0.0, 50.0]],
                holes: vec![],
            }]),
            fill: None,
        }));
        assert_eq!(counts(&editor.map), (1, 2));
        let mut harness = harness(editor);
        harness.get_by_label("1 Object · 2 Layers");
        // The count follows the Map.
        harness.state_mut().editor.new_layer();
        harness.run();
        harness.get_by_label("1 Object · 3 Layers");
    }

    #[test]
    fn grouping_is_one_undo_step_and_ungrouping_restores_the_layers() {
        let (mut editor, a, b) = editor_with_two_layers();
        let before = editor.map.view();
        editor.select(vec![a, b]);
        let group = editor.group_selection().expect("a Group");
        assert_eq!(editor.selection(), [group]);
        assert_eq!(editor.map.parent(a), Some(group));
        assert_eq!(editor.map.parent(b), Some(group));
        let grouped = editor.map.view();
        assert!(editor.ungroup_selection());
        assert_eq!(editor.map.parent(a), None);
        assert_eq!(editor.selection().len(), 2);
        editor.undo();
        assert_eq!(editor.map.view(), grouped);
        editor.undo();
        assert_eq!(editor.map.view(), before);
    }

    #[test]
    fn an_empty_group_can_be_created_and_collapses_with_its_contents() {
        let (mut editor, a, _) = editor_with_two_layers();
        let group = editor.new_group().expect("a Group");
        assert!(editor.map.children(Some(group)).is_empty());
        let command = move_command(&editor.map, a, DropTarget::Into(group)).unwrap();
        assert!(editor.run(command));
        let mut collapsed = HashSet::new();
        assert!(rows(&editor.map, &collapsed).iter().any(|r| r.id == a));
        collapsed.insert(group);
        let visible = rows(&editor.map, &collapsed);
        assert!(!visible.iter().any(|r| r.id == a));
        assert!(visible.iter().any(|r| r.id == group));
        let open = rows(&editor.map, &HashSet::new());
        let depth = |id| open.iter().find(|r| r.id == id).unwrap().depth;
        assert_eq!(depth(a), depth(group) + 1);
    }

    #[test]
    fn layers_drag_into_and_out_of_a_group() {
        let (mut editor, a, b) = editor_with_two_layers();
        let group = editor.new_group().unwrap();
        let into = move_command(&editor.map, a, DropTarget::Into(group)).unwrap();
        assert!(editor.run(into));
        assert_eq!(editor.map.parent(a), Some(group));
        let out = move_command(&editor.map, a, DropTarget::Above(b)).unwrap();
        assert!(editor.run(out));
        assert_eq!(editor.map.parent(a), None);
        editor.undo();
        assert_eq!(editor.map.parent(a), Some(group));
    }

    #[test]
    fn a_locked_group_locks_its_contents_and_refuses_drops() {
        let (mut editor, a, b) = editor_with_two_layers();
        let group = editor.new_group().unwrap();
        let into = move_command(&editor.map, a, DropTarget::Into(group)).unwrap();
        assert!(editor.run(into));
        assert!(editor.run(Command::SetLocked {
            id: group,
            locked: true
        }));
        assert!(RowState::of(&editor.map, a).locked_by_parent);
        assert!(move_command(&editor.map, b, DropTarget::Into(group)).is_none());
        assert!(move_command(&editor.map, b, DropTarget::Above(a)).is_none());
    }

    #[test]
    fn deleting_a_group_asks_first_naming_it_and_its_contents() {
        let (mut editor, a, _) = editor_with_two_layers();
        let group = editor.new_group().unwrap();
        let into = move_command(&editor.map, a, DropTarget::Into(group)).unwrap();
        assert!(editor.run(into));
        assert!(editor.run(Command::RenameLayer {
            id: group,
            name: "Coast".into()
        }));
        assert_eq!(filled_group(&editor.map, &[group]), Some(group));
        assert_eq!(filled_group(&editor.map, &[a]), None);
        assert_eq!(contents(&editor.map, &[group]), 1);
        assert_eq!(
            delete_message("Coast", 1),
            "Delete the Group \"Coast\"? Its 1 item will be deleted too."
        );
        editor.select(vec![group]);
        let mut harness = harness(editor);
        harness.state_mut().tree.confirm_delete = Some((group, 1));
        harness.run();
        harness.get_by_label_contains("\"Coast\"");
        harness.get_by_label("Cancel").click();
        harness.run();
        assert!(harness.state().editor.map.contains(group));
        harness.state_mut().tree.confirm_delete = Some((group, 1));
        harness.run();
        harness.get_by_label("Delete").click();
        harness.run();
        let editor = &harness.state().editor;
        assert!(!editor.map.contains(group) && !editor.map.contains(a));
    }

    /// Three Layers, the lowest inside a Group: the Group, its Layer, then the two others.
    fn editor_with_a_group() -> (Editor, Vec<Id>) {
        let (mut editor, a, _) = editor_with_two_layers();
        editor.new_layer().unwrap();
        let group = editor.new_group().unwrap();
        let command = move_command(&editor.map, a, DropTarget::Into(group)).unwrap();
        assert!(editor.run(command));
        editor.select(Vec::new());
        let ids = rows(&editor.map, &HashSet::new())
            .iter()
            .map(|row| row.id)
            .collect();
        (editor, ids)
    }

    fn click_row(harness: &mut Harness<'static, Fixture>, id: Id, modifiers: egui::Modifiers) {
        let name = node_name(&harness.state().editor.map, id);
        harness.get_by_label(&name).click_modifiers(modifiers);
        harness.run();
    }

    fn sorted(mut ids: Vec<Id>) -> Vec<Id> {
        ids.sort();
        ids
    }

    #[test]
    fn command_click_toggles_a_row_in_and_out_of_the_selection() {
        let (editor, ids) = editor_with_a_group();
        let mut harness = harness(editor);
        click_row(&mut harness, ids[0], egui::Modifiers::NONE);
        click_row(&mut harness, ids[2], egui::Modifiers::COMMAND);
        assert_eq!(
            sorted(harness.state().editor.selection().to_vec()),
            sorted(vec![ids[0], ids[2]])
        );
        click_row(&mut harness, ids[0], egui::Modifiers::COMMAND);
        assert_eq!(harness.state().editor.selection(), &[ids[2]]);
    }

    #[test]
    fn shift_click_selects_the_rows_between_across_groups() {
        let (editor, ids) = editor_with_a_group();
        assert_eq!(ids.len(), 4, "the Group, its Layer and two more");
        let mut harness = harness(editor);
        // From the Group at the top down past its own Layer, then back up to the middle.
        click_row(&mut harness, ids[0], egui::Modifiers::NONE);
        click_row(&mut harness, ids[3], egui::Modifiers::SHIFT);
        assert_eq!(
            sorted(harness.state().editor.selection().to_vec()),
            sorted(ids.clone())
        );
        click_row(&mut harness, ids[2], egui::Modifiers::SHIFT);
        assert_eq!(
            sorted(harness.state().editor.selection().to_vec()),
            sorted(ids[0..=2].to_vec()),
            "the range grows from the same anchor"
        );
    }

    #[test]
    fn a_panel_selection_is_the_canvas_selection() {
        // The panel writes the Editor's Selection, which the Canvas reads for its handles
        // and outline: one list, so the two cannot disagree.
        let (editor, ids) = editor_with_a_group();
        let mut harness = harness(editor);
        click_row(&mut harness, ids[1], egui::Modifiers::NONE);
        assert!(harness.state().editor.is_selected(ids[1]));
        // A selection made elsewhere (the Canvas) shows in the panel's menu: Link is enabled
        // only for a row that is part of a selection of two or more.
        harness.state_mut().editor.select(vec![ids[1], ids[2]]);
        harness.run();
        let name = node_name(&harness.state().editor.map, ids[1]);
        harness.get_by_label(&name).click_secondary();
        harness.run();
        assert!(!NodeT::accesskit_node(&harness.get_by_label("Link")).is_disabled());
    }

    fn open_menu(harness: &mut Harness<'static, Fixture>, id: Id) {
        let name = node_name(&harness.state().editor.map, id);
        harness.get_by_label(&name).click_secondary();
        harness.run();
    }

    #[test]
    fn the_menu_copies_and_pastes_the_selection() {
        let (editor, ids) = editor_with_a_group();
        let mut harness = harness(editor);
        click_row(&mut harness, ids[2], egui::Modifiers::NONE);
        click_row(&mut harness, ids[3], egui::Modifiers::COMMAND);
        assert!(!harness.state().editor.has_clipboard());
        open_menu(&mut harness, ids[3]);
        harness.get_by_label("Copy").click();
        harness.run();
        assert!(harness.state().editor.has_clipboard());
        assert!(
            harness.state().editor.map.contains(ids[2]),
            "Copy keeps them"
        );

        open_menu(&mut harness, ids[3]);
        harness.get_by_label("Paste").click();
        harness.run();
        assert!(harness.state().tree.paste_requested);
    }

    #[test]
    fn the_menu_cuts_groups_and_deletes_the_selection() {
        let (editor, ids) = editor_with_a_group();
        let mut harness = harness(editor);
        click_row(&mut harness, ids[2], egui::Modifiers::NONE);
        click_row(&mut harness, ids[3], egui::Modifiers::SHIFT);
        open_menu(&mut harness, ids[3]);
        harness.get_by_label("Group").click();
        harness.run();
        let editor = &harness.state().editor;
        let [group] = editor.selection() else {
            panic!("the new Group is selected")
        };
        assert_eq!(editor.map.children(Some(*group)).len(), 2);
        let group = *group;

        open_menu(&mut harness, group);
        harness.get_by_label("Cut").click();
        harness.run();
        assert!(harness.state().editor.has_clipboard());
        assert!(!harness.state().editor.map.contains(group));

        // The Layer inside the first Group is empty, so no confirmation is asked.
        let inner = ids[1];
        click_row(&mut harness, inner, egui::Modifiers::NONE);
        open_menu(&mut harness, inner);
        harness.get_by_label("Delete").click();
        harness.run();
        assert!(!harness.state().editor.map.contains(inner));
    }
}
