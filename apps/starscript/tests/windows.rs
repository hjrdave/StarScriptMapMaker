//! Popping a tab out into a window of its own and back: the popped-out window has its own tab
//! strip and dialogs, edits and undo keep working in it, and closing it follows the usual
//! unsaved-changes rules (no OS window needed: egui floats the viewport over the main window).

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
mod common;

use ssmap_app::MapMakerApp;
use ssmap_app::menu::Command;
use ssmap_core::{Canvas, DistanceUnit, Scale};

fn canvas() -> Canvas {
    Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles))
}

/// An app with two Maps open, the second active.
fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.state_mut().set_close_hides_window(false);
    harness.run();
    harness.state_mut().new_map(canvas());
    harness.state_mut().new_map(canvas());
    harness.run();
    assert_eq!(harness.state().tabs().len(), 2);
    harness
}

fn popped_layers(harness: &Harness<'_, MapMakerApp>) -> usize {
    harness.state().popped_tabs(0)[0]
        .editor()
        .map
        .children(None)
        .len()
}

#[test]
fn a_tab_pops_out_and_comes_back() {
    let mut harness = app();
    assert_eq!(harness.state().window_count(), 1);

    harness.state_mut().run_command(Command::MoveTabToNewWindow);
    harness.run();
    assert_eq!(harness.state().window_count(), 2);
    assert_eq!(harness.state().tabs().len(), 1);
    assert_eq!(harness.state().popped_tabs(0).len(), 1);

    harness
        .state_mut()
        .in_popped_window(0, |app| app.run_command(Command::MoveTabToMainWindow));
    harness.run();
    assert_eq!(harness.state().window_count(), 1);
    assert_eq!(harness.state().tabs().len(), 2);
    assert_eq!(harness.state().active_tab(), 1);
    assert!(!harness.state().home_active());
}

#[test]
fn edits_and_undo_keep_working_in_the_popped_out_window() {
    let mut harness = app();
    harness.state_mut().run_command(Command::MoveTabToNewWindow);
    harness.run();
    let before = popped_layers(&harness);

    harness.state_mut().in_popped_window(0, |app| {
        app.run_command(Command::NewLayer);
        app.run_command(Command::NewLayer);
    });
    harness.run();
    assert_eq!(popped_layers(&harness), before + 2);
    // The main window's Map is a different one.
    assert_eq!(harness.state().editor().map.children(None).len(), before);

    harness
        .state_mut()
        .in_popped_window(0, |app| app.run_command(Command::Undo));
    harness.run();
    assert_eq!(popped_layers(&harness), before + 1);
    assert!(harness.state().popped_tabs(0)[0].editor().can_redo());

    // The history travelled with the tab.
    harness
        .state_mut()
        .in_popped_window(0, |app| app.run_command(Command::MoveTabToMainWindow));
    harness.run();
    assert_eq!(
        harness.state().tabs()[1].editor().map.children(None).len(),
        before + 1
    );
    assert!(harness.state().tabs()[1].editor().can_undo());
    assert!(harness.state().tabs()[1].editor().can_redo());
}

#[test]
fn the_popped_out_window_has_its_own_menu_and_no_home_tab() {
    let mut harness = app();
    harness.state_mut().run_command(Command::MoveTabToNewWindow);
    harness.run();
    // Two Window menus on screen: the main window's and the popped-out one's.
    assert_eq!(harness.get_all_by_label("Window").count(), 2);
    assert_eq!(harness.get_all_by_label("Home").count(), 1);

    // Layer ▸ New Layer in the popped-out window's own bar.
    let before = popped_layers(&harness);
    harness
        .get_all_by_label("Layer")
        .nth(1)
        .expect("the popped-out window's Layer menu")
        .click();
    harness.run();
    harness
        .get_by_label(&common::item_label("New Layer", Command::NewLayer))
        .click();
    harness.run();
    assert_eq!(
        (
            popped_layers(&harness),
            harness.state().editor().map.children(None).len()
        ),
        (before + 1, before)
    );
}

#[test]
fn the_tab_context_menu_moves_a_tab_out() {
    let mut harness = app();
    harness
        .get_all_by_label("Untitled")
        .next()
        .expect("a Map tab")
        .click_secondary();
    harness.run();
    harness.get_by_label("Move Tab to New Window").click();
    harness.run();
    assert_eq!(harness.state().window_count(), 2);
    assert_eq!(harness.state().tabs().len(), 1);
}

#[test]
fn the_only_tab_of_a_popped_out_window_cannot_pop_out_again() {
    let mut harness = app();
    harness.state_mut().run_command(Command::MoveTabToNewWindow);
    harness.run();
    harness
        .state_mut()
        .in_popped_window(0, |app| app.run_command(Command::MoveTabToNewWindow));
    harness.run();
    assert_eq!(harness.state().window_count(), 2);
}

#[test]
fn closing_a_popped_out_window_asks_about_unsaved_changes() {
    let mut harness = app();
    harness.state_mut().run_command(Command::MoveTabToNewWindow);
    harness.run();
    harness
        .state_mut()
        .in_popped_window(0, |app| app.run_command(Command::NewLayer));
    harness.run();

    // Cancel keeps the window and its Map.
    request_popped_close(&mut harness);
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().window_count(), 2);
    assert_eq!(harness.state().popped_tabs(0).len(), 1);

    request_popped_close(&mut harness);
    harness.get_by_label("Don’t Save").click();
    harness.run();
    assert_eq!(harness.state().window_count(), 1);
    assert_eq!(harness.state().tabs().len(), 1);
    assert!(!harness.state().is_closing());
}

/// Asks the popped-out window to close, as its close button does.
fn request_popped_close(harness: &mut Harness<'_, MapMakerApp>) {
    harness
        .state_mut()
        .in_popped_window(0, MapMakerApp::request_window_close);
    harness.run();
}

#[test]
fn quitting_asks_the_popped_out_windows_first() {
    let mut harness = app();
    harness.state_mut().run_command(Command::MoveTabToNewWindow);
    harness.run();
    harness
        .state_mut()
        .in_popped_window(0, |app| app.run_command(Command::NewLayer));
    harness.run();

    harness.state_mut().request_quit();
    harness.run();
    assert!(!harness.state().is_closing());
    harness.get_by_label("Cancel").click();
    harness.run();
    harness.run();
    assert!(!harness.state().is_closing());
    assert_eq!(harness.state().window_count(), 2);

    harness.state_mut().request_quit();
    harness.run();
    harness.get_by_label("Don’t Save").click();
    harness.run();
    harness.run();
    assert!(harness.state().is_closing());
}
