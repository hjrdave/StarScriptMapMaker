//! The properties of the selected board, the Canvas or an Art board: its name, size, place
//! and background, and making it the Canvas, duplicating or deleting it.

use eframe::egui;
use ssmap_core::{Command, Id, Map, Membership, Node};

use super::{
    PropertiesPanel, caption, heading,
    map::{merge_edits, resize},
    row,
};
use crate::editor::Editor;
use crate::prop_widgets::{self, Handle, ScrubSpec};

/// What a board's buttons ask the window for, which owns the dialogs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BoardRequest {
    /// Delete the Art board, after asking.
    Delete(Id),
    /// Add a copy of the board.
    Duplicate(Id),
}

impl PropertiesPanel {
    /// What the board buttons asked for since the last call.
    pub(crate) fn take_board_requests(&mut self) -> Vec<BoardRequest> {
        std::mem::take(&mut self.board_requests)
    }

    /// The selected board's settings. Every edit is one Undo step and none moves an Object.
    pub(super) fn board_properties(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id) {
        let Some(canvas) = editor.map().board(id).map(|board| board.canvas.clone()) else {
            return;
        };
        let Some(label) = editor.map().board_label(id) else {
            return;
        };
        let primary = id == editor.map().primary_board();
        heading(ui, &label);
        if !primary {
            self.board_name(ui, editor, id, &label);
        }
        self.canvas_fields(ui, editor, id, &canvas);
        let spec = |name, handle| {
            ScrubSpec::new(name)
                .handle(Handle::Text(handle))
                .range(-1_000_000.0, 1_000_000.0)
                .step(10.0)
        };
        let (mut x, mut y) = (canvas.origin[0], canvas.origin[1]);
        let (x_edit, y_edit) = prop_widgets::pair(
            ui,
            |ui| prop_widgets::scrub_field(ui, &mut x, &spec("Board X", "X")),
            |ui| prop_widgets::scrub_field(ui, &mut y, &spec("Board Y", "Y")),
        );
        let name = if primary { "Move Canvas" } else { "Move Board" };
        let command = resize(editor, id, [x, y], canvas.width, canvas.height);
        self.live
            .apply(editor, merge_edits(x_edit, y_edit), name, || command);
        caption(ui, "Background");
        self.background_fields(ui, editor, id, &canvas);
        caption(ui, "Board");
        ui.horizontal_wrapped(|ui| {
            if !primary && ui.button("Set as Canvas").clicked() {
                editor.make_canvas(id);
            }
            if ui.button("Duplicate").clicked() {
                self.board_requests.push(BoardRequest::Duplicate(id));
            }
            if !primary && ui.button("Delete").clicked() {
                self.board_requests.push(BoardRequest::Delete(id));
            }
        });
    }

    /// For the selected Objects: which board they belong to, and a way to move them to
    /// another, to the Canvas, or off the boards. One Undo step. Nothing for a selection that
    /// is not all Objects.
    pub(super) fn membership_row(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let ids: Vec<Id> = editor.selection().to_vec();
        let memberships: Vec<Membership> = ids
            .iter()
            .filter_map(|&id| match editor.map().node(id) {
                Some(Node::Object(object)) => Some(object.membership),
                _ => None,
            })
            .collect();
        if ids.is_empty() || memberships.len() != ids.len() {
            return;
        }
        let map = editor.map();
        let same = memberships.iter().all(|m| *m == memberships[0]);
        let shown = if same {
            membership_name(map, memberships[0])
        } else {
            "Mixed".to_owned()
        };
        let mut options = vec![Membership::Canvas];
        options.extend(
            map.boards()
                .iter()
                .filter(|board| board.id != map.primary_board())
                .map(|board| Membership::Board(board.id)),
        );
        options.push(Membership::OffCanvas);
        let names: Vec<String> = options.iter().map(|&m| membership_name(map, m)).collect();
        let mut chosen = None;
        row(ui, "Board", |ui| {
            egui::ComboBox::from_id_salt("object board")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (option, name) in options.iter().zip(&names) {
                        let current = same && memberships[0] == *option;
                        if ui.selectable_label(current, name).clicked() && !current {
                            chosen = Some(*option);
                        }
                    }
                });
        });
        if let Some(membership) = chosen {
            let _ = editor.run(Command::SetMembership { ids, membership });
        }
    }

    /// The board's name; empty goes back to the default "Art board N". Committed when the
    /// field loses focus.
    fn board_name(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, shown: &str) {
        let current = editor
            .map()
            .board(id)
            .and_then(|board| board.name.clone())
            .unwrap_or_default();
        let mut text = match &self.text_edit {
            Some((editing, text)) if *editing == id => text.clone(),
            _ => current.clone(),
        };
        let field = ui
            .horizontal(|ui| {
                ui.label("Name");
                ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text(shown)
                        .desired_width(f32::INFINITY),
                )
            })
            .inner;
        field.widget_info(|| {
            egui::WidgetInfo::text_edit(true, "Board name", text.clone(), text.clone())
        });
        if field.changed() {
            self.text_edit = Some((id, text.clone()));
        }
        if field.lost_focus() {
            self.text_edit = None;
            let name = text.trim();
            if name != current {
                let name = (!name.is_empty()).then(|| name.to_owned());
                let _ = editor.run(Command::RenameBoard { id, name });
            }
        }
    }
}

/// What a membership is called in lists: the board's name, "Canvas", or "Off the boards".
fn membership_name(map: &Map, membership: Membership) -> String {
    match membership {
        Membership::Canvas => "Canvas".to_owned(),
        Membership::OffCanvas => "Off the boards".to_owned(),
        Membership::Board(id) => map
            .board_label(id)
            .unwrap_or_else(|| "Unknown board".to_owned()),
    }
}
