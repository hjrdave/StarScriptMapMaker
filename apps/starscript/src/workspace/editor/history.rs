use super::*;

impl Editor {
    pub fn can_undo(&self) -> bool {
        self.gesture.is_none() && self.history.undo_len() > 0
    }

    pub fn can_redo(&self) -> bool {
        self.gesture.is_none() && self.history.redo_len() > 0
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo_label()
    }

    /// Every step in the undo history, oldest first, and how many of them are applied. The
    /// steps past that count are the ones undo moved over and redo can bring back.
    pub fn history_steps(&self) -> (Vec<String>, usize) {
        let applied = self.history.undo_len();
        let steps = self
            .history
            .undo_labels()
            .chain(self.history.redo_labels())
            .map(str::to_owned)
            .collect();
        (steps, applied)
    }

    /// Moves the Map to the point where `applied` steps are applied, by undoing or redoing
    /// the steps between. Later steps stay available to redo.
    pub fn jump_to_history(&mut self, applied: usize) {
        self.end_open_gesture();
        let total = self.history.undo_len() + self.history.redo_len();
        let applied = applied.min(total);
        // Each pass must move the history, or something is wrong and the jump stops.
        while self.history.undo_len() > applied {
            let before = self.history.undo_len();
            self.undo();
            if self.history.undo_len() == before {
                break;
            }
        }
        while self.history.undo_len() < applied {
            let before = self.history.undo_len();
            self.redo();
            if self.history.undo_len() == before {
                break;
            }
        }
    }

    pub fn undo(&mut self) {
        self.end_open_gesture();
        if let Ok(Some(selection)) = self.history.undo(&mut self.map) {
            self.restore(selection);
        }
    }

    pub fn redo(&mut self) {
        self.end_open_gesture();
        if let Ok(Some(selection)) = self.history.redo(&mut self.map) {
            self.restore(selection);
        }
    }
}
