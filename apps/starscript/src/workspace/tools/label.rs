//! The Label tool, and typing a Label's text in the box over the Canvas.

use ssmap_core::{Command, Id};
use ssmap_render::View;

use super::{CanvasFrame, ToolId, ToolState};
use crate::editor::{Editor, GestureId};

/// Typing a Label's text in a box overlaid at the Label, inside an open gesture so the
/// whole edit is one Undo step.
pub struct TextEditing {
    pub id: Id,
    pub text: String,
    /// Set until the box has taken the keyboard and selected the text.
    pub just_opened: bool,
    /// The gesture the text is typed in: "Edit Label", or "Add Label" when the Label was just
    /// placed.
    pub(super) gesture: GestureId,
}

impl TextEditing {
    /// Runs a Command in the gesture the text is typed in.
    pub fn run(&self, editor: &mut Editor, command: Command) -> bool {
        editor.gesture(&self.gesture, command)
    }

    /// Commits the gesture the text was typed in.
    pub fn finish(self, editor: &mut Editor) {
        editor.end_gesture(self.gesture);
    }
}

impl ToolState {
    /// Opens the text box on a Label, inside a new "Edit Label" gesture.
    pub fn edit_text(&mut self, editor: &mut Editor, id: Id) {
        if editor.label(id).is_none() {
            return;
        }
        let gesture = editor.begin_gesture("Edit Label");
        self.edit_placed_text(editor, id, gesture);
    }

    /// Opens the text box on the Label `id` inside `gesture`, which already holds its
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
            gesture,
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
