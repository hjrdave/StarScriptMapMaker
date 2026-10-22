//! Swapping the installed packs under a live `SpriteStore`, as installing or deleting a pack
//! does. CPU only.

use std::path::Path;
use std::sync::Arc;

use ssmap_core::asset::{AssetRef, PackLibrary, PackRoots};
use ssmap_render::{SpriteState, SpriteStore, placeholder};

fn test_pack_library() -> Arc<PackLibrary> {
    let roots = PackRoots {
        built_in: Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    Arc::new(library)
}

fn mountain() -> AssetRef {
    "test-pack/mountain-1".parse().unwrap()
}

#[test]
fn a_swapped_in_library_makes_new_assets_appear_without_a_new_store() {
    let store = SpriteStore::synchronous(Arc::new(PackLibrary::default()));
    let clone = store.clone();
    assert!(store.wait_for_installed(&mountain()).is_none());
    assert!(matches!(store.poll(&mountain()), SpriteState::Missing));
    let before = store.generation();

    store.set_library(test_pack_library());

    assert_ne!(store.generation(), before, "renderers must upload again");
    assert!(
        clone.library().pack("test-pack").is_some(),
        "clones share it"
    );
    assert!(store.wait_for_installed(&mountain()).is_some());
    assert!(matches!(clone.poll(&mountain()), SpriteState::Ready(_)));
}

#[test]
fn a_swapped_out_library_makes_its_assets_missing_again() {
    let store = SpriteStore::synchronous(test_pack_library());
    assert!(matches!(store.poll(&mountain()), SpriteState::Ready(_)));

    store.set_library(Arc::new(PackLibrary::default()));

    assert!(matches!(store.poll(&mountain()), SpriteState::Missing));
    assert!(store.wait_for(&mountain()).is_none());
}

#[test]
fn the_placeholder_is_a_box_centred_on_the_anchor() {
    let sprite = placeholder();
    assert_eq!(sprite.shape.anchor, [0.5, 0.5]);
    assert!(sprite.contains([0.5, 0.5]));
    let (w, h) = (sprite.art.width, sprite.art.height);
    assert_eq!(sprite.art.pixel(0, 0)[3], 255, "opaque border");
    assert!(
        sprite.art.pixel(w / 2, h / 2)[3] < 255,
        "see-through middle"
    );
}
