//! The Art boards section of the left sidebar: the Canvas and the Art boards in the order they
//! draw, with the actions on each. Every edit is a Command, so one Undo step.

use eframe::egui;
use ssmap_core::Id;

use crate::editor::Editor;
use crate::properties::BoardRequest;
use crate::theme;

/// The height of a row, in points.
const ROW_HEIGHT: f32 = 24.0;

/// What a row's menu asked for.
enum RowAction {
    Rename,
    Duplicate,
    MakeCanvas,
    Delete,
    Up,
    Down,
}

#[derive(Default)]
pub(crate) struct BoardList {
    /// The Art board whose name is being typed, and the text so far.
    renaming: Option<(Id, String)>,
    /// A board was clicked or made: the window frames it.
    jump: Option<Id>,
    /// Delete asks the window to confirm first.
    requests: Vec<BoardRequest>,
}

impl BoardList {
    /// The board to frame, if one was clicked or made since the last call.
    pub(crate) fn take_jump(&mut self) -> Option<Id> {
        self.jump.take()
    }

    /// Makes `board` the one to frame, as when a new one is added.
    pub(crate) fn jump_to(&mut self, board: Id) {
        self.jump = Some(board);
    }

    /// What the row menus asked the window for since the last call.
    pub(crate) fn take_requests(&mut self) -> Vec<BoardRequest> {
        std::mem::take(&mut self.requests)
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let rows: Vec<(Id, String, bool)> = editor
            .map()
            .boards()
            .iter()
            .filter_map(|board| {
                let label = editor.map().board_label(board.id)?;
                Some((board.id, label, board.id == editor.map().primary_board()))
            })
            .collect();
        let selected = editor.selected_board();
        let last = rows.len().saturating_sub(1);
        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, (id, label, primary)) in rows.iter().enumerate() {
                    if self.renaming.as_ref().is_some_and(|(r, _)| r == id) {
                        self.rename_row(ui, editor, *id, label);
                        continue;
                    }
                    let color = theme::palette(ui.ctx()).text_primary;
                    let row = ui.add_sized(
                        [ui.available_width(), ROW_HEIGHT],
                        egui::Button::selectable(
                            selected == Some(*id),
                            egui::RichText::new(label).color(color),
                        )
                        .frame_when_inactive(false),
                    );
                    if row.clicked() {
                        editor.select_board(Some(*id));
                        self.jump = Some(*id);
                    }
                    if row.double_clicked() && !primary {
                        self.renaming = Some((*id, own_name(editor, *id)));
                    }
                    row.context_menu(|ui| {
                        action = row_menu(ui, *id, *primary, index, last).map(|a| (*id, a));
                    });
                }
            });
        if let Some((id, action)) = action {
            self.apply(editor, id, action);
        }
    }

    /// The text box that renames a board. Enter or leaving the box keeps the name, Esc keeps
    /// the old one; an empty name goes back to the default.
    fn rename_row(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, shown: &str) {
        let Some((_, text)) = self.renaming.as_mut() else {
            return;
        };
        let field = ui.add_sized(
            [ui.available_width(), ROW_HEIGHT],
            egui::TextEdit::singleline(text).hint_text(shown),
        );
        if !field.has_focus() && !field.lost_focus() {
            field.request_focus();
        }
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if escaped {
            self.renaming = None;
        } else if field.lost_focus()
            && let Some((_, text)) = self.renaming.take()
        {
            editor.rename_board(id, Some(text));
        }
    }

    fn apply(&mut self, editor: &mut Editor, id: Id, action: RowAction) {
        match action {
            RowAction::Rename => self.renaming = Some((id, own_name(editor, id))),
            RowAction::Duplicate => {
                if let Some(copy) = editor.duplicate_board(id) {
                    self.jump = Some(copy);
                }
            }
            RowAction::MakeCanvas => {
                editor.make_canvas(id);
            }
            RowAction::Delete => self.requests.push(BoardRequest::Delete(id)),
            RowAction::Up => {
                editor.move_board(id, -1);
            }
            RowAction::Down => {
                editor.move_board(id, 1);
            }
        }
    }
}

/// The name the user gave the board, empty if it has none.
fn own_name(editor: &Editor, id: Id) -> String {
    editor
        .map()
        .board(id)
        .and_then(|board| board.name.clone())
        .unwrap_or_default()
}

/// The right-click menu of a row. The Canvas can't be renamed, made the Canvas again or
/// deleted.
fn row_menu(
    ui: &mut egui::Ui,
    _id: Id,
    primary: bool,
    index: usize,
    last: usize,
) -> Option<RowAction> {
    let mut chosen = None;
    let mut entry = |ui: &mut egui::Ui, enabled: bool, text: &str, action: RowAction| {
        if ui.add_enabled(enabled, egui::Button::new(text)).clicked() {
            chosen = Some(action);
            ui.close();
        }
    };
    entry(ui, !primary, "Rename", RowAction::Rename);
    entry(ui, true, "Duplicate", RowAction::Duplicate);
    entry(ui, !primary, "Set as Canvas", RowAction::MakeCanvas);
    entry(ui, index > 0, "Move Up", RowAction::Up);
    entry(ui, index < last, "Move Down", RowAction::Down);
    entry(ui, !primary, "Delete", RowAction::Delete);
    chosen
}
