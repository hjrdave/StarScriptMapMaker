//! The Select tool: click, box-select, move/scale/rotate drags, and double-click to edit.

use eframe::egui;
use ssmap_core::{Id, Point};
use ssmap_render::View;

use super::drag::{drag_commands, membership_command, start_drag};
use super::{CanvasFrame, ToolState};
use crate::editor::Editor;
use crate::handles::{self, Bounds};

/// A box-select in progress, in Canvas units. Shift keeps what was selected when it began.
pub(super) struct SelectBox {
    start: Point,
    end: Point,
    keep: Vec<Id>,
}

/// The Select tool: click, box-select, move/scale/rotate drags, and double-click to edit.
pub(super) fn select_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
) -> Option<egui::CursorIcon> {
    let shift = input.shift;
    let bounds = Bounds::of_selection(editor);
    let rotatable = selection_has_label(editor);
    let grip_at = |pos: egui::Pos2| {
        let bounds = bounds?;
        handles::hit(
            &bounds,
            view,
            input.rect_min,
            input.pixels_per_point,
            pos,
            rotatable,
        )
    };
    let mut cursor = None;
    if state.drag.is_none()
        && let Some(pos) = input.hover_pos
        && let Some(grip) = grip_at(pos)
    {
        cursor = Some(grip.cursor());
    }
    if input.drag_started
        && !editor.gesture_open()
        && let Some(pos) = input.press_origin
    {
        let duplicate = input.alt;
        let point = input.canvas_point(view, pos);
        let grip = grip_at(pos);
        let on_selection = bounds.is_some_and(|b| b.contains(point));
        if grip.is_none() && !on_selection && editor.pick(point).is_none() {
            // Empty Canvas or off-Canvas: the drag draws a selection box.
            let keep = if shift {
                editor.selection().to_vec()
            } else {
                Vec::new()
            };
            state.select_box = Some(SelectBox {
                start: point,
                end: point,
                keep,
            });
        } else {
            start_drag(editor, state, bounds, grip, point, duplicate);
        }
    }
    select_box_tool(editor, view, state, input);
    let over_selection = || {
        let pos = input.hover_pos?;
        let inside = grip_at(pos).is_none()
            && bounds.is_some_and(|b| b.contains(input.canvas_point(view, pos)));
        inside.then_some(())
    };
    if state
        .drag
        .as_ref()
        .is_some_and(|drag| drag.copies.is_some())
        || (state.drag.is_none() && input.alt && over_selection().is_some())
    {
        cursor = Some(egui::CursorIcon::Copy);
    }
    // Escape or undo ended the gesture under the drag.
    if state.drag.is_some() && !state.holds_open(editor) {
        state.drag = None;
    }
    if let Some(drag) = &state.drag
        && input.dragging
        && let Some(pos) = input.pointer_pos
        && let Some(commands) = drag_commands(drag, input.canvas_point(view, pos), shift)
    {
        let mut commands = commands.into_iter();
        if let Some(first) = commands.next() {
            state.replace_held(editor, first);
        }
        for command in commands {
            state.run_held(editor, command);
        }
        if let Some(command) = membership_command(editor, drag, input.canvas_point(view, pos)) {
            state.run_held(editor, command);
        }
    }
    if input.drag_stopped && state.drag.take().is_some() {
        state.end_held(editor);
    }
    if let Some(pos) = input.pointer_pos {
        let point = input.canvas_point(view, pos);
        if input.double_clicked {
            if let Some(id) = editor.pick(point) {
                if editor.label(id).is_some() {
                    editor.select(vec![id]);
                    state.edit_text(editor, id);
                } else {
                    editor.enter_node_edit(id);
                }
            }
        } else if input.clicked {
            let hit = editor.pick(point);
            click_select(editor, hit, shift);
        }
    }
    cursor
}

/// Grows the box-select started with the Select tool while the pointer drags, and selects the
/// Objects inside it on release.
fn select_box_tool(editor: &mut Editor, view: &View, state: &mut ToolState, input: &CanvasFrame) {
    let Some(select_box) = &mut state.select_box else {
        return;
    };
    if input.dragging
        && let Some(pos) = input.pointer_pos
    {
        select_box.end = input.canvas_point(view, pos);
    }
    if input.drag_stopped
        && let Some(done) = state.select_box.take()
    {
        let mut selection = done.keep;
        for id in objects_in_box(editor, done.start, done.end) {
            if !selection.contains(&id) {
                selection.push(id);
            }
        }
        editor.select(selection);
    }
}

/// Draws a box-select in progress.
pub(super) fn paint_select_box(
    painter: &egui::Painter,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    ppp: f64,
) {
    if let Some(select_box) = &state.select_box {
        let select = crate::theme::palette(painter.ctx()).select;
        let to_screen = |p: Point| crate::handles::to_screen(view, origin, ppp, p);
        let rect = egui::Rect::from_two_pos(to_screen(select_box.start), to_screen(select_box.end));
        painter.rect_filled(rect, 0.0, select.gamma_multiply(0.16));
        painter.rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.5, select),
            egui::StrokeKind::Middle,
        );
    }
}

/// The unlocked Objects whose bounds meet the box between `a` and `b`, in draw order. Reaches
/// the off-Canvas as well as the Canvas.
fn objects_in_box(editor: &mut Editor, a: Point, b: Point) -> Vec<Id> {
    let (min, max) = (
        [a[0].min(b[0]), a[1].min(b[1])],
        [a[0].max(b[0]), a[1].max(b[1])],
    );
    let candidates: Vec<Id> = editor
        .map
        .draw_order()
        .into_iter()
        .filter(|&id| !editor.map.is_locked(id))
        .collect();
    candidates
        .into_iter()
        .filter(|&id| {
            Bounds::of_ids(editor, &[id]).is_some_and(|bounds| {
                bounds.min[0] <= max[0]
                    && bounds.max[0] >= min[0]
                    && bounds.min[1] <= max[1]
                    && bounds.max[1] >= min[1]
            })
        })
        .collect()
}

/// A click: selects what is under the pointer, Shift adds or removes it, and empty Canvas
/// clears the Selection.
fn click_select(editor: &mut Editor, hit: Option<Id>, shift: bool) {
    match (hit, shift) {
        (Some(id), true) => {
            let mut selection = editor.selection().to_vec();
            match selection.iter().position(|&s| s == id) {
                Some(i) => {
                    selection.remove(i);
                }
                None => selection.push(id),
            }
            editor.select(selection);
        }
        (Some(id), false) => editor.select(vec![id]),
        (None, true) => {}
        (None, false) => editor.select(Vec::new()),
    }
}

/// Whether the selection holds a Label or a Stamp, the only selected things that rotate.
pub(crate) fn selection_has_label(editor: &Editor) -> bool {
    editor
        .selection()
        .iter()
        .any(|&id| editor.label(id).is_some() || editor.stamp(id).is_some())
}

#[cfg(test)]
mod tests {
    use super::super::ToolId;
    use super::super::label::label_tool;
    use super::super::test_util::*;
    use super::*;

    #[test]
    fn a_select_box_selects_the_objects_it_meets_on_release() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        paint_stroke(&mut editor, &view, &mut state, false);
        editor.select(Vec::new());
        state.select_box = Some(SelectBox {
            start: [0.0, 0.0],
            end: [0.0, 0.0],
            keep: Vec::new(),
        });
        select_box_tool(&mut editor, &view, &mut state, &drag_to(300.0, 300.0));
        assert_eq!(
            state.select_box.as_ref().map(|b| b.end),
            Some([300.0, 300.0])
        );
        assert!(editor.selection().is_empty());
        select_box_tool(&mut editor, &view, &mut state, &release(300.0, 300.0));
        assert!(state.select_box.is_none());
        assert_eq!(editor.selection().len(), 1);
    }

    #[test]
    fn a_select_box_keeps_the_selection_it_was_given() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let kept = Id::new();
        state.select_box = Some(SelectBox {
            start: [400.0, 400.0],
            end: [400.0, 400.0],
            keep: vec![kept],
        });
        select_box_tool(&mut editor, &view, &mut state, &release(450.0, 450.0));
        assert_eq!(editor.selection(), [kept]);
    }

    fn alt(input: CanvasFrame) -> CanvasFrame {
        CanvasFrame { alt: true, ..input }
    }

    #[test]
    fn a_click_selects_the_object_under_it_and_empty_space_clears() {
        let (mut editor, view, mut state) = with_land();
        select_tool(&mut editor, &view, &mut state, &click(150.0, 125.0));
        assert_eq!(editor.selection().len(), 1);
        select_tool(&mut editor, &view, &mut state, &click(400.0, 400.0));
        assert!(editor.selection().is_empty());
    }

    #[test]
    fn a_select_drag_on_empty_space_box_selects() {
        let (mut editor, view, mut state) = with_land();
        select_tool(&mut editor, &view, &mut state, &press(50.0, 50.0));
        assert!(state.select_box.is_some());
        select_tool(&mut editor, &view, &mut state, &drag_to(300.0, 300.0));
        select_tool(&mut editor, &view, &mut state, &release(300.0, 300.0));
        assert!(state.select_box.is_none());
        assert_eq!(editor.selection().len(), 1);
    }

    #[test]
    fn dragging_the_selection_moves_it_in_one_undo_step() {
        let (mut editor, view, mut state) = with_land();
        select_tool(&mut editor, &view, &mut state, &click(150.0, 125.0));
        let before = editor.history_steps().1;
        let start = Bounds::of_selection(&mut editor).map(|b| b.min);
        select_tool(&mut editor, &view, &mut state, &press(150.0, 125.0));
        select_tool(&mut editor, &view, &mut state, &drag_to(170.0, 145.0));
        select_tool(&mut editor, &view, &mut state, &drag_to(190.0, 165.0));
        assert!(editor.gesture_open());
        select_tool(&mut editor, &view, &mut state, &release(190.0, 165.0));
        assert_ne!(Bounds::of_selection(&mut editor).map(|b| b.min), start);
        assert!(state.drag.is_none() && !editor.gesture_open());
        assert_eq!(editor.history_steps().1, before + 1);
    }

    #[test]
    fn switching_tools_cancels_a_selection_drag() {
        let (mut editor, view, mut state) = with_land();
        select_tool(&mut editor, &view, &mut state, &click(150.0, 125.0));
        let start = Bounds::of_selection(&mut editor).map(|b| b.min);
        let before = editor.history_steps().1;
        select_tool(&mut editor, &view, &mut state, &press(150.0, 125.0));
        select_tool(&mut editor, &view, &mut state, &drag_to(190.0, 165.0));
        assert!(state.drag.is_some());
        editor.active_tool = ToolId::Lasso;
        state.drop_held_for_other_tool(&mut editor);
        assert!(state.drag.is_none() && !editor.gesture_open());
        assert_eq!(Bounds::of_selection(&mut editor).map(|b| b.min), start);
        assert_eq!(editor.history_steps().1, before);
    }

    #[test]
    fn double_click_on_a_shape_enters_node_edit() {
        let (mut editor, view, mut state) = with_land();
        let double = CanvasFrame {
            double_clicked: true,
            ..click(150.0, 125.0)
        };
        select_tool(&mut editor, &view, &mut state, &double);
        assert!(editor.node_edit().is_some());
    }

    #[test]
    fn double_click_on_a_label_opens_its_text() {
        let (mut editor, view, mut state) = with_land();
        editor.active_tool = ToolId::Label;
        label_tool(&mut editor, &view, &mut state, &click(300.0, 300.0));
        assert_eq!(editor.active_tool, ToolId::Select);
        let placed = state.editing.take().map(|e| e.gesture);
        assert!(placed.is_some());
        if let Some(gesture) = placed {
            editor.end_gesture(gesture);
        }
        editor.select(Vec::new());
        let double = CanvasFrame {
            double_clicked: true,
            ..click(300.0, 300.0)
        };
        select_tool(&mut editor, &view, &mut state, &double);
        let reopened = state.editing.take().map(|e| e.gesture);
        assert!(reopened.is_some());
        if let Some(gesture) = reopened {
            editor.end_gesture(gesture);
        }
    }

    #[test]
    fn an_alt_drag_copies_the_selection() {
        let (mut editor, view, mut state) = with_land();
        select_tool(&mut editor, &view, &mut state, &click(150.0, 125.0));
        let original = editor.selection().to_vec();
        select_tool(&mut editor, &view, &mut state, &alt(press(150.0, 125.0)));
        assert!(state.drag.as_ref().is_some_and(|d| d.copies.is_some()));
        let cursor = select_tool(&mut editor, &view, &mut state, &alt(drag_to(250.0, 225.0)));
        assert_eq!(cursor, Some(egui::CursorIcon::Copy));
        select_tool(&mut editor, &view, &mut state, &release(250.0, 225.0));
        assert!(!editor.gesture_open());
        assert_ne!(editor.selection(), original);
    }
}
