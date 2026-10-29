//! The River tool: place a River's points from source to mouth.

use eframe::egui;
use ssmap_core::{Point, River, RiverPoint};
use ssmap_render::View;

use super::brush::{brush_ring, region_mesh};
use super::{CanvasFrame, NODE_GRAB, ToolState};
use crate::editor::Editor;

/// A River being placed: its points so far, and the bends it will have.
pub(super) struct RiverDraft {
    points: Vec<RiverPoint>,
    seed: u32,
    /// The River the preview shows, and the points, seed and bending it was planned from.
    preview: Option<(Vec<RiverPoint>, u32, f64, River)>,
}

impl RiverDraft {
    /// The bends of the seed `step` along, as Shift and the mouse wheel give.
    pub(super) fn step_seed(&mut self, step: i32) {
        self.seed = self.seed.wrapping_add_signed(step);
    }
}

/// Click to place a River's points from source to mouth, each as wide as the tool's width
/// (`[` and `]` change it as you go). Double-click or Enter finishes it, and Escape drops it.
/// Shift snaps the next point to a multiple of 45 degrees from the last, and the erase
/// modifier (Alt) runs the River to it without bends. A River that runs off land ends at
/// the shore. Finishing is one Undo step.
pub(super) fn river_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
) -> Option<egui::CursorIcon> {
    let cursor = input.hovered.then_some(egui::CursorIcon::Crosshair);
    if std::mem::take(&mut editor.cancel_tool) {
        state.river = None;
        return cursor;
    }
    let reach = NODE_GRAB * input.pixels_per_point / view.zoom;
    let place = |draft: Option<&RiverDraft>, pos: egui::Pos2| {
        let mut position = input.canvas_point(view, pos);
        let last = draft.and_then(|d| d.points.last());
        if input.shift
            && let Some(last) = last
        {
            position = snapped(last.position, position);
        }
        let mut point = RiverPoint::new(position, editor.river_width);
        point.straight = input.alt && last.is_some();
        point
    };

    let mut finish = input.enter;
    if (input.clicked || input.double_clicked)
        && let Some(pos) = input.pointer_pos
    {
        let point = place(state.river.as_ref(), pos);
        let draft = state.river.get_or_insert_with(|| RiverDraft {
            points: Vec::new(),
            seed: new_seed(),
            preview: None,
        });
        // The second click of a double-click lands on the point the first one placed.
        let repeat = draft
            .points
            .last()
            .is_some_and(|last| distance(last.position, point.position) <= reach);
        if !repeat {
            draft.points.push(point);
        }
        finish |= input.double_clicked;
    }

    if finish && let Some(draft) = state.river.take() {
        if draft.points.len() < 2 {
            editor.info("A River needs at least two points");
        } else {
            editor.finish_river(&draft.points, draft.seed);
        }
        return cursor;
    }

    // The preview runs on to the pointer, as the next click would.
    let hover = input.hover_pos.map(|pos| place(state.river.as_ref(), pos));
    if let Some(draft) = &mut state.river {
        let mut planned = draft.points.clone();
        if let Some(hover) = hover
            && planned
                .last()
                .is_none_or(|last| distance(last.position, hover.position) > reach)
        {
            planned.push(hover);
        }
        let current = draft
            .preview
            .as_ref()
            .is_some_and(|(points, seed, meander, _)| {
                *points == planned && *seed == draft.seed && *meander == editor.river_meander
            });
        if !current {
            let river = editor.planned_river(&planned, draft.seed);
            draft.preview = Some((planned, draft.seed, editor.river_meander, river));
        }
    }
    cursor
}

/// `point`, moved to the nearest direction from `from` that is a multiple of 45 degrees, at the
/// same distance along it.
fn snapped(from: Point, point: Point) -> Point {
    let (dx, dy) = (point[0] - from[0], point[1] - from[1]);
    let step = std::f64::consts::FRAC_PI_4;
    let angle = (dy.atan2(dx) / step).round() * step;
    let length = dx.hypot(dy);
    [
        from[0] + length * angle.cos(),
        from[1] + length * angle.sin(),
    ]
}

/// A seed for a new River. The River stores it, so nothing needs to replay it.
fn new_seed() -> u32 {
    ssmap_core::Id::new().as_uuid().as_u128() as u32
}

fn distance(a: Point, b: Point) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Draws the River being placed: the River it would make, its points so far, and a ring as
/// wide as the next point under the pointer.
pub(super) fn paint_river(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &ToolState,
    origin: egui::Pos2,
    hover: Option<egui::Pos2>,
    ppp: f64,
) {
    let palette = crate::theme::palette(painter.ctx());
    let to_screen = |p: Point| crate::handles::to_screen(view, origin, ppp, p);
    if let Some(draft) = &state.river {
        if let Some((_, _, _, river)) = &draft.preview {
            let fill = palette.gold.gamma_multiply(0.3);
            painter.add(egui::Shape::mesh(region_mesh(
                river.outline(),
                fill,
                to_screen,
            )));
            let line = egui::Stroke::new(1.0, palette.gold.gamma_multiply(0.8));
            for bank in river.banks() {
                let points: Vec<egui::Pos2> = bank.iter().map(|&p| to_screen(p)).collect();
                painter.add(egui::Shape::line(points, line));
            }
        }
        let points: Vec<egui::Pos2> = draft.points.iter().map(|p| to_screen(p.position)).collect();
        painter.add(egui::Shape::line(
            points.clone(),
            egui::Stroke::new(1.0, palette.gold.gamma_multiply(0.6)),
        ));
        for pos in points {
            painter.circle_filled(pos, 3.5, palette.gold);
        }
    }
    if let Some(pos) = hover {
        let radius = (editor.river_width / 2.0 * view.zoom / ppp) as f32;
        brush_ring(painter, pos, radius.max(2.0));
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;
    use ssmap_core::{Id, Node, ObjectKind};

    fn enter_key() -> CanvasFrame {
        CanvasFrame {
            enter: true,
            ..input()
        }
    }

    fn rivers(editor: &Editor) -> Vec<River> {
        editor
            .map()
            .draw_order()
            .into_iter()
            .filter_map(|id| match editor.map().node(id) {
                Some(Node::Object(object)) => match &object.kind {
                    ObjectKind::River(river) => Some((**river).clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn clicks_place_points_and_enter_adds_the_river_in_one_step() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        for (x, y) in [(100.0, 100.0), (200.0, 150.0), (300.0, 120.0)] {
            river_tool(&mut editor, &view, &mut state, &click_at(x, y));
        }
        assert_eq!(state.river.as_ref().map(|d| d.points.len()), Some(3));
        river_tool(&mut editor, &view, &mut state, &enter_key());
        assert!(state.river.is_none());
        assert_eq!(editor.undo_label(), Some("Add River"));
        assert_eq!(editor.history_steps().1, before + 1);
        let added = rivers(&editor);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].path().len(), 3);
        assert_eq!(
            editor.selected_rivers().len(),
            1,
            "the new River is selected"
        );
    }

    #[test]
    fn a_double_click_places_the_last_point_and_finishes() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        river_tool(&mut editor, &view, &mut state, &click_at(100.0, 100.0));
        river_tool(&mut editor, &view, &mut state, &click_at(250.0, 100.0));
        let double = CanvasFrame {
            double_clicked: true,
            ..click_at(250.0, 100.0)
        };
        river_tool(&mut editor, &view, &mut state, &double);
        assert!(state.river.is_none());
        assert_eq!(rivers(&editor)[0].path().len(), 2);
    }

    #[test]
    fn points_take_the_tool_width_and_alt_makes_a_straight_run() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.river_width = 5.0;
        river_tool(&mut editor, &view, &mut state, &click_at(100.0, 100.0));
        editor.river_width = 20.0;
        let alt_click = CanvasFrame {
            alt: true,
            ..click_at(300.0, 100.0)
        };
        river_tool(&mut editor, &view, &mut state, &alt_click);
        river_tool(&mut editor, &view, &mut state, &enter_key());
        let path = rivers(&editor)[0].path().to_vec();
        assert_eq!((path[0].width, path[1].width), (5.0, 20.0));
        assert!(!path[0].straight && path[1].straight);
    }

    #[test]
    fn shift_snaps_the_next_point_to_45_degrees() {
        let level = snapped([0.0, 0.0], [100.0, 8.0]);
        assert!(level[1].abs() < 1e-9 && level[0] > 99.0, "{level:?}");
        let diagonal = snapped([0.0, 0.0], [70.0, 75.0]);
        assert!((diagonal[0] - diagonal[1]).abs() < 1e-9, "{diagonal:?}");
    }

    #[test]
    fn the_starting_width_scales_with_the_canvas() {
        use crate::editor::default_river_width;
        assert_eq!(
            default_river_width(400),
            8.0,
            "never thinner than the shore effects"
        );
        assert!((default_river_width(4096) - 4096.0 / 150.0).abs() < 1e-9);
        assert_eq!(default_river_width(100_000), 64.0);
        let mut big = ssmap_core::Map::new(ssmap_core::Canvas::new(
            4096,
            3072,
            ssmap_core::Scale::new(0.25, ssmap_core::DistanceUnit::Miles),
        ));
        let layer = Id::new();
        let _ = big.execute(&ssmap_core::Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        });
        let editor = Editor::new(big);
        assert!(editor.river_width > 20.0, "{}", editor.river_width);
        assert_eq!(editor.river_width, editor.default_river_width());
    }

    #[test]
    fn one_point_is_not_a_river_and_escape_drops_a_draft() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        let before = editor.history_steps().1;
        river_tool(&mut editor, &view, &mut state, &click_at(100.0, 100.0));
        river_tool(&mut editor, &view, &mut state, &enter_key());
        assert_eq!(editor.history_steps().1, before);
        river_tool(&mut editor, &view, &mut state, &click_at(100.0, 100.0));
        editor.cancel_tool = true;
        river_tool(&mut editor, &view, &mut state, &input());
        assert!(state.river.is_none());
        assert_eq!(editor.history_steps().1, before);
    }

    #[test]
    fn the_preview_follows_the_pointer_and_the_seed() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        river_tool(&mut editor, &view, &mut state, &click_at(100.0, 100.0));
        let hover = |x, y| CanvasFrame {
            hovered: true,
            hover_pos: Some(at(x, y)),
            ..input()
        };
        river_tool(&mut editor, &view, &mut state, &hover(300.0, 200.0));
        let outline = |state: &ToolState| {
            state
                .river
                .as_ref()
                .and_then(|d| d.preview.as_ref())
                .map(|(_, _, _, river)| river.outline().clone())
        };
        let first = outline(&state).expect("a preview");
        assert!(!first.is_empty());
        if let Some(draft) = &mut state.river {
            draft.step_seed(1);
        }
        river_tool(&mut editor, &view, &mut state, &hover(300.0, 200.0));
        assert_ne!(outline(&state), Some(first));
    }
}
