//! The Stamp tool and the Stamp eraser.

use eframe::egui;
use ssmap_render::View;

use super::brush::{Stroke, brush_ring};
use super::{CanvasFrame, ToolId, ToolState};
use crate::editor::Editor;

/// The Stamp tool and the Stamp eraser. Stamp: a click places one Stamp, and a drag scatters
/// Stamps along the path. Eraser: a click or drag removes Stamps. On release one Command applies
/// ([`Editor::place_stamp`], [`Editor::scatter_stamps`], [`Editor::erase_stamps`]). Nothing
/// changes in the Map while dragging; Escape drops the stroke. egui's drag threshold separates
/// a slightly shaky click from a drag.
pub(super) fn scatter_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
    erase: bool,
) -> Option<egui::CursorIcon> {
    let cursor = input.hovered.then_some(egui::CursorIcon::Crosshair);
    if std::mem::take(&mut editor.cancel_tool) {
        state.stroke = None;
        return cursor;
    }
    if !erase
        && input.clicked
        && let Some(pos) = input.pointer_pos
    {
        if editor.selected_asset.is_none() {
            editor.error("Choose an Asset in the Asset browser first");
        } else {
            editor.place_stamp(input.canvas_point(view, pos));
        }
        return cursor;
    }
    let pressed = input.drag_started || (erase && input.clicked);
    if pressed
        && state.stroke.is_none()
        && let Some(origin) = input.press_origin.or(input.pointer_pos)
    {
        state.stroke = Some(Stroke {
            points: vec![input.canvas_point(view, origin)],
            erase,
            preview: None,
        });
    }
    let Some(stroke) = &mut state.stroke else {
        return cursor;
    };
    if let Some(pos) = input.pointer_pos {
        let point = input.canvas_point(view, pos);
        let spacing = (editor.brush_size * 0.05).max(1.0 / view.zoom);
        if let Some(&last) = stroke.points.last()
            && (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing
        {
            stroke.points.push(point);
        }
    }
    if (input.drag_stopped || input.clicked)
        && let Some(stroke) = state.stroke.take()
    {
        if erase {
            editor.erase_stamps(&stroke.points);
        } else {
            editor.scatter_stamps(&stroke.points);
        }
    }
    cursor
}

/// The scatter brush's and Stamp eraser's cursor: the brush circle, and the path of a stroke
/// in progress.
pub(super) fn paint_scatter_cursor(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    let line = if editor.active_tool == ToolId::StampEraser {
        egui::Color32::from_rgb(200, 60, 50)
    } else {
        egui::Color32::from_rgb(60, 110, 70)
    };
    let screen_radius = (editor.brush_size / 2.0 * view.zoom / ppp) as f32;
    if let Some(stroke) = &state.stroke {
        let points: Vec<egui::Pos2> = stroke
            .points
            .iter()
            .map(|&p| crate::handles::to_screen(view, origin, ppp, p))
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(2.0, line.gamma_multiply(0.5)),
        ));
    }
    if let Some(pos) = hover {
        brush_ring(painter, pos, screen_radius);
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;

    #[test]
    fn a_scatter_drag_collects_points_and_ends_on_release() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        scatter_tool(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &drag_to(160.0, 100.0),
            false,
        );
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &drag_to(220.0, 100.0),
            false,
        );
        let points = state.stroke.as_ref().map(|s| s.points.len());
        assert!(points.is_some_and(|n| n >= 3), "points: {points:?}");
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &release(220.0, 100.0),
            false,
        );
        assert!(state.stroke.is_none());
    }

    #[test]
    fn a_scatter_click_without_an_asset_asks_for_one_and_changes_nothing() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &click_at(100.0, 100.0),
            false,
        );
        assert!(state.stroke.is_none());
        assert_eq!(editor.history_steps().1, 0);
    }

    #[test]
    fn the_stamp_eraser_starts_a_stroke_on_a_click() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        scatter_tool(&mut editor, &view, &mut state, &press(100.0, 100.0), true);
        assert!(state.stroke.as_ref().is_some_and(|s| s.erase));
        scatter_tool(&mut editor, &view, &mut state, &release(100.0, 100.0), true);
        assert!(state.stroke.is_none());
    }

    #[test]
    fn escape_drops_a_scatter_stroke() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        scatter_tool(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        assert!(state.stroke.is_some());
        editor.cancel_tool = true;
        scatter_tool(
            &mut editor,
            &view,
            &mut state,
            &drag_to(200.0, 100.0),
            false,
        );
        assert!(state.stroke.is_none());
        assert!(!editor.cancel_tool);
    }
}
