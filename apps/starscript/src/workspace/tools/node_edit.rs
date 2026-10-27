//! Node-edit mode: moving, adding and removing the nodes of a Landmass or Lake.

use eframe::egui;
use ssmap_core::{Command, MultiPolygon, NodeRef, Point};
use ssmap_render::View;

use super::{CanvasFrame, EDGE_GRAB, NODE_GRAB, ToolState};
use crate::editor::Editor;

/// A node drag in progress: where it began, and the outline's shape then.
pub(super) struct NodeDrag {
    start: Point,
    shape: Vec<usize>,
}

/// Past this many nodes in view, unselected ones are thinned out so the Canvas stays legible
/// and responsive.
const MAX_DRAWN_NODES: usize = 3000;

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
pub(super) fn node_edit_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
) -> Option<egui::CursorIcon> {
    let shift = input.shift;
    let mut cursor = None;
    let per_point = input.pixels_per_point / view.zoom;
    let hit_at = |editor: &Editor, pos: egui::Pos2| {
        let point = input.canvas_point(view, pos);
        editor.edited_outline().map(|outline| {
            hit_outline(outline, point, NODE_GRAB * per_point, EDGE_GRAB * per_point)
        })
    };

    if let Some(pos) = input.hover_pos
        && state.node_drag.is_none()
        && let Some(NodeHit::Node(_)) = hit_at(editor, pos)
    {
        cursor = Some(egui::CursorIcon::PointingHand);
    }

    // Drag: move nodes, or box-select.
    if input.drag_started && !editor.gesture_open() {
        let origin = input.press_origin.or(input.pointer_pos);
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
                    let shape = editor.edited_shape();
                    if state.begin_held(editor, super::ToolId::Select, "Move Points") {
                        state.node_drag = Some(NodeDrag { start, shape });
                    }
                }
                _ => state.marquee = Some((start, start)),
            }
        }
    }

    if input.dragging
        && let Some(pos) = input.pointer_pos
    {
        let point = input.canvas_point(view, pos);
        // Escape rolled the gesture back.
        if state.node_drag.is_some() && !state.holds_open(editor) {
            state.node_drag = None;
        }
        if let Some(drag) = &state.node_drag {
            if let Some(edit) = editor.node_edit().cloned() {
                let offset = [point[0] - drag.start[0], point[1] - drag.start[1]];
                state.replace_held(
                    editor,
                    Command::MoveNodes {
                        id: edit.id,
                        nodes: edit.nodes,
                        offset,
                    },
                );
            }
        } else if let Some((_, end)) = &mut state.marquee {
            *end = point;
        }
    }

    if input.drag_stopped {
        if let Some(drag) = state.node_drag.take() {
            state.end_held(editor);
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

    if input.double_clicked
        && let Some(pos) = input.pointer_pos
        && let Some(NodeHit::Edge(edge, point)) = hit_at(editor, pos)
    {
        editor.insert_node(edge, point);
        return cursor;
    }

    if input.clicked
        && let Some(pos) = input.pointer_pos
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
    cursor
}

/// Draws the outline being edited, its nodes (those in view, thinned when crowded) and a
/// box-select in progress.
pub(super) fn paint_nodes(
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
    let select = crate::theme::palette(painter.ctx()).select;
    let line = egui::Stroke::new(1.5, select);

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
        let (fill, edge) = if selected {
            (select, egui::Color32::WHITE)
        } else {
            (egui::Color32::WHITE, select)
        };
        painter.rect_filled(rect, 1.0, fill);
        painter.rect_stroke(
            rect,
            1.0,
            egui::Stroke::new(1.0, edge),
            egui::StrokeKind::Middle,
        );
    }
    // Double-clicking an edge adds a node there: show where, as a ghost + under the pointer.
    let per_point = ppp / view.zoom;
    if hovered.is_none()
        && state.node_drag.is_none()
        && state.marquee.is_none()
        && let Some(pos) = hover
        && let Some(on_edge) = {
            let local = pos - origin;
            let point = view.to_canvas([f64::from(local.x) * ppp, f64::from(local.y) * ppp]);
            match hit_outline(outline, point, NODE_GRAB * per_point, EDGE_GRAB * per_point) {
                NodeHit::Edge(_, on_edge) => Some(on_edge),
                _ => None,
            }
        }
    {
        let at = to_screen(on_edge);
        painter.circle(
            at,
            5.0,
            select.gamma_multiply(0.35),
            egui::Stroke::new(1.0, select),
        );
        let arm = 2.5;
        let plus = egui::Stroke::new(1.25, egui::Color32::WHITE);
        painter.line_segment([at - egui::vec2(arm, 0.0), at + egui::vec2(arm, 0.0)], plus);
        painter.line_segment([at - egui::vec2(0.0, arm), at + egui::vec2(0.0, arm)], plus);
    }
    if let Some((a, b)) = state.marquee {
        let rect = egui::Rect::from_two_pos(to_screen(a), to_screen(b));
        painter.rect_filled(rect, 0.0, select.gamma_multiply(0.16));
        painter.rect_stroke(rect, 0.0, line, egui::StrokeKind::Middle);
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;
    use ssmap_core::{Id, Polygon};

    fn outline() -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            holes: Vec::new(),
        }])
    }

    #[test]
    fn hit_outline_finds_a_node_within_reach() {
        let hit = hit_outline(&outline(), [103.0, 2.0], 5.0, 5.0);
        let node = NodeRef {
            part: 0,
            ring: 0,
            index: 1,
        };
        assert!(matches!(hit, NodeHit::Node(found) if found == node));
    }

    #[test]
    fn hit_outline_prefers_a_node_over_an_edge() {
        // Within reach of both the corner and the two edges meeting there.
        let hit = hit_outline(&outline(), [1.0, 1.0], 5.0, 5.0);
        assert!(matches!(hit, NodeHit::Node(NodeRef { index: 0, .. })));
    }

    #[test]
    fn hit_outline_finds_the_nearest_point_on_an_edge() {
        match hit_outline(&outline(), [50.0, 3.0], 5.0, 5.0) {
            NodeHit::Edge(edge, nearest) => {
                assert_eq!(edge.index, 0);
                assert_eq!(nearest, [50.0, 0.0]);
            }
            _ => panic!("expected an edge hit"),
        }
    }

    #[test]
    fn hit_outline_checks_the_closing_edge() {
        match hit_outline(&outline(), [2.0, 50.0], 5.0, 5.0) {
            NodeHit::Edge(edge, nearest) => {
                assert_eq!(edge.index, 3);
                assert_eq!(nearest, [0.0, 50.0]);
            }
            _ => panic!("expected an edge hit"),
        }
    }

    #[test]
    fn hit_outline_tells_inside_from_outside() {
        let inside = hit_outline(&outline(), [50.0, 50.0], 5.0, 5.0);
        assert!(matches!(inside, NodeHit::Inside));
        let outside = hit_outline(&outline(), [150.0, 50.0], 5.0, 5.0);
        assert!(matches!(outside, NodeHit::Outside));
    }

    /// An Editor in node-edit mode on a Landmass shaped like `outline()`.
    fn node_editor() -> Editor {
        let mut editor = land_editor();
        let layer = editor.map.children(None).first().copied();
        let id = Id::new();
        assert!(
            editor
                .run(Command::CreateLandmass {
                    id,
                    parent: layer.unwrap_or_else(Id::new),
                    index: None,
                    geometry: outline(),
                    fill: None,
                })
                .is_ok()
        );
        editor.enter_node_edit(id);
        editor
    }

    fn selected_nodes(editor: &Editor) -> Vec<NodeRef> {
        editor
            .node_edit()
            .map(|e| e.nodes.clone())
            .unwrap_or_default()
    }

    #[test]
    fn node_edit_click_selects_a_node_and_shift_adds_another() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        node_edit_tool(&mut editor, &view, &mut state, &click_at(100.0, 0.0));
        let first = NodeRef {
            part: 0,
            ring: 0,
            index: 1,
        };
        assert_eq!(selected_nodes(&editor), vec![first]);
        let shift_click = CanvasFrame {
            shift: true,
            ..click_at(100.0, 100.0)
        };
        node_edit_tool(&mut editor, &view, &mut state, &shift_click);
        assert_eq!(selected_nodes(&editor).len(), 2);
    }

    #[test]
    fn node_edit_click_outside_leaves_node_edit() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        node_edit_tool(&mut editor, &view, &mut state, &click_at(300.0, 300.0));
        assert!(editor.node_edit().is_none());
    }

    #[test]
    fn node_edit_double_click_on_an_edge_adds_a_node() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        let double = CanvasFrame {
            double_clicked: true,
            ..click_at(50.0, 1.0)
        };
        node_edit_tool(&mut editor, &view, &mut state, &double);
        assert_eq!(editor.edited_shape(), vec![5]);
    }

    #[test]
    fn node_edit_drag_moves_the_selected_node_in_one_step() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        node_edit_tool(&mut editor, &view, &mut state, &press(100.0, 0.0));
        node_edit_tool(&mut editor, &view, &mut state, &drag_to(120.0, 10.0));
        node_edit_tool(&mut editor, &view, &mut state, &release(120.0, 10.0));
        let moved = editor.edited_outline().map(|o| o.0[0].exterior[1]);
        assert_eq!(moved, Some([120.0, 10.0]));
    }

    #[test]
    fn node_edit_drag_on_empty_space_box_selects() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        node_edit_tool(&mut editor, &view, &mut state, &press(50.0, 50.0));
        node_edit_tool(&mut editor, &view, &mut state, &drag_to(150.0, 150.0));
        node_edit_tool(&mut editor, &view, &mut state, &release(150.0, 150.0));
        assert_eq!(selected_nodes(&editor).len(), 1);
    }

    #[test]
    fn node_edit_hover_over_a_node_asks_for_a_pointing_hand() {
        let mut editor = node_editor();
        let (view, mut state) = (view(), ToolState::default());
        let hover = |x, y| CanvasFrame {
            hovered: true,
            hover_pos: Some(at(x, y)),
            ..input()
        };
        let icon = node_edit_tool(&mut editor, &view, &mut state, &hover(2.0, 2.0));
        assert_eq!(icon, Some(egui::CursorIcon::PointingHand));
        let icon = node_edit_tool(&mut editor, &view, &mut state, &hover(50.0, 50.0));
        assert_eq!(icon, None);
    }

    #[test]
    fn nearest_on_segment_projects_onto_the_segment() {
        let (d, nearest) = nearest_on_segment([0.0, 0.0], [10.0, 0.0], [4.0, 3.0]);
        assert_eq!((d, nearest), (3.0, [4.0, 0.0]));
    }

    #[test]
    fn nearest_on_segment_clamps_to_the_ends() {
        let (d, nearest) = nearest_on_segment([0.0, 0.0], [10.0, 0.0], [-3.0, 4.0]);
        assert_eq!((d, nearest), (5.0, [0.0, 0.0]));
        let (d, nearest) = nearest_on_segment([0.0, 0.0], [10.0, 0.0], [13.0, 4.0]);
        assert_eq!((d, nearest), (5.0, [10.0, 0.0]));
    }

    #[test]
    fn nearest_on_segment_handles_a_zero_length_segment() {
        let (d, nearest) = nearest_on_segment([2.0, 2.0], [2.0, 2.0], [5.0, 6.0]);
        assert_eq!((d, nearest), (5.0, [2.0, 2.0]));
    }
}
