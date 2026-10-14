//! Editor state around one Map: history, Selection, the active Layer and tool, and the
//! message bar. Every change to the Map goes through a Command here.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ssmap_core::{
    Command, CommandError, ExecuteError, History, Id, Label, Map, Node, ObjectKind, Op, Selection,
};
use ssmap_render::{Fonts, LabelLayout, MapScene};

use crate::layer_tree::node_name;
use crate::tools::ToolId;

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
    notice: Option<Notice>,
    /// Set while a gesture's transaction is open.
    gesture_open: bool,
    /// The Selection when the open gesture began, restored if it is cancelled.
    gesture_selection: Selection,
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
            notice: None,
            gesture_open: false,
            gesture_selection: Vec::new(),
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
        self.selection = ids;
        if let Some(&last) = self.selection.last() {
            self.active_layer = match self.map.node(last) {
                Some(Node::Layer(_)) => Some(last),
                _ => self.map.parent(last),
            };
        }
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
        fn walk(editor: &mut Editor, parent: Option<Id>, point: [f64; 2]) -> Option<Id> {
            let children = editor.map.children(parent).to_vec();
            for id in children.into_iter().rev() {
                let Some(node) = editor.map.node(id) else {
                    continue;
                };
                if node.locked() {
                    continue;
                }
                match node {
                    Node::Layer(layer) if !layer.visible => {}
                    Node::Layer(_) => {
                        if let Some(hit) = walk(editor, Some(id), point) {
                            return Some(hit);
                        }
                    }
                    Node::Object(_) => {
                        if let Some((label, layout)) = editor.label_layout(id)
                            && layout.contains(&label, point)
                        {
                            return Some(id);
                        }
                    }
                }
            }
            None
        }
        walk(self, None, point)
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
}
