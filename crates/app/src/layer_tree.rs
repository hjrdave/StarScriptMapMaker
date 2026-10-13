//! The layer tree: the Map's tree shown top of the Stacking order first, with selection,
//! rename, hide, lock, collapse and drag-and-drop. Every change goes through a Command.
//! Rows are virtualised, so a Layer holding 100k Objects only lays out what is on screen.

use std::collections::HashSet;

use eframe::egui;
use ssmap_core::{Command, Id, Map, Node, ObjectKind};

use crate::editor::Editor;

const ROW_HEIGHT: f32 = 22.0;
const INDENT: f32 = 14.0;

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
        for &id in map.children(parent).iter().rev() {
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
        && map.is_ancestor_or_self(dragged, parent)
    {
        return None;
    }
    // Move counts the index after the dragged node leaves its old place.
    let old_parent = map.parent(dragged);
    let old_index = position(dragged)?;
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

pub fn node_name(map: &Map, id: Id) -> String {
    match map.node(id) {
        Some(Node::Layer(layer)) => layer.name.clone(),
        Some(Node::Object(object)) => match object.kind {
            ObjectKind::Landmass(_) => "Landmass",
            ObjectKind::Lake(_) => "Lake",
            ObjectKind::Stamp(_) => "Stamp",
            ObjectKind::Label(_) => "Label",
        }
        .to_owned(),
        None => String::new(),
    }
}

#[derive(Default)]
pub struct LayerTree {
    collapsed: HashSet<Id>,
    /// The row being renamed and its text so far.
    renaming: Option<(Id, String)>,
    /// Rows cached by Map revision and collapse state, so huge trees aren't walked every frame.
    cache: Option<(u64, u64, Vec<Row>)>,
    collapse_generation: u64,
}

enum Action {
    Select(Id, bool),
    Run(Command),
    Toggle(Id),
    StartRename(Id),
    FinishRename,
    CancelRename,
}

impl LayerTree {
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        ui.heading("Layers");
        ui.separator();
        let revision = editor.map.revision();
        let rows = match self.cache.take() {
            Some((r, g, rows)) if r == revision && g == self.collapse_generation => rows,
            _ => rows(&editor.map, &self.collapsed),
        };
        if rows.is_empty() {
            ui.weak("No Layers yet. Use New Layer to add one.");
        }

        let mut actions = Vec::new();
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            ROW_HEIGHT,
            rows.len(),
            |ui, range| {
                for row in &rows[range] {
                    self.row(ui, editor, *row, &mut actions);
                }
            },
        );
        self.cache = Some((revision, self.collapse_generation, rows));

        for action in actions {
            match action {
                Action::Select(id, add) => {
                    let mut selection = if add {
                        editor.selection().to_vec()
                    } else {
                        Vec::new()
                    };
                    if add && selection.contains(&id) {
                        selection.retain(|&s| s != id);
                    } else {
                        selection.push(id);
                    }
                    editor.select(selection);
                }
                Action::Run(command) => {
                    editor.run(command);
                }
                Action::Toggle(id) => {
                    if !self.collapsed.remove(&id) {
                        self.collapsed.insert(id);
                    }
                    self.collapse_generation += 1;
                }
                Action::StartRename(id) => {
                    self.renaming = Some((id, node_name(&editor.map, id)));
                }
                Action::FinishRename => {
                    if let Some((id, name)) = self.renaming.take()
                        && name != node_name(&editor.map, id)
                    {
                        editor.run(Command::RenameLayer { id, name });
                    }
                }
                Action::CancelRename => self.renaming = None,
            }
        }
    }

    fn row(&mut self, ui: &mut egui::Ui, editor: &Editor, row: Row, actions: &mut Vec<Action>) {
        let map = &editor.map;
        let Some(node) = map.node(row.id) else { return };
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click_and_drag(),
        );
        let visuals = ui.visuals();
        if editor.is_selected(row.id) {
            ui.painter()
                .rect_filled(rect, 2.0, visuals.selection.bg_fill);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, 2.0, visuals.widgets.hovered.weak_bg_fill);
        }

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.add_space(4.0 + INDENT * row.depth as f32);
        let layer = match node {
            Node::Layer(layer) => Some(layer),
            Node::Object(_) => None,
        };
        let has_children = !map.children(Some(row.id)).is_empty();
        if layer.is_some() && has_children {
            let collapsed = self.collapsed.contains(&row.id);
            let (arrow, name) = if collapsed {
                ("⏵", "Expand")
            } else {
                ("⏷", "Collapse")
            };
            if icon_button(&mut child, arrow, name).clicked() {
                actions.push(Action::Toggle(row.id));
            }
        } else {
            child.add_space(16.0);
        }
        if let Some(layer) = layer {
            let (eye, name) = if layer.visible {
                ("👁", "Hide")
            } else {
                ("◌", "Show")
            };
            if icon_button(&mut child, eye, name).clicked() {
                actions.push(Action::Run(Command::SetVisible {
                    id: row.id,
                    visible: !layer.visible,
                }));
            }
        }
        let locked = node.locked();
        let (lock, name) = if locked {
            ("🔒", "Unlock")
        } else {
            ("🔓", "Lock")
        };
        if icon_button(&mut child, lock, name).clicked() {
            actions.push(Action::Run(Command::SetLocked {
                id: row.id,
                locked: !locked,
            }));
        }

        match &mut self.renaming {
            Some((id, text)) if *id == row.id => {
                let edit = child.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
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
                let name = node_name(map, row.id);
                let mut text = egui::RichText::new(name);
                if layer.is_some_and(|l| !l.visible) {
                    text = text.weak();
                }
                child.add(egui::Label::new(text).selectable(false).truncate());
            }
        }

        if response.clicked() {
            let add = ui.input(|i| i.modifiers.command);
            actions.push(Action::Select(row.id, add));
        }
        if response.double_clicked() && layer.is_some() {
            actions.push(Action::StartRename(row.id));
        }
        if response.dragged() {
            response.dnd_set_drag_payload(row.id);
        }
        if response.dnd_hover_payload::<Id>().is_some()
            && let Some(target) = drop_target(ui, rect, row.id, layer.is_some())
        {
            paint_drop_indicator(ui, rect, target);
            if let Some(dropped) = response.dnd_release_payload::<Id>()
                && let Some(command) = move_command(map, *dropped, target)
            {
                actions.push(Action::Run(command));
            }
        }
    }
}

/// A frameless icon button whose tooltip and accessible name is `name`.
fn icon_button(ui: &mut egui::Ui, icon: &str, name: &str) -> egui::Response {
    let response = ui.add(egui::Button::new(icon).frame(false));
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), name));
    response.on_hover_text(name)
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

fn paint_drop_indicator(ui: &egui::Ui, rect: egui::Rect, target: DropTarget) {
    let stroke = egui::Stroke::new(2.0, ui.visuals().selection.stroke.color);
    let painter = ui.painter();
    match target {
        DropTarget::Above(_) => {
            painter.hline(rect.x_range(), rect.top(), stroke);
        }
        DropTarget::Below(_) => {
            painter.hline(rect.x_range(), rect.bottom(), stroke);
        }
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
        let mut map = Map::new(Canvas {
            width: 10,
            height: 10,
            scale: Scale {
                distance_per_unit: 1.0,
                unit: DistanceUnit::Miles,
            },
        });
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
    fn every_tree_action_undoes_and_redoes_cleanly() {
        use crate::editor::Editor;
        let mut editor = Editor::new(Map::new(Canvas {
            width: 10,
            height: 10,
            scale: Scale {
                distance_per_unit: 1.0,
                unit: DistanceUnit::Miles,
            },
        }));
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
}
