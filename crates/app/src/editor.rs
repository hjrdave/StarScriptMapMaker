//! Editor state around one Map: history, Selection, the active Layer and tool, and the
//! message bar. Every change to the Map goes through a Command here.

use std::time::{Duration, Instant};

use ssmap_core::{Command, CommandError, ExecuteError, History, Id, Map, Node, Selection};

use crate::layer_tree::node_name;
use crate::tools::ToolId;

/// How long a message stays in the status bar.
pub const NOTICE_DURATION: Duration = Duration::from_secs(5);

pub struct Editor {
    pub map: Map,
    history: History,
    selection: Selection,
    /// The Layer new Objects go into.
    active_layer: Option<Id>,
    pub active_tool: ToolId,
    notice: Option<Notice>,
    /// Set while a gesture's transaction is open.
    gesture_open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
    pub shown_at: Instant,
}

impl Editor {
    pub fn new(map: Map) -> Self {
        Self {
            map,
            history: History::default(),
            selection: Vec::new(),
            active_layer: None,
            active_tool: ToolId::Select,
            notice: None,
            gesture_open: false,
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

    /// Escape: undoes everything the open gesture did.
    pub fn cancel_gesture(&mut self) {
        if std::mem::take(&mut self.gesture_open) {
            self.history
                .rollback(&mut self.map)
                .expect("the gesture's transaction is open");
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

    fn opacity(editor: &Editor, id: Id) -> f32 {
        match editor.map.node(id) {
            Some(Node::Layer(layer)) => layer.opacity,
            _ => panic!("a Layer"),
        }
    }
}
