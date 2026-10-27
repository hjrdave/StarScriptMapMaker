//! Adding a pack from the Asset packs page: the folder dialog button, drag and drop, progress
//! and the result message (no window needed).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::pack_install::Status;
use ssmap_core::asset::PackRoots;
use tempfile::TempDir;

/// A dropped file, as winit hands one to egui.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        fs::read(&self.0).map_err(|err| err.to_string())
    }
}

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

struct Fixture {
    harness: Harness<'static, MapMakerApp>,
    /// A valid pack folder to add.
    pack: PathBuf,
    user: PathBuf,
    _dir: TempDir,
}

/// The app on its Asset packs page, with no packs installed and an empty user packs folder.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let pack = dir.path().join("downloads/test-pack");
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/core/tests/packs/test-pack");
    copy_dir(&source, &pack);
    let user = dir.path().join("user-packs");
    fs::create_dir_all(&user).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.state_mut().set_pack_roots(PackRoots {
        built_in: dir.path().join("no-built-in-packs"),
        user: Some(user.clone()),
    });
    harness.state_mut().reload_packs(&PackRoots {
        built_in: dir.path().join("no-built-in-packs"),
        user: Some(user.clone()),
    });
    harness.run();
    harness.get_by_label_contains("Asset packs").click();
    harness.run();
    Fixture {
        harness,
        pack,
        user,
        _dir: dir,
    }
}

impl Fixture {
    fn drop_folder(&mut self, folder: &Path) {
        self.harness
            .input_mut()
            .dropped_files
            .push(std::sync::Arc::new(Dropped(folder.to_path_buf())));
        self.harness.step();
    }

    /// Runs frames until the install has finished.
    fn finish(&mut self) -> Status {
        for _ in 0..500 {
            self.harness.step();
            let state = self.harness.state();
            if let Some(status) = state.pack_install_status()
                && !matches!(status, Status::Installing { .. })
            {
                return status.clone();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the install did not finish");
    }

    fn installed(&self, id: &str) -> bool {
        self.harness.state().sprites().library().pack(id).is_some()
    }
}

#[test]
fn add_pack_opens_the_folder_dialog() {
    let mut f = fixture();
    assert!(!f.harness.state().pack_dialog_open());
    f.harness.get_by_label("Add pack…").click();
    f.harness.run();
    assert!(f.harness.state().pack_dialog_open());
}

#[test]
fn a_pack_folder_is_added_and_shows_up() {
    let mut f = fixture();
    assert!(!f.installed("test-pack"));
    let ctx = f.harness.ctx.clone();
    let folder = f.pack.clone();
    f.harness.state_mut().add_pack(&ctx, folder);
    match f.finish() {
        Status::Done(message) => assert!(message.contains("Added"), "{message}"),
        other => panic!("unexpected {other:?}"),
    }
    assert!(f.installed("test-pack"));
    assert!(f.user.join("test-pack/pack.toml").is_file());
    // The page lists the new pack, and the message can be dismissed.
    f.harness.run();
    f.harness.get_by_label_contains("Open Test pack");
    f.harness.get_by_label("Dismiss").click();
    f.harness.run();
    assert!(f.harness.state().pack_install_status().is_none());
}

#[test]
fn an_invalid_folder_is_refused_with_the_reason() {
    let mut f = fixture();
    let empty = f.user.parent().unwrap().join("empty");
    fs::create_dir_all(&empty).unwrap();
    f.drop_folder(&empty);
    match f.finish() {
        Status::Failed(message) => {
            assert!(message.starts_with("Couldn’t add the pack"), "{message}");
            assert!(message.contains("pack.toml"), "{message}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(fs::read_dir(&f.user).unwrap().next().is_none());
}

#[test]
fn a_pack_that_is_already_installed_is_refused() {
    let mut f = fixture();
    f.drop_folder(&f.pack.clone());
    assert!(matches!(f.finish(), Status::Done(_)));
    f.harness.get_by_label("Dismiss").click();
    f.harness.run();
    f.drop_folder(&f.pack.clone());
    match f.finish() {
        Status::Failed(message) => assert!(message.contains("already installed"), "{message}"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn dropping_a_folder_on_the_page_adds_it() {
    let mut f = fixture();
    f.drop_folder(&f.pack.clone());
    assert!(matches!(f.finish(), Status::Done(_)));
    assert!(f.installed("test-pack"));
}

#[test]
fn a_dropped_file_that_is_not_a_folder_is_ignored() {
    let mut f = fixture();
    let file = f.pack.join("pack.toml");
    f.drop_folder(&file);
    f.harness.run();
    assert!(f.harness.state().pack_install_status().is_none());
}
