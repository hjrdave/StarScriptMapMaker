//! Editor state around one Map: history, Selection, the active Layer and tool, and the
//! message bar. Every change to the Map goes through a Command here.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ssmap_core::land::{self, BrushStroke};
use ssmap_core::{
    Command, CommandError, ExecuteError, History, Id, Label, Map, MultiPolygon, Node, NodeRef,
    ObjectKind, Op, PickIndex, Point, RoughnessCache, Selection,
};
use ssmap_render::{Content, Fonts, LabelLayout, MapScene};

use crate::layer_tree::node_name;
use crate::tools::ToolId;

/// The land brush's starting diameter, in Canvas units.
pub const DEFAULT_BRUSH_SIZE: f64 = 48.0;

/// How long a message stays in the status bar.
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);

pub struct Editor {
    pub map: Map,
    /// What the Canvas draws, derived from the Map, and the Label layouts that picking and
    /// handles share with it.
    pub scene: MapScene,
    history: History,
    selection: Selection,
    /// The Layer new Objects go into.
    active_layer: Option<Id>,
    pub active_tool: ToolId,
    /// The land brush's diameter, in Canvas units.
    pub brush_size: f64,
    node_edit: Option<NodeEdit>,
    /// Set by a tool while it holds something Escape should drop (a lasso half drawn).
    pub tool_busy: bool,
    /// Set by Escape for the busy tool to act on.
    pub cancel_tool: bool,
    notice: Option<Notice>,
    /// Set while a gesture's transaction is open.
    gesture_open: bool,
    /// The Selection when the open gesture began, restored if it is cancelled.
    gesture_selection: Selection,
    /// Rebuilt lazily once the Map has changed.
    pick_index: Option<PickIndex>,
    /// Roughened outlines, shared by the screen and picking so they agree.
    roughness: RoughnessCache,
}

/// Node-edit mode: one Landmass or Lake whose stored vertices show as nodes, and the nodes
/// selected among them.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeEdit {
    pub id: Id,
    pub nodes: Vec<NodeRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
    pub shown_at: Instant,
}

impl Editor {
    pub fn new(map: Map) -> Self {
        let mut history = History::default();
        history.set_recording(true);
        Self {
            map,
            scene: MapScene::new(Fonts::bundled()),
            history,
            selection: Vec::new(),
            active_layer: None,
            active_tool: ToolId::Select,
            brush_size: DEFAULT_BRUSH_SIZE,
            node_edit: None,
            tool_busy: false,
            cancel_tool: false,
            notice: None,
            gesture_open: false,
            gesture_selection: Vec::new(),
            pick_index: None,
            roughness: RoughnessCache::new(),
        }
    }

    pub fn selection(&self) -> &[Id] {
        &self.selection
    }

    pub fn active_layer(&self) -> Option<Id> {
        self.active_layer
    }

    /// Replaces the Selection. The active Layer follows it: a selected Layer, or the Layer
    /// holding a selected Object.
    pub fn select(&mut self, ids: Vec<Id>) {
        if self.node_edit.as_ref().is_some_and(|edit| ids != [edit.id]) {
            self.node_edit = None;
        }
        self.selection = ids;
        if let Some(&last) = self.selection.last() {
            self.active_layer = match self.map.node(last) {
                Some(Node::Layer(_)) => Some(last),
                _ => self.map.parent(last),
            };
        }
    }

    pub fn node_edit(&self) -> Option<&NodeEdit> {
        self.node_edit.as_ref()
    }

    /// The stored outline (not the roughened one) of the Object in node-edit mode.
    pub fn edited_outline(&self) -> Option<&MultiPolygon> {
        match self.map.node(self.node_edit.as_ref()?.id)? {
            Node::Object(object) => object.kind.geometry(),
            Node::Layer(_) => None,
        }
    }

    /// Enters node-edit mode on a Landmass or Lake, selecting it.
    pub fn enter_node_edit(&mut self, id: Id) {
        self.node_edit = None;
        self.select(vec![id]);
        self.node_edit = Some(NodeEdit {
            id,
            nodes: Vec::new(),
        });
    }

    pub fn leave_node_edit(&mut self) {
        self.node_edit = None;
    }

    /// Selects nodes of the Object being edited.
    pub fn select_nodes(&mut self, nodes: Vec<NodeRef>) {
        if let Some(edit) = &mut self.node_edit {
            edit.nodes = nodes;
        }
    }

    /// The shape of the outline being edited: how many points each ring has. Node numbers
    /// stay meaningful while it does not change.
    pub fn edited_shape(&self) -> Vec<usize> {
        self.edited_outline()
            .map(|outline| {
                outline
                    .0
                    .iter()
                    .flat_map(|polygon| polygon.rings().map(<[Point]>::len))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Forgets the node selection unless the outline still has `shape`: a repair may have
    /// renumbered its nodes.
    pub fn keep_nodes_if_shape(&mut self, shape: &[usize]) {
        if self.edited_shape() != shape {
            self.select_nodes(Vec::new());
        }
    }

    /// Adds a node on an edge and selects it. One Undo step.
    pub fn insert_node(&mut self, edge: NodeRef, point: Point) {
        let Some(id) = self.node_edit.as_ref().map(|edit| edit.id) else {
            return;
        };
        if self.run(Command::InsertNode { id, edge, point }) {
            let added = NodeRef {
                index: edge.index + 1,
                ..edge
            };
            let landed = self
                .edited_outline()
                .is_some_and(|outline| outline.node(added) == Some(point));
            self.select_nodes(if landed { vec![added] } else { Vec::new() });
        }
    }

    /// Deletes the selected nodes. One Undo step; refused if a ring would be left with fewer
    /// than three points.
    pub fn delete_selected_nodes(&mut self) {
        let Some(edit) = self.node_edit.clone() else {
            return;
        };
        if edit.nodes.is_empty() {
            return;
        }
        let command = Command::DeleteNodes {
            id: edit.id,
            nodes: edit.nodes,
        };
        if self.run(command) {
            self.select_nodes(Vec::new());
        }
    }

    /// The topmost unlocked Landmass or Lake drawn at `point`, in Canvas units.
    fn pick_outline(&mut self, point: [f64; 2]) -> Option<Id> {
        let map = &self.map;
        let index = match &mut self.pick_index {
            Some(index) if index.is_current(map) => index,
            slot => slot.insert(PickIndex::build(map, &mut self.roughness)),
        };
        index.pick(point)
    }

    /// What the Canvas draws for the Map, rebuilt only once it has changed. Drops roughened
    /// outlines of Objects that are gone.
    pub fn scene_content(&mut self) -> Content {
        self.roughness.retain(&self.map);
        self.scene.content(&self.map, &mut self.roughness)
    }

    /// The selected Landmasses and Lakes, which move and resize with the handles.
    pub fn selected_outlines(&self) -> Vec<Id> {
        self.selection
            .iter()
            .copied()
            .filter(|&id| {
                matches!(self.map.node(id), Some(Node::Object(o)) if o.kind.geometry().is_some())
            })
            .collect()
    }

    pub fn is_selected(&self, id: Id) -> bool {
        self.selection.contains(&id)
    }

    /// Runs one Command as its own Undo step. A refusal shows in the status bar.
    pub fn run(&mut self, command: Command) -> bool {
        let selection = self.selection.clone();
        self.run_selecting(command, selection)
    }

    /// Runs one Command as its own Undo step and selects `selection` afterwards.
    pub fn run_selecting(&mut self, command: Command, selection: Selection) -> bool {
        if self.gesture_open {
            return self.gesture(command);
        }
        let begun = self.history.begin(command.label(), self.selection.clone());
        debug_assert!(begun.is_ok(), "no transaction is open outside gestures");
        match self.history.execute(&mut self.map, &command) {
            Ok(()) => {
                self.history
                    .commit(selection.clone())
                    .expect("opened above");
                self.restore(selection);
                true
            }
            Err(err) => {
                self.history.rollback(&mut self.map).expect("opened above");
                self.refused(&err);
                false
            }
        }
    }

    /// Opens a gesture: Commands until `end_gesture` make one Undo step, applied live.
    pub fn begin_gesture(&mut self, label: &str) {
        if self.gesture_open {
            return;
        }
        self.history
            .begin(label, self.selection.clone())
            .expect("no other transaction is open");
        self.gesture_open = true;
        self.gesture_selection = self.selection.clone();
    }

    /// Runs a Command inside the open gesture, or as its own step if none is open.
    pub fn gesture(&mut self, command: Command) -> bool {
        if !self.gesture_open {
            return self.run(command);
        }
        match self.history.execute(&mut self.map, &command) {
            Ok(()) => true,
            Err(err) => {
                self.refused(&err);
                false
            }
        }
    }

    pub fn end_gesture(&mut self) {
        if std::mem::take(&mut self.gesture_open) {
            self.history
                .commit(self.selection.clone())
                .expect("the gesture's transaction is open");
        }
    }

    /// What a land brush stroke started now would be called in the undo history: it paints or
    /// erases a selected Lake, and otherwise land.
    pub fn brush_label(&self, erase: bool) -> &'static str {
        let lake = self.selection.iter().any(|&id| {
            matches!(self.map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Lake(_)))
        });
        match (erase, lake) {
            (false, false) => "Paint Land",
            (false, true) => "Paint Lake",
            (true, false) => "Erase Land",
            (true, true) => "Erase Lake",
        }
    }

    /// Finishes the land brush stroke whose gesture is open: runs the one Command the brush
    /// rules choose ([`land::brush_command`]) and commits it as the gesture's Undo step. A new
    /// Landmass becomes the Selection, so the next stroke merges into it. A stroke that changes
    /// nothing, or is refused, leaves no Undo step.
    pub fn finish_brush_stroke(&mut self, stroke: BrushStroke) {
        let new_id = Id::new();
        let planned = land::brush_command(
            &self.map,
            &mut self.roughness,
            &self.selection,
            self.active_layer,
            stroke,
            new_id,
        );
        match planned {
            Ok(Some(command)) => {
                let creates = matches!(command, Command::CreateLandmass { .. });
                if self.gesture(command) {
                    if creates {
                        self.select(vec![new_id]);
                    }
                    self.end_gesture();
                } else {
                    self.cancel_gesture();
                }
            }
            Ok(None) => self.cancel_gesture(),
            Err(err) => {
                self.cancel_gesture();
                let text = err.to_string();
                let mut chars = text.chars();
                let first = chars.next().map(|c| c.to_uppercase().to_string());
                self.error(first.unwrap_or_default() + chars.as_str());
            }
        }
    }

    /// The selected Landmasses, topmost first.
    fn selected_landmasses(&self) -> Vec<Id> {
        let order = self.map.draw_order();
        let mut ids: Vec<Id> = self
            .selection
            .iter()
            .copied()
            .filter(|&id| {
                matches!(self.map.node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::Landmass(_)))
            })
            .collect();
        ids.sort_by_key(|&id| std::cmp::Reverse(order.iter().position(|&o| o == id)));
        ids
    }

    /// Whether Union and Subtract have enough to work on: two or more selected Landmasses.
    pub fn can_combine(&self) -> bool {
        self.selected_landmasses().len() >= 2
    }

    /// Whether a selected Landmass has more than one part.
    pub fn can_split(&self) -> bool {
        self.selected_landmasses()
            .iter()
            .any(|&id| self.part_count(id) > 1)
    }

    fn part_count(&self, id: Id) -> usize {
        match self.map.node(id) {
            Some(Node::Object(o)) => o.kind.geometry().map_or(0, |g| g.0.len()),
            _ => 0,
        }
    }

    /// Merges the selected Landmasses into the topmost one, which stays selected.
    pub fn union_selection(&mut self) {
        let ids = self.selected_landmasses();
        if let Some(&keeper) = ids.first() {
            self.run_selecting(Command::Union { ids }, vec![keeper]);
        }
    }

    /// Cuts the topmost selected Landmass out of the others.
    pub fn subtract_selection(&mut self) {
        let ids = self.selected_landmasses();
        let rest: Vec<Id> = ids.iter().skip(1).copied().collect();
        if !ids.is_empty() {
            self.run_selecting(Command::Subtract { ids }, rest);
        }
    }

    /// Makes each part of every selected Landmass a Landmass of its own, as one Undo step.
    pub fn split_selection(&mut self) {
        let targets: Vec<Id> = self
            .selected_landmasses()
            .into_iter()
            .filter(|&id| self.part_count(id) > 1)
            .collect();
        if targets.is_empty() {
            return;
        }
        let mut selection = Vec::new();
        self.begin_gesture("Split Apart");
        for id in targets {
            let new_ids: Vec<Id> = (1..self.part_count(id)).map(|_| Id::new()).collect();
            selection.push(id);
            selection.extend(&new_ids);
            if !self.gesture(Command::SplitApart { id, new_ids }) {
                self.cancel_gesture();
                return;
            }
        }
        self.select(selection);
        self.end_gesture();
    }

    /// Escape: undoes everything the open gesture did, Selection included.
    pub fn cancel_gesture(&mut self) {
        if std::mem::take(&mut self.gesture_open) {
            self.history
                .rollback(&mut self.map)
                .expect("the gesture's transaction is open");
            let selection = std::mem::take(&mut self.gesture_selection);
            self.restore(selection);
        }
    }

    /// Replaces everything the open gesture has done so far with `command`. Drags use it to
    /// re-issue their whole effect each frame, so the Undo step holds one Command and nothing
    /// drifts. Without an open gesture it runs `command` as its own step.
    pub fn gesture_replace(&mut self, command: Command) -> bool {
        if !self.gesture_open {
            return self.run(command);
        }
        self.history
            .rewind(&mut self.map)
            .expect("the gesture's transaction is open");
        self.gesture(command)
    }

    pub fn gesture_open(&self) -> bool {
        self.gesture_open
    }

    pub fn can_undo(&self) -> bool {
        !self.gesture_open && self.history.undo_len() > 0
    }

    pub fn can_redo(&self) -> bool {
        !self.gesture_open && self.history.redo_len() > 0
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    pub fn undo(&mut self) {
        self.end_gesture();
        if let Ok(Some(selection)) = self.history.undo(&mut self.map) {
            self.restore(selection);
        }
    }

    pub fn redo(&mut self) {
        self.end_gesture();
        if let Ok(Some(selection)) = self.history.redo(&mut self.map) {
            self.restore(selection);
        }
    }

    /// Takes the Ops that committed steps, undo and redo applied since the last call, for the
    /// Recovery journal.
    pub fn take_applied(&mut self) -> Vec<Op> {
        self.history.take_applied()
    }

    /// Creates a Layer above the active one (or on top), selects it and returns its id.
    pub fn new_layer(&mut self) -> Option<Id> {
        let (parent, index) = match self.active_layer.filter(|&id| self.map.contains(id)) {
            Some(active) => {
                let parent = self.map.parent(active);
                let position = self
                    .map
                    .children(parent)
                    .iter()
                    .position(|&id| id == active)
                    .expect("a node is listed by its parent");
                (parent, Some(position + 1))
            }
            None => (None, None),
        };
        let id = Id::new();
        let name = format!("Layer {}", self.layer_count() + 1);
        let command = Command::CreateLayer {
            id,
            parent,
            index,
            name,
        };
        self.run_selecting(command, vec![id]).then_some(id)
    }

    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let ids = self.selection.clone();
        self.run_selecting(Command::Delete { ids }, Vec::new());
    }

    pub fn notice(&self) -> Option<&Notice> {
        self.notice
            .as_ref()
            .filter(|notice| notice.shown_at.elapsed() < NOTICE_DURATION)
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            is_error: false,
            shown_at: Instant::now(),
        });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            is_error: true,
            shown_at: Instant::now(),
        });
    }

    /// Shows why a Command was refused, naming Layers and Objects the way the tree does.
    fn refused(&mut self, err: &ExecuteError) {
        let name = |id: Id| format!("“{}”", node_name(&self.map, id));
        let text = match err {
            ExecuteError::Command(CommandError::Locked(id)) => format!("{} is locked", name(*id)),
            ExecuteError::Command(CommandError::InLockedLayer { target, layer }) => {
                format!("{} is inside locked Layer {}", name(*target), name(*layer))
            }
            ExecuteError::Command(CommandError::ContainsLocked { target, locked }) => {
                format!("{} contains locked {}", name(*target), name(*locked))
            }
            other => other.to_string(),
        };
        self.error(text);
    }

    /// The Label with id `id`, if it is one.
    pub fn label(&self, id: Id) -> Option<&Label> {
        match self.map.node(id) {
            Some(Node::Object(object)) => match &object.kind {
                ObjectKind::Label(label) => Some(label),
                _ => None,
            },
            _ => None,
        }
    }

    /// A Label and its shaped text.
    pub fn label_layout(&mut self, id: Id) -> Option<(Label, Arc<LabelLayout>)> {
        let label = self.label(id)?.clone();
        let layout = self.scene.label_layout(id, &label);
        Some((label, layout))
    }

    /// The topmost Object under a Canvas point that can be selected there: visible, and
    /// neither locked nor in a locked Layer.
    pub fn pick(&mut self, point: [f64; 2]) -> Option<Id> {
        // The index finds the topmost Landmass or Lake whose roughened fill is under the
        // point; Labels are weighed against it in draw order, so what shows on top is
        // what's picked.
        let outline = self.pick_outline(point);
        for id in self.map.draw_order().into_iter().rev() {
            if Some(id) == outline {
                return outline;
            }
            if !self.map.is_locked(id)
                && let Some((label, layout)) = self.label_layout(id)
                && layout.contains(&label, point)
            {
                return Some(id);
            }
        }
        None
    }

    /// Opens an "Add Label" gesture and places a Label at `position` in the active Layer,
    /// creating a Layer if the Map has none. The gesture stays open for typing the text.
    pub fn place_label(&mut self, position: [f64; 2]) -> Option<Id> {
        self.end_gesture();
        self.begin_gesture("Add Label");
        let parent = match self.active_layer.filter(|&id| self.map.contains(id)) {
            Some(layer) => layer,
            None => match self.map.children(None).last() {
                Some(&top) => top,
                None => {
                    let id = Id::new();
                    let name = format!("Layer {}", self.layer_count() + 1);
                    let command = Command::CreateLayer {
                        id,
                        parent: None,
                        index: None,
                        name,
                    };
                    if !self.gesture(command) {
                        self.cancel_gesture();
                        return None;
                    }
                    id
                }
            },
        };
        let id = Id::new();
        let label = Label {
            text: "Label".into(),
            position,
            ..Label::default()
        };
        if !self.gesture(Command::CreateLabel {
            id,
            parent,
            index: None,
            label: Box::new(label),
        }) {
            self.cancel_gesture();
            return None;
        }
        self.select(vec![id]);
        Some(id)
    }

    fn layer_count(&self) -> usize {
        self.map
            .ids()
            .filter(|&id| self.map.node(id).is_some_and(Node::is_layer))
            .count()
    }

    /// Restores a recorded Selection, dropping anything that no longer exists.
    fn restore(&mut self, mut selection: Selection) {
        // Undo and redo can renumber nodes.
        self.select_nodes(Vec::new());
        selection.retain(|&id| self.map.contains(id));
        self.select(selection);
        if self.active_layer.is_some_and(|id| !self.map.contains(id)) {
            self.active_layer = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Canvas, DistanceUnit, Scale};

    fn editor() -> Editor {
        Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )))
    }

    #[test]
    fn new_layers_go_above_the_active_one_and_get_selected() {
        let mut editor = editor();
        let first = editor.new_layer().unwrap();
        let second = editor.new_layer().unwrap();
        editor.select(vec![first]);
        let third = editor.new_layer().unwrap();
        assert_eq!(editor.map.children(None), &[first, third, second]);
        assert_eq!(editor.selection(), &[third]);
        assert_eq!(editor.active_layer(), Some(third));
    }

    #[test]
    fn undo_restores_the_selection_and_redo_reapplies() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        editor.undo();
        assert!(!editor.map.contains(layer));
        assert!(editor.selection().is_empty());
        assert_eq!(editor.active_layer(), None);
        editor.redo();
        assert_eq!(editor.selection(), &[layer]);
    }

    #[test]
    fn refusals_show_a_message_and_change_nothing() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        editor.run(Command::SetLocked {
            id: layer,
            locked: true,
        });
        let before = editor.map.view();
        assert!(!editor.run(Command::RenameLayer {
            id: layer,
            name: "Nope".into(),
        }));
        assert_eq!(editor.map.view(), before);
        let notice = editor.notice().expect("a visible message");
        assert!(notice.is_error);
        assert_eq!(notice.text, "“Layer 1” is locked");
    }

    #[test]
    fn a_gesture_is_one_undo_step_and_cancel_restores() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        editor.begin_gesture("Change Opacity");
        for opacity in [0.8, 0.5, 0.2] {
            editor.gesture(Command::SetOpacity { id: layer, opacity });
        }
        editor.end_gesture();
        assert_eq!(editor.undo_label(), Some("Change Opacity"));
        editor.undo();
        assert_eq!(opacity(&editor, layer), 1.0);

        editor.begin_gesture("Change Opacity");
        editor.gesture(Command::SetOpacity {
            id: layer,
            opacity: 0.3,
        });
        editor.cancel_gesture();
        assert_eq!(opacity(&editor, layer), 1.0);
        assert_eq!(editor.redo_label(), Some("Change Opacity"));
    }

    #[test]
    fn placing_a_label_creates_a_layer_and_is_one_undo_step() {
        let mut editor = editor();
        let id = editor.place_label([50.0, 50.0]).unwrap();
        let mut label = editor.label(id).unwrap().clone();
        label.text = "Thule".into();
        editor.gesture(Command::SetLabel {
            id,
            label: Box::new(label),
        });
        editor.end_gesture();
        assert_eq!(editor.selection(), &[id]);
        assert_eq!(editor.label(id).unwrap().text, "Thule");
        assert_eq!(editor.undo_label(), Some("Add Label"));
        editor.undo();
        assert!(editor.map.children(None).is_empty());
        assert!(editor.selection().is_empty());
    }

    #[test]
    fn cancelling_a_placement_restores_the_selection() {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        editor.place_label([50.0, 50.0]).unwrap();
        editor.cancel_gesture();
        assert_eq!(editor.map.children(Some(layer)).len(), 0);
        assert_eq!(editor.selection(), &[layer]);
    }

    #[test]
    fn picking_finds_the_topmost_unlocked_visible_label() {
        let mut editor = editor();
        let below = editor.place_label([50.0, 50.0]).unwrap();
        editor.end_gesture();
        let above = editor.place_label([52.0, 50.0]).unwrap();
        editor.end_gesture();
        assert_eq!(editor.pick([50.0, 50.0]), Some(above));
        assert_eq!(editor.pick([-500.0, 50.0]), None);
        editor.run(Command::SetLocked {
            id: above,
            locked: true,
        });
        assert_eq!(editor.pick([50.0, 50.0]), Some(below));
        let layer = editor.map.parent(below).unwrap();
        editor.run(Command::SetVisible {
            id: layer,
            visible: false,
        });
        assert_eq!(editor.pick([50.0, 50.0]), None);
    }

    fn opacity(editor: &Editor, id: Id) -> f32 {
        match editor.map.node(id) {
            Some(Node::Layer(layer)) => layer.opacity,
            _ => panic!("a Layer"),
        }
    }

    fn editor_with_square() -> (Editor, Id) {
        let mut editor = editor();
        let layer = editor.new_layer().unwrap();
        let id = Id::new();
        editor.run(Command::CreateLandmass {
            id,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                holes: Vec::new(),
            }]),
            fill: None,
        });
        (editor, id)
    }

    fn node(index: usize) -> NodeRef {
        NodeRef {
            part: 0,
            ring: 0,
            index,
        }
    }

    #[test]
    fn node_edit_mode_follows_the_selection_and_edits_undo_cleanly() {
        let (mut editor, id) = editor_with_square();
        editor.enter_node_edit(id);
        assert_eq!(editor.selection(), &[id]);

        // A drag is one gesture and one Undo step, however many frames it takes.
        editor.select_nodes(vec![node(2)]);
        editor.begin_gesture("Move Nodes");
        for step in 1..=5 {
            editor.gesture_replace(Command::MoveNodes {
                id,
                nodes: vec![node(2)],
                offset: [f64::from(step), 0.0],
            });
        }
        editor.end_gesture();
        assert_eq!(
            editor.edited_outline().unwrap().node(node(2)),
            Some([15.0, 10.0])
        );
        editor.undo();
        assert_eq!(
            editor.edited_outline().unwrap().node(node(2)),
            Some([10.0, 10.0])
        );
        assert!(editor.node_edit().is_some_and(|edit| edit.nodes.is_empty()));

        editor.insert_node(node(0), [5.0, -2.0]);
        assert_eq!(editor.node_edit().unwrap().nodes, vec![node(1)]);
        assert_eq!(editor.edited_shape(), vec![5]);
        editor.delete_selected_nodes();
        assert_eq!(editor.edited_shape(), vec![4]);

        // Leaving, or selecting something else, ends the mode.
        editor.select(Vec::new());
        assert!(editor.node_edit().is_none());
    }

    #[test]
    fn deleting_nodes_that_would_leave_fewer_than_three_is_refused() {
        let (mut editor, id) = editor_with_square();
        editor.enter_node_edit(id);
        editor.select_nodes(vec![node(0), node(1)]);
        editor.delete_selected_nodes();
        assert_eq!(editor.edited_shape(), vec![4]);
        assert!(editor.notice().is_some_and(|notice| notice.is_error));
    }

    #[test]
    fn union_and_split_apart_work_on_the_selection_in_one_undo_step() {
        let (mut editor, a) = editor_with_square();
        let layer = editor.map.parent(a).unwrap();
        let b = Id::new();
        editor.run(Command::CreateLandmass {
            id: b,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[5.0, 5.0], [20.0, 5.0], [20.0, 20.0], [5.0, 20.0]],
                holes: Vec::new(),
            }]),
            fill: None,
        });
        editor.select(vec![a, b]);
        assert!(editor.can_combine() && !editor.can_split());
        editor.union_selection();
        assert_eq!(editor.selection(), &[b]);
        assert!(!editor.map.contains(a));

        editor.undo();
        assert!(editor.map.contains(a) && editor.map.contains(b));

        // Two islands split into two Landmasses with one Undo step.
        editor.select(vec![a]);
        editor.run(Command::AddRegion {
            id: a,
            region: MultiPolygon(vec![ssmap_core::Polygon {
                exterior: vec![[50.0, 50.0], [60.0, 50.0], [60.0, 60.0], [50.0, 60.0]],
                holes: Vec::new(),
            }]),
        });
        assert!(editor.can_split());
        let before = editor.map.children(Some(layer)).len();
        editor.split_selection();
        assert_eq!(editor.map.children(Some(layer)).len(), before + 1);
        assert_eq!(editor.selection().len(), 2);
        editor.undo();
        assert_eq!(editor.map.children(Some(layer)).len(), before);
    }
}
