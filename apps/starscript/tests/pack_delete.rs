//! Deleting a pack from its card on the Asset packs page: the menu, the confirmation, the
//! blocked built-in pack and the picker and Stamps afterwards (no window needed).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ssmap_app::MapMakerApp;
use ssmap_core::asset::{PackRoots, TrashError};
use ssmap_core::{Canvas, DistanceUnit, Scale};
use tempfile::TempDir;

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Stands in for the system Trash: the folder is gone from the packs folder.
fn fake_trash(dir: &Path) -> Result<(), TrashError> {
    fs::remove_dir_all(dir).map_err(|err| err.into())
}

struct Fixture {
    harness: Harness<'static, MapMakerApp>,
    user: PathBuf,
    _dir: TempDir,
}

fn test_packs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/core/tests/packs")
}

/// The app on its Asset packs page. `test-pack` is in the user packs folder, or built in.
fn fixture(built_in: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user-packs");
    fs::create_dir_all(&user).unwrap();
    let roots = if built_in {
        PackRoots {
            built_in: test_packs(),
            user: Some(user.clone()),
        }
    } else {
        copy_dir(&test_packs().join("test-pack"), &user.join("test-pack"));
        PackRoots {
            built_in: dir.path().join("no-built-in-packs"),
            user: Some(user.clone()),
        }
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    let state = harness.state_mut();
    state.set_pack_roots(roots.clone());
    state.set_pack_trash(fake_trash);
    state.reload_packs(&roots);
    state.new_map(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    harness.run();
    let store = harness.state().sprites().clone();
    harness
        .state_mut()
        .active_editor_mut()
        .unwrap()
        .set_sprites(store);
    harness.run();
    Fixture {
        harness,
        user,
        _dir: dir,
    }
}

impl Fixture {
    fn open_packs_page(&mut self) {
        self.harness.get_by_label("Home").click();
        self.harness.run();
        self.harness.get_by_label_contains("Asset packs").click();
        self.harness.run();
    }

    fn open_card_menu(&mut self) {
        self.harness.get_by_label("Pack menu").click();
        self.harness.run();
    }

    fn installed(&self) -> bool {
        self.harness
            .state()
            .sprites()
            .library()
            .pack("test-pack")
            .is_some()
    }
}

#[test]
fn confirming_moves_the_pack_to_the_trash_and_refreshes_the_picker() {
    let mut f = fixture(false);
    // A Stamp from the pack in the open Map, counted in the confirmation.
    let editor = f.harness.state_mut().active_editor_mut().unwrap();
    editor.selected_asset = Some("test-pack/mountain-1".parse().unwrap());
    let stamp = editor.place_stamp([10.0, 10.0]).expect("a Stamp is placed");
    f.open_packs_page();
    f.open_card_menu();
    f.harness.get_by_label_contains("Delete…").click();
    f.harness.run();
    assert!(f.harness.state().pack_delete_prompt_open());
    f.harness.get_by_label("Delete “Test pack”?");
    f.harness.get_by_label_contains("1 Stamp in the open Maps");
    assert_eq!(f.harness.state().pack_stamp_count("test-pack"), 1);
    // Nothing happens until it is confirmed.
    assert!(f.installed());

    f.harness.get_by_label("Delete").click();
    f.harness.run();
    assert!(!f.harness.state().pack_delete_prompt_open());
    assert!(!f.installed());
    assert!(!f.user.join("test-pack").exists());
    assert!(
        f.harness
            .query_by_label_contains("Open Test pack")
            .is_none()
    );
    // The Stamp now shows the placeholder, and the picker's library is without the pack.
    f.harness.get_by_label("Untitled").click();
    f.harness.run();
    let editor = f.harness.state_mut().active_editor_mut().unwrap();
    assert!(editor.stamp_bounds(stamp).is_none());
    assert_eq!(editor.missing_stamps().len(), 1);
}

#[test]
fn cancelling_keeps_the_pack() {
    let mut f = fixture(false);
    f.open_packs_page();
    f.open_card_menu();
    f.harness.get_by_label_contains("Delete…").click();
    f.harness.run();
    f.harness
        .get_by_label_contains("No Stamps in the open Maps");
    f.harness.get_by_label("Cancel").click();
    f.harness.run();
    assert!(!f.harness.state().pack_delete_prompt_open());
    assert!(f.installed());
    assert!(f.user.join("test-pack/pack.toml").is_file());
    f.harness.get_by_label_contains("Open Test pack");
}

#[test]
fn a_built_in_pack_cannot_be_deleted() {
    let mut f = fixture(true);
    f.open_packs_page();
    f.open_card_menu();
    let item = f.harness.get_by_label_contains("Delete…");
    assert!(item.accesskit_node().is_disabled());
    item.click();
    f.harness.run();
    assert!(!f.harness.state().pack_delete_prompt_open());
    assert!(f.installed());
}
