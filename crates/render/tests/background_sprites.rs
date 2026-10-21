//! Sprites rasterise on background threads: a sprite that is not done reads as pending, never
//! as missing, and a repaint is requested as each one finishes. CPU only.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ssmap_core::asset::{AssetRef, PackLibrary, PackRoots};
use ssmap_render::{SpriteState, SpriteStore};

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
    assert!(store.get(&asset).is_some());
    assert_eq!(store.completed(), 1);
}
