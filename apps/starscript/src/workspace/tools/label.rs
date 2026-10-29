//! The Label tool, and editing a Label's text in place on the Canvas.

use ssmap_core::{Command, Id};
use ssmap_render::View;

use super::{CanvasFrame, ToolId, ToolState};
use crate::editor::{Editor, GestureId};

/// Editing a Label's text on the Canvas, inside an open gesture so the whole edit is one Undo
/// step. A straight Label is edited in place; a Label with an arch in a box under it.
pub struct TextEditing {
    pub id: Id,
    pub text: String,
    /// Set until the editor has taken the keyboard and selected the text.
    pub just_opened: bool,
    /// The gesture the text is typed in: "Edit Label", or "Add Label" when the Label was just
    /// placed. `None` once a handle committed it mid-edit; typing opens a new one.
    pub(super) gesture: Option<GestureId>,
}

impl TextEditing {
    /// Runs a Command in the gesture the text is typed in, opening an "Edit Label" one if a
    /// handle ended the last.
    pub fn run(&mut self, editor: &mut Editor, command: Command) -> bool {
        let gesture = self
            .gesture
            .get_or_insert_with(|| editor.begin_gesture("Edit Label"));
        editor.gesture(gesture, command)
    }

    /// Commits the gesture the text was typed in, as when a handle takes over. The text stays
    /// open for editing.
    pub fn commit(&mut self, editor: &mut Editor) {
        if let Some(gesture) = self.gesture.take() {
            editor.end_gesture(gesture);
        }
    }

    /// Commits the gesture the text was typed in.
    pub fn finish(mut self, editor: &mut Editor) {
        self.commit(editor);
    }
}

/// Whether a Label with this arch is edited in place. Text along a curve is typed in a box
/// under the Label instead.
pub fn edits_in_place(arch: f64) -> bool {
    arch == 0.0
}

impl ToolState {
    /// Opens the text editor on a Label, inside a new "Edit Label" gesture.
    pub fn edit_text(&mut self, editor: &mut Editor, id: Id) {
        if editor.label(id).is_none() {
            return;
        }
        let gesture = editor.begin_gesture("Edit Label");
        self.edit_placed_text(editor, id, gesture);
    }

    /// Opens the text editor on the Label `id` inside `gesture`, which already holds its
    /// placement ("Add Label").
    fn edit_placed_text(&mut self, editor: &mut Editor, id: Id, gesture: GestureId) {
        let Some(label) = editor.label(id) else {
            editor.cancel_gesture(gesture);
            return;
        };
        let text = label.text.clone();
        self.editing = Some(TextEditing {
            id,
            text,
            just_opened: true,
            gesture: Some(gesture),
        });
    }
}

/// The Label tool: a click places a Label and starts editing its text.
pub(super) fn label_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
) {
    if input.clicked
        && let Some(pos) = input.pointer_pos
        && let Some((id, gesture)) = editor.place_label(input.canvas_point(view, pos))
    {
        // Typing joins the "Add Label" gesture, so placing and naming undo together.
        state.edit_placed_text(editor, id, gesture);
        editor.active_tool = ToolId::Select;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_straight_label_is_edited_in_place() {
        assert!(edits_in_place(0.0));
        assert!(!edits_in_place(0.25));
        assert!(!edits_in_place(-1.0));
    }
}
