//! The Recovery journal in the real app (ADR 0005): edits survive a crash, and closing
//! cleanly or saving leaves no journal behind.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
mod common;

use ssmap_app::MapMakerApp;
use ssmap_core::file::{MapFile, recovery_path};
use ssmap_core::{Canvas, DistanceUnit, Id, Layer, Map, Node, Op, Scale, Subtree};

/// Environment variables telling `edit_child` which Map file to edit and where journals of
/// Maps never saved go.
const CHILD_FILE: &str = "SSMAP_TEST_RECOVERY_CHILD_FILE";
const CHILD_DIR: &str = "SSMAP_TEST_RECOVERY_CHILD_DIR";

fn app(recovery_dir: &Path) -> Harness<'static, MapMakerApp> {
    let recovery_dir = recovery_dir.to_path_buf();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| MapMakerApp::with_recovery_dir(cc, Some(recovery_dir.clone())));
    // These tests cover the quit path, which is what closing does off macOS.
    harness.state_mut().set_close_hides_window(false);
    harness.run();
    // The app starts on Home with no Map; these tests edit one.
    harness.state_mut().new_map(Canvas::new(
        4096,
        3072,
        Scale::new(0.25, DistanceUnit::Miles),
    ));
    harness.run();
    harness
}

/// Runs frames until every edit has reached the Recovery journal.
fn wait_until_journaled(harness: &mut Harness<'_, MapMakerApp>) {
    let started = Instant::now();
    loop {
        // Single frames: a flush due within one frame asks for an immediate repaint.
        harness.step();
        if !harness.state().has_unjournaled_changes() {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the journal was never written"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn new_layer(harness: &mut Harness<'_, MapMakerApp>) {
    common::new_layer(harness);
}

fn layer_names(map: &Map) -> Vec<String> {
    map.children(None)
        .iter()
        .map(|&id| match map.node(id) {
            Some(Node::Layer(layer)) => layer.name.clone(),
            other => panic!("{other:?}"),
        })
        .collect()
}

/// A saved Map with one Layer named `name`.
fn saved_map(path: &Path, name: &str) {
    let mut map = Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    map.apply(Op::Insert {
        parent: None,
        index: 0,
        subtree: Subtree {
            id: Id::new(),
            node: Node::Layer(Layer::new(name)),
            children: Vec::new(),
        },
    })
    .unwrap();
    MapFile::create(path, &map, None).unwrap();
}

/// Run by the kill tests as a child process: opens the Map file, makes edits through the UI,
/// waits until they reach the journal, then hangs until killed.
#[test]
#[ignore = "runs only as the child of the recovery kill tests"]
fn edit_child() {
    let (Ok(file), Ok(dir)) = (std::env::var(CHILD_FILE), std::env::var(CHILD_DIR)) else {
        return;
    };
    let mut harness = app(Path::new(&dir));
    assert!(harness.state_mut().open_path(Path::new(&file)));
    harness.run();
    // Three new Layers, one undone and redone, one undone: two survive.
    for _ in 0..3 {
        new_layer(&mut harness);
    }
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    wait_until_journaled(&mut harness);
    println!(
        "journaled {:?}",
        layer_names(harness.state().editor().map())
    );
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

/// Edits `file` in a child process and kills it once the edits are journaled. Returns the
/// Layer names the child had.
fn edit_and_kill(file: &Path, dir: &Path) -> String {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "edit_child", "--ignored", "--nocapture"])
        .env(CHILD_FILE, file)
        .env(CHILD_DIR, dir)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut names = None;
    for line in stdout.lines() {
        if let Some(rest) = line.unwrap().strip_prefix("journaled ") {
            names = Some(rest.to_owned());
            break;
        }
    }
    child.kill().unwrap();
    child.wait().unwrap();
    names.expect("the child journaled its edits")
}

#[test]
fn killing_the_app_after_edits_then_reopening_replays_them() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("coast.ssmap");
    saved_map(&file, "Base");
    let saved_bytes = std::fs::read(&file).unwrap();

    let child_names = edit_and_kill(&file, dir.path());
    assert_eq!(child_names, r#"["Base", "Layer 2", "Layer 3"]"#);
    let journal = recovery_path(&file);
    assert!(journal.exists());
    assert_eq!(std::fs::read(&file).unwrap(), saved_bytes, "nothing saved");

    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    harness.run();
    assert_eq!(layer_names(harness.state().editor().map()), ["Base"]);
    harness.get_by_label("Recover Changes").click();
    harness.run();
    let state = harness.state();
    assert_eq!(
        layer_names(state.editor().map()),
        ["Base", "Layer 2", "Layer 3"]
    );
    assert!(state.document().has_unsaved_changes(state.editor().map()));
    assert!(
        !state.editor().can_undo(),
        "recovered changes are not undo steps"
    );
    // The journal goes on holding the recovered changes until a save.
    assert_eq!(state.recovery_journal(), Some(journal.as_path()));

    // A second crash before saving still recovers everything.
    drop(harness);
    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    harness.run();
    harness.get_by_label("Recover Changes").click();
    harness.run();
    new_layer(&mut harness);
    wait_until_journaled(&mut harness);
    drop(harness);
    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    harness.run();
    harness.get_by_label("Recover Changes").click();
    harness.run();
    assert_eq!(
        layer_names(harness.state().editor().map()),
        ["Base", "Layer 2", "Layer 3", "Layer 4"]
    );

    assert!(harness.state_mut().save());
    harness.run();
    assert!(!journal.exists(), "Save deletes the journal");
    let (_, saved) = MapFile::open(&file).unwrap();
    assert_eq!(layer_names(&saved).len(), 4);
}

#[test]
fn a_replaced_map_file_gets_a_separate_recovered_copy_and_is_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("coast.ssmap");
    saved_map(&file, "Base");
    edit_and_kill(&file, dir.path());
    // Another program replaces the file: a new save generation.
    saved_map(&file, "Elsewhere");
    let replaced = std::fs::read(&file).unwrap();

    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    harness.run();
    assert!(harness.query_by_label("Recover Changes").is_none());
    harness.get_by_label("Save Recovered Copy").click();
    harness.run();

    let copy = dir.path().join("coast (recovered).ssmap");
    let state = harness.state();
    assert_eq!(
        state.editor().notice().map(|n| n.text.as_str()),
        Some("Saved the recovered changes as “coast (recovered).ssmap”")
    );
    assert_eq!(layer_names(state.editor().map()), ["Elsewhere"]);
    assert!(!state.document().has_unsaved_changes(state.editor().map()));
    assert_eq!(
        std::fs::read(&file).unwrap(),
        replaced,
        "the file is untouched"
    );
    assert!(!recovery_path(&file).exists());
    let (_, recovered) = MapFile::open(&copy).unwrap();
    assert_eq!(layer_names(&recovered), ["Elsewhere", "Layer 2", "Layer 3"]);
}

#[test]
fn discarding_recovered_changes_deletes_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("coast.ssmap");
    saved_map(&file, "Base");
    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    new_layer(&mut harness);
    wait_until_journaled(&mut harness);
    drop(harness);

    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    harness.run();
    harness.get_by_label("Discard").click();
    harness.run();
    assert!(!recovery_path(&file).exists());
    assert_eq!(layer_names(harness.state().editor().map()), ["Base"]);
}

fn journals_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default()
}

#[test]
fn a_map_never_saved_journals_under_the_data_dir_and_is_offered_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(dir.path());
    new_layer(&mut harness);
    new_layer(&mut harness);
    wait_until_journaled(&mut harness);
    let journal = harness.state().recovery_journal().unwrap().to_path_buf();
    assert_eq!(journals_in(dir.path()), std::slice::from_ref(&journal));
    // Another window doesn't offer a journal still being written.
    let other = app(dir.path());
    assert!(other.query_by_label("Recover").is_none());
    drop(other);
    drop(harness);

    let mut harness = app(dir.path());
    harness.get_by_label("Recover").click();
    harness.run();
    let state = harness.state();
    // The starter template's four Layers, then the two made above.
    assert_eq!(
        layer_names(state.editor().map()),
        ["Backdrop", "Land", "Assets", "Labels", "Layer 5", "Layer 6"]
    );
    assert!(state.document().path().is_none());
    assert!(state.document().has_unsaved_changes(state.editor().map()));
    assert_eq!(state.recovery_journal(), Some(journal.as_path()));

    // Saving it deletes the journal under the data dir.
    let file = dir.path().join("saved.ssmap");
    assert!(harness.state_mut().save_as_path(&file));
    harness.run();
    assert_eq!(journals_in(dir.path()), [file]);
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
fn closing_with_unsaved_changes_asks_and_a_clean_close_deletes_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("coast.ssmap");
    saved_map(&file, "Base");
    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    new_layer(&mut harness);
    wait_until_journaled(&mut harness);

    request_close(&mut harness);
    harness.get_by_label("Save changes to “coast.ssmap”?");
    assert!(!harness.state().is_closing());
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(!harness.state().is_closing());
    assert!(recovery_path(&file).exists());

    request_close(&mut harness);
    harness.get_by_label("Don’t Save").click();
    harness.run();
    assert!(harness.state().is_closing());
    assert!(!recovery_path(&file).exists());
    let (_, saved) = MapFile::open(&file).unwrap();
    assert_eq!(layer_names(&saved), ["Base"]);
}

#[test]
fn saving_from_the_close_prompt_saves_then_closes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("coast.ssmap");
    saved_map(&file, "Base");
    let mut harness = app(dir.path());
    assert!(harness.state_mut().open_path(&file));
    new_layer(&mut harness);
    request_close(&mut harness);
    harness.get_by_label("Save").click();
    harness.run();
    assert!(harness.state().is_closing());
    assert!(!recovery_path(&file).exists());
    let (_, saved) = MapFile::open(&file).unwrap();
    assert_eq!(layer_names(&saved), ["Base", "Layer 2"]);
}

#[test]
fn a_journal_that_cannot_be_deleted_is_reported_on_home_until_dismissed() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(dir.path());
    new_layer(&mut harness);
    wait_until_journaled(&mut harness);
    let journal = harness.state().recovery_journal().unwrap().to_path_buf();
    // A directory where the journal was can't be removed as a file.
    std::fs::remove_file(&journal).unwrap();
    std::fs::create_dir(&journal).unwrap();
    assert!(harness.state().home_notices().is_empty());

    // Closing the only Tab leaves no Tab to show the error on, so Home lists it.
    harness.state_mut().close_tab(0);
    harness.run();
    harness.get_by_label("Don’t Save").click();
    harness.run();
    let notices = harness.state().home_notices();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].starts_with("Couldn’t delete the unsaved changes"));

    harness.get_by_label("Dismiss").click();
    harness.run();
    assert!(harness.state().home_notices().is_empty());
}

#[test]
fn closing_without_unsaved_changes_does_not_ask() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(dir.path());
    request_close(&mut harness);
    assert!(harness.query_by_label("Don’t Save").is_none());
}
