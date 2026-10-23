//! Panning the view, with the Pan tool or whatever tool is active.

use ssmap_render::View;

use super::{CanvasFrame, ToolId};
use crate::editor::Editor;

/// Pans the view for a middle-button drag, a Space-drag, or a drag with the Pan tool, whatever
/// the active tool. Returns whether the frame was a pan.
pub(super) fn pan_tool(editor: &Editor, view: &mut View, input: &CanvasFrame) -> bool {
    let temporary = input.middle_dragging || (input.space_held && input.dragging);
    if !(temporary || (editor.active_tool == ToolId::Pan && input.any_dragging)) {
        return false;
    }
    let ppp = input.pixels_per_point;
    view.pan([
        f64::from(input.drag_delta.x) * ppp,
        f64::from(input.drag_delta.y) * ppp,
    ]);
    true
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;
    use eframe::egui;

    #[test]
    fn pan_moves_the_view_by_the_drag_delta() {
        let mut editor = editor();
        let mut view = view();
        let before = view.to_canvas([0.0, 0.0]);
        let dragged = CanvasFrame {
            any_dragging: true,
            drag_delta: egui::vec2(10.0, 5.0),
            pixels_per_point: 2.0,
            ..input()
        };
        // A plain drag pans only with the Pan tool.
        editor.active_tool = ToolId::Select;
        assert!(!pan_tool(&editor, &mut view, &dragged));
        editor.active_tool = ToolId::Pan;
        assert!(pan_tool(&editor, &mut view, &dragged));
        assert_ne!(view.to_canvas([0.0, 0.0]), before);
        // Space-drag and middle-drag pan under any tool.
        editor.active_tool = ToolId::Select;
        let space = CanvasFrame {
            space_held: true,
            dragging: true,
            ..dragged.clone()
        };
        assert!(pan_tool(&editor, &mut view, &space));
        let middle = CanvasFrame {
            middle_dragging: true,
            ..dragged
        };
        assert!(pan_tool(&editor, &mut view, &middle));
    }
}
