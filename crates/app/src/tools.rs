//! The tool framework. The active tool is editor state; each tool turns Canvas input into
//! view changes or Commands. Object tools (stamps, labels) arrive with their tickets and
//! register here.

use eframe::egui;
use lyon::math::point;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};
use ssmap_core::land::{self, BrushStroke};
use ssmap_core::{Affine, Command, MultiPolygon, Point};
use ssmap_render::View;

use crate::editor::Editor;
use crate::handles::{self, Bounds, Handle};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolId {
    Select,
    Pan,
    LandBrush,
}

impl ToolId {
    pub const ALL: [ToolId; 3] = [ToolId::Select, ToolId::Pan, ToolId::LandBrush];

    pub fn label(self) -> &'static str {
        match self {
            ToolId::Select => "Select",
            ToolId::Pan => "Pan",
            ToolId::LandBrush => "Land Brush",
        }
    }

    pub fn shortcut(self) -> egui::Key {
        match self {
            ToolId::Select => egui::Key::V,
            ToolId::Pan => egui::Key::H,
            ToolId::LandBrush => egui::Key::B,
        }
    }
}

/// The modifier that turns the land brush into an eraser (Option on macOS).
pub const ERASE_MODIFIER: &str = if cfg!(target_os = "macos") {
    "Option"
} else {
    "Alt"
};

/// The land brush size range, as a diameter in Canvas units.
pub const BRUSH_SIZES: std::ops::RangeInclusive<f64> = 2.0..=2000.0;

/// What a tool sees of one frame of Canvas input.
pub struct CanvasInput<'a> {
    pub response: &'a egui::Response,
    /// Physical pixels per egui point.
    pub pixels_per_point: f64,
}

impl CanvasInput<'_> {
    /// The Canvas point under a screen position.
    fn canvas_point(&self, view: &View, pos: egui::Pos2) -> [f64; 2] {
        let local = pos - self.response.rect.min;
        view.to_canvas([
            f64::from(local.x) * self.pixels_per_point,
            f64::from(local.y) * self.pixels_per_point,
        ])
    }
}

/// Tool state that lasts across frames, such as a drag in progress.
#[derive(Default)]
pub struct ToolState {
    drag: Option<Drag>,
    stroke: Option<Stroke>,
}

/// A land brush stroke in progress. Nothing changes in the Map until it is released; until
/// then an overlay shows the area it covers.
struct Stroke {
    /// The pointer's path in Canvas units, from where the button went down.
    points: Vec<Point>,
    erase: bool,
    /// The overlay as last computed, and how many points it covers.
    preview: Option<(usize, MultiPolygon)>,
}

/// A Select-tool drag. Each frame re-issues the whole move or resize from where it started.
struct Drag {
    /// The Canvas point where the drag began.
    start: [f64; 2],
    kind: DragKind,
}

enum DragKind {
    Move,
    Resize { handle: Handle, bounds: Bounds },
}

/// Lets the active tool handle Canvas input. Panning with the middle button, or with Space
/// held, works whatever the tool.
pub fn canvas_input(
    editor: &mut Editor,
    view: &mut View,
    state: &mut ToolState,
    input: &CanvasInput<'_>,
) {
    let response = input.response;
    let space_held = response.ctx.input(|i| i.key_down(egui::Key::Space));
    let temporary_pan = response.dragged_by(egui::PointerButton::Middle)
        || (space_held && response.dragged_by(egui::PointerButton::Primary));
    if temporary_pan || (editor.active_tool == ToolId::Pan && response.dragged()) {
        let delta = response.drag_delta();
        view.pan([
            f64::from(delta.x) * input.pixels_per_point,
            f64::from(delta.y) * input.pixels_per_point,
        ]);
        return;
    }
    match editor.active_tool {
        ToolId::Select => select_tool(editor, view, state, input),
        ToolId::Pan => {}
        ToolId::LandBrush => land_brush(editor, view, state, input),
    }
}

/// Paint land by dragging, or erase with the erase modifier held. Each stroke is one Undo
/// step, made on release; Escape drops a stroke in progress. The brush rules (what merges,
/// what is created, what is erased) live in [`land::brush_command`].
fn land_brush(editor: &mut Editor, view: &View, state: &mut ToolState, input: &CanvasInput<'_>) {
    let response = input.response;
    let radius = editor.brush_size / 2.0;
    let erase_held = response.ctx.input(|i| i.modifiers.alt);
    if response.hovered() {
        response.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    }

    // A click is a stroke of one point: a single dab.
    let pressed = response.drag_started_by(egui::PointerButton::Primary)
        || response.clicked_by(egui::PointerButton::Primary);
    if pressed && state.stroke.is_none() && !editor.gesture_open() {
        let origin = response.ctx.input(|i| i.pointer.press_origin());
        if let Some(origin) = origin.or(response.interact_pointer_pos()) {
            editor.begin_gesture(editor.brush_label(erase_held));
            state.stroke = Some(Stroke {
                points: vec![input.canvas_point(view, origin)],
                erase: erase_held,
                preview: None,
            });
        }
    }

    let Some(stroke) = &mut state.stroke else {
        return;
    };
    // Escape rolled the gesture back: drop the stroke.
    if !editor.gesture_open() {
        state.stroke = None;
        return;
    }
    if let Some(pos) = response.interact_pointer_pos() {
        let point = input.canvas_point(view, pos);
        // A tenth of the radius, but at least a screen pixel, keeps the path light.
        let spacing = (radius * 0.1).max(1.0 / view.zoom);
        let last = *stroke.points.last().expect("a stroke starts with a point");
        if (point[0] - last[0]).hypot(point[1] - last[1]) >= spacing {
            stroke.points.push(point);
        }
    }

    let released = response.drag_stopped() || response.clicked();
    if released && let Some(stroke) = state.stroke.take() {
        let tolerance = land::simplify_tolerance(radius, 1.0 / view.zoom);
        editor.finish_brush_stroke(BrushStroke {
            region: land::brush_region(&stroke.points, radius, tolerance),
            start: stroke.points[0],
            erase: stroke.erase,
        });
    }
}

/// Draws what the active tool shows over the Canvas: the land brush's outline under the
/// pointer, and the area a stroke in progress covers.
pub fn paint_overlay(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &mut ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    if editor.active_tool != ToolId::LandBrush {
        return;
    }
    let erase_held = painter.ctx().input(|i| i.modifiers.alt);
    let erasing = state.stroke.as_ref().map_or(erase_held, |s| s.erase);
    let (fill, line) = if erasing {
        (
            egui::Color32::from_rgba_unmultiplied(200, 60, 50, 70),
            egui::Color32::from_rgb(200, 60, 50),
        )
    } else {
        (
            egui::Color32::from_rgba_unmultiplied(216, 196, 154, 140),
            egui::Color32::from_rgb(90, 70, 40),
        )
    };
    let radius = editor.brush_size / 2.0;
    if let Some(stroke) = &mut state.stroke {
        let preview = match &stroke.preview {
            Some((count, region)) if *count == stroke.points.len() => region,
            _ => {
                let region = land::stroke_region(&stroke.points, radius);
                &stroke.preview.insert((stroke.points.len(), region)).1
            }
        };
        let to_screen = |p: Point| crate::handles::to_screen(view, origin, ppp, p);
        painter.add(egui::Shape::mesh(region_mesh(preview, fill, to_screen)));
        for ring in preview.0.iter().flat_map(|polygon| polygon.rings()) {
            let points = ring.iter().map(|&p| to_screen(p)).collect();
            painter.add(egui::Shape::closed_line(
                points,
                egui::Stroke::new(1.0, line),
            ));
        }
    }
    if let Some(pos) = hover {
        let screen_radius = (radius * view.zoom / ppp) as f32;
        painter.circle_stroke(
            pos,
            screen_radius,
            egui::Stroke::new(1.5, egui::Color32::WHITE),
        );
        painter.circle_stroke(pos, screen_radius, egui::Stroke::new(0.75, line));
    }
}

/// A filled region as an egui mesh in screen points. Holes stay open (even-odd).
fn region_mesh(
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

/// Click to select (Shift toggles), drag a selected Object to move it, drag a handle to
/// resize. Each drag is one Undo step, and Escape cancels it.
fn select_tool(editor: &mut Editor, view: &View, state: &mut ToolState, input: &CanvasInput<'_>) {
    let response = input.response;
    let ppp = input.pixels_per_point;
    let shift = response.ctx.input(|i| i.modifiers.shift);
    let bounds = Bounds::of_selection(&editor.map, editor.selection());
    let handle_at =
        |pos: egui::Pos2| bounds.and_then(|b| handles::hit(&b, view, response.rect.min, ppp, pos));

    if let Some(pos) = response.hover_pos()
        && state.drag.is_none()
        && let Some(handle) = handle_at(pos)
    {
        response.ctx.set_cursor_icon(handle.cursor());
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
    {
        let hit = editor.pick(input.canvas_point(view, pos));
        let selection = match hit {
            Some(id) if shift && editor.is_selected(id) => editor
                .selection()
                .iter()
                .copied()
                .filter(|&s| s != id)
                .collect(),
            Some(id) if shift => {
                let mut selection = editor.selection().to_vec();
                selection.push(id);
                selection
            }
            Some(id) => vec![id],
            None if shift => editor.selection().to_vec(),
            None => Vec::new(),
        };
        editor.select(selection);
    }

    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
        && !editor.gesture_open()
    {
        // `press_origin` is where the button went down, before the drag threshold.
        let origin = response
            .ctx
            .input(|i| i.pointer.press_origin())
            .unwrap_or(pos);
        let start_point = input.canvas_point(view, origin);
        state.drag = match (handle_at(origin), bounds) {
            (Some(handle), Some(bounds)) => Some(Drag {
                start: start_point,
                kind: DragKind::Resize { handle, bounds },
            }),
            _ => match editor.pick(start_point) {
                Some(id) => {
                    if !editor.is_selected(id) {
                        editor.select(vec![id]);
                    }
                    Some(Drag {
                        start: start_point,
                        kind: DragKind::Move,
                    })
                }
                None => None,
            },
        };
        if let Some(drag) = &state.drag {
            editor.begin_gesture(match drag.kind {
                DragKind::Move => "Move",
                DragKind::Resize { .. } => "Resize",
            });
        }
    }

    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(drag) = &state.drag
        && let Some(pos) = response.interact_pointer_pos()
    {
        // Escape rolled the gesture back: ignore the rest of this drag.
        if !editor.gesture_open() {
            state.drag = None;
            return;
        }
        let point = input.canvas_point(view, pos);
        let delta = [point[0] - drag.start[0], point[1] - drag.start[1]];
        let ids = editor.selected_outlines();
        let command = match &drag.kind {
            DragKind::Move => Some(Command::Translate { ids, offset: delta }),
            DragKind::Resize { handle, bounds } => {
                let resized = bounds.resized(*handle, delta);
                let transform = Affine::rect_to_rect(&bounds.rect(), &resized.rect());
                // At the opposite edge the box has no size; keep the last frame's shape.
                transform
                    .is_invertible()
                    .then_some(Command::Transform { ids, transform })
            }
        };
        if let Some(command) = command {
            editor.gesture_replace(command);
        }
    }

    if response.drag_stopped() && state.drag.take().is_some() {
        editor.end_gesture();
    }
}
