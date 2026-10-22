//! Sprites rasterise on background threads: a sprite that is not done reads as pending, never
//! as missing, and a repaint is requested as each one finishes. CPU only.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ssmap_core::asset::{AssetRef, PackLibrary, PackRoots};
use ssmap_render::{SpriteError, SpriteState, SpriteStore};

fn library() -> Arc<PackLibrary> {
    let roots = PackRoots {
        built_in: Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    Arc::new(library)
}

fn reference(asset: &str) -> AssetRef {
    AssetRef::new("test-pack", asset).unwrap()
}

/// Polls until `asset` is no longer pending.
fn settle(store: &SpriteStore, asset: &AssetRef) -> SpriteState {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match store.poll(asset) {
            SpriteState::Pending => {
                assert!(Instant::now() < deadline, "still pending");
                std::thread::sleep(Duration::from_millis(5));
            }
            state => return state,
        }
    }
}

#[test]
fn a_sprite_not_built_yet_is_pending_then_ready() {
    let store = SpriteStore::new(library());
    let asset = reference("mountain-1");
    assert!(matches!(store.poll(&asset), SpriteState::Pending));
    assert!(matches!(settle(&store, &asset), SpriteState::Ready(_)));
    // Built once: later polls find it.
    let done = store.completed();
    assert!(matches!(store.poll(&asset), SpriteState::Ready(_)));
    assert_eq!(store.completed(), done);
}

#[test]
fn pending_is_not_cached_as_missing() {
    let store = SpriteStore::new(library());
    let asset = reference("mountain-1");
    for _ in 0..3 {
        // Whatever the thread has reached, this is never Missing for a real Asset.
        assert!(!matches!(store.poll(&asset), SpriteState::Missing));
    }
    assert!(matches!(settle(&store, &asset), SpriteState::Ready(_)));
}

#[test]
fn many_assets_prefetched_all_become_ready() {
    let store = SpriteStore::new(library());
    let assets: Vec<AssetRef> = (0..300)
        .map(|i| reference(&format!("absent-{i}")))
        .collect();
    store.prefetch(&assets);
    store.wait_until_ready();
    assert_eq!(store.pending(), 0);
    for asset in &assets {
        assert!(matches!(store.poll(asset), SpriteState::Missing));
    }
}

#[test]
fn an_unknown_asset_ends_up_missing() {
    let store = SpriteStore::new(library());
    let asset = reference("no-such-asset");
    assert!(matches!(settle(&store, &asset), SpriteState::Missing));
}

#[test]
fn a_repaint_is_requested_when_a_raster_finishes() {
    let store = SpriteStore::new(library());
    let repaints = Arc::new(AtomicUsize::new(0));
    let counter = repaints.clone();
    store.set_repaint(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let asset = reference("mountain-1");
    settle(&store, &asset);
    let deadline = Instant::now() + Duration::from_secs(30);
    // The repaint follows the sprite landing, from the same thread.
    while repaints.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "no repaint");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(repaints.load(Ordering::SeqCst), 1);
    assert_eq!(store.completed(), 1);
}

#[test]
fn a_synchronous_store_is_ready_at_once() {
    let store = SpriteStore::synchronous(library());
    assert!(matches!(
        store.poll(&reference("mountain-1")),
        SpriteState::Ready(_)
    ));
}

#[test]
fn get_waits_for_a_pending_sprite() {
    let store = SpriteStore::new(library());
    let asset = reference("mountain-1");
    assert!(matches!(store.poll(&asset), SpriteState::Pending));
    assert!(store.wait_for(&asset).is_some());
    assert_eq!(store.completed(), 1);
}

#[test]
fn a_broken_art_file_keeps_its_error() {
    let dir = tempfile::tempdir().unwrap();
    let pack = dir.path().join("broken-pack");
    std::fs::create_dir_all(pack.join("art")).unwrap();
    std::fs::write(pack.join("art/bad.svg"), "this is not an svg").unwrap();
    std::fs::write(
        pack.join("pack.toml"),
        "[pack]\n\
         id = \"broken-pack\"\n\
         name = \"Broken pack\"\n\
         licence = \"CC0-1.0\"\n\
         credits = \"Test.\"\n\
         author = \"Tests\"\n\
         modified = \"2026-10-13\"\n\
         \n\
         [[asset]]\n\
         id = \"bad\"\n\
         name = \"Bad\"\n\
         category = \"mountains\"\n\
         file = \"art/bad.svg\"\n\
         anchor = [0.5, 0.5]\n\
         width = 64.0\n",
    )
    .unwrap();
    let roots = PackRoots {
        built_in: dir.path().to_path_buf(),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    let store = SpriteStore::synchronous(Arc::new(library));
    let asset = AssetRef::new("broken-pack", "bad").unwrap();
    assert!(matches!(store.poll(&asset), SpriteState::Missing));
    let error = store.error(&asset).expect("the error is kept");
    let message = error.to_string();
    assert!(message.contains("broken-pack/bad"), "{message}");
    assert!(message.contains("bad.svg"), "{message}");
    assert!(matches!(*error, SpriteError::Art { .. }));
}

#[test]
fn an_unknown_asset_is_unknown_not_a_load_failure() {
    let store = SpriteStore::synchronous(library());
    let asset = reference("no-such-asset");
    store.poll(&asset);
    assert!(matches!(
        store.error(&asset).as_deref(),
        Some(SpriteError::Unknown(_))
    ));
}
