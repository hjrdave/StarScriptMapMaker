//! Several Maps open at once, one per tab: opening, switching, closing, quitting and the
//! Recovery journal of each (no window needed).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_core::file::MapFile;
use ssmap_core::{Canvas, DistanceUnit, Map, Scale};

fn canvas() -> Canvas {
    Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles))
}

fn app(recovery_dir: Option<&Path>) -> Harness<'static, MapMakerApp> {
    let recovery_dir = recovery_dir.map(Path::to_path_buf);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| MapMakerApp::with_recovery_dir(cc, recovery_dir.clone()));
    harness.run();
    // The app starts on Home with no Map; these tests begin with one open.
    assert!(harness.state().tabs().is_empty());
    harness.state_mut().new_map(canvas());
    harness.run();
    harness
}

fn saved_map(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    MapFile::create(&path, &Map::new(canvas()), None).unwrap();
    path
}

fn layers(harness: &Harness<'_, MapMakerApp>, tab: usize) -> usize {
    harness.state().tabs()[tab]
        .editor()
        .map
        .children(None)
        .len()
}

fn new_layer(harness: &mut Harness<'_, MapMakerApp>) {
    harness.get_by_label("New Layer").click();
    harness.run();
}

fn request_close(harness: &mut Harness<'_, MapMakerApp>) {
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    harness.run();
}

#[test]
fn opening_two_maps_gives_two_tabs_and_an_open_map_is_focused_not_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let first = saved_map(dir.path(), "first.ssmap");
    let second = saved_map(dir.path(), "second.ssmap");
    let mut harness = app(None);
    assert_eq!(harness.state().tabs().len(), 1);

    assert!(harness.state_mut().open_path(&first));
    assert!(harness.state_mut().open_path(&second));
    harness.run();
    assert_eq!(harness.state().tabs().len(), 3);
    assert_eq!(harness.state().active_tab(), 2);
    assert_eq!(
        harness.state().title(),
        "second.ssmap — StarScript Map Maker"
    );

    // The same file by another route to it focuses its tab.
    let roundabout = dir.path().join(".").join("first.ssmap");
    assert!(harness.state_mut().open_path(&roundabout));
    harness.run();
    assert_eq!(harness.state().tabs().len(), 3);
    assert_eq!(harness.state().active_tab(), 1);
    assert_eq!(
        harness.state().title(),
        "first.ssmap — StarScript Map Maker"
    );
}

#[test]
fn new_map_opens_in_a_new_tab_and_leaves_the_current_map_alone() {
    let mut harness = app(None);
    new_layer(&mut harness);
    let edited = layers(&harness, 0);
    harness.state_mut().new_map(canvas());
    harness.run();
    assert_eq!(harness.state().tabs().len(), 2);
    assert_eq!(harness.state().active_tab(), 1);
    assert_eq!(layers(&harness, 0), edited);
    assert!(harness.query_by_label("Discard").is_none());
}

#[test]
fn each_tab_keeps_its_own_undo_history() {
    let dir = tempfile::tempdir().unwrap();
    let first = saved_map(dir.path(), "first.ssmap");
    let second = saved_map(dir.path(), "second.ssmap");
    let mut harness = app(None);
    harness.state_mut().open_path(&first);
    harness.state_mut().open_path(&second);
    harness.run();
    new_layer(&mut harness);
    harness.state_mut().focus_tab(1);
    harness.run();
    new_layer(&mut harness);
    new_layer(&mut harness);
    assert_eq!((layers(&harness, 1), layers(&harness, 2)), (2, 1));

    // Undo in the tab being shown touches only that tab.
    harness.get_by_label("Undo").click();
    harness.run();
    assert_eq!((layers(&harness, 1), layers(&harness, 2)), (1, 1));
    assert!(harness.state().tabs()[2].editor().can_undo());
    harness.state_mut().focus_tab(2);
    harness.run();
    assert!(harness.state().tabs()[2].editor().can_undo());
    assert!(harness.state().tabs()[1].editor().can_redo());
    assert!(!harness.state().tabs()[2].editor().can_redo());
}

#[test]
fn closing_a_dirty_tab_asks_and_cancel_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let first = saved_map(dir.path(), "first.ssmap");
    let mut harness = app(None);
    harness.state_mut().open_path(&first);
    harness.run();
    new_layer(&mut harness);

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::W);
    harness.run();
    harness.get_by_label("Save the changes to “first.ssmap” before closing?");
    harness.get_by_label("Cancel").click();
    harness.run();
    assert_eq!(harness.state().tabs().len(), 2);
    assert!(harness.state().tabs()[1].has_unsaved_changes());

    harness.state_mut().close_tab(1);
    harness.run();
    harness.get_by_label("Don’t Save").click();
    harness.run();
    assert_eq!(harness.state().tabs().len(), 1);
    let (_, saved) = MapFile::open(&first).unwrap();
    assert_eq!(saved.children(None).len(), 0);
}

#[test]
fn saving_from_the_close_question_saves_the_tab_then_closes_it() {
    let dir = tempfile::tempdir().unwrap();
    let first = saved_map(dir.path(), "first.ssmap");
    let mut harness = app(None);
    harness.state_mut().open_path(&first);
    harness.run();
    new_layer(&mut harness);
    harness.state_mut().close_tab(1);
    harness.run();
    harness.get_by_label("Save").click();
    harness.run();
    assert_eq!(harness.state().tabs().len(), 1);
    let (_, saved) = MapFile::open(&first).unwrap();
    assert_eq!(saved.children(None).len(), 1);
}

#[test]
fn saving_a_map_never_saved_from_the_close_question_goes_through_save_as() {
    let mut harness = app(None);
    new_layer(&mut harness);
    harness.state_mut().close_tab(0);
    harness.run();
    harness.get_by_label("Save").click();
    harness.run();
    harness.get_by_label("Save Map As");
    assert_eq!(harness.state().tabs().len(), 1, "closes only once saved");
}

#[test]
fn a_clean_tab_closes_at_once_and_the_last_tab_leaves_an_empty_window() {
    let mut harness = app(None);
    harness.state_mut().new_map(canvas());
    harness.run();
    harness.state_mut().close_tab(0);
    harness.run();
    assert_eq!(harness.state().tabs().len(), 1);
    assert_eq!(harness.state().active_tab(), 0);
    assert!(harness.query_by_label("Unsaved changes").is_none());

    harness.state_mut().close_tab(0);
    harness.run();
    assert!(harness.state().tabs().is_empty());
    assert_eq!(harness.state().title(), "StarScript Map Maker");
    assert!(!harness.state().is_closing());
}

#[test]
fn quitting_with_two_dirty_tabs_asks_about_each_and_cancel_stops_the_quit() {
    let dir = tempfile::tempdir().unwrap();
    let first = saved_map(dir.path(), "first.ssmap");
    let second = saved_map(dir.path(), "second.ssmap");
    let mut harness = app(None);
    harness.state_mut().open_path(&first);
    harness.state_mut().open_path(&second);
    harness.run();
    new_layer(&mut harness);
    harness.state_mut().focus_tab(1);
    harness.run();
    new_layer(&mut harness);
    harness.state_mut().focus_tab(0);
    harness.run();

    request_close(&mut harness);
    harness.get_by_label("Save the changes to “first.ssmap” before closing?");
    assert_eq!(harness.state().active_tab(), 1, "its tab is shown first");
    harness.get_by_label("Don’t Save").click();
    harness.run();
    assert!(!harness.state().is_closing());
    harness.get_by_label("Save the changes to “second.ssmap” before closing?");
    assert_eq!(harness.state().active_tab(), 2);
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(!harness.state().is_closing());
    assert_eq!(harness.state().tabs().len(), 3, "every tab stays open");
    assert!(harness.state().tabs()[1].has_unsaved_changes());

    // Quitting again asks about both again; answering both closes the window.
    request_close(&mut harness);
    harness.get_by_label("Save the changes to “first.ssmap” before closing?");
    harness.get_by_label("Save").click();
    harness.run();
    harness.get_by_label("Save the changes to “second.ssmap” before closing?");
    harness.get_by_label("Don’t Save").click();
    harness.run();
    assert!(harness.state().is_closing());
    let (_, saved) = MapFile::open(&first).unwrap();
    assert_eq!(saved.children(None).len(), 1);
    let (_, saved) = MapFile::open(&second).unwrap();
    assert_eq!(saved.children(None).len(), 0);
}

/// Runs frames until every tab's edits have reached its Recovery journal.
fn wait_until_journaled(harness: &mut Harness<'_, MapMakerApp>, tabs: usize) {
    let started = Instant::now();
    loop {
        harness.step();
        if harness
            .state()
            .tabs()
            .iter()
            .take(tabs)
            .all(|tab| tab.recovery_journal().is_some())
        {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the journals were never written"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn each_tab_writes_its_own_journal_and_recovery_opens_each_in_a_tab() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(Some(dir.path()));
    new_layer(&mut harness);
    harness.state_mut().new_map(canvas());
    harness.run();
    new_layer(&mut harness);
    new_layer(&mut harness);
    wait_until_journaled(&mut harness, 2);
    let journals: Vec<PathBuf> = harness
        .state()
        .tabs()
        .iter()
        .map(|tab| tab.recovery_journal().unwrap().to_path_buf())
        .collect();
    assert_ne!(journals[0], journals[1]);
    assert!(journals.iter().all(|journal| journal.exists()));
    let counts = (layers(&harness, 0), layers(&harness, 1));
    // A crash: the app goes away without closing anything.
    drop(harness);

    let mut harness = app(Some(dir.path()));
    let mut recovered = Vec::new();
    for _ in 0..2 {
        harness.get_by_label("Recover").click();
        harness.run();
        recovered.push(layers(&harness, harness.state().active_tab()));
    }
    assert_eq!(harness.state().tabs().len(), 3);
    recovered.sort_unstable();
    let mut expected = [counts.0, counts.1];
    expected.sort_unstable();
    assert_eq!(recovered, expected);

    // Closing a recovered tab cleanly removes its journal; the other keeps its own.
    let first = harness.state().tabs()[1]
        .recovery_journal()
        .unwrap()
        .to_path_buf();
    let second = harness.state().tabs()[2]
        .recovery_journal()
        .unwrap()
        .to_path_buf();
    harness.state_mut().close_tab(1);
    harness.run();
    harness.get_by_label("Don’t Save").click();
    harness.run();
    assert!(!first.exists());
    assert!(second.exists());
}

fn open_three(harness: &mut Harness<'_, MapMakerApp>, dir: &Path) {
    for name in ["a.ssmap", "b.ssmap"] {
        let path = saved_map(dir, name);
        harness.state_mut().open_path(&path);
    }
    harness.run();
    assert_eq!(harness.state().tabs().len(), 3);
}

#[test]
fn number_shortcuts_jump_to_home_a_tab_and_the_last_tab() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(None);
    open_three(&mut harness, dir.path());
    let command = egui::Modifiers::COMMAND;

    harness.key_press_modifiers(command, egui::Key::Num1);
    harness.run();
    assert!(harness.state().home_active());
    harness.key_press_modifiers(command, egui::Key::Num3);
    harness.run();
    assert!(!harness.state().home_active());
    assert_eq!(harness.state().active_tab(), 1);
    harness.key_press_modifiers(command, egui::Key::Num2);
    harness.run();
    assert_eq!(harness.state().active_tab(), 0);
    harness.key_press_modifiers(command, egui::Key::Num9);
    harness.run();
    assert_eq!(harness.state().active_tab(), 2);
    // No fifth tab: nothing moves.
    harness.key_press_modifiers(command, egui::Key::Num6);
    harness.run();
    assert_eq!(harness.state().active_tab(), 2);
}

#[test]
fn control_tab_cycles_through_home_and_the_maps() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(None);
    open_three(&mut harness, dir.path());

    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    harness.run();
    assert!(harness.state().home_active());
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    harness.run();
    assert!(!harness.state().home_active());
    assert_eq!(harness.state().active_tab(), 0);
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    harness.run();
    assert_eq!(harness.state().active_tab(), 1);
    // Backwards (the wrap from Home to the last tab is tested on `tab_strip::cycle`; egui's
    // Shift+Tab focus handling leaves the focus on a widget of the tab left, which the test
    // harness's accessibility tree rejects).
    harness.key_press_modifiers(
        egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        egui::Key::Tab,
    );
    harness.run();
    assert_eq!(harness.state().active_tab(), 0);
}

#[test]
fn close_tab_shortcut_closes_a_map_but_never_home() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(None);
    open_three(&mut harness, dir.path());
    let command = egui::Modifiers::COMMAND;

    harness.key_press_modifiers(command, egui::Key::W);
    harness.run();
    assert_eq!(harness.state().tabs().len(), 2);
    harness.key_press_modifiers(command, egui::Key::Num1);
    harness.run();
    harness.key_press_modifiers(command, egui::Key::W);
    harness.run();
    assert_eq!(harness.state().tabs().len(), 2);
    assert!(harness.state().home_active());
}

#[test]
fn new_map_shortcut_and_plus_open_the_dialog() {
    let mut harness = app(None);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::T);
    harness.run();
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(harness.query_by_label("Cancel").is_none());
    harness.get_by_label("New Map").click();
    harness.run();
    harness.get_by_label("Cancel");
}

#[test]
fn clicking_home_and_a_map_tab_switches_between_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(None);
    open_three(&mut harness, dir.path());
    harness.get_by_label("Home").click();
    harness.run();
    assert!(harness.state().home_active());
    harness.get_by_label("a.ssmap").click();
    harness.run();
    assert!(!harness.state().home_active());
    assert_eq!(harness.state().active_tab(), 1);
}
