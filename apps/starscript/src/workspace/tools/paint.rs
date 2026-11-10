//! The Paint tool: drag to paint a stroke of colour over the Map, or erase paint with the
//! eraser mode (ADR 0017).

use eframe::egui;
use ssmap_core::{MAX_RAW_STROKE_POINTS, Point};
use ssmap_render::View;

use super::{CanvasFrame, ToolId, ToolState};
use crate::editor::Editor;

/// A paint stroke in progress. Nothing changes in the Map until it is released; until then an
/// overlay shows the path.
pub(super) struct PaintDrag {
    /// The pointer's path in Canvas units, from where the button went down.
    pub(super) points: Vec<Point>,
    pub(super) erase: bool,
}

/// Paint by dragging, or erase with the erase modifier held (or the Eraser mode). One
/// press-drag-release is one `AddPaintStroke` Command, made on release, so one Undo step;
/// Escape drops a stroke in progress.
pub(super) fn paint_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
) -> Option<egui::CursorIcon> {
    let radius = editor.paint.size / 2.0;
    let erase_held = input.alt != editor.paint.erase;
    let cursor = input.hovered.then_some(egui::CursorIcon::Crosshair);

    // A click is a stroke of one point: a single dab.
    let pressed = input.drag_started || input.clicked;
    if pressed
        && state.paint.is_none()
        && !editor.gesture_open()
        && let Some(origin) = input.press_origin.or(input.pointer_pos)
        && state.begin_held(editor, ToolId::Paint, Editor::paint_label(erase_held))
    {
        state.paint = Some(PaintDrag {
            points: vec![input.canvas_point(view, origin)],
            erase: erase_held,
        });
    }

    // Escape rolled the gesture back: drop the stroke.
    if state.paint.is_some() && !state.holds_open(editor) {
        state.paint = None;
    }
    let Some(drag) = state.paint.as_mut() else {
        return cursor;
    };
    if let Some(pos) = input.pointer_pos {
        let point = input.canvas_point(view, pos);
        // A tenth of the radius, but at least a screen pixel, keeps the path light.
        let spacing = (radius * 0.1).max(1.0 / view.zoom);
        if let Some(&last) = drag.points.last()
            && drag.points.len() < MAX_RAW_STROKE_POINTS
            && (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing
        {
            drag.points.push(point);
        }
    }
    if (input.drag_stopped || input.clicked)
        && let Some(drag) = state.paint.take()
        && let Some(id) = state.take_held()
    {
        editor.finish_paint_stroke(id, drag.points, drag.erase);
    }
    cursor
}

/// Draws the brush's outline under the pointer, and the path of a stroke in progress.
pub(super) fn paint_paint_cursor(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    let palette = crate::theme::palette(painter.ctx());
    let erase_held = painter.ctx().input(|i| i.modifiers.alt) != editor.paint.erase;
    let erasing = state.paint.as_ref().map_or(erase_held, |drag| drag.erase);
    let radius = (editor.paint.size / 2.0 * view.zoom / ppp) as f32;
    if let Some(drag) = &state.paint {
        let color = if erasing {
            palette.danger
        } else {
            let c = editor.paint.color;
            egui::Color32::from_rgb(c.r, c.g, c.b)
        };
        let color = color.gamma_multiply(0.6 * editor.paint.opacity as f32);
        let points: Vec<egui::Pos2> = drag
            .points
            .iter()
            .map(|&p| crate::handles::to_screen(view, origin, ppp, p))
            .collect();
        if points.len() > 1 {
            painter.add(egui::Shape::line(
                points.clone(),
                egui::Stroke::new(radius * 2.0, color),
            ));
        }
        for end in [points.first(), points.last()].into_iter().flatten() {
            painter.circle_filled(*end, radius, color);
        }
    }
    if let Some(pos) = hover {
        super::brush::brush_ring(painter, pos, radius);
        // The soft edge starts inside the ring, so a softer brush shows a smaller core.
        let core = radius * (1.0 - editor.paint.softness as f32);
        if core >= 1.0 && core < radius {
            painter.circle_stroke(
                pos,
                core,
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(110)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;
    use ssmap_core::Color;

    fn paint_editor() -> Editor {
        let mut editor = editor();
        editor.active_tool = ToolId::Paint;
        editor
    }

    fn paint(editor: &mut Editor, state: &mut ToolState, frames: &[CanvasFrame]) {
        let view = view();
        for frame in frames {
            paint_tool(editor, &view, state, frame);
        }
    }

    fn drag(editor: &mut Editor, state: &mut ToolState) {
        paint(
            editor,
            state,
            &[
                press(100.0, 100.0),
                drag_to(150.0, 120.0),
                drag_to(200.0, 150.0),
                release(200.0, 150.0),
            ],
        );
    }

    #[test]
    fn a_drag_is_one_undo_step_made_on_release() {
        let (mut editor, view, mut state) = (paint_editor(), view(), ToolState::default());
        paint_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        paint_tool(&mut editor, &view, &mut state, &drag_to(200.0, 150.0));
        assert!(editor.gesture_open() && state.paint.is_some());
        assert!(editor.map().paint_strokes().is_empty());
        paint_tool(&mut editor, &view, &mut state, &release(200.0, 150.0));
        assert!(!editor.gesture_open() && state.paint.is_none());
        assert_eq!(editor.map().paint_strokes().len(), 1);
        assert_eq!(editor.undo_label(), Some("Paint"));
        assert_eq!(editor.history_steps().1, 1);
        editor.undo();
        assert!(editor.map().paint_strokes().is_empty());
    }

    #[test]
    fn the_stroke_takes_the_brush_settings() {
        let (mut editor, mut state) = (paint_editor(), ToolState::default());
        editor.paint.color = Color::rgb(1, 2, 3);
        editor.paint.size = 20.0;
        editor.paint.softness = 0.5;
        editor.paint.opacity = 0.25;
        drag(&mut editor, &mut state);
        let stroke = &editor.map().paint_strokes()[0];
        assert_eq!(stroke.color, Color::rgb(1, 2, 3));
        assert_eq!(
            (stroke.size, stroke.softness, stroke.opacity),
            (20.0, 0.5, 0.25)
        );
        assert!(!stroke.erase);
        assert_eq!(stroke.points.first(), Some(&[100.0, 100.0]));
        assert_eq!(stroke.points.last(), Some(&[200.0, 150.0]));
    }

    #[test]
    fn a_click_paints_a_dab_in_one_frame() {
        let (mut editor, mut state) = (paint_editor(), ToolState::default());
        paint(&mut editor, &mut state, &[click_at(250.0, 250.0)]);
        assert!(!editor.gesture_open() && state.paint.is_none());
        assert_eq!(editor.map().paint_strokes()[0].points.len(), 1);
        assert_eq!(editor.undo_label(), Some("Paint"));
    }

    #[test]
    fn the_eraser_mode_and_the_erase_modifier_add_an_eraser_stroke() {
        let (mut editor, mut state) = (paint_editor(), ToolState::default());
        editor.paint.erase = true;
        drag(&mut editor, &mut state);
        assert!(editor.map().paint_strokes()[0].erase);
        assert_eq!(editor.undo_label(), Some("Erase Paint"));

        // The modifier flips the mode: Alt in the Eraser mode paints.
        let alt = |frame: CanvasFrame| CanvasFrame { alt: true, ..frame };
        paint(
            &mut editor,
            &mut state,
            &[alt(press(100.0, 100.0)), alt(release(120.0, 100.0))],
        );
        assert!(!editor.map().paint_strokes()[1].erase);
        assert_eq!(editor.undo_label(), Some("Paint"));
    }

    #[test]
    fn escape_mid_stroke_drops_it() {
        let (mut editor, mut state) = (paint_editor(), ToolState::default());
        paint(
            &mut editor,
            &mut state,
            &[press(100.0, 100.0), drag_to(150.0, 120.0)],
        );
        assert!(editor.cancel_open_gesture());
        paint(
            &mut editor,
            &mut state,
            &[drag_to(200.0, 150.0), release(200.0, 150.0)],
        );
        assert!(state.paint.is_none() && !editor.gesture_open());
        assert!(editor.map().paint_strokes().is_empty());
        assert_eq!(editor.history_steps().1, 0);
    }

    #[test]
    fn switching_tools_mid_stroke_rolls_it_back() {
        let (mut editor, mut state) = (paint_editor(), ToolState::default());
        paint(&mut editor, &mut state, &[press(100.0, 100.0)]);
        assert!(editor.gesture_open());
        editor.active_tool = ToolId::Select;
        state.drop_held_for_other_tool(&mut editor);
        assert!(state.paint.is_none() && !editor.gesture_open());
        assert_eq!(editor.history_steps().1, 0);
    }

    #[test]
    fn a_refused_stroke_leaves_no_undo_step() {
        let (mut editor, mut state) = (paint_editor(), ToolState::default());
        editor.paint.size = 0.0;
        drag(&mut editor, &mut state);
        assert!(!editor.gesture_open());
        assert!(editor.map().paint_strokes().is_empty());
        assert_eq!(editor.history_steps().1, 0);
    }

    #[test]
    fn the_cursor_is_a_crosshair_over_the_canvas() {
        let (mut editor, view, mut state) = (paint_editor(), view(), ToolState::default());
        assert_eq!(
            paint_tool(&mut editor, &view, &mut state, &hovering()),
            Some(egui::CursorIcon::Crosshair)
        );
        assert_eq!(paint_tool(&mut editor, &view, &mut state, &input()), None);
    }
}
