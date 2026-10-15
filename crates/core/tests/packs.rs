//! Asset pack manifests and the pack loader.

use std::path::{Path, PathBuf};

use ssmap_core::AssetRef;
use ssmap_core::asset::{
    ArtFormat, LoadProblem, PackLibrary, PackRoots, PackSource, parse_manifest, read_pack,
};

fn test_packs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/packs")
}

/// The errors for `text`, as a user would read them.
fn errors(text: &str) -> String {
    parse_manifest(Path::new("bad/pack.toml"), text, PackSource::User)
        .expect_err("the manifest is invalid")
        .to_string()
}

const VALID_PACK: &str = r#"
[pack]
id = "p"
name = "P"
licence = "CC0-1.0"
credits = "Nobody."
"#;

#[test]
fn the_test_pack_loads() {
    let pack = read_pack(&test_packs().join("test-pack"), PackSource::BuiltIn).unwrap();
    assert_eq!(pack.id, "test-pack");
    assert_eq!(pack.licence, "CC0-1.0");
    assert!(!pack.attribution_required);
    let ids: Vec<&str> = pack.assets.iter().map(|asset| asset.id.as_str()).collect();
    assert_eq!(ids, ["mountain-1", "mountain-2", "tree", "castle", "well"]);

    let mountain = pack.asset("mountain-1").unwrap();
    assert_eq!(mountain.format, ArtFormat::Svg);
    assert_eq!(mountain.anchor, [0.5, 0.92]);
    assert_eq!(mountain.width, 64.0);
    assert_eq!(mountain.tags, ["peak", "rocky"]);
    // Inherited from [pack].
    assert_eq!(mountain.author, "StarScript Map Maker contributors");
    assert_eq!(mountain.licence, "CC0-1.0");
    assert_eq!(mountain.modified, "2026-10-13");
    assert_eq!(mountain.source_url, None);
    assert!(pack.path(&mountain.file).is_file());

    let well = pack.asset("well").unwrap();
    assert_eq!(well.format, ArtFormat::Png);
    assert_eq!(well.modified, "2026-10-12", "a bare TOML date is accepted");

    let group: Vec<&str> = pack
        .variant_group("mountains")
        .map(|asset| asset.id.as_str())
        .collect();
    assert_eq!(group, ["mountain-1", "mountain-2"]);

    for asset in &pack.assets {
        let silhouette = asset
            .silhouette
            .as_ref()
            .unwrap_or_else(|| panic!("{} has a Silhouette; run the silhouettes tool", asset.id));
        assert!(pack.path(silhouette).is_file());
    }
}

#[test]
fn the_library_finds_packs_by_reference() {
    let roots = PackRoots {
        built_in: test_packs(),
        user: Some(test_packs().join("does-not-exist")),
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert_eq!(problems, []);
    assert_eq!(library.packs().len(), 1);
    let reference: AssetRef = "test-pack/castle".parse().unwrap();
    let (pack, asset) = library.asset(&reference).unwrap();
    assert_eq!(pack.source, PackSource::BuiltIn);
    assert_eq!(asset.name, "Castle");
    assert!(
        library
            .asset(&"test-pack/dragon".parse().unwrap())
            .is_none()
    );
}

#[test]
fn a_broken_pack_does_not_stop_the_others() {
    let user = tempfile::tempdir().unwrap();
    // Same id as a built-in pack: skipped.
    copy_dir(&test_packs().join("test-pack"), &user.path().join("a-copy"));
    // Not a pack: ignored.
    std::fs::create_dir(user.path().join("notes")).unwrap();
    // Invalid.
    let broken = user.path().join("broken");
    std::fs::create_dir(&broken).unwrap();
    std::fs::write(broken.join("pack.toml"), "[pack]\nid = \"Broken!\"\n").unwrap();

    let roots = PackRoots {
        built_in: test_packs(),
        user: Some(user.path().to_path_buf()),
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert_eq!(library.packs().len(), 1);
    assert_eq!(library.packs()[0].source, PackSource::BuiltIn);
    assert_eq!(problems.len(), 2, "{problems:#?}");
    assert!(matches!(&problems[0], LoadProblem::DuplicateId { id, .. } if id == "test-pack"));
    let LoadProblem::Manifest(errors) = &problems[1] else {
        panic!("{:?}", problems[1]);
    };
    assert_eq!(errors.0[0].file, broken.join("pack.toml"));
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

#[test]
fn missing_files_are_named() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = format!(
        "{VALID_PACK}
[[asset]]
id = \"ghost\"
name = \"Ghost\"
category = \"spirits\"
file = \"art/ghost.svg\"
anchor = [0.5, 1.0]
width = 10
silhouette = \"ghost-silhouette.png\"
author = \"Me\"
modified = \"2026-01-01\"
"
    );
    std::fs::write(dir.path().join("pack.toml"), manifest).unwrap();
    let errors = read_pack(dir.path(), PackSource::User).unwrap_err();
    let file = dir.path().join("pack.toml");
    let fields: Vec<_> = errors
        .0
        .iter()
        .map(|e| (&e.file, e.asset.as_deref(), e.field.as_deref()))
        .collect();
    assert_eq!(
        fields,
        [
            (&file, Some("ghost"), Some("file")),
            (&file, Some("ghost"), Some("silhouette")),
        ]
    );
}

#[test]
fn syntax_errors() {
    insta::assert_snapshot!(errors("[pack\nid = 1"));
}

#[test]
fn pack_table_errors() {
    insta::assert_snapshot!(errors(
        r#"
title = "stray"

[pack]
id = "My Pack"
name = ""
credits = 3
attribution_required = "yes"
licnece = "CC0-1.0"
source_url = "kenney.nl"
"#
    ));
}

#[test]
fn asset_errors() {
    insta::assert_snapshot!(errors(&format!(
        r#"{VALID_PACK}
[[asset]]
id = "ok"
name = "Fine"
category = "c"
file = "ok.svg"
anchor = [0.5, 1]
width = 4
author = "Me"
modified = "2026-01-01"

[[asset]]
name = "No id"
category = "c"
file = "../outside.svg"
anchor = [0.5]
width = -1
author = "Me"
modified = "2026-13-01"

[[asset]]
id = "ok"
name = "Duplicate"
category = "c"
file = "art\\dup.gif"
anchor = [1.5, 0.5]
width = "wide"
tags = ["a", 2]
variant_group = "Peaks"
source_url = "ftp://example.com"
silhouette = "sil.jpg"
anchr = [0, 0]
"#
    )));
}

#[test]
fn a_pack_without_assets_is_valid() {
    let pack = parse_manifest(Path::new("p/pack.toml"), VALID_PACK, PackSource::User).unwrap();
    assert_eq!(pack.dir, Path::new("p"));
    assert!(pack.assets.is_empty());
}

/// The default pack that ships in `assets/packs/default` (Kenney's Cartography Pack, CC0).
#[test]
fn the_shipped_default_pack_loads_and_is_the_default() {
    let built_in = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/packs");
    let pack = read_pack(&built_in.join("default"), PackSource::BuiltIn).unwrap();
    assert_eq!(pack.id, "default");
    assert_eq!(pack.licence, "CC0-1.0");
    assert!(
        pack.default,
        "the Asset browser selects the pack flagged default"
    );
    assert!(
        !pack.attribution_required,
        "Kenney's credit is optional, so the pack shows no credit"
    );
    assert!(!pack.assets.is_empty());

    for asset in &pack.assets {
        assert_eq!(asset.format, ArtFormat::Png, "{} is PNG art", asset.id);
        assert!(pack.path(&asset.file).is_file(), "{} has its art", asset.id);
        let silhouette = asset
            .silhouette
            .as_ref()
            .unwrap_or_else(|| panic!("{} has a Silhouette; run the silhouettes tool", asset.id));
        assert!(pack.path(silhouette).is_file(), "{} has its file", asset.id);
        assert_eq!(asset.author, "Kenney");
        assert_eq!(asset.licence, "CC0-1.0");
    }

    let roots = PackRoots {
        built_in,
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert_eq!(problems, []);
    assert_eq!(
        library.default_pack().map(|pack| pack.id.as_str()),
        Some("default")
    );
}
