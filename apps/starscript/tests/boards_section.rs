//! The Art boards section of the left sidebar: it appears once there are Art boards, lists
//! the boards in order, a click selects and frames a board, and + adds one. The real app,
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

#[test]
fn with_only_the_canvas_there_is_no_boards_section() {
    let mut harness = app();
    show_boards(&mut harness);
    assert_eq!(section(&harness), SidebarSection::Layers);
    assert!(
        !harness
            .state()
            .shell()
            .section_available(SidebarSection::Boards)
    );
}

#[test]
fn the_shortcut_lists_the_boards_once_there_are_art_boards() {
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
fn clicking_a_row_selects_that_board() {
    let mut harness = app();
    harness.state_mut().run_command(Command::NewArtBoard);
    harness.run();
    show_boards(&mut harness);
    let canvas = harness.state().editor().map().primary_board();
    harness.get_by_label("Canvas").click();
    harness.run();
    assert_eq!(harness.state().editor().selected_board(), Some(canvas));
    harness.get_by_label("Art board 1").click();
    harness.run();
    let art = harness.state().editor().selected_board().unwrap();
    assert_ne!(art, canvas);
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
fn the_section_goes_back_to_layers_when_the_last_art_board_is_gone() {
    let mut harness = app();
    harness.state_mut().run_command(Command::NewArtBoard);
    harness.run();
    show_boards(&mut harness);
    assert_eq!(section(&harness), SidebarSection::Boards);
    harness.state_mut().active_editor_mut().unwrap().undo();
    harness.run();
    assert_eq!(boards(&harness), 1);
    assert_eq!(section(&harness), SidebarSection::Layers);
}
