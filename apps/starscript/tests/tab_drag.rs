//! Dragging a tab along the strip reorders it; dropping an Asset on the Canvas places a Stamp.

use eframe::egui;
use egui_kittest::Harness;
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

/// A point on the strip over the left half of Map tab `tab`, so a dropped tab lands before it.
fn on_strip(harness: &Harness<'_, MapMakerApp>, tab: usize) -> egui::Pos2 {
    let (strip, left, width) = harness.state().tab_layout();
    egui::pos2(left + width * tab as f32 + 5.0, strip.center().y)
}

#[test]
fn a_tab_dropped_on_the_strip_reorders() {
    let mut harness = app();
    harness.state_mut().run_command(Command::NewLayer);
    harness.run();
    let layers = harness.state().editor().map().children(None).len();
    // The active second tab goes before the first.
    let at = on_strip(&harness, 0);
    harness.state_mut().drop_tab(1, at);
    harness.run();
    assert_eq!(harness.state().tabs().len(), 2);
    assert_eq!(harness.state().active_tab(), 0);
    assert_eq!(harness.state().editor().map().children(None).len(), layers);
}

#[test]
fn a_tab_dropped_off_the_strip_stays_put() {
    let mut harness = app();
    harness.state_mut().run_command(Command::NewLayer);
    harness.run();
    let layers = harness.state().editor().map().children(None).len();
    harness.state_mut().drop_tab(1, egui::pos2(600.0, 400.0));
    harness.run();
    assert_eq!(harness.state().active_tab(), 1);
    assert_eq!(harness.state().editor().map().children(None).len(), layers);
}

#[test]
fn an_asset_dropped_on_the_canvas_is_placed_as_one_undo_step() {
    use ssmap_core::asset::{PackLibrary, PackRoots};
    use ssmap_render::SpriteStore;
    let mut harness = app();
    let roots = PackRoots {
        built_in: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    let layer = {
        let editor = harness.state_mut().active_editor_mut().unwrap();
        editor.set_sprites(SpriteStore::new(std::sync::Arc::new(library)));
        editor.new_layer().unwrap()
    };
    harness.run();
    let zone = harness.state().canvas_zone().expect("a Canvas");
    let asset = "test-pack/mountain-1".parse().unwrap();

    // Off the Canvas: nothing happens.
    harness.state_mut().drop_asset(asset, egui::pos2(5.0, 5.0));
    harness.run();
    assert!(
        harness
            .state()
            .editor()
            .map()
            .children(Some(layer))
            .is_empty()
    );

    let asset = "test-pack/mountain-1".parse().unwrap();
    harness.state_mut().drop_asset(asset, zone.center());
    harness.run();
    let editor = harness.state().editor();
    assert_eq!(editor.map().children(Some(layer)).len(), 1);
    assert_eq!(editor.undo_label(), Some("Add Stamp"));

    harness.state_mut().run_command(Command::Undo);
    harness.run();
    assert!(
        harness
            .state()
            .editor()
            .map()
            .children(Some(layer))
            .is_empty()
    );
}
