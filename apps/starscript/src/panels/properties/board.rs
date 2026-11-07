//! The Art boards panel's settings for the selected row, the Canvas or an Art board: its name,
//! size, place and background, and making it the Canvas, duplicating, deleting or moving it.
//! Also the Objects' Board row.

use eframe::egui;
use ssmap_core::{Command, DistanceUnit, Id, Map, Membership, Node};

use super::{
    PropertiesPanel,
    map::{background_command, merge_edits, resize},
    row,
};
use crate::board_list::BoardList;
use crate::editor::Editor;
use crate::prop_widgets::{self, Edit, Handle, Provenance, ScrubSpec};
use crate::theme;

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

    /// The Art boards section: the list of boards, then the settings of the selected row (the
    /// Canvas when no board is selected). Every edit is one Undo step and none moves an Object.
    pub(crate) fn boards_panel(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        list: &mut BoardList,
    ) {
        // A change ends when the pointer is released, or when Escape cancelled it.
        self.live.apply(editor, Edit::default(), "", || {
            unreachable!("nothing changed")
        });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                list.show(ui, editor);
                let id = editor
                    .selected_board()
                    .unwrap_or_else(|| editor.map().primary_board());
                ui.add_space(theme::SPACING[2]);
                self.board_settings(ui, editor, id);
            });
    }

    /// The settings of the board `id`, under the list.
    pub(super) fn board_settings(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id) {
        let Some(canvas) = editor.map().board(id).map(|board| board.canvas.clone()) else {
            return;
        };
        let Some(label) = editor.map().board_label(id) else {
            return;
        };
        let primary = id == editor.map().primary_board();
        self.board_name(ui, editor, id, &label, primary);
        let scale = primary.then(|| {
            let unit = match canvas.scale.unit {
                DistanceUnit::Miles => "mi",
                DistanceUnit::Kilometres => "km",
            };
            format!("{} {unit} per unit", canvas.scale.distance_per_unit)
        });
        self.canvas_fields(ui, editor, id, &canvas, scale.as_deref());
        prop_widgets::field_row(ui, "Position", Provenance::Plain, |ui| {
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
        });
        self.board_colour(ui, editor, id, &canvas);
        self.board_actions(ui, editor, id, primary);
    }

    /// The Background colour row: the water colour, changed live as one Undo step.
    fn board_colour(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        canvas: &ssmap_core::Canvas,
    ) {
        let mut colour = Some(canvas.background.water);
        let response =
            prop_widgets::swatch(ui, "Background", &mut colour, false, Provenance::Plain);
        let mut next = canvas.background.clone();
        next.water = colour.unwrap_or(next.water);
        let command = background_command(editor, id, next);
        self.live
            .apply(editor, response.edit, "Change Background", || command);
    }

    /// Duplicate, Set as Canvas and Delete (the last two not on the Canvas), then Move up and
    /// Move down among the boards, which draw in the order of the list.
    fn board_actions(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, primary: bool) {
        let index = editor
            .map()
            .boards()
            .iter()
            .position(|board| board.id == id);
        let last = editor.map().boards().len().saturating_sub(1);
        ui.add_space(theme::SPACING[2]);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Duplicate").clicked() {
                self.board_requests.push(BoardRequest::Duplicate(id));
            }
            if !primary && ui.button("Set as Canvas").clicked() {
                editor.make_canvas(id);
            }
            if !primary && ui.button("Delete").clicked() {
                self.board_requests.push(BoardRequest::Delete(id));
            }
        });
        ui.horizontal_wrapped(|ui| {
            let up = ui.add_enabled(index.is_some_and(|i| i > 0), egui::Button::new("Move up"));
            if up.clicked() {
                editor.move_board(id, -1);
            }
            let can_down = index.is_some_and(|i| i < last);
            if ui
                .add_enabled(can_down, egui::Button::new("Move down"))
                .clicked()
            {
                editor.move_board(id, 1);
            }
        });
    }

    /// For the selected Objects: which board they belong to, and a way to move them to
    /// another, to the Canvas, or off the boards. One Undo step. Nothing for a selection that
    /// is not all Objects.
    pub(super) fn membership_row(&mut self, ui: &mut egui::Ui, editor: &mut Editor, above: bool) {
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
        let combo = |ui: &mut egui::Ui| {
            let mut picker = egui::ComboBox::from_id_salt("object board");
            if above {
                // Full width, with the name above it.
                picker = picker.width(ui.available_width());
            }
            picker.selected_text(shown).show_ui(ui, |ui| {
                for (option, name) in options.iter().zip(&names) {
                    let current = same && memberships[0] == *option;
                    if ui.selectable_label(current, name).clicked() && !current {
                        chosen = Some(*option);
                    }
                }
            });
        };
        if above {
            prop_widgets::named_above(ui, "Board", Provenance::Plain, combo);
        } else {
            row(ui, "Board", combo);
        }
        if let Some(membership) = chosen {
            let _ = editor.run(Command::SetMembership { ids, membership });
        }
    }

    /// The board's name; empty goes back to the default "Art board N". Committed when the
    /// field loses focus. The Canvas's name is shown but can't be edited.
    fn board_name(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        shown: &str,
        primary: bool,
    ) {
        let current = editor
            .map()
            .board(id)
            .and_then(|board| board.name.clone())
            .unwrap_or_default();
        let mut text = if primary {
            shown.to_owned()
        } else {
            match &self.text_edit {
                Some((editing, text)) if *editing == id => text.clone(),
                _ => current.clone(),
            }
        };
        let mut field = None;
        prop_widgets::field_row(ui, "Name", Provenance::Plain, |ui| {
            field = Some(
                ui.add_enabled(
                    !primary,
                    egui::TextEdit::singleline(&mut text)
                        .hint_text(shown)
                        .desired_width(f32::INFINITY),
                ),
            );
        });
        let Some(field) = field else {
            return;
        };
        field.widget_info(|| {
            egui::WidgetInfo::text_edit(!primary, "Board name", text.clone(), text.clone())
        });
        if primary {
            return;
        }
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
