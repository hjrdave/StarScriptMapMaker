//! Assets and fonts embedded in a Map: the Ops, the Command, and the Map file.

use std::path::{Path, PathBuf};

use ssmap_core::asset::{AssetRef, PackSource, read_pack};
use ssmap_core::file::{FORMAT_VERSION, MapFile};
use ssmap_core::{
    Blob, Canvas, Command, ContentHash, DistanceUnit, EmbedAsset, EmbedFont, History, Id, Map, Op,
    OpError, Scale, Stamp,
};

fn empty_map() -> Map {
    Map::new(Canvas::new(300, 200, Scale::new(1.0, DistanceUnit::Miles)))
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

/// Installs the test pack into `dir` as a user pack and returns its folder.
fn install_pack(dir: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/packs/test-pack");
    let installed = dir.join("user-packs/test-pack");
    copy_dir(&source, &installed);
    installed
}

fn embeds(pack_dir: &Path, assets: &[&str]) -> Vec<EmbedAsset> {
    let pack = read_pack(pack_dir, PackSource::User).unwrap();
    assets
        .iter()
        .map(|id| EmbedAsset::read(&pack, pack.asset(id).unwrap()).unwrap())
        .collect()
}

fn font(file: &str, data: &[u8]) -> EmbedFont {
    EmbedFont {
        family: "Cinzel".into(),
        file: file.into(),
        licence: "SIL Open Font License".into(),
        data: Blob::new(data.to_vec()),
    }
}

/// A Map with a Layer and a Stamp of each of `assets`.
fn map_with_stamps(assets: &[&str]) -> Map {
    let mut map = empty_map();
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    for asset in assets {
        map.execute(&Command::CreateStamp {
            id: Id::new(),
            parent: layer,
            index: None,
            stamp: Box::new(Stamp::new(
                AssetRef::new("test-pack", *asset).unwrap(),
                [10.0, 20.0],
            )),
        })
        .unwrap();
    }
    map
}

fn embed_used(map: &mut Map, pack_dir: &Path) {
    let used = map.used_assets();
    let ids: Vec<&str> = used.iter().map(|r| r.asset.as_str()).collect();
    map.execute(&Command::EmbedAssets {
        assets: embeds(pack_dir, &ids),
        fonts: vec![],
    })
    .unwrap();
}

#[test]
fn a_map_saved_with_a_user_pack_opens_identically_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["mountain-1", "well"]);
    map.execute(&Command::EmbedAssets {
        assets: embeds(&pack_dir, &["mountain-1", "well"]),
        fonts: vec![font("Cinzel[wght].ttf", b"font bytes")],
    })
    .unwrap();
    let expected_art = std::fs::read(pack_dir.join("art/mountain-1.svg")).unwrap();
    let expected_silhouette = std::fs::read(pack_dir.join("silhouettes/mountain-1.png")).unwrap();

    let path = dir.path().join("world.ssmap");
    MapFile::create(&path, &map, None).unwrap();
    std::fs::remove_dir_all(dir.path().join("user-packs")).unwrap();

    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.view(), map.view());
    let embedded = reopened.embedded();
    let mountain: AssetRef = "test-pack/mountain-1".parse().unwrap();
    assert_eq!(&**embedded.art(&mountain).unwrap(), &expected_art[..]);
    assert_eq!(
        &**embedded.silhouette(&mountain).unwrap(),
        &expected_silhouette[..]
    );
    let record = &embedded.assets[&mountain];
    assert_eq!(record.name, "Mountain");
    assert_eq!(record.pack_name, "Test pack");
    assert_eq!(record.licence, "CC0-1.0");
    assert_eq!(record.anchor, [0.5, 0.92]);
    assert_eq!(embedded.family_files("Cinzel").count(), 1);
    assert_eq!(
        embedded.fonts["Cinzel[wght].ttf"].licence,
        "SIL Open Font License"
    );
}

#[test]
fn files_are_written_with_format_1_4() {
    assert_eq!((FORMAT_VERSION.major, FORMAT_VERSION.minor), (1, 4));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ssmap");
    let (file, _) = MapFile::create(&path, &empty_map(), None).unwrap();
    assert_eq!(file.format_version(), FORMAT_VERSION);
}

#[test]
fn identical_files_are_stored_once() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree"]);
    let mut twin = embeds(&pack_dir, &["tree"]).remove(0);
    twin.reference = "test-pack/tree-copy".parse().unwrap();
    let mut assets = embeds(&pack_dir, &["tree"]);
    assets.push(twin);
    map.execute(&Command::EmbedAssets {
        assets,
        fonts: vec![],
    })
    .unwrap();
    assert_eq!(map.embedded().assets.len(), 2);
    // The art and the Silhouette; the twin shares both.
    assert_eq!(map.embedded().blobs.len(), 2);

    let path = dir.path().join("world.ssmap");
    MapFile::create(&path, &map, None).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM blobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn the_embedded_copy_wins_over_the_pack() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree"]);
    embed_used(&mut map, &pack_dir);
    let before = map.embedded().clone();

    std::fs::write(pack_dir.join("art/tree.svg"), "<svg/>").unwrap();
    embed_used(&mut map, &pack_dir);
    assert_eq!(map.embedded(), &before);
}

#[test]
fn embedding_is_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree", "well"]);
    let view = map.view();
    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::EmbedAssets {
                assets: embeds(&pack_dir, &["tree", "well"]),
                fonts: vec![font("a.ttf", b"a")],
            },
            vec![],
        )
        .unwrap();
    assert!(!map.embedded().is_empty());
    assert_eq!(history.undo_len(), 1);

    history.undo(&mut map).unwrap();
    assert_eq!(map.view(), view);
    assert!(map.embedded().is_empty());
    history.redo(&mut map).unwrap();
    assert_eq!(map.embedded().assets.len(), 2);
    assert_eq!(map.embedded().fonts.len(), 1);
}

#[test]
fn a_file_that_does_not_match_its_hash_is_refused() {
    let wrong = Op::SetBlob {
        hash: ContentHash::of(b"one"),
        data: Some(Blob::new(b"two".to_vec())),
    };
    let mut map = empty_map();
    assert!(matches!(
        map.apply(wrong),
        Err(OpError::BlobHashMismatch(_))
    ));

    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut embed = embeds(&pack_dir, &["tree"]);
    embed[0].art = Blob::new(b"not the art".to_vec());
    assert!(
        map.execute(&Command::EmbedAssets {
            assets: embed,
            fonts: vec![]
        })
        .is_err()
    );
    assert!(map.embedded().is_empty());
}

#[test]
fn later_saves_write_only_new_files_and_drop_removed_ones() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree"]);
    embed_used(&mut map, &pack_dir);
    let path = dir.path().join("world.ssmap");
    let (mut file, _) = MapFile::create(&path, &map, None).unwrap();

    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::EmbedAssets {
                assets: embeds(&pack_dir, &["well"]),
                fonts: vec![],
            },
            vec![],
        )
        .unwrap();
    file.save(&map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.embedded(), map.embedded());

    history.undo(&mut map).unwrap();
    file.save(&map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.embedded().assets.len(), 1);
    assert_eq!(reopened.embedded(), map.embedded());
}

#[test]
fn update_from_pack_swaps_the_art_and_can_be_undone() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree", "well"]);
    embed_used(&mut map, &pack_dir);
    let before = map.embedded().clone();
    let tree: AssetRef = "test-pack/tree".parse().unwrap();
    let well: AssetRef = "test-pack/well".parse().unwrap();

    std::fs::write(pack_dir.join("art/tree.svg"), "<svg id='new'/>").unwrap();
    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::UpdateFromPack {
                assets: embeds(&pack_dir, &["tree"]),
            },
            vec![],
        )
        .unwrap();
    assert_eq!(history.undo_len(), 1);
    assert_eq!(&**map.embedded().art(&tree).unwrap(), b"<svg id='new'/>");
    assert_eq!(map.embedded().assets[&well], before.assets[&well]);
    // The replaced art is dropped, so nothing stale is saved.
    assert!(!map.embedded().blobs.contains_key(&before.assets[&tree].art));
    assert_eq!(map.embedded().first_missing_blob(), None);

    history.undo(&mut map).unwrap();
    assert_eq!(map.embedded(), &before);
    history.redo(&mut map).unwrap();
    assert_eq!(&**map.embedded().art(&tree).unwrap(), b"<svg id='new'/>");
}

#[test]
fn update_from_pack_refuses_an_asset_that_is_not_embedded() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree"]);
    assert!(
        map.execute(&Command::UpdateFromPack {
            assets: embeds(&pack_dir, &["tree"]),
        })
        .is_err()
    );
    assert!(map.embedded().is_empty());
}

#[test]
fn save_as_keeps_the_embedded_copies() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree"]);
    embed_used(&mut map, &pack_dir);
    let (mut file, _) = MapFile::create(dir.path().join("a.ssmap"), &map, None).unwrap();
    let copy = dir.path().join("b.ssmap");
    file.save_as(&copy, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&copy).unwrap();
    assert_eq!(reopened.embedded(), map.embedded());
}

#[test]
fn a_format_1_0_file_opens_and_gains_the_tables_on_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.ssmap");
    MapFile::create(&path, &empty_map(), None).unwrap();
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE blobs; DROP TABLE embedded;
             UPDATE meta SET value = '1.0' WHERE key = 'format_version';",
        )
        .unwrap();
    }
    let (mut file, mut map) = MapFile::open(&path).unwrap();
    assert!(map.embedded().is_empty());

    let pack_dir = install_pack(dir.path());
    map.execute(&Command::EmbedAssets {
        assets: embeds(&pack_dir, &["tree"]),
        fonts: vec![],
    })
    .unwrap();
    file.save(&map, None).unwrap();
    assert_eq!(file.format_version(), FORMAT_VERSION);
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.embedded(), map.embedded());
}

#[test]
fn a_record_without_its_file_is_reported_as_damage() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree"]);
    embed_used(&mut map, &pack_dir);
    let path = dir.path().join("world.ssmap");
    MapFile::create(&path, &map, None).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("DELETE FROM blobs", [])
        .unwrap();
    let err = MapFile::open(&path).unwrap_err();
    assert!(err.to_string().contains("damaged"), "{err}");
}

#[test]
fn recovery_replays_embedded_files() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree"]);
    let mut history = History::default();
    history.set_recording(true);
    history
        .execute_step(
            &mut map,
            &Command::EmbedAssets {
                assets: embeds(&pack_dir, &["tree"]),
                fonts: vec![font("a.ttf", b"a")],
            },
            vec![],
        )
        .unwrap();
    let ops = history.take_applied();

    // Ops survive the journal's JSON lines.
    let ops: Vec<Op> = serde_json::from_str(&serde_json::to_string(&ops).unwrap()).unwrap();
    let mut recovered = map_with_stamps(&[]);
    for op in ops {
        recovered.apply(op).unwrap();
    }
    assert_eq!(recovered.embedded(), map.embedded());
}

#[test]
fn the_view_round_trips_the_embedded_state_through_json() {
    let dir = tempfile::tempdir().unwrap();
    let pack_dir = install_pack(dir.path());
    let mut map = map_with_stamps(&["tree"]);
    embed_used(&mut map, &pack_dir);
    let json = serde_json::to_string(&map.view()).unwrap();
    let view = serde_json::from_str(&json).unwrap();
    assert_eq!(Map::from_view(view).unwrap().embedded(), map.embedded());
}
