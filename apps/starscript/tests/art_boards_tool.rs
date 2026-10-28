//! Art boards in the editor: New Art Board, the board section of Properties, deleting with a
//! confirmation, making a board the Canvas, duplicating, and framing boards. The real app,
//! driven headlessly.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::menu::Command;
use ssmap_app::tools::ToolId;
use ssmap_core::{Canvas, DistanceUnit, Scale};

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness.state_mut().new_map(Canvas::new(
        4096,
        3072,
        Scale::new(0.25, DistanceUnit::Miles),
    ));
    harness.state_mut().active_editor_mut().unwrap().active_tool = ToolId::Canvas;
    harness.run();
    harness
}

/// The Delete button of the confirmation, which comes after the one in Properties.
fn click_dialog_delete(harness: &mut Harness<'_, MapMakerApp>) {
    harness
        .get_all_by_label("Delete")
        .last()
        .expect("the dialog's Delete button")
        .click();
    harness.run();
}

fn boards(harness: &Harness<'_, MapMakerApp>) -> usize {
    harness.state().editor().map().boards().len()
}

/// A Map with one Art board, selected.
fn with_art_board() -> Harness<'static, MapMakerApp> {
    let mut harness = app();
    harness.state_mut().run_command(Command::NewArtBoard);
    harness.run();
    harness
}

#[test]
fn new_art_board_adds_and_selects_a_board_beside_the_canvas() {
    let mut harness = app();
    assert_eq!(boards(&harness), 1);
    harness.state_mut().run_command(Command::NewArtBoard);
    harness.run();
    assert_eq!(boards(&harness), 2);
    let editor = harness.state().editor();
    let board = editor.selected_board().expect("the new board is selected");
    assert_ne!(board, editor.map().primary_board());
    assert!(editor.map().board(board).unwrap().canvas.origin[0] > 4096.0);
    assert_eq!(editor.undo_label(), Some("Add Art board"));
    // Properties shows the board.
    harness.get_by_label("Increase Board width");
    harness.get_by_label("Increase Board X");
}

#[test]
fn deleting_a_selected_board_asks_first_and_cancel_keeps_it() {
    let mut harness = with_art_board();
    harness.state_mut().run_command(Command::Delete);
    harness.run();
    harness.get_by_label("Delete “Art board 1”?");
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(boards(&harness), 2);

    harness.state_mut().run_command(Command::Delete);
    harness.run();
    click_dialog_delete(&mut harness);
    assert_eq!(boards(&harness), 1);
    assert_eq!(harness.state().editor().undo_label(), Some("Delete Board"));
}

#[test]
fn the_canvas_cannot_be_deleted() {
    let mut harness = app();
    harness
        .state_mut()
        .active_editor_mut()
        .unwrap()
        .select_canvas(true);
    harness.run();
    harness.state_mut().run_command(Command::Delete);
    harness.run();
    assert_eq!(boards(&harness), 1);
    assert!(
        harness.query_by_label_contains("Delete “").is_none(),
        "no dialog"
    );
    // Properties offers no Delete and no Set as Canvas for the Canvas itself.
    assert!(harness.query_by_label("Set as Canvas").is_none());
}

#[test]
fn the_properties_buttons_make_a_board_the_canvas_duplicate_and_delete() {
    let mut harness = with_art_board();
    let board = harness.state().editor().selected_board().unwrap();
    harness.get_by_label("Duplicate").click();
    harness.run();
    assert_eq!(boards(&harness), 3);
    let copy = harness.state().editor().selected_board().unwrap();
    assert_ne!(copy, board);

    harness.get_by_label("Set as Canvas").click();
    harness.run();
    assert_eq!(harness.state().editor().map().primary_board(), copy);
    assert_eq!(harness.state().editor().undo_label(), Some("Change Canvas"));

    // The Canvas now has no Delete; select the first board and delete it from Properties.
    harness
        .state_mut()
        .active_editor_mut()
        .unwrap()
        .select_board(Some(board));
    harness.run();
    harness.get_by_label("Delete").click();
    harness.run();
    let name = harness.state().editor().map().board_label(board).unwrap();
    harness.get_by_label(&format!("Delete “{name}”?"));
    click_dialog_delete(&mut harness);
    assert_eq!(boards(&harness), 2);
}

#[test]
fn zooming_to_a_board_and_to_all_boards_works_with_the_menu_commands() {
    let mut harness = with_art_board();
    // Both are available now; running them must leave the Map as it is.
    let before = harness.state().editor().map().view();
    harness.state_mut().run_command(Command::ZoomBoard);
    harness.run();
    harness.state_mut().run_command(Command::ZoomAll);
    harness.run();
    assert_eq!(harness.state().editor().map().view(), before);
}
