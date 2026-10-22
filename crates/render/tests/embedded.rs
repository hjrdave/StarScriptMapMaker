//! Rendering a Map from its embedded Assets and fonts (ADR 0005): without the packs, with
//! the packs changed, and with font files of its own. CPU only.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ssmap_core::asset::{AssetRef, PackLibrary, PackRoots, PackSource, read_pack};
use ssmap_core::{
    Blob, Canvas, Command, DistanceUnit, EmbedAsset, EmbedFont, Map, ResolvedLabel, Scale,
};
use ssmap_render::{Fonts, LabelLayout, SpriteStore};

fn pack_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/packs/test-pack")
}

/// A library of the packs under `root`.
fn library(root: &Path) -> Arc<PackLibrary> {
    let roots = PackRoots {
        built_in: root.to_path_buf(),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    Arc::new(library)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn map_embedding(assets: &[&str], fonts: Vec<EmbedFont>) -> Map {
    let pack = read_pack(&pack_source(), PackSource::BuiltIn).unwrap();
    let mut map = Map::new(Canvas::new(300, 200, Scale::new(1.0, DistanceUnit::Miles)));
    map.execute(&Command::EmbedAssets {
        assets: assets
            .iter()
            .map(|id| EmbedAsset::read(&pack, pack.asset(id).unwrap()).unwrap())
            .collect(),
        fonts,
    })
    .unwrap();
    map
}

fn reference(asset: &str) -> AssetRef {
    AssetRef::new("test-pack", asset).unwrap()
}

#[test]
fn embedded_art_renders_without_the_pack() {
    let installed = tempfile::tempdir().unwrap();
    copy_dir(&pack_source(), &installed.path().join("test-pack"));
    let with_pack = SpriteStore::new(library(installed.path()));
    let expected = with_pack.wait_for(&reference("mountain-1")).unwrap();

    let empty = tempfile::tempdir().unwrap();
    let without_pack = SpriteStore::new(library(empty.path()));
    assert!(without_pack.wait_for(&reference("mountain-1")).is_none());

    let map = map_embedding(&["mountain-1", "well"], vec![]);
    assert!(without_pack.set_embedded(map.embedded()));
    for asset in ["mountain-1", "well"] {
        assert!(
            without_pack.wait_for(&reference(asset)).is_some(),
            "{asset}"
        );
    }
    let sprite = without_pack.wait_for(&reference("mountain-1")).unwrap();
    assert_eq!(sprite.art, expected.art);
    assert_eq!(sprite.silhouette, expected.silhouette);
    assert_eq!(sprite.shape, expected.shape);
}

#[test]
fn the_embedded_copy_wins_over_an_updated_pack() {
    let installed = tempfile::tempdir().unwrap();
    let pack_dir = installed.path().join("test-pack");
    copy_dir(&pack_source(), &pack_dir);
    let store = SpriteStore::new(library(installed.path()));
    let map = map_embedding(&["mountain-1"], vec![]);
    store.set_embedded(map.embedded());
    let embedded = store.wait_for(&reference("mountain-1")).unwrap();

    // The pack now ships different art under the same name.
    std::fs::copy(
        pack_dir.join("art/tree.svg"),
        pack_dir.join("art/mountain-1.svg"),
    )
    .unwrap();
    let updated = SpriteStore::new(library(installed.path()));
    assert_ne!(
        updated.wait_for(&reference("mountain-1")).unwrap().art,
        embedded.art
    );
    updated.set_embedded(map.embedded());
    assert_eq!(
        updated.wait_for(&reference("mountain-1")).unwrap().art,
        embedded.art
    );
    // The Asset browser still shows what is installed.
    assert_ne!(
        updated
            .wait_for_installed(&reference("mountain-1"))
            .unwrap()
            .art,
        embedded.art
    );
}

#[test]
fn assets_the_map_does_not_embed_come_from_the_packs() {
    let installed = tempfile::tempdir().unwrap();
    copy_dir(&pack_source(), &installed.path().join("test-pack"));
    let store = SpriteStore::new(library(installed.path()));
    store.set_embedded(map_embedding(&["mountain-1"], vec![]).embedded());
    assert!(store.wait_for(&reference("tree")).is_some());
}

#[test]
fn the_generation_moves_only_when_the_embedded_assets_change() {
    let empty = tempfile::tempdir().unwrap();
    let store = SpriteStore::new(library(empty.path()));
    let start = store.generation();
    let map = map_embedding(&["tree"], vec![]);
    assert!(store.set_embedded(map.embedded()));
    assert_eq!(store.generation(), start + 1);
    assert!(!store.set_embedded(map.embedded()));
    assert_eq!(store.generation(), start + 1);
    assert!(store.set_embedded(&Default::default()));
    assert!(store.wait_for(&reference("tree")).is_none());
}

fn width(fonts: &mut Fonts, family: &str) -> f64 {
    let label = ResolvedLabel {
        text: "Hamlet".into(),
        font: ssmap_core::FontFace {
            family: family.into(),
            ..Default::default()
        },
        size: 100.0,
        ..ResolvedLabel::default()
    };
    let [min, max] = LabelLayout::shape(fonts, &label).bounds();
    max[0] - min[0]
}

fn bundled(file_prefix: &str) -> &'static ssmap_fonts::BundledFont {
    ssmap_fonts::FONTS
        .iter()
        .find(|font| font.file.starts_with(file_prefix))
        .unwrap()
}

#[test]
fn unchanged_bundled_fonts_need_no_override() {
    let font = bundled("Cinzel");
    let map = map_embedding(
        &[],
        vec![EmbedFont {
            family: font.family.into(),
            file: font.file.into(),
            licence: "OFL".into(),
            data: Blob::new(font.data),
        }],
    );
    assert!(Fonts::embedded_overrides(map.embedded()).is_empty());
}

#[test]
fn an_embedded_font_wins_over_the_bundled_one() {
    let cinzel = bundled("Cinzel");
    let noto = bundled("NotoSerif[");
    // A Map whose "Cinzel" file is really Noto Serif.
    let map = map_embedding(
        &[],
        vec![EmbedFont {
            family: cinzel.family.into(),
            file: cinzel.file.into(),
            licence: "OFL".into(),
            data: Blob::new(noto.data),
        }],
    );
    assert_eq!(Fonts::embedded_overrides(map.embedded()).len(), 1);
    let plain = width(&mut Fonts::bundled(), "Cinzel");
    let embedded = width(&mut Fonts::with_embedded(map.embedded()), "Cinzel");
    assert!((plain - embedded).abs() > 1.0, "{plain} vs {embedded}");
}
