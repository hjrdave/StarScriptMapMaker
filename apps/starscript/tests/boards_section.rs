//! The Art boards panel in the left sidebar: the Canvas and every Art board in one list, with
//! the settings of the selected row under it. It is always available. The real app, driven
//! headlessly.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};
use ssmap_app::MapMakerApp;
use ssmap_app::menu::Command;
use ssmap_app::shell::SidebarSection;
use ssmap_core::{Canvas, DistanceUnit, Scale};

const BOTH: egui::Modifiers = egui::Modifiers {
    alt: true,
    command: true,
    ctrl: false,
    shift: false,
    mac_cmd: false,
};

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_step_dt(0.25)
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness.state_mut().new_map(Canvas::new(
        4096,
        3072,
        Scale::new(0.25, DistanceUnit::Miles),
    ));
    harness.run();
    harness
}

/// An app showing the Art boards panel on a Map with one Art board, selected.
fn with_art_board() -> Harness<'static, MapMakerApp> {
    let mut harness = app();
    harness.state_mut().run_command(Command::NewArtBoard);
    harness.run();
    show_boards(&mut harness);
    harness.get_by_label("Art board 1").click();
    harness.run();
    harness
}

fn section(harness: &Harness<'_, MapMakerApp>) -> SidebarSection {
    harness.state().shell().sidebar_section()
}

fn show_boards(harness: &mut Harness<'_, MapMakerApp>) {
    harness.key_press_modifiers(BOTH, egui::Key::Num3);
    harness.run();
}

fn boards(harness: &Harness<'_, MapMakerApp>) -> usize {
    harness.state().editor().map().boards().len()
}

fn board_ids(harness: &Harness<'_, MapMakerApp>) -> Vec<ssmap_core::Id> {
    let map = harness.state().editor().map();
    map.boards().iter().map(|board| board.id).collect()
}

/// The Name field, the first text box in the panel (the number boxes follow it).
fn first_text_input<'a>(harness: &'a Harness<'_, MapMakerApp>) -> egui_kittest::Node<'a> {
    harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .expect("the Name field")
}

/// Types `text` into the number box that shows `shown`.
fn type_into(harness: &mut Harness<'_, MapMakerApp>, shown: &str, text: &str) {
    harness
        .get_all_by_value(shown)
        .next()
        .expect("a box showing that value")
        .click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    harness
        .get_by(|n| n.role() == egui::accesskit::Role::TextInput && n.is_focused())
        .type_text(text);
    harness.key_press(egui::Key::Enter);
    harness.run();
}

#[test]
fn with_only_the_canvas_the_section_is_still_there() {
    let mut harness = app();
    show_boards(&mut harness);
    assert_eq!(section(&harness), SidebarSection::Boards);
    assert_eq!(boards(&harness), 1);
    harness.get_by_label("Canvas");
    harness.get_by_label("New Art Board");
    // The Canvas is the selected row, so its settings show.
    harness.get_by_label("Resize from");
    harness.get_by_label("Scale");
}

#[test]
fn the_shortcut_lists_the_boards() {
    let mut harness = app();
    harness.state_mut().run_command(Command::NewArtBoard);
    harness.run();
    show_boards(&mut harness);
    assert_eq!(section(&harness), SidebarSection::Boards);
    harness.get_by_label("Canvas");
    harness.get_by_label("Art board 1");
    // Only one section shows at a time.
    assert!(harness.query_by_label("New Layer (⇧⌘N)").is_none());
}

#[test]
fn clicking_a_row_selects_that_board_and_shows_its_settings() {
    let mut harness = app();
    harness.state_mut().run_command(Command::NewArtBoard);
    harness.run();
    show_boards(&mut harness);
    let canvas = harness.state().editor().map().primary_board();
    harness.get_by_label("Canvas").click();
    harness.run();
    assert_eq!(harness.state().editor().selected_board(), Some(canvas));
    // Scale and the Set as Canvas / Delete actions are not for an Art board.
    harness.get_by_label("Scale");
    assert!(harness.query_by_label("Set as Canvas").is_none());
    harness.get_by_label("Art board 1").click();
    harness.run();
    let art = harness.state().editor().selected_board().unwrap();
    assert_ne!(art, canvas);
    assert!(harness.query_by_label("Scale").is_none());
    harness.get_by_label("Set as Canvas");
    harness.get_by_label("Delete");
}

#[test]
fn the_canvas_name_is_shown_and_disabled() {
    let mut harness = app();
    show_boards(&mut harness);
    let name = first_text_input(&harness);
    assert!(name.accesskit_node().is_disabled());
    assert_eq!(name.value().as_deref(), Some("Canvas"));
}

#[test]
fn an_art_board_is_renamed_from_the_name_row_in_one_undo_step() {
    let mut harness = with_art_board();
    first_text_input(&harness).click();
    harness.run();
    harness
        .get_by(|n| n.role() == egui::accesskit::Role::TextInput && n.is_focused())
        .type_text("Harbour");
    harness.key_press(egui::Key::Enter);
    harness.run();
    harness.get_by_label("Harbour");
    let steps = harness.state().editor().history_steps().0.len();
    harness.state_mut().active_editor_mut().unwrap().undo();
    harness.run();
    assert_eq!(harness.state().editor().history_steps().0.len(), steps);
    harness.get_by_label("Art board 1");
}

#[test]
fn resizing_a_board_from_the_panel_is_one_undo_step() {
    let mut harness = with_art_board();
    let id = harness.state().editor().selected_board().unwrap();
    type_into(&mut harness, "4096", "2000");
    let width = |harness: &Harness<'_, MapMakerApp>| {
        harness
            .state()
            .editor()
            .map()
            .board(id)
            .unwrap()
            .canvas
            .width
    };
    assert_eq!(width(&harness), 2000);
    harness.state_mut().active_editor_mut().unwrap().undo();
    harness.run();
    assert_eq!(width(&harness), 4096);
}

#[test]
fn set_as_canvas_makes_the_selected_board_the_canvas() {
    let mut harness = with_art_board();
    let id = harness.state().editor().selected_board().unwrap();
    harness.get_by_label("Set as Canvas").click();
    harness.run();
    assert_eq!(harness.state().editor().map().primary_board(), id);
    // The Canvas has no Set as Canvas or Delete of its own.
    assert!(harness.query_by_label("Set as Canvas").is_none());
    assert!(harness.query_by_label("Delete").is_none());
}

#[test]
fn delete_asks_and_then_removes_the_board() {
    let mut harness = with_art_board();
    harness.get_by_label("Delete").click();
    harness.run();
    harness.get_by_label("Its Objects are kept and stay where they are, off the boards.");
    assert_eq!(boards(&harness), 2);
    // The dialog's button is the one drawn last.
    harness.get_all_by_label("Delete").last().unwrap().click();
    harness.run();
    assert_eq!(boards(&harness), 1);
}

#[test]
fn move_up_and_move_down_reorder_the_boards() {
    let mut harness = with_art_board();
    harness.state_mut().run_command(Command::NewArtBoard);
    harness.run();
    harness.get_by_label("Art board 1").click();
    harness.run();
    let [canvas, one, two] = board_ids(&harness)[..] else {
        panic!("three boards")
    };
    harness.get_by_label("Move up").click();
    harness.run();
    assert_eq!(board_ids(&harness), [one, canvas, two]);
    harness.get_by_label("Move down").click();
    harness.run();
    assert_eq!(board_ids(&harness), [canvas, one, two]);
    // The first row can't move up.
    harness.get_by_label("Canvas").click();
    harness.run();
    assert!(
        harness
            .get_by_label("Move up")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn the_plus_button_adds_and_selects_a_board() {
    let mut harness = app();
    harness.state_mut().run_command(Command::NewArtBoard);
    harness.run();
    show_boards(&mut harness);
    assert_eq!(boards(&harness), 2);
    harness.get_by_label("New Art Board").click();
    harness.run();
    assert_eq!(boards(&harness), 3);
    assert!(harness.query_all_by_label("Art board 2").next().is_some());
    assert!(harness.state().editor().selected_board().is_some());
}

#[test]
fn the_canvas_tab_is_gone_from_the_map_panel() {
    let mut harness = app();
    harness.key_press_modifiers(BOTH, egui::Key::Num6);
    harness.run();
    harness.get_by_label("Style");
    harness.get_by_label("Shores");
    assert!(harness.query_by_label("Canvas width").is_none());
}
