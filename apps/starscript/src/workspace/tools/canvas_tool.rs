//! The Art board tool: select a board (the Canvas or an Art board) by its edge or label, then
//! resize it from its handles, move it by its label, or draw a new Art board on empty
//! pasteboard.

use eframe::egui;
use ssmap_core::{Canvas, Command, Id};
use ssmap_render::View;

use super::{CanvasFrame, EDGE_GRAB, ToolId, ToolState};
use crate::canvas_resize::{self, BoardCreate, BoardMove, CanvasDrag};
use crate::editor::Editor;
use crate::handles::{self, Bounds, Grip};

/// How far the pointer must move, in points, before a drag on empty pasteboard makes a board.
const DRAW_THRESHOLD: f32 = 4.0;

/// A board's box in screen points.
fn board_screen_rect(canvas: &Canvas, view: &View, origin: egui::Pos2, ppp: f64) -> egui::Rect {
    let rect = canvas.rect();
    egui::Rect::from_two_pos(
        handles::to_screen(view, origin, ppp, rect.min),
        handles::to_screen(view, origin, ppp, rect.max),
    )
}

/// What the pointer can hit, worked out once per frame.
struct Hits<'a> {
    view: &'a View,
    origin: egui::Pos2,
    ppp: f64,
    /// In the Map's order: later boards draw over earlier ones, so they are hit first.
    boards: Vec<(Id, Canvas)>,
    selected: Option<Id>,
    /// Where the labels were last drawn.
    labels: Vec<(Id, egui::Rect)>,
}

impl Hits<'_> {
    fn screen(&self, canvas: &Canvas) -> egui::Rect {
        board_screen_rect(canvas, self.view, self.origin, self.ppp)
    }

    fn on_edge(rect: egui::Rect, pos: egui::Pos2) -> bool {
        rect.expand(EDGE_GRAB as f32).contains(pos) && !rect.shrink(EDGE_GRAB as f32).contains(pos)
    }

    fn label_of(&self, id: Id) -> Option<egui::Rect> {
        self.labels
            .iter()
            .find(|(label, _)| *label == id)
            .map(|(_, rect)| *rect)
    }

    /// The topmost board whose edge or label is at `pos`.
    fn board_at(&self, pos: egui::Pos2) -> Option<Id> {
        self.boards
            .iter()
            .rev()
            .find(|(id, canvas)| {
                Self::on_edge(self.screen(canvas), pos)
                    || self.label_of(*id).is_some_and(|l| l.contains(pos))
            })
            .map(|(id, _)| *id)
    }

    /// Whether `pos` is over a board, or within reach of its edge.
    fn inside_a_board(&self, pos: egui::Pos2) -> bool {
        self.boards
            .iter()
            .any(|(_, canvas)| self.screen(canvas).expand(EDGE_GRAB as f32).contains(pos))
    }

    fn selected_canvas(&self) -> Option<&Canvas> {
        let selected = self.selected?;
        self.boards
            .iter()
            .find(|(board, _)| *board == selected)
            .map(|(_, canvas)| canvas)
    }

    /// The resize handle of the selected board at `pos`.
    fn grip_at(&self, pos: egui::Pos2) -> Option<handles::Handle> {
        let canvas = self.selected_canvas()?;
        let bounds = Bounds {
            min: canvas.origin,
            max: canvas.rect().max,
        };
        match handles::hit(&bounds, self.view, self.origin, self.ppp, pos, false) {
            Some(Grip::Resize(handle)) => Some(handle),
            _ => None,
        }
    }

    fn on_selected_label(&self, pos: egui::Pos2) -> bool {
        self.selected
            .and_then(|id| self.label_of(id))
            .is_some_and(|label| label.contains(pos))
    }
}

/// The Art board tool. Click a board's edge or label to select it (not its inside, so Objects
/// on it are not selected by accident), then drag a handle to resize it (Shift keeps the
/// aspect ratio, Alt resizes from the centre) or its label to move it. Dragging on empty
/// pasteboard draws a new Art board. Each of these is one Undo step and none touches an
/// Object.
pub(super) fn canvas_tool(
    editor: &mut Editor,
    view: &View,
    state: &mut ToolState,
    input: &CanvasFrame,
) -> Option<egui::CursorIcon> {
    let hits = Hits {
        view,
        origin: input.rect_min,
        ppp: input.pixels_per_point,
        boards: editor
            .map()
            .boards()
            .iter()
            .map(|board| (board.id, board.canvas.clone()))
            .collect(),
        selected: editor.selected_board(),
        labels: state.board_labels.clone(),
    };
    let cursor = hover_cursor(state, &hits, input);
    if input.drag_started
        && !editor.gesture_open()
        && let Some(pos) = input.press_origin
    {
        begin_drag(editor, state, &hits, input, pos);
    }
    continue_drags(editor, state, input, view);
    if input.drag_stopped {
        end_drags(editor, state);
    }
    if input.clicked
        && let Some(pos) = input.pointer_pos
        && hits.grip_at(pos).is_none()
    {
        editor.select_board(hits.board_at(pos));
    }
    cursor
}

/// The cursor for what the pointer is over, while no drag is going.
fn hover_cursor(
    state: &ToolState,
    hits: &Hits<'_>,
    input: &CanvasFrame,
) -> Option<egui::CursorIcon> {
    let idle =
        state.canvas_drag.is_none() && state.board_move.is_none() && state.board_create.is_none();
    let pos = input.hover_pos.filter(|_| idle)?;
    if let Some(handle) = hits.grip_at(pos) {
        Some(handle.cursor())
    } else if hits.on_selected_label(pos) {
        Some(egui::CursorIcon::Grab)
    } else if hits.board_at(pos).is_some() {
        Some(egui::CursorIcon::PointingHand)
    } else if !hits.inside_a_board(pos) {
        Some(egui::CursorIcon::Crosshair)
    } else {
        None
    }
}

/// Starts a resize on a handle of the selected board, a move on its label, or a new board on
/// empty pasteboard.
fn begin_drag(
    editor: &mut Editor,
    state: &mut ToolState,
    hits: &Hits<'_>,
    input: &CanvasFrame,
    pos: egui::Pos2,
) {
    let point = input.canvas_point(hits.view, pos);
    let primary = editor.map().primary_board();
    let selected = hits.selected.zip(hits.selected_canvas());
    if let Some(handle) = hits.grip_at(pos)
        && let Some((id, canvas)) = selected
    {
        let label = if id == primary {
            "Resize Canvas"
        } else {
            "Resize Art board"
        };
        if state.begin_held(editor, ToolId::Canvas, label) {
            state.canvas_drag = Some(CanvasDrag {
                board: id,
                handle,
                start: point,
                origin: canvas.origin,
                size: [canvas.width, canvas.height],
            });
        }
    } else if hits.on_selected_label(pos)
        && let Some((id, canvas)) = selected
    {
        let label = if id == primary {
            "Move Canvas"
        } else {
            "Move Art board"
        };
        if state.begin_held(editor, ToolId::Canvas, label) {
            state.board_move = Some(BoardMove {
                board: id,
                start: point,
                origin: canvas.origin,
                size: [canvas.width, canvas.height],
            });
        }
    } else if hits.board_at(pos).is_none()
        && !hits.inside_a_board(pos)
        && state.begin_held(editor, ToolId::Canvas, "Add Art board")
    {
        state.board_create = Some(BoardCreate {
            board: Id::new(),
            start: point,
            created: false,
        });
    }
}

/// Follows the pointer with the resize, move or new board that is going.
fn continue_drags(editor: &mut Editor, state: &mut ToolState, input: &CanvasFrame, view: &View) {
    if let Some(drag) = state.canvas_drag {
        // Escape or undo ended the gesture under the drag.
        if !state.holds_open(editor) {
            state.canvas_drag = None;
        } else if input.dragging
            && let Some(pos) = input.pointer_pos
        {
            let (origin, width, height) =
                drag.resized(input.canvas_point(view, pos), input.shift, input.alt);
            state.replace_held(
                editor,
                Command::ResizeBoard {
                    id: drag.board,
                    origin,
                    width,
                    height,
                },
            );
        }
    }
    if let Some(moving) = state.board_move {
        if !state.holds_open(editor) {
            state.board_move = None;
        } else if input.dragging
            && let Some(pos) = input.pointer_pos
        {
            state.replace_held(
                editor,
                Command::ResizeBoard {
                    id: moving.board,
                    origin: moving.moved(input.canvas_point(view, pos)),
                    width: moving.size[0],
                    height: moving.size[1],
                },
            );
        }
    }
    if let Some(create) = state.board_create {
        if !state.holds_open(editor) {
            state.board_create = None;
        } else if input.dragging
            && let Some(pos) = input.pointer_pos
        {
            draw_board(editor, state, input, view, create, pos);
        }
    }
}

/// Draws the new board from where the press began to `pos`, once the pointer has moved far
/// enough that it is not a click.
fn draw_board(
    editor: &mut Editor,
    state: &mut ToolState,
    input: &CanvasFrame,
    view: &View,
    create: BoardCreate,
    pos: egui::Pos2,
) {
    let (origin, width, height) = canvas_resize::drawn(create.start, input.canvas_point(view, pos));
    let from = handles::to_screen(view, input.rect_min, input.pixels_per_point, create.start);
    if !create.created && (pos - from).length() < DRAW_THRESHOLD {
        return;
    }
    let command = Command::CreateBoard {
        id: create.board,
        name: None,
        origin,
        width,
        height,
        index: None,
    };
    if state.replace_held(editor, command)
        && let Some(create) = state.board_create.as_mut()
    {
        create.created = true;
    }
}

/// Ends the drag that was going: one Undo step, or none for a click that made no board.
fn end_drags(editor: &mut Editor, state: &mut ToolState) {
    let resized = state.canvas_drag.take().is_some();
    let moved = state.board_move.take().is_some();
    if resized || moved {
        state.end_held(editor);
    }
    if let Some(create) = state.board_create.take() {
        if create.created {
            state.end_held(editor);
            editor.select_board(Some(create.board));
        } else {
            state.cancel_held(editor);
        }
    }
}

/// Draws each board's name above its top-left corner, and the handles of the selected one.
pub(super) fn paint_canvas(
    painter: &egui::Painter,
    editor: &Editor,
    view: &View,
    state: &mut ToolState,
    origin: egui::Pos2,
    ppp: f64,
) {
    let palette = crate::theme::palette(painter.ctx());
    let map = editor.map();
    let selected = editor.selected_board();
    state.board_labels.clear();
    for board in map.boards() {
        let Some(label) = map.board_label(board.id) else {
            continue;
        };
        let screen = board_screen_rect(&board.canvas, view, origin, ppp);
        let color = if selected == Some(board.id) {
            palette.select
        } else {
            palette.text_secondary
        };
        let text = painter.text(
            screen.left_top() - egui::vec2(0.0, 4.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(crate::theme::CAPTION),
            color,
        );
        state.board_labels.push((board.id, text.expand(3.0)));
        if selected == Some(board.id) {
            let bounds = Bounds {
                min: board.canvas.origin,
                max: board.canvas.rect().max,
            };
            handles::paint(painter, &bounds, view, origin, ppp, false, palette.select);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;
    use ssmap_core::Id;

    #[test]
    fn clicking_the_canvas_edge_selects_the_canvas_but_its_inside_does_not() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        let click = |x, y| CanvasFrame {
            clicked: true,
            pointer_pos: Some(at(x, y)),
            ..input()
        };
        canvas_tool(&mut editor, &view, &mut state, &click(250.0, 250.0));
        assert!(!editor.canvas_selected());
        canvas_tool(&mut editor, &view, &mut state, &click(2.0, 250.0));
        assert!(editor.canvas_selected());
        canvas_tool(&mut editor, &view, &mut state, &click(250.0, 250.0));
        assert!(!editor.canvas_selected());
    }

    #[test]
    fn dragging_a_handle_of_the_selected_canvas_resizes_the_canvas() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.select_canvas(true);
        canvas_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        assert!(state.canvas_drag.is_some());
        canvas_tool(&mut editor, &view, &mut state, &drag_to(600.0, 560.0));
        assert_eq!(
            (editor.map().canvas().width, editor.map().canvas().height),
            (600, 560)
        );
        canvas_tool(&mut editor, &view, &mut state, &release(600.0, 560.0));
        assert!(state.canvas_drag.is_none());
        assert!(!editor.gesture_open());
    }

    #[test]
    fn escape_mid_drag_rolls_the_canvas_resize_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.select_canvas(true);
        canvas_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        canvas_tool(&mut editor, &view, &mut state, &drag_to(600.0, 560.0));
        assert_eq!(editor.map().canvas().width, 600);
        assert!(editor.cancel_open_gesture());
        canvas_tool(&mut editor, &view, &mut state, &drag_to(620.0, 580.0));
        assert!(state.canvas_drag.is_none() && state.held.is_none());
        assert_eq!(
            (editor.map().canvas().width, editor.map().canvas().height),
            (500, 500)
        );
        assert!(!editor.gesture_open());
    }

    #[test]
    fn switching_tools_mid_drag_rolls_the_gesture_back() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        editor.select_canvas(true);
        editor.active_tool = ToolId::Canvas;
        canvas_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        canvas_tool(&mut editor, &view, &mut state, &drag_to(600.0, 560.0));
        assert_eq!(editor.map().canvas().width, 600);
        editor.active_tool = ToolId::Select;
        state.drop_held_for_other_tool(&mut editor);
        assert!(state.canvas_drag.is_none() && state.held.is_none());
        assert!(!editor.gesture_open());
        assert_eq!(
            (editor.map().canvas().width, editor.map().canvas().height),
            (500, 500)
        );
        assert_eq!(editor.undo_label(), None);
    }

    #[test]
    fn a_handle_of_an_unselected_canvas_is_not_grabbed() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        canvas_tool(&mut editor, &view, &mut state, &press(500.0, 500.0));
        assert!(state.canvas_drag.is_none());
    }

    /// The Canvas (500 wide at the origin) and an Art board 200 wide at x = 600.
    fn with_art_board() -> (Editor, View, ToolState, Id) {
        let (mut editor, view, state) = (editor(), view(), ToolState::default());
        let board = Id::new();
        editor
            .run(Command::CreateBoard {
                id: board,
                name: None,
                origin: [600.0, 100.0],
                width: 200,
                height: 150,
                index: None,
            })
            .unwrap();
        (editor, view, state, board)
    }

    #[test]
    fn clicking_an_art_boards_edge_selects_it_and_empty_pasteboard_clears() {
        let (mut editor, view, mut state, board) = with_art_board();
        let click = |x, y| CanvasFrame {
            clicked: true,
            pointer_pos: Some(at(x, y)),
            ..input()
        };
        // Inside the Art board: not selected.
        canvas_tool(&mut editor, &view, &mut state, &click(700.0, 170.0));
        assert_eq!(editor.selected_board(), None);
        canvas_tool(&mut editor, &view, &mut state, &click(600.0, 170.0));
        assert_eq!(editor.selected_board(), Some(board));
        assert!(!editor.canvas_selected());
        // Another click on the Canvas edge moves the selection.
        canvas_tool(&mut editor, &view, &mut state, &click(2.0, 250.0));
        assert!(editor.canvas_selected());
        canvas_tool(&mut editor, &view, &mut state, &click(900.0, 900.0));
        assert_eq!(editor.selected_board(), None);
    }

    #[test]
    fn dragging_a_handle_of_a_selected_art_board_resizes_only_that_board() {
        let (mut editor, view, mut state, board) = with_art_board();
        editor.select_board(Some(board));
        // Its bottom-right corner is at (800, 250).
        canvas_tool(&mut editor, &view, &mut state, &press(800.0, 250.0));
        assert!(state.canvas_drag.is_some());
        canvas_tool(&mut editor, &view, &mut state, &drag_to(850.0, 300.0));
        canvas_tool(&mut editor, &view, &mut state, &release(850.0, 300.0));
        let resized = &editor.map().board(board).unwrap().canvas;
        assert_eq!((resized.width, resized.height), (250, 200));
        assert_eq!(resized.origin, [600.0, 100.0]);
        assert_eq!(editor.map().canvas().width, 500, "the Canvas is untouched");
        assert_eq!(editor.undo_label(), Some("Resize Art board"));
        assert!(!editor.gesture_open());
    }

    #[test]
    fn dragging_the_selected_boards_label_moves_it() {
        let (mut editor, view, mut state, board) = with_art_board();
        editor.select_board(Some(board));
        state.board_labels.push((
            board,
            egui::Rect::from_min_size(at(600.0, 80.0), egui::vec2(60.0, 14.0)),
        ));
        canvas_tool(&mut editor, &view, &mut state, &press(620.0, 87.0));
        assert!(state.board_move.is_some());
        canvas_tool(&mut editor, &view, &mut state, &drag_to(640.0, 117.0));
        canvas_tool(&mut editor, &view, &mut state, &release(640.0, 117.0));
        let moved = &editor.map().board(board).unwrap().canvas;
        assert_eq!(moved.origin, [620.0, 130.0]);
        assert_eq!((moved.width, moved.height), (200, 150), "size is kept");
        assert_eq!(editor.undo_label(), Some("Move Art board"));
    }

    #[test]
    fn dragging_on_empty_pasteboard_draws_a_new_art_board_in_one_undo_step() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        canvas_tool(&mut editor, &view, &mut state, &press(600.0, 100.0));
        assert!(state.board_create.is_some());
        // Not yet far enough: no board.
        canvas_tool(&mut editor, &view, &mut state, &drag_to(601.0, 101.0));
        assert_eq!(editor.map().boards().len(), 1);
        canvas_tool(&mut editor, &view, &mut state, &drag_to(700.0, 180.0));
        canvas_tool(&mut editor, &view, &mut state, &drag_to(800.0, 220.0));
        assert_eq!(editor.map().boards().len(), 2);
        canvas_tool(&mut editor, &view, &mut state, &release(800.0, 220.0));
        let board = editor.map().boards()[1].clone();
        assert_eq!(board.canvas.origin, [600.0, 100.0]);
        assert_eq!((board.canvas.width, board.canvas.height), (200, 120));
        assert_eq!(editor.selected_board(), Some(board.id));
        assert_eq!(editor.undo_label(), Some("Add Art board"));
        editor.undo();
        assert_eq!(editor.map().boards().len(), 1);
    }

    #[test]
    fn a_click_on_empty_pasteboard_makes_no_board() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        canvas_tool(&mut editor, &view, &mut state, &press(600.0, 100.0));
        canvas_tool(&mut editor, &view, &mut state, &release(600.0, 100.0));
        assert_eq!(editor.map().boards().len(), 1);
        assert!(!editor.gesture_open());
        assert_eq!(editor.undo_label(), None);
    }

    #[test]
    fn a_press_inside_a_board_starts_no_board() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        canvas_tool(&mut editor, &view, &mut state, &press(250.0, 250.0));
        assert!(state.board_create.is_none() && state.canvas_drag.is_none());
    }

    #[test]
    fn the_hud_names_the_board_being_drawn() {
        let (mut editor, view, mut state) = (editor(), view(), ToolState::default());
        canvas_tool(&mut editor, &view, &mut state, &press(600.0, 100.0));
        canvas_tool(&mut editor, &view, &mut state, &drag_to(800.0, 220.0));
        assert_eq!(
            state.canvas_hud(&editor).as_deref(),
            Some("Art board 1 · 200 × 120")
        );
    }
}
