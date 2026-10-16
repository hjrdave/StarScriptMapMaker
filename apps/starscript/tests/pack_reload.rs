//! Installing or deleting a pack while the app runs: open Maps and the Asset picker see it
//! without a restart, and a Stamp whose pack is gone shows a placeholder (no window needed).

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::Harness;
use ssmap_app::MapMakerApp;
use ssmap_core::asset::PackRoots;
use ssmap_core::{Canvas, DistanceUnit, Scale};

fn roots(with_pack: bool) -> PackRoots {
    // An empty directory stands for the packs after the only one was deleted.
    let packs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/core/tests/packs");
    PackRoots {
        built_in: if with_pack {
            packs
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("no-such-packs")
        },
        user: None,
    }
}

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness
        .state_mut()
        .new_map(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    harness.run();
    harness
}

#[test]
fn a_reloaded_library_reaches_open_maps_and_the_picker() {
    let mut harness = app();
    let state = harness.state_mut();
    state.reload_packs(&roots(true));
    let store = state.sprites().clone();
    let editor = state.active_editor_mut().unwrap();
    editor.set_sprites(store.clone());
    assert!(
        editor
            .sprites()
            .unwrap()
            .library()
            .pack("test-pack")
            .is_some()
    );
    editor.selected_asset = Some("test-pack/mountain-1".parse().unwrap());
    let stamp = editor.place_stamp([10.0, 10.0]).expect("a Stamp is placed");
    assert!(editor.stamp_bounds(stamp).is_some());
    assert!(editor.missing_stamps().is_empty());

    // Deleting the pack: the same store, and so the same Editor, now has no such Asset.
    harness.state_mut().reload_packs(&roots(false));
    harness.run();
    let editor = harness.state_mut().active_editor_mut().unwrap();
    assert!(
        editor
            .sprites()
            .unwrap()
            .library()
            .pack("test-pack")
            .is_none()
    );
    assert!(editor.stamp_bounds(stamp).is_none());
    let missing = editor.missing_stamps();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].0, [10.0, 10.0]);
    // Placing picks a random member of the Variant group, so name whichever it got.
    assert_eq!(missing[0].2, editor.stamp(stamp).unwrap().asset.to_string());

    // Installing it again brings the art back.
    harness.state_mut().reload_packs(&roots(true));
    harness.run();
    let editor = harness.state_mut().active_editor_mut().unwrap();
    assert!(editor.stamp_bounds(stamp).is_some());
    assert!(editor.missing_stamps().is_empty());
}
