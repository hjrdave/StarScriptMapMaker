//! Art boards from the editor's side: making, duplicating, deleting and promoting them, each
//! one Undo step.

use ssmap_core::{Command, Id, Point};

use super::Editor;

/// The empty space left between a new board and the ones before it, in Canvas units.
const BOARD_GAP: f64 = 256.0;

impl Editor {
    /// Where a new board goes: level with the Canvas's top, to the right of every board.
    fn next_board_origin(&self) -> Point {
        let right = self
            .map
            .boards()
            .iter()
            .map(|board| board.canvas.rect().max[0])
            .fold(f64::MIN, f64::max);
        [right + BOARD_GAP, self.map.canvas().origin[1]]
    }

    /// Adds an Art board the size of the Canvas, to the right of the other boards, and selects
    /// it. `None` if the Command was refused.
    pub fn new_art_board(&mut self) -> Option<Id> {
        let (width, height) = (self.map.canvas().width, self.map.canvas().height);
        let id = Id::new();
        let command = Command::CreateBoard {
            id,
            name: None,
            origin: self.next_board_origin(),
            width,
            height,
            index: None,
        };
        self.in_gesture("Add Art board", |g| {
            if !g.run(command) {
                return Err(());
            }
            g.editor.select_board(Some(id));
            Ok(id)
        })?
        .ok()
    }

    /// Adds a copy of the board `id`, with its size and background but none of its Objects,
    /// to the right of the other boards, and selects it.
    pub fn duplicate_board(&mut self, id: Id) -> Option<Id> {
        let source = self.map.board(id)?.canvas.clone();
        let name = format!("{} copy", self.map.board_label(id)?);
        let copy = Id::new();
        let create = Command::CreateBoard {
            id: copy,
            name: Some(name),
            origin: self.next_board_origin(),
            width: source.width,
            height: source.height,
            index: None,
        };
        let background = Command::SetBoardBackground {
            id: copy,
            background: source.background,
        };
        self.in_gesture("Duplicate Art board", |g| {
            if !g.run(create) || !g.run(background) {
                return Err(());
            }
            g.editor.select_board(Some(copy));
            Ok(copy)
        })?
        .ok()
    }

    /// Deletes the Art board `id`. Its Objects are kept and sit off the boards. The Canvas is
    /// refused, with a message on the status bar.
    pub fn delete_board(&mut self, id: Id) -> bool {
        if id == self.map.primary_board() {
            self.error("The Canvas can’t be deleted. Make another board the Canvas first.");
            return false;
        }
        self.run(Command::DeleteBoard { id }).is_ok()
    }

    /// Makes the board `id` the Canvas, which exports. The old Canvas becomes an Art board and
    /// every Object stays on its board.
    pub fn make_canvas(&mut self, id: Id) -> bool {
        id != self.map.primary_board() && self.run(Command::SetPrimaryBoard { id }).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Canvas, DistanceUnit, Map, Scale};

    fn editor() -> Editor {
        Editor::new(Map::new(Canvas::new(
            500,
            400,
            Scale::new(1.0, DistanceUnit::Miles),
        )))
    }

    #[test]
    fn a_new_board_is_the_canvas_size_to_the_right_and_one_undo_step() {
        let mut editor = editor();
        let board = editor.new_art_board().unwrap();
        let made = &editor.map().board(board).unwrap().canvas;
        assert_eq!((made.width, made.height), (500, 400));
        assert_eq!(made.origin, [500.0 + BOARD_GAP, 0.0]);
        assert_eq!(editor.selected_board(), Some(board));
        assert_eq!(editor.undo_label(), Some("Add Art board"));
        // The next one goes after it.
        let next = editor.new_art_board().unwrap();
        let next = &editor.map().board(next).unwrap().canvas;
        assert_eq!(next.origin[0], 500.0 + BOARD_GAP + 500.0 + BOARD_GAP);
        editor.undo();
        editor.undo();
        assert_eq!(editor.map().boards().len(), 1);
    }

    #[test]
    fn a_duplicate_keeps_size_and_background_but_not_objects() {
        let mut editor = editor();
        let board = editor.new_art_board().unwrap();
        let mut background = editor.map().board(board).unwrap().canvas.background.clone();
        background.water = ssmap_core::Color::rgb(9, 9, 9);
        editor
            .run(Command::SetBoardBackground {
                id: board,
                background: background.clone(),
            })
            .unwrap();
        let copy = editor.duplicate_board(board).unwrap();
        let copy_canvas = &editor.map().board(copy).unwrap().canvas;
        assert_eq!(copy_canvas.background, background);
        assert_eq!(
            editor.map().board_label(copy).as_deref(),
            Some("Art board 1 copy")
        );
        assert_eq!(editor.selected_board(), Some(copy));
        assert_eq!(editor.undo_label(), Some("Duplicate Art board"));
        // Both Commands are one step.
        editor.undo();
        assert!(editor.map().board(copy).is_none());
        assert!(editor.map().board(board).is_some());
    }

    #[test]
    fn deleting_the_canvas_is_refused_and_an_art_board_goes() {
        let mut editor = editor();
        let canvas = editor.map().primary_board();
        assert!(!editor.delete_board(canvas));
        assert_eq!(editor.map().boards().len(), 1);
        let board = editor.new_art_board().unwrap();
        assert!(editor.delete_board(board));
        assert_eq!(editor.map().boards().len(), 1);
        assert_eq!(
            editor.selected_board(),
            None,
            "a deleted board is not selected"
        );
    }

    #[test]
    fn making_a_board_the_canvas_is_one_undo_step_and_keeps_the_selection() {
        let mut editor = editor();
        let board = editor.new_art_board().unwrap();
        assert!(editor.make_canvas(board));
        assert_eq!(editor.map().primary_board(), board);
        assert_eq!(editor.selected_board(), Some(board));
        assert_eq!(editor.undo_label(), Some("Change Canvas"));
        assert!(!editor.make_canvas(board), "it already is the Canvas");
    }
}
