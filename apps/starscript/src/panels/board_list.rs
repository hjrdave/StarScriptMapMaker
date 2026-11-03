//! The list in the Art boards section of the left sidebar: the Canvas and the Art boards in the
//! order they draw, with a menu of actions on each row. The settings of the selected row are
//! under it (`PropertiesPanel::boards_panel`). Every edit is a Command, so one Undo step.

use eframe::egui;
use ssmap_core::Id;

use crate::editor::Editor;
use crate::icons;
use crate::properties::BoardRequest;
use crate::theme;

/// The height of a row, in points.
const ROW_HEIGHT: f32 = 24.0;
/// The panel is at least this wide, in points, for a row to show its size.
const SIZE_MIN_WIDTH: f32 = 260.0;

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

    /// The rows: the Canvas, then the Art boards. The selected row has the gold fill and the
    /// size shows at the right while the panel is wide enough. With no board selected the
    /// Canvas counts as the selected row.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let primary_id = editor.map().primary_board();
        let rows: Vec<(Id, String, String, bool)> = editor
            .map()
            .boards()
            .iter()
            .filter_map(|board| {
                let label = editor.map().board_label(board.id)?;
                let size = format!("{} × {}", board.canvas.width, board.canvas.height);
                Some((board.id, label, size, board.id == primary_id))
            })
            .collect();
        let selected = editor.selected_board().unwrap_or(primary_id);
        let last = rows.len().saturating_sub(1);
        let show_size = ui.available_width() >= SIZE_MIN_WIDTH;
        let mut action = None;
        for (index, (id, label, size, primary)) in rows.iter().enumerate() {
            if self.renaming.as_ref().is_some_and(|(r, _)| r == id) {
                self.rename_row(ui, editor, *id, label);
                continue;
            }
            let row = board_row(ui, label, size, show_size, selected == *id);
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

/// One row: the Art board icon, the name and, when `show_size`, the size at the right. Named
/// after the board for assistive tools.
fn board_row(
    ui: &mut egui::Ui,
    label: &str,
    size: &str,
    show_size: bool,
    selected: bool,
) -> egui::Response {
    let palette = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
    });
    if selected {
        ui.painter().rect_filled(rect, 2.0, palette.gold_subtle);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 2.0, palette.border_subtle.gamma_multiply(0.5));
    }
    let pad = theme::SPACING[3];
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.left() + pad + icons::GRID / 2.0, rect.center().y),
        egui::Vec2::splat(icons::GRID),
    );
    icons::paint(
        ui,
        icon,
        icons::Icon::ExportRegion,
        icons::GRID,
        palette.text_primary,
    );
    let mut text_right = rect.right() - pad;
    if show_size {
        let galley = ui.painter().layout_no_wrap(
            size.to_owned(),
            egui::FontId::proportional(theme::CAPTION),
            palette.text_secondary,
        );
        let at = egui::pos2(
            text_right - galley.size().x,
            rect.center().y - galley.size().y / 2.0,
        );
        text_right = at.x - theme::SPACING[2];
        ui.painter().galley(at, galley, palette.text_secondary);
    }
    let name_left = icon.right() + theme::SPACING[2];
    let clip = egui::Rect::from_min_max(
        egui::pos2(name_left, rect.top()),
        egui::pos2(text_right.max(name_left), rect.bottom()),
    );
    ui.painter().with_clip_rect(clip).text(
        egui::pos2(name_left, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(theme::BODY),
        palette.text_primary,
    );
    response
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
