//! The land and water brushes.

use eframe::egui;
use lyon::math::point;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};
use ssmap_core::land::{self, BrushStroke};
use ssmap_core::{MultiPolygon, Point};
use ssmap_render::View;

use super::{CanvasFrame, ToolId, ToolState};
use crate::editor::Editor;

/// A land brush stroke in progress. Nothing changes in the Map until it is released; until
/// then an overlay shows the area it covers.
pub(super) struct Stroke {
    /// The pointer's path in Canvas units, from where the button went down.
    pub(super) points: Vec<Point>,
    pub(super) erase: bool,
    /// The overlay as last computed, and how many points it covers.
    pub(super) preview: Option<(usize, MultiPolygon)>,
}

/// Paint land by dragging, or erase with the erase modifier held. Each stroke is one Undo
/// step, made on release; Escape drops a stroke in progress. The brush rules (what merges,
/// what is created, what is erased) live in [`land::brush_command`], or with `water`, in
/// [`land::water_command`].
pub(super) fn land_brush(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
    water: bool,
) -> Option<egui::CursorIcon> {
    let radius = editor.brush_size / 2.0;
    let erase_held = input.alt != editor.erase_default;
    let cursor = input.hovered.then_some(egui::CursorIcon::Crosshair);

    // A click is a stroke of one point: a single dab.
    let pressed = input.drag_started || input.clicked;
    if pressed
        && state.stroke.is_none()
        && !editor.gesture_open()
        && let Some(origin) = input.press_origin.or(input.pointer_pos)
    {
        let label = if water {
            Editor::water_label(erase_held)
        } else {
            editor.brush_label(erase_held)
        };
        let tool = editor.active_tool;
        if state.begin_held(editor, tool, label) {
            state.stroke = Some(Stroke {
                points: vec![input.canvas_point(view, origin)],
                erase: erase_held,
                preview: None,
            });
        }
    }

    // Escape rolled the gesture back: drop the stroke.
    if state.stroke.is_some() && !state.holds_open(editor) {
        state.stroke = None;
    }
    let Some(stroke) = &mut state.stroke else {
        return cursor;
    };
    if let Some(pos) = input.pointer_pos {
        let point = input.canvas_point(view, pos);
        // A tenth of the radius, but at least a screen pixel, keeps the path light.
        let spacing = (radius * 0.1).max(1.0 / view.zoom);
        if let Some(&last) = stroke.points.last()
            && (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing
        {
            stroke.points.push(point);
        }
    }

    let tolerance = land::simplify_tolerance(radius, 1.0 / view.zoom);
    // Irregularity is the land brush's; the water brush paints round strokes.
    let irregularity = if water {
        0.0
    } else {
        editor.brush_irregularity
    };
    let finished = |stroke: &Stroke| {
        let start = stroke.points.first().copied().unwrap_or_default();
        BrushStroke {
            // Seeded by where the stroke starts, so the preview and the commit agree.
            region: land::irregular_region(
                &stroke.points,
                radius,
                tolerance,
                irregularity,
                seed_at(start),
            ),
            start,
            erase: stroke.erase,
        }
    };
    let released = input.drag_stopped || input.clicked;
    if !released
        && stroke
            .preview
            .as_ref()
            .is_none_or(|(n, _)| *n != stroke.points.len())
    {
        // The shore the release would commit, rough as it will be, once per change.
        let area = editor
            .brush_preview(finished(stroke), water)
            .unwrap_or_default();
        stroke.preview = Some((stroke.points.len(), area));
    }
    if released && let Some(stroke) = state.stroke.take() {
        let finished = finished(&stroke);
        if let Some(id) = state.take_held() {
            if water {
                editor.finish_water_stroke(id, finished);
            } else {
                editor.finish_brush_stroke(id, finished);
            }
        }
    }
    cursor
}

/// A seed for a stroke's Irregularity from where it starts, so each dab gets its own shape.
fn seed_at(start: Point) -> u32 {
    let bits = start[0].to_bits() ^ start[1].to_bits().rotate_left(32);
    (bits ^ (bits >> 32)) as u32
}

/// Draws the brush's outline under the pointer, and the area a stroke in progress covers.
pub(super) fn paint_brush(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    if !matches!(editor.active_tool, ToolId::LandBrush | ToolId::WaterBrush) {
        return;
    }
    let palette = crate::theme::palette(painter.ctx());
    let erase_held = painter.ctx().input(|i| i.modifiers.alt) != editor.erase_default;
    let erasing = state.stroke.as_ref().map_or(erase_held, |s| s.erase);
    let (fill, line) = if erasing {
        (palette.danger.gamma_multiply(0.3), palette.danger)
    } else {
        (palette.gold.gamma_multiply(0.25), palette.gold)
    };
    let radius = editor.brush_size / 2.0;
    if let Some(stroke) = &state.stroke
        && let Some((_, preview)) = &stroke.preview
    {
        let to_screen = |p: Point| crate::handles::to_screen(view, origin, ppp, p);
        painter.add(egui::Shape::mesh(region_mesh(preview, fill, to_screen)));
        // A thin guide that follows the rough edge without hiding it.
        let guide = egui::Stroke::new(1.0, line.gamma_multiply(0.7));
        for ring in preview.0.iter().flat_map(|polygon| polygon.rings()) {
            let mut points: Vec<egui::Pos2> = ring.iter().map(|&p| to_screen(p)).collect();
            if let Some(&first) = points.first() {
                points.push(first);
            }
            painter.add(egui::Shape::line(points, guide));
        }
    }
    if let Some(pos) = hover {
        brush_ring(painter, pos, (radius * view.zoom / ppp) as f32);
    }
}

/// The brush cursor: a white ring over a dark one, so it shows on any map colour.
pub(super) fn brush_ring(painter: &egui::Painter, pos: egui::Pos2, radius: f32) {
    painter.circle_stroke(
        pos,
        radius,
        egui::Stroke::new(3.0, egui::Color32::from_black_alpha(140)),
    );
    painter.circle_stroke(pos, radius, egui::Stroke::new(1.25, egui::Color32::WHITE));
}

/// A filled region as an egui mesh in screen points. Holes stay open (even-odd).
pub(super) fn region_mesh(
    region: &MultiPolygon,
    color: egui::Color32,
    to_screen: impl Fn(Point) -> egui::Pos2,
) -> egui::Mesh {
    let mut builder = Path::builder();
    for ring in region.0.iter().flat_map(|polygon| polygon.rings()) {
        let mut points = ring.iter().map(|&p| {
            let screen = to_screen(p);
            point(screen.x, screen.y)
        });
        let Some(first) = points.next() else {
            continue;
        };
        builder.begin(first);
        for p in points {
            builder.line_to(p);
        }
        builder.close();
    }
    let mut buffers: VertexBuffers<egui::Pos2, u32> = VertexBuffers::new();
    let options = FillOptions::tolerance(0.25).with_fill_rule(FillRule::EvenOdd);
    let tessellated = FillTessellator::new().tessellate_path(
        &builder.build(),
        &options,
        &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| {
            let p = vertex.position();
            egui::pos2(p.x, p.y)
        }),
    );
    let mut mesh = egui::Mesh::default();
    if tessellated.is_ok() {
        mesh.indices = buffers.indices;
        mesh.vertices = buffers
            .vertices
            .into_iter()
            .map(|pos| egui::epaint::Vertex {
                pos,
                uv: egui::epaint::WHITE_UV,
                color,
            })
            .collect();
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::super::scatter::scatter_tool;
    use super::super::test_util::*;
    use super::*;

    #[test]
    fn a_land_brush_drag_is_one_undo_step_made_on_release() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        land_brush(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        assert!(editor.gesture_open() && state.stroke.is_some());
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &drag_to(200.0, 150.0),
            false,
        );
        assert!(editor.gesture_open());
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &release(200.0, 150.0),
            false,
        );
        assert!(!editor.gesture_open() && state.stroke.is_none());
        assert_eq!(editor.undo_label(), Some("Paint Land"));
        assert_eq!(editor.history_steps().1, before + 1);
    }

    #[test]
    fn a_land_brush_click_starts_and_ends_a_stroke_in_one_frame() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &click_at(250.0, 250.0),
            false,
        );
        // A lone point has no outline to paint, so no Undo step is left, and nothing is held.
        assert!(!editor.gesture_open() && state.stroke.is_none());
        assert_eq!(editor.history_steps().1, before);
    }

    #[test]
    fn irregularity_shapes_land_strokes_and_zero_paints_them_round() {
        let outline = |irregularity: f64| {
            let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
            editor.brush_irregularity = irregularity;
            paint_stroke(&mut editor, &view, &mut state, false);
            editor
                .map()
                .draw_order()
                .into_iter()
                .find_map(|id| match editor.map().node(id) {
                    Some(ssmap_core::Node::Object(o)) => o.kind.geometry().cloned(),
                    _ => None,
                })
                .unwrap()
        };
        let points = [[100.0, 100.0], [150.0, 120.0], [200.0, 150.0]];
        let radius = crate::editor::DEFAULT_BRUSH_SIZE / 2.0;
        let round = land::brush_region(&points, radius, land::simplify_tolerance(radius, 1.0));
        assert_eq!(outline(0.0), round);
        assert_ne!(outline(crate::editor::DEFAULT_BRUSH_IRREGULARITY), round);
    }

    #[test]
    fn the_water_brush_paints_water() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        paint_stroke(&mut editor, &view, &mut state, true);
        assert_eq!(editor.undo_label(), Some("Paint Water"));
    }

    #[test]
    fn the_erase_modifier_flips_the_land_brush_to_erasing() {
        let (mut editor, view, mut state) = (land_editor(), view(), ToolState::default());
        paint_stroke(&mut editor, &view, &mut state, false);
        let erase = |input: CanvasFrame| CanvasFrame { alt: true, ..input };
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &erase(press(120.0, 110.0)),
            false,
        );
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &erase(drag_to(180.0, 140.0)),
            false,
        );
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &erase(release(180.0, 140.0)),
            false,
        );
        assert_eq!(editor.undo_label(), Some("Erase Land"));
    }

    #[test]
    fn escape_mid_stroke_drops_the_land_brush_stroke() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        land_brush(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        assert!(editor.cancel_open_gesture());
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &drag_to(200.0, 150.0),
            false,
        );
        assert!(state.stroke.is_none());
        land_brush(
            &mut editor,
            &view,
            &mut state,
            &release(200.0, 150.0),
            false,
        );
        assert!(!editor.gesture_open());
        assert_eq!(editor.history_steps().1, 0);
    }

    #[test]
    fn switching_tools_mid_stroke_rolls_the_land_brush_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.active_tool = ToolId::LandBrush;
        land_brush(&mut editor, &view, &mut state, &press(100.0, 100.0), false);
        assert!(editor.gesture_open());
        editor.active_tool = ToolId::Select;
        state.drop_held_for_other_tool(&mut editor);
        assert!(!editor.gesture_open());
        assert_eq!(editor.history_steps().1, 0);
    }

    #[test]
    fn the_brush_and_scatter_cursors_are_a_crosshair_over_the_canvas() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        let over = Some(egui::CursorIcon::Crosshair);
        assert_eq!(
            land_brush(&mut editor, &view, &mut state, &hovering(), false),
            over
        );
        assert_eq!(
            scatter_tool(&mut editor, &view, &mut state, &hovering(), false),
            over
        );
        assert_eq!(
            land_brush(&mut editor, &view, &mut state, &input(), false),
            None
        );
        assert_eq!(
            scatter_tool(&mut editor, &view, &mut state, &input(), true),
            None
        );
    }
}
