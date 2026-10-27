//! The Canvas tool: select the Canvas by its edge or label, then resize it.

use eframe::egui;
use ssmap_core::Command;
use ssmap_render::View;

use super::{CanvasFrame, EDGE_GRAB, ToolId, ToolState};
use crate::editor::Editor;
use crate::handles::{self, Bounds, Grip};

/// The Canvas's box in screen points.
fn canvas_screen_rect(editor: &Editor, view: &View, origin: egui::Pos2, ppp: f64) -> egui::Rect {
    let rect = editor.map().canvas().rect();
    egui::Rect::from_two_pos(
        handles::to_screen(view, origin, ppp, rect.min),
        handles::to_screen(view, origin, ppp, rect.max),
    )
}

/// Select the Canvas by clicking its edge or its label (not its inside, so Objects on it are
/// not selected by accident), then resize it from its handles. Shift keeps the aspect ratio
/// and Alt resizes from the centre. Each resize is one `ResizeCanvas` Undo step and never
/// touches an Object.
pub(super) fn canvas_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
) -> Option<egui::CursorIcon> {
    let ppp = input.pixels_per_point;
    let canvas = editor.map().canvas().clone();
    let bounds = Bounds {
        min: canvas.origin,
        max: canvas.rect().max,
    };
    let selected = editor.canvas_selected();
    let grip_at =
        |pos: egui::Pos2| match handles::hit(&bounds, view, input.rect_min, ppp, pos, false) {
            Some(Grip::Resize(handle)) if selected => Some(handle),
            _ => None,
        };
    let screen = canvas_screen_rect(editor, view, input.rect_min, ppp);
    let on_edge = |pos: egui::Pos2| {
        screen.expand(EDGE_GRAB as f32).contains(pos)
            && !screen.shrink(EDGE_GRAB as f32).contains(pos)
    };
    let canvas_label = state.canvas_label;
    let on_label = |pos: egui::Pos2| canvas_label.is_some_and(|label| label.contains(pos));
    let mut cursor = None;
    if state.canvas_drag.is_none()
        && let Some(pos) = input.hover_pos
    {
        if let Some(handle) = grip_at(pos) {
            cursor = Some(handle.cursor());
        } else if on_edge(pos) || on_label(pos) {
            cursor = Some(egui::CursorIcon::PointingHand);
        }
    }
    if input.drag_started
        && !editor.gesture_open()
        && let Some(pos) = input.press_origin
        && let Some(handle) = grip_at(pos)
        && state.begin_held(editor, ToolId::Canvas, "Resize Canvas")
    {
        state.canvas_drag = Some(crate::canvas_resize::CanvasDrag {
            handle,
            start: input.canvas_point(view, pos),
            origin: canvas.origin,
            size: [canvas.width, canvas.height],
        });
    }
    if let Some(drag) = state.canvas_drag {
        // Escape or undo ended the gesture under the drag.
        if !state.holds_open(editor) {
            state.canvas_drag = None;
        } else if input.dragging
            && let Some(pos) = input.pointer_pos
        {
            let (origin, width, height) =
                drag.resized(input.canvas_point(view, pos), input.shift, input.alt);
            state.replace_held(
                editor,
                Command::ResizeCanvas {
                    origin,
                    width,
                    height,
                },
            );
        }
    }
    if input.drag_stopped && state.canvas_drag.take().is_some() {
        state.end_held(editor);
    }
    if input.clicked
        && let Some(pos) = input.pointer_pos
        && grip_at(pos).is_none()
    {
        editor.select_canvas(on_edge(pos) || on_label(pos));
    }
    cursor
}

/// Draws the Canvas's label above its top-left corner, and its handles when it is selected.
pub(super) fn paint_canvas(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &mut ToolState,
    origin: egui::Pos2,
    ppp: f64,
) {
    let palette = crate::theme::palette(painter.ctx());
    let screen = canvas_screen_rect(editor, view, origin, ppp);
    let selected = editor.canvas_selected();
    let color = if selected {
        palette.select
    } else {
        palette.text_secondary
    };
    let text = painter.text(
        screen.left_top() - egui::vec2(0.0, 4.0),
        egui::Align2::LEFT_BOTTOM,
        "Canvas",
        egui::FontId::proportional(crate::theme::CAPTION),
        color,
    );
    state.canvas_label = Some(text.expand(3.0));
    if selected {
        let bounds = Bounds {
            min: editor.map().canvas().origin,
            max: editor.map().canvas().rect().max,
        };
        handles::paint(painter, &bounds, view, origin, ppp, false, palette.select);
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;

    #[test]
    fn clicking_the_canvas_edge_selects_the_canvas_but_its_inside_does_not() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        let click = |x, y| CanvasFrame {
            clicked: true,
            pointer_pos: Some(at(x, y)),
            ..input()
        };
        canvas_tool(&mut editor, &view, &mut state, &click(250.0, 250.0));
        assert!(!editor.canvas_selected());
        canvas_tool(&mut editor, &view, &mut state, &click(2.0, 250.0));
        assert!(editor.canvas_selected());
        canvas_tool(&mut editor, &view, &mut state, &click(250.0, 250.0));
        assert!(!editor.canvas_selected());
    }

    #[test]
    fn dragging_a_handle_of_the_selected_canvas_resizes_the_canvas() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.select_canvas(true);
        canvas_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        assert!(state.canvas_drag.is_some());
        canvas_tool(&mut editor, &view, &mut state, &drag_to(600.0, 560.0));
        assert_eq!(
            (editor.map().canvas().width, editor.map().canvas().height),
            (600, 560)
        );
        canvas_tool(&mut editor, &view, &mut state, &release(600.0, 560.0));
        assert!(state.canvas_drag.is_none());
        assert!(!editor.gesture_open());
    }

    #[test]
    fn escape_mid_drag_rolls_the_canvas_resize_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.select_canvas(true);
        canvas_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        canvas_tool(&mut editor, &view, &mut state, &drag_to(600.0, 560.0));
        assert_eq!(editor.map().canvas().width, 600);
        assert!(editor.cancel_open_gesture());
        canvas_tool(&mut editor, &view, &mut state, &drag_to(620.0, 580.0));
        assert!(state.canvas_drag.is_none() && state.held.is_none());
        assert_eq!(
            (editor.map().canvas().width, editor.map().canvas().height),
            (500, 500)
        );
        assert!(!editor.gesture_open());
    }

    #[test]
    fn switching_tools_mid_drag_rolls_the_gesture_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.select_canvas(true);
        editor.active_tool = ToolId::Canvas;
        canvas_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        canvas_tool(&mut editor, &view, &mut state, &drag_to(600.0, 560.0));
        assert_eq!(editor.map().canvas().width, 600);
        editor.active_tool = ToolId::Select;
        state.drop_held_for_other_tool(&mut editor);
        assert!(state.canvas_drag.is_none() && state.held.is_none());
        assert!(!editor.gesture_open());
        assert_eq!(
            (editor.map().canvas().width, editor.map().canvas().height),
            (500, 500)
        );
        assert_eq!(editor.undo_label(), None);
    }

    #[test]
    fn a_handle_of_an_unselected_canvas_is_not_grabbed() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        canvas_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        assert!(state.canvas_drag.is_none());
    }
}
