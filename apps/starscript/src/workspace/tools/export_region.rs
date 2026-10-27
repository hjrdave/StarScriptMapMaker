//! The Export Region tool: draw, move and resize the custom export region.

use eframe::egui;
use ssmap_core::ExportRegion;
use ssmap_render::View;

use super::{CanvasFrame, ToolId, ToolState};
use crate::editor::Editor;
use crate::export_region::{self, RegionDrag};
use crate::handles::{self, Bounds, Grip};

/// The custom export region, if the Map has one.
pub fn custom_region(editor: &Editor) -> Option<Bounds> {
    match &editor.map().export_settings.region {
        ExportRegion::Custom { rect } => Some(Bounds {
            min: rect.min,
            max: rect.max,
        }),
        ExportRegion::Canvas => None,
    }
}

/// Draw the custom export region by dragging on empty Canvas, move it by dragging inside,
/// and resize it from its handles. The region is stored in the Map's export settings as
/// `ExportRegion::Custom` while dragging; Escape puts it back as it was.
pub(super) fn export_region_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
) -> Option<egui::CursorIcon> {
    let ppp = input.pixels_per_point;
    let canvas = [
        f64::from(editor.map().canvas().width),
        f64::from(editor.map().canvas().height),
    ];
    if std::mem::take(&mut editor.cancel_tool)
        && let Some((_, before)) = state.region_drag.take()
    {
        editor.export_settings_mut().region = before;
        return None;
    }
    let existing = custom_region(editor);
    let grip_at = |pos: egui::Pos2| {
        let bounds = existing?;
        handles::hit(&bounds, view, input.rect_min, ppp, pos, false)
    };
    let cursor = input.hovered.then(|| match input.hover_pos {
        Some(pos) if state.region_drag.is_none() => match grip_at(pos) {
            Some(grip) => grip.cursor(),
            None if existing.is_some_and(|b| b.contains(input.canvas_point(view, pos))) => {
                egui::CursorIcon::Move
            }
            None => egui::CursorIcon::Crosshair,
        },
        _ => egui::CursorIcon::Crosshair,
    });
    if input.drag_started
        && state.region_drag.is_none()
        && let Some(pos) = input.press_origin
    {
        let start = input.canvas_point(view, pos);
        let drag = match (existing, grip_at(pos)) {
            (Some(bounds), Some(Grip::Resize(handle))) => RegionDrag::Resize {
                handle,
                start,
                original: bounds.rect(),
            },
            (Some(bounds), _) if bounds.contains(start) => RegionDrag::Move {
                start,
                original: bounds.rect(),
            },
            _ => RegionDrag::Draw { anchor: start },
        };
        state.region_drag = Some((drag, editor.map().export_settings.region.clone()));
    }
    let Some((drag, before)) = state.region_drag.clone() else {
        return cursor;
    };
    if input.dragging
        && let Some(pos) = input.pointer_pos
    {
        let rect = drag.rect(input.canvas_point(view, pos), canvas);
        editor.export_settings_mut().region = ExportRegion::Custom { rect };
    }
    if input.drag_stopped {
        state.region_drag = None;
        if let ExportRegion::Custom { rect } = &editor.map().export_settings.region
            && !export_region::is_usable(rect)
        {
            editor.export_settings_mut().region = before;
        }
    }
    cursor
}

/// Draws the custom export region over the Canvas.
pub(super) fn paint_region(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    origin: egui::Pos2,
    ppp: f64,
) {
    if let Some(bounds) = custom_region(editor) {
        // Editor-only: drawn with egui over the Canvas, so it never reaches an export.
        let accent = crate::theme::palette(painter.ctx()).gold;
        if editor.active_tool == ToolId::ExportRegion {
            handles::paint(painter, &bounds, view, origin, ppp, false, accent);
        } else {
            handles::paint_outline(painter, &bounds, view, origin, ppp, accent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;

    fn bounds(min: [f64; 2], max: [f64; 2]) -> Option<Bounds> {
        Some(Bounds { min, max })
    }

    /// Draws a region from (100, 100) to (300, 300) and lets go.
    fn with_region() -> (Editor, View, ToolState) {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(300.0, 300.0));
        export_region_tool(&mut editor, &view, &mut state, &release(300.0, 300.0));
        (editor, view, state)
    }

    #[test]
    fn dragging_on_empty_canvas_draws_the_export_region() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(300.0, 250.0));
        assert_eq!(
            custom_region(&editor),
            bounds([100.0, 100.0], [300.0, 250.0])
        );
        export_region_tool(&mut editor, &view, &mut state, &release(300.0, 250.0));
        assert!(state.region_drag.is_none());
        assert!(custom_region(&editor).is_some());
    }

    #[test]
    fn a_region_drag_is_kept_inside_the_canvas() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(900.0, -50.0));
        assert_eq!(custom_region(&editor), bounds([100.0, 0.0], [500.0, 100.0]));
    }

    #[test]
    fn a_click_without_a_drag_leaves_no_region() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &release(100.0, 100.0));
        assert_eq!(custom_region(&editor), None);
        assert!(state.region_drag.is_none());
    }

    #[test]
    fn dragging_inside_an_existing_region_moves_it() {
        let (mut editor, view, mut state) = with_region();
        export_region_tool(&mut editor, &view, &mut state, &press(200.0, 200.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(230.0, 220.0));
        assert_eq!(
            custom_region(&editor),
            bounds([130.0, 120.0], [330.0, 320.0])
        );
    }

    #[test]
    fn dragging_a_handle_resizes_the_region() {
        let (mut editor, view, mut state) = with_region();
        // The bottom-right handle.
        export_region_tool(&mut editor, &view, &mut state, &press(300.0, 300.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(350.0, 340.0));
        assert_eq!(
            custom_region(&editor),
            bounds([100.0, 100.0], [350.0, 340.0])
        );
    }

    #[test]
    fn escape_puts_the_previous_region_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        export_region_tool(&mut editor, &view, &mut state, &press(100.0, 100.0));
        export_region_tool(&mut editor, &view, &mut state, &drag_to(300.0, 300.0));
        editor.cancel_tool = true;
        export_region_tool(&mut editor, &view, &mut state, &input());
        assert_eq!(custom_region(&editor), None);
        assert!(state.region_drag.is_none());
    }

    #[test]
    fn the_export_region_cursor_follows_the_pointer() {
        let (mut editor, view, mut state) = with_region();
        let hover = |x, y| CanvasFrame {
            hovered: true,
            hover_pos: Some(at(x, y)),
            ..input()
        };
        let mut cursor = |input| export_region_tool(&mut editor, &view, &mut state, &input);
        assert_eq!(cursor(input()), None);
        assert_eq!(cursor(hover(10.0, 10.0)), Some(egui::CursorIcon::Crosshair));
        assert_eq!(cursor(hover(200.0, 200.0)), Some(egui::CursorIcon::Move));
    }
}
