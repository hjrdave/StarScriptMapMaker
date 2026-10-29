//! The Lasso tool.

use eframe::egui;
use ssmap_core::Point;
use ssmap_core::land::{self, BrushStroke};
use ssmap_render::View;

use super::{CanvasFrame, NODE_GRAB, ToolState};
use crate::editor::Editor;

/// A lasso loop being drawn: freehand while the button is held, or corner by corner.
pub(super) struct Lasso {
    points: Vec<Point>,
    freehand: bool,
    erase: bool,
}

/// The dark line under an in-progress stroke's gold dashes, so they show on any map colour.
const HALO: egui::Stroke = egui::Stroke {
    width: 3.0,
    color: egui::Color32::from_black_alpha(110),
};
/// The dash and gap lengths of an in-progress stroke's outline, in points.
const DASH: f32 = 6.0;
const GAP: f32 = 4.0;

/// Draw a region to add to land, or with the erase modifier to cut out: drag a freehand loop, or
/// click corners and close the loop by clicking the first one, double-clicking or pressing
/// Enter. Escape drops a loop in progress. The region follows the land brush's merge rules.
pub(super) fn lasso_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
) {
    let (alt, enter) = (input.alt, input.enter);
    if std::mem::take(&mut editor.cancel_tool) {
        state.lasso = None;
        return;
    }
    let pointer = input.pointer_pos.map(|pos| input.canvas_point(view, pos));
    let near_first = |lasso: &Lasso, point: Point| {
        let reach = NODE_GRAB * input.pixels_per_point / view.zoom;
        match lasso.points.first() {
            Some(first) if lasso.points.len() >= 3 => {
                (point[0] - first[0]).hypot(point[1] - first[1]) <= reach
            }
            _ => false,
        }
    };

    if input.drag_started
        && state.lasso.is_none()
        && let Some(origin) = input.press_origin
    {
        state.lasso = Some(Lasso {
            points: vec![input.canvas_point(view, origin)],
            freehand: true,
            erase: alt,
        });
    }
    if input.dragging
        && let Some(lasso) = &mut state.lasso
        && lasso.freehand
        && let Some(point) = pointer
    {
        let spacing = 2.0 * input.pixels_per_point / view.zoom;
        if let Some(&last) = lasso.points.last()
            && (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing
        {
            lasso.points.push(point);
        }
    }

    let mut close = false;
    if input.drag_stopped && state.lasso.as_ref().is_some_and(|l| l.freehand) {
        close = true;
    } else if input.clicked
        && let Some(point) = pointer
    {
        match &mut state.lasso {
            None => {
                state.lasso = Some(Lasso {
                    points: vec![point],
                    freehand: false,
                    erase: alt,
                });
            }
            Some(lasso) if near_first(lasso, point) => close = true,
            Some(lasso) => lasso.points.push(point),
        }
    }
    if enter || (input.double_clicked && state.lasso.is_some()) {
        close = true;
    }

    if close && let Some(lasso) = state.lasso.take() {
        let tolerance = if lasso.freehand {
            land::simplify_tolerance(4.0 / view.zoom, 1.0 / view.zoom)
        } else {
            0.0
        };
        let region = land::lasso_region(&lasso.points, tolerance);
        if region.is_empty() {
            editor.info("The loop doesn't enclose any area");
            return;
        }
        let id = editor.begin_gesture(editor.brush_label(lasso.erase));
        editor.finish_brush_stroke(
            id,
            BrushStroke {
                region,
                start: lasso.points.first().copied().unwrap_or_default(),
                erase: lasso.erase,
            },
        );
    }
}

/// Draws a lasso loop in progress: the corners so far, and a line to the pointer.
pub(super) fn paint_lasso(
    painter: &egui::Painter,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    let Some(lasso) = &state.lasso else {
        return;
    };
    let palette = crate::theme::palette(painter.ctx());
    let color = if lasso.erase {
        palette.danger
    } else {
        palette.gold
    };
    let mut points: Vec<egui::Pos2> = lasso
        .points
        .iter()
        .map(|&p| crate::handles::to_screen(view, origin, ppp, p))
        .collect();
    if !lasso.freehand
        && let Some(hover) = hover
    {
        points.push(hover);
    }
    painter.add(egui::Shape::line(points.clone(), HALO));
    painter.extend(egui::Shape::dashed_line(
        &points,
        egui::Stroke::new(1.5, color),
        DASH,
        GAP,
    ));
    if !lasso.freehand {
        for &p in &lasso.points {
            let pos = crate::handles::to_screen(view, origin, ppp, p);
            painter.circle_filled(pos, 3.5, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;

    fn enter_key() -> CanvasFrame {
        CanvasFrame {
            enter: true,
            ..input()
        }
    }

    fn lasso_points(state: &ToolState) -> usize {
        state.lasso.as_ref().map_or(0, |l| l.points.len())
    }

    #[test]
    fn a_freehand_lasso_closes_on_release_into_one_undo_step() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        lasso_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        for (x, y) in [(200.0, 100.0), (200.0, 200.0), (100.0, 200.0)] {
            lasso_tool(&mut editor, &view, &mut state, &drag_to(x, y));
        }
        assert!(state.lasso.as_ref().is_some_and(|l| l.freehand));
        lasso_tool(&mut editor, &view, &mut state, &release(100.0, 200.0));
        assert!(state.lasso.is_none());
        assert_eq!(editor.undo_label(), Some("Raise Land"));
        assert_eq!(editor.history_steps().1, before + 1);
    }

    #[test]
    fn clicking_the_first_corner_closes_a_polygon_lasso() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        for (x, y) in [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0)] {
            lasso_tool(&mut editor, &view, &mut state, &click(x, y));
        }
        assert_eq!(lasso_points(&state), 3);
        lasso_tool(&mut editor, &view, &mut state, &click(100.0, 100.0));
        assert!(state.lasso.is_none());
        assert_eq!(editor.undo_label(), Some("Raise Land"));
    }

    #[test]
    fn enter_closes_a_polygon_lasso_and_alt_makes_it_an_erase() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        for (x, y) in [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0)] {
            let alt_click = CanvasFrame {
                alt: true,
                ..click(x, y)
            };
            lasso_tool(&mut editor, &view, &mut state, &alt_click);
        }
        assert!(state.lasso.as_ref().is_some_and(|l| l.erase));
        lasso_tool(&mut editor, &view, &mut state, &enter_key());
        assert!(state.lasso.is_none());
    }

    #[test]
    fn a_lasso_that_encloses_nothing_changes_nothing() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        lasso_tool(&mut editor, &view, &mut state, &click(100.0, 100.0));
        lasso_tool(&mut editor, &view, &mut state, &enter_key());
        assert!(state.lasso.is_none());
        assert_eq!(editor.history_steps().1, before);
    }

    #[test]
    fn escape_drops_a_lasso_in_progress() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        lasso_tool(&mut editor, &view, &mut state, &click(100.0, 100.0));
        assert_eq!(lasso_points(&state), 1);
        editor.cancel_tool = true;
        lasso_tool(&mut editor, &view, &mut state, &input());
        assert!(state.lasso.is_none());
        assert!(!editor.cancel_tool);
    }
}
