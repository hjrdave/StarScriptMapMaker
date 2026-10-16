//! The Home tab in the real app: it is where the app starts, and where Maps are found and
//! opened (no window needed).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_core::file::MapFile;
use ssmap_core::{Canvas, DistanceUnit, Map, Scale};

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness
}

fn saved_map(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let canvas = Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles));
    MapFile::create(&path, &Map::new(canvas), None).unwrap();
    path
}

#[test]
fn launching_with_no_arguments_shows_home_and_no_map_tab() {
    let harness = app();
    assert!(harness.state().home_active());
    assert!(harness.state().tabs().is_empty());
    // The New Map card, the Open… button and the rail's lists are there.
    harness.get_by_label("New Map: blank or from template");
    harness.get_by_label("Open…");
    // "All maps" is both a rail row and the title of the list it shows.
    assert_eq!(harness.get_all_by_label("All maps").count(), 2);
    harness.get_by_label("Recent");
    harness.get_by_label("Open in tabs");
    harness.get_by_label("Credits & licences");
    assert!(harness.query_by_label("New Layer").is_none());
}

#[test]
fn launching_with_a_file_opens_it_in_a_tab_and_focuses_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = saved_map(dir.path(), "first.ssmap");
    let mut harness = app();
    // What `main` does with a file argument.
    assert!(harness.state_mut().open_path(&path));
    harness.run();
    assert_eq!(harness.state().tabs().len(), 1);
    assert!(!harness.state().home_active());
}

#[test]
fn clicking_a_map_card_opens_it_and_an_open_map_is_focused_not_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let path = saved_map(dir.path(), "first.ssmap");
    let mut harness = app();
    harness.state_mut().open_path(&path);
    harness.state_mut().close_tab(0);
    harness.run();
    assert!(harness.state().home_active());
    assert!(harness.state().tabs().is_empty());

    harness.get_by_label("Open first").click();
    harness.run();
    assert_eq!(harness.state().tabs().len(), 1);
    assert!(!harness.state().home_active());

    // Back on Home the Map is still listed; clicking it shows its tab again.
    harness.get_by_label("Home").click();
    harness.run();
    assert!(harness.state().home_active());
    harness.get_by_label("Open first").click();
    harness.run();
    assert_eq!(harness.state().tabs().len(), 1);
    assert!(!harness.state().home_active());
}

#[test]
fn the_new_map_card_opens_the_new_map_dialog() {
    let mut harness = app();
    harness
        .get_by_label("New Map: blank or from template")
        .click();
    harness.run();
    harness.get_by_label("Cancel");
}

#[test]
fn a_missing_file_can_be_removed_from_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = saved_map(dir.path(), "gone.ssmap");
    let mut harness = app();
    harness.state_mut().open_path(&path);
    harness.state_mut().close_tab(0);
    std::fs::remove_file(&path).unwrap();
    harness.run();
    assert_eq!(harness.state().recent_files().len(), 1);

    // The file is read on a worker thread, so the card learns it is missing a little later.
    let started = Instant::now();
    while harness.query_by_label("Remove from list").is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the card never showed the file as missing"
        );
        std::thread::sleep(Duration::from_millis(20));
        harness.step();
    }
    harness.get_by_label("Remove from list").click();
    harness.run();
    assert!(harness.state().recent_files().is_empty());
}
