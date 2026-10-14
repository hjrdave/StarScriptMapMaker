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
use ssmap_core::{Affine, Command, MultiPolygon, NodeRef, Point};
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
    node_drag: Option<NodeDrag>,
    /// A box-select in node-edit mode: where it began and where the pointer is, in Canvas units.
    marquee: Option<(Point, Point)>,
}

/// A node drag in progress: where it began, and the outline's shape then.
struct NodeDrag {
    start: Point,
    shape: Vec<usize>,
}

/// How close, in egui points, the pointer must be to grab a node or an edge.
const NODE_GRAB: f64 = 8.0;
const EDGE_GRAB: f64 = 6.0;
/// Past this many nodes in view, unselected ones are thinned out so the Canvas stays legible
/// and responsive.
const MAX_DRAWN_NODES: usize = 3000;

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
    if editor.active_tool != ToolId::Select {
        editor.leave_node_edit();
    }
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
        ToolId::Select if editor.node_edit().is_some() => {
            node_edit_tool(editor, view, state, input);
        }
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

/// Where the pointer is in an outline being edited.
enum NodeHit {
    Node(NodeRef),
    /// On an edge: the edge's first node, and the nearest point on it.
    Edge(NodeRef, Point),
    Inside,
    Outside,
}

fn hit_outline(outline: &MultiPolygon, point: Point, node_reach: f64, edge_reach: f64) -> NodeHit {
    let mut best_node: Option<(f64, NodeRef)> = None;
    let mut best_edge: Option<(f64, NodeRef, Point)> = None;
    for (part, polygon) in outline.0.iter().enumerate() {
        for (ring_number, ring) in polygon.rings().enumerate() {
            for (index, &a) in ring.iter().enumerate() {
                let node = NodeRef {
                    part,
                    ring: ring_number,
                    index,
                };
                let d = (a[0] - point[0]).hypot(a[1] - point[1]);
                if d <= node_reach && best_node.is_none_or(|(best, _)| d < best) {
                    best_node = Some((d, node));
                }
                let b = ring[(index + 1) % ring.len()];
                let (d, nearest) = nearest_on_segment(a, b, point);
                if d <= edge_reach && best_edge.is_none_or(|(best, ..)| d < best) {
                    best_edge = Some((d, node, nearest));
                }
            }
        }
    }
    if let Some((_, node)) = best_node {
        NodeHit::Node(node)
    } else if let Some((_, edge, nearest)) = best_edge {
        NodeHit::Edge(edge, nearest)
    } else if outline.contains(point) {
        NodeHit::Inside
    } else {
        NodeHit::Outside
    }
}

fn nearest_on_segment(a: Point, b: Point, p: Point) -> (f64, Point) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length_squared).clamp(0.0, 1.0)
    };
    let nearest = [a[0] + t * dx, a[1] + t * dy];
    ((p[0] - nearest[0]).hypot(p[1] - nearest[1]), nearest)
}

/// Node-edit mode (double-click a Landmass or Lake): click a node to select it (Shift adds),
/// drag nodes to move them, drag empty space to box-select, double-click an edge to add a node,
/// Delete removes the selected nodes, and Escape or a click outside the shape leaves. Each
/// drag is one Undo step.
fn node_edit_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasInput<'_>,
) {
    let response = input.response;
    let shift = response.ctx.input(|i| i.modifiers.shift);
    let per_point = input.pixels_per_point / view.zoom;
    let hit_at = |editor: &Editor, pos: egui::Pos2| {
        let point = input.canvas_point(view, pos);
        editor.edited_outline().map(|outline| {
            hit_outline(outline, point, NODE_GRAB * per_point, EDGE_GRAB * per_point)
        })
    };

    if let Some(pos) = response.hover_pos()
        && state.node_drag.is_none()
        && let Some(NodeHit::Node(_)) = hit_at(editor, pos)
    {
        response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // Drag: move nodes, or box-select.
    if response.drag_started_by(egui::PointerButton::Primary) && !editor.gesture_open() {
        let origin = response
            .ctx
            .input(|i| i.pointer.press_origin())
            .or(response.interact_pointer_pos());
        if let Some(origin) = origin {
            let start = input.canvas_point(view, origin);
            match hit_at(editor, origin) {
                Some(NodeHit::Node(node)) => {
                    let mut nodes = editor
                        .node_edit()
                        .map(|e| e.nodes.clone())
                        .unwrap_or_default();
                    if !nodes.contains(&node) {
                        if shift {
                            nodes.push(node);
                        } else {
                            nodes = vec![node];
                        }
                        editor.select_nodes(nodes);
                    }
                    state.node_drag = Some(NodeDrag {
                        start,
                        shape: editor.edited_shape(),
                    });
                    editor.begin_gesture("Move Nodes");
                }
                _ => state.marquee = Some((start, start)),
            }
        }
    }

    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
    {
        let point = input.canvas_point(view, pos);
        if let Some(drag) = &state.node_drag {
            if !editor.gesture_open() {
                // Escape rolled the gesture back.
                state.node_drag = None;
            } else if let Some(edit) = editor.node_edit().cloned() {
                let offset = [point[0] - drag.start[0], point[1] - drag.start[1]];
                editor.gesture_replace(Command::MoveNodes {
                    id: edit.id,
                    nodes: edit.nodes,
                    offset,
                });
            }
        } else if let Some((_, end)) = &mut state.marquee {
            *end = point;
        }
    }

    if response.drag_stopped() {
        if let Some(drag) = state.node_drag.take() {
            editor.end_gesture();
            editor.keep_nodes_if_shape(&drag.shape);
        } else if let Some((a, b)) = state.marquee.take() {
            let (min, max) = (
                [a[0].min(b[0]), a[1].min(b[1])],
                [a[0].max(b[0]), a[1].max(b[1])],
            );
            let mut nodes = if shift {
                editor
                    .node_edit()
                    .map(|e| e.nodes.clone())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if let Some(outline) = editor.edited_outline() {
                for (part, polygon) in outline.0.iter().enumerate() {
                    for (ring_number, ring) in polygon.rings().enumerate() {
                        for (index, p) in ring.iter().enumerate() {
                            let node = NodeRef {
                                part,
                                ring: ring_number,
                                index,
                            };
                            if p[0] >= min[0]
                                && p[0] <= max[0]
                                && p[1] >= min[1]
                                && p[1] <= max[1]
                                && !nodes.contains(&node)
                            {
                                nodes.push(node);
                            }
                        }
                    }
                }
            }
            editor.select_nodes(nodes);
        }
    }

    if response.double_clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(NodeHit::Edge(edge, point)) = hit_at(editor, pos)
    {
        editor.insert_node(edge, point);
        return;
    }

    if response.clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
    {
        let current = editor
            .node_edit()
            .map(|e| e.nodes.clone())
            .unwrap_or_default();
        match hit_at(editor, pos) {
            Some(NodeHit::Node(node)) if shift && current.contains(&node) => {
                editor.select_nodes(current.into_iter().filter(|&n| n != node).collect());
            }
            Some(NodeHit::Node(node)) if shift => {
                let mut nodes = current;
                nodes.push(node);
                editor.select_nodes(nodes);
            }
            Some(NodeHit::Node(node)) => editor.select_nodes(vec![node]),
            Some(NodeHit::Edge(..) | NodeHit::Inside) => {
                if !shift {
                    editor.select_nodes(Vec::new());
                }
            }
            Some(NodeHit::Outside) | None => editor.leave_node_edit(),
        }
    }
}

/// Draws the outline being edited, its nodes (those in view, thinned when crowded) and a
/// box-select in progress.
fn paint_nodes(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    let (Some(edit), Some(outline)) = (editor.node_edit(), editor.edited_outline()) else {
        return;
    };
    let to_screen = |p: Point| crate::handles::to_screen(view, origin, ppp, p);
    let area = painter.clip_rect().expand(24.0);
    let line = egui::Stroke::new(1.5, egui::Color32::from_rgb(40, 120, 220));

    let mut visible: Vec<(NodeRef, egui::Pos2)> = Vec::new();
    for (part, polygon) in outline.0.iter().enumerate() {
        for (ring_number, ring) in polygon.rings().enumerate() {
            let mut run: Vec<egui::Pos2> = Vec::new();
            let screen: Vec<egui::Pos2> = ring.iter().map(|&p| to_screen(p)).collect();
            for (index, &a) in screen.iter().enumerate() {
                let b = screen[(index + 1) % screen.len()];
                if area.contains(a) {
                    visible.push((
                        NodeRef {
                            part,
                            ring: ring_number,
                            index,
                        },
                        a,
                    ));
                }
                if area.intersects(egui::Rect::from_two_pos(a, b)) {
                    if run.is_empty() {
                        run.push(a);
                    }
                    run.push(b);
                } else if !run.is_empty() {
                    painter.add(egui::Shape::line(std::mem::take(&mut run), line));
                }
            }
            if !run.is_empty() {
                painter.add(egui::Shape::line(run, line));
            }
        }
    }
    let stride = visible.len().div_ceil(MAX_DRAWN_NODES).max(1);
    let hovered = hover.and_then(|pos| {
        let reach = NODE_GRAB as f32;
        visible
            .iter()
            .find(|(_, p)| p.distance(pos) <= reach)
            .map(|(node, _)| *node)
    });
    for (n, (node, pos)) in visible.iter().enumerate() {
        let selected = edit.nodes.contains(node);
        if !selected && hovered != Some(*node) && n % stride != 0 {
            continue;
        }
        let half = if selected || hovered == Some(*node) {
            4.5
        } else {
            3.0
        };
        let rect = egui::Rect::from_center_size(*pos, egui::vec2(half * 2.0, half * 2.0));
        let fill = if selected {
            egui::Color32::from_rgb(40, 120, 220)
        } else {
            egui::Color32::WHITE
        };
        painter.rect_filled(rect, 1.0, fill);
        painter.rect_stroke(
            rect,
            1.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(40, 120, 220)),
            egui::StrokeKind::Middle,
        );
    }
    if let Some((a, b)) = state.marquee {
        let rect = egui::Rect::from_two_pos(to_screen(a), to_screen(b));
        painter.rect_filled(
            rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(40, 120, 220, 40),
        );
        painter.rect_stroke(rect, 0.0, line, egui::StrokeKind::Middle);
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
    if editor.active_tool == ToolId::Select {
        paint_nodes(painter, editor, view, state, origin, hover, ppp);
    }
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

    if response.double_clicked_by(egui::PointerButton::Primary)
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(id) = editor.pick(input.canvas_point(view, pos))
    {
        editor.enter_node_edit(id);
        return;
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
