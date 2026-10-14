//! Map file save and open (ADR 0005).

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{Value, json};
use ssmap_core::file::{FORMAT_VERSION, FileError, MapFile, SaveStats, read_preview};
use ssmap_core::{
    Canvas, Color, Command, DistanceUnit, Id, Label, Lake, Landmass, Layer, Map, MapDefaults,
    MultiPolygon, Node, Object, ObjectKind, Op, Polygon, Property, Roughness, Scale, Stamp,
    Subtree,
};

fn insert(map: &mut Map, parent: Option<Id>, node: Node) -> Id {
    let id = Id::new();
    let index = map.children(parent).len();
    map.apply(Op::Insert {
        parent,
        index,
        subtree: Subtree {
            id,
            node,
            children: Vec::new(),
        },
    })
    .unwrap();
    id
}

fn object(kind: ObjectKind) -> Node {
    Node::Object(Object::new(kind))
}

fn island(offset: f64) -> MultiPolygon {
    MultiPolygon(vec![
        Polygon {
            exterior: vec![[offset, 0.0], [100.25, 0.0], [100.0, 1e-9], [0.0, 80.0]],
            holes: vec![vec![[10.0, 10.0], [20.0, 10.0], [15.0, 20.0]]],
        },
        Polygon {
            exterior: vec![[200.0, 200.0], [260.0, 210.0], [230.0, 0.1 + 0.2]],
            holes: Vec::new(),
        },
    ])
}

/// A Map using every kind of node and property.
fn sample_map() -> (Map, Id, Id) {
    let mut map = Map::new(Canvas::new(
        4096,
        3072,
        Scale::new(0.25, DistanceUnit::Miles),
    ));
    map.apply(Op::SetDefaults {
        defaults: MapDefaults {
            roughness: Roughness::new(4.0, 48.0, 11),
            ..Default::default()
        },
    })
    .unwrap();
    let land = insert(&mut map, None, Node::Layer(Layer::new("Land")));
    let landmass = insert(
        &mut map,
        Some(land),
        object(ObjectKind::Landmass(Landmass {
            geometry: island(0.0),
            roughness: Some(Roughness::new(0.1 + 0.2, 1e-3, u32::MAX)),
            ..Default::default()
        })),
    );
    insert(
        &mut map,
        Some(land),
        object(ObjectKind::Lake(Lake {
            geometry: island(5.0),
            ..Default::default()
        })),
    );
    let mut assets = Layer::new("Assets");
    assets.opacity = 0.3;
    assets.clip = Some(landmass);
    let assets = insert(&mut map, None, Node::Layer(assets));
    let mut group = Layer::new("Forest “north”");
    group.visible = false;
    group.locked = true;
    let group = insert(&mut map, Some(assets), Node::Layer(group));
    for _ in 0..3 {
        insert(
            &mut map,
            Some(group),
            object(ObjectKind::Stamp(Stamp::default())),
        );
    }
    let mut label = Object::new(ObjectKind::Label(Label::default()));
    label.locked = true;
    insert(&mut map, Some(assets), Node::Object(label));
    insert(&mut map, None, Node::Layer(Layer::new("Empty")));
    (map, land, landmass)
}

fn view_json(map: &Map) -> String {
    serde_json::to_string(&map.view()).unwrap()
}

fn path(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

fn sql(path: &Path, statements: &str) {
    Connection::open(path)
        .unwrap()
        .execute_batch(statements)
        .unwrap();
}

fn meta(path: &Path, key: &str) -> Option<String> {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .ok()
}

#[test]
fn save_open_and_save_again_round_trips_the_map() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "world.ssmap");
    let (map, _, _) = sample_map();
    let (file, stats) = MapFile::create(&file_path, &map, Some(b"png")).unwrap();
    assert_eq!(stats.written, map.ids().count());
    assert_eq!(file.path(), file_path);
    drop(file);

    let (mut file, opened) = MapFile::open(&file_path).unwrap();
    assert_eq!(view_json(&opened), view_json(&map));
    assert_eq!(opened.view(), map.view());
    opened.validate().unwrap();

    // Nothing changed, so nothing is rewritten.
    assert_eq!(
        file.save(&opened, Some(b"png")).unwrap(),
        SaveStats::default()
    );
    drop(file);
    let (_, reopened) = MapFile::open(&file_path).unwrap();
    assert_eq!(view_json(&reopened), view_json(&map));
}

#[test]
fn saves_write_only_what_changed() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "world.ssmap");
    let (map, land, landmass) = sample_map();
    MapFile::create(&file_path, &map, None).unwrap();
    let (mut file, mut map) = MapFile::open(&file_path).unwrap();

    map.apply(Op::SetProperty {
        id: land,
        property: Property::Name("Continents".into()),
    })
    .unwrap();
    map.apply(Op::ReplaceGeometry {
        id: landmass,
        geometry: island(1.5),
    })
    .unwrap();
    let stats = file.save(&map, None).unwrap();
    assert_eq!(
        stats,
        SaveStats {
            written: 2,
            moved: 0,
            deleted: 0
        }
    );

    // Moving the bottom Layer to the top shifts its siblings, which only change place.
    map.apply(Op::Move {
        id: land,
        parent: None,
        index: 2,
    })
    .unwrap();
    let stats = file.save(&map, None).unwrap();
    assert_eq!(stats.moved, 2);
    assert!(stats.written <= 1, "{stats:?}");

    let assets = map.children(None)[0];
    let removed = map.subtree(assets);
    let count = 1 + 1 + 3 + 1;
    map.apply(Op::Remove { id: assets }).unwrap();
    let stats = file.save(&map, None).unwrap();
    assert_eq!(stats.deleted, count);
    // Undoing the delete writes the branch back.
    map.apply(Op::Insert {
        parent: None,
        index: 0,
        subtree: removed,
    })
    .unwrap();
    assert_eq!(file.save(&map, None).unwrap().written, count);

    drop(file);
    let (_, reopened) = MapFile::open(&file_path).unwrap();
    assert_eq!(reopened.view(), map.view());
}

#[test]
fn every_save_stamps_a_new_generation() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "world.ssmap");
    let (map, _, _) = sample_map();
    let (mut file, _) = MapFile::create(&file_path, &map, None).unwrap();
    let first = file.save_generation();
    assert!(!first.is_nil());
    file.save(&map, None).unwrap();
    let second = file.save_generation();
    assert_ne!(first, second);
    assert_eq!(
        meta(&file_path, "save_generation"),
        Some(second.to_string())
    );
    drop(file);
    let (file, _) = MapFile::open(&file_path).unwrap();
    assert_eq!(file.save_generation(), second);
}

#[test]
fn the_preview_is_refreshed_on_every_save() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "world.ssmap");
    let (map, _, _) = sample_map();
    let (mut file, _) = MapFile::create(&file_path, &map, Some(b"first")).unwrap();
    assert_eq!(
        read_preview(&file_path).unwrap().as_deref(),
        Some(&b"first"[..])
    );
    file.save(&map, Some(b"second")).unwrap();
    assert_eq!(
        read_preview(&file_path).unwrap().as_deref(),
        Some(&b"second"[..])
    );
    file.save(&map, None).unwrap();
    assert_eq!(read_preview(&file_path).unwrap(), None);
}

#[test]
fn save_as_writes_a_copy_that_becomes_the_open_file() {
    let dir = tempfile::tempdir().unwrap();
    let first = path(&dir, "first.ssmap");
    let second = path(&dir, "second.ssmap");
    let (map, land, _) = sample_map();
    let (mut file, _) = MapFile::create(&first, &map, None).unwrap();
    let mut map = map;
    map.apply(Op::SetProperty {
        id: land,
        property: Property::Name("Renamed".into()),
    })
    .unwrap();
    let stats = file.save_as(&second, &map, Some(b"png")).unwrap();
    assert_eq!(stats.written, 1, "the copy starts from the last save");
    assert_eq!(file.path(), second);

    let (_, at_first) = MapFile::open(&first).unwrap();
    assert!(view_json(&at_first).contains("\"Land\""));
    let (_, at_second) = MapFile::open(&second).unwrap();
    assert_eq!(at_second.view(), map.view());
    assert!(!dir.path().join("second.ssmap.saving").exists());

    // Later saves go to the new file.
    map.apply(Op::SetProperty {
        id: land,
        property: Property::Name("Again".into()),
    })
    .unwrap();
    file.save(&map, None).unwrap();
    drop(file);
    let (_, at_second) = MapFile::open(&second).unwrap();
    assert!(view_json(&at_second).contains("\"Again\""));
    let (_, at_first) = MapFile::open(&first).unwrap();
    assert!(!view_json(&at_first).contains("\"Again\""));
}

#[test]
fn save_as_replaces_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = path(&dir, "target.ssmap");
    std::fs::write(&target, b"old contents").unwrap();
    let (map, _, _) = sample_map();
    MapFile::create(&target, &map, None).unwrap();
    let (_, opened) = MapFile::open(&target).unwrap();
    assert_eq!(opened.view(), map.view());
}

#[test]
fn a_changed_default_is_saved_without_rewriting_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "defaults.ssmap");
    let (map, _, _) = sample_map();
    MapFile::create(&file_path, &map, None).unwrap();

    let (mut file, mut opened) = MapFile::open(&file_path).unwrap();
    let defaults = MapDefaults {
        roughness: Roughness::new(9.5, 64.0, 3),
        ..Default::default()
    };
    opened
        .apply(Op::SetDefaults {
            defaults: defaults.clone(),
        })
        .unwrap();
    assert_eq!(file.save(&opened, None).unwrap(), SaveStats::default());
    drop(file);
    let (_, reopened) = MapFile::open(&file_path).unwrap();
    assert_eq!(reopened.defaults(), &defaults);
}

#[test]
fn files_without_defaults_open_with_the_built_in_ones() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "older.ssmap");
    let (map, _, _) = sample_map();
    MapFile::create(&file_path, &map, None).unwrap();
    sql(&file_path, "DELETE FROM meta WHERE key = 'defaults';");
    let (_, opened) = MapFile::open(&file_path).unwrap();
    assert_eq!(opened.defaults(), &MapDefaults::default());
}

#[test]
fn unknown_fields_from_a_newer_minor_version_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "newer.ssmap");
    let (map, land, landmass) = sample_map();
    MapFile::create(&file_path, &map, None).unwrap();

    // A newer minor version adds fields everywhere, a meta key, a column and a table.
    let land_hex = hex(land);
    let landmass_hex = hex(landmass);
    sql(
        &file_path,
        &format!(
            "UPDATE meta SET value = '1.9' WHERE key = 'format_version';
             UPDATE meta SET value = json_set(value, '$.grid', json('{{\"size\":64}}'),
                                              '$.scale.precise', 1) WHERE key = 'canvas';
             INSERT INTO meta (key, value) VALUES ('future_key', 'kept');
             ALTER TABLE nodes ADD COLUMN future_column TEXT;
             UPDATE nodes SET future_column = 'kept';
             CREATE TABLE future_table (x); INSERT INTO future_table VALUES (42);
             UPDATE nodes SET props = json_set(props, '$.blend', 'multiply')
                 WHERE id = X'{land_hex}';
             UPDATE nodes SET props = json_set(props, '$.note', 'object field',
                                               '$.kind.tide', json('{{\"height\":0.5}}'))
                 WHERE id = X'{landmass_hex}';"
        ),
    );

    let (mut file, mut opened) = MapFile::open(&file_path).unwrap();
    assert_eq!(file.format_version().to_string(), "1.9");
    let Some(Node::Layer(layer)) = opened.node(land) else {
        panic!("a Layer")
    };
    assert_eq!(layer.extra["blend"], "multiply");
    // Edit the nodes that carry unknown fields, so their rows are rewritten.
    opened
        .apply(Op::SetProperty {
            id: land,
            property: Property::Name("Edited".into()),
        })
        .unwrap();
    opened
        .apply(Op::ReplaceGeometry {
            id: landmass,
            geometry: island(3.0),
        })
        .unwrap();
    let mut canvas = opened.canvas.clone();
    canvas.width = 5000;
    opened.canvas = canvas;
    file.save(&opened, None).unwrap();
    // Save As keeps what this version doesn't know, too.
    let copy_path = path(&dir, "copy.ssmap");
    file.save_as(&copy_path, &opened, None).unwrap();
    drop(file);

    for file_path in [&file_path, &copy_path] {
        let (_, reopened) = MapFile::open(file_path).unwrap();
        let view = serde_json::to_value(reopened.view()).unwrap();
        assert_eq!(view["canvas"]["grid"], json!({"size": 64}));
        assert_eq!(view["canvas"]["scale"]["precise"], 1);
        assert_eq!(view["canvas"]["width"], 5000);
        let land_view = find(&view, land);
        assert_eq!(land_view["blend"], "multiply");
        assert_eq!(land_view["name"], "Edited");
        let landmass_view = find(&view, landmass);
        assert_eq!(landmass_view["note"], "object field");
        assert_eq!(landmass_view["kind"]["tide"], json!({"height": 0.5}));
        assert_eq!(meta(file_path, "format_version").as_deref(), Some("1.9"));
        assert_eq!(meta(file_path, "future_key").as_deref(), Some("kept"));
        let conn = Connection::open(file_path).unwrap();
        let kept: i64 = conn
            .query_row(
                "SELECT count(*) FROM nodes WHERE future_column = 'kept'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept as usize, reopened.ids().count());
        let future: i64 = conn
            .query_row("SELECT x FROM future_table", [], |row| row.get(0))
            .unwrap();
        assert_eq!(future, 42);
    }
}

fn hex(id: Id) -> String {
    id.as_uuid().simple().to_string()
}

fn find(view: &Value, id: Id) -> Value {
    fn walk(nodes: &Value, id: &str) -> Option<Value> {
        nodes.as_array()?.iter().find_map(|node| {
            if node["id"] == id {
                Some(node.clone())
            } else {
                walk(&node["children"], id)
            }
        })
    }
    walk(&view["layers"], &id.to_string()).expect("the node is in the view")
}

#[test]
fn a_newer_major_version_is_refused_with_a_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "future.ssmap");
    let (map, _, _) = sample_map();
    MapFile::create(&file_path, &map, None).unwrap();
    sql(
        &file_path,
        "UPDATE meta SET value = '2.0' WHERE key = 'format_version'",
    );
    let before = std::fs::read(&file_path).unwrap();
    let err = MapFile::open(&file_path).unwrap_err();
    assert!(matches!(err, FileError::NewerMajor { .. }));
    assert_eq!(
        err.to_string(),
        format!(
            "this Map was saved by a newer StarScript Map Maker (file format 2.0); this version \
             opens format {}.x and older",
            FORMAT_VERSION.major
        )
    );
    assert_eq!(std::fs::read(&file_path).unwrap(), before);
}

#[test]
fn other_files_are_not_map_files() {
    let dir = tempfile::tempdir().unwrap();
    let text = path(&dir, "notes.ssmap");
    std::fs::write(
        &text,
        "not a database, but long enough to look like a header".repeat(20),
    )
    .unwrap();
    assert!(matches!(
        MapFile::open(&text).unwrap_err(),
        FileError::NotAMapFile
    ));
    let other_db = path(&dir, "other.ssmap");
    sql(&other_db, "CREATE TABLE meta (key, value)");
    assert!(matches!(
        MapFile::open(&other_db).unwrap_err(),
        FileError::NotAMapFile
    ));
    assert!(matches!(
        MapFile::open(path(&dir, "missing.ssmap")).unwrap_err(),
        FileError::Io(_)
    ));
}

#[test]
fn damaged_rows_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "damaged.ssmap");
    let (map, land, _) = sample_map();
    MapFile::create(&file_path, &map, None).unwrap();
    let orphan = Id::new();
    sql(
        &file_path,
        &format!(
            "INSERT INTO nodes (id, parent, ord, kind, props) SELECT X'{}', X'{}', 0, kind, props
             FROM nodes WHERE id = X'{}'",
            hex(Id::new()),
            hex(orphan),
            hex(land)
        ),
    );
    let err = MapFile::open(&file_path).unwrap_err();
    assert!(matches!(err, FileError::Damaged(_)), "{err}");
}

/// Environment variable telling the child process in `killing_the_process_mid_save_*` which
/// file to save in a loop.
const KILL_CHILD_FILE: &str = "SSMAP_TEST_KILL_CHILD_FILE";
const KILL_LANDMASSES: usize = 20_000;

/// The first point of each outline records which save wrote it.
fn generation_map(map: &mut Map, layer: Id, generation: u32) {
    let ids = map.children(Some(layer)).to_vec();
    for id in ids {
        map.apply(Op::ReplaceGeometry {
            id,
            geometry: island(f64::from(generation)),
        })
        .unwrap();
    }
}

fn saved_generations(map: &Map) -> Vec<f64> {
    let layer = map.children(None)[0];
    let mut generations: Vec<f64> = map
        .children(Some(layer))
        .iter()
        .map(|&id| match map.node(id) {
            Some(Node::Object(object)) => object.kind.geometry().unwrap().0[0].exterior[0][0],
            _ => panic!("a Landmass"),
        })
        .collect();
    generations.dedup();
    generations
}

/// Run by `killing_the_process_mid_save_leaves_the_previous_save_intact` as a child process:
/// saves a large Map over and over until killed.
#[test]
#[ignore = "runs only as the child of the mid-save kill test"]
fn kill_child() {
    let Ok(file_path) = std::env::var(KILL_CHILD_FILE) else {
        return;
    };
    let (mut file, mut map) = MapFile::open(&file_path).unwrap();
    let layer = map.children(None)[0];
    for generation in 1.. {
        generation_map(&mut map, layer, generation);
        println!("saving {generation}");
        file.save(&map, None).unwrap();
        println!("saved {generation}");
    }
}

#[test]
fn killing_the_process_mid_save_leaves_the_previous_save_intact() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "killed.ssmap");
    let mut map = Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = insert(&mut map, None, Node::Layer(Layer::new("Land")));
    for _ in 0..KILL_LANDMASSES {
        insert(
            &mut map,
            Some(layer),
            object(ObjectKind::Landmass(Landmass::default())),
        );
    }
    generation_map(&mut map, layer, 0);
    MapFile::create(&file_path, &map, None).unwrap();

    let journal = path(&dir, "killed.ssmap-journal");
    let mut killed_mid_transaction = 0;
    for round in 0..3 {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "kill_child", "--ignored", "--nocapture"])
            .env(KILL_CHILD_FILE, &file_path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        // Kill it during a save: the second one in round 0, then the third, then the fourth.
        let target = format!("saving {}", round + 2);
        let mut last_saved = None;
        for line in stdout.lines() {
            let line = line.unwrap();
            if let Some(n) = line.strip_prefix("saved ") {
                last_saved = Some(n.parse::<f64>().unwrap());
            }
            if line == target {
                std::thread::sleep(std::time::Duration::from_millis(5 + 10 * round));
                child.kill().unwrap();
                break;
            }
        }
        child.wait().unwrap();
        // SQLite's rollback journal outlives the process only if it died inside a save.
        if journal.exists() {
            killed_mid_transaction += 1;
        }

        let (_, opened) = MapFile::open(&file_path).expect("the file still opens");
        opened.validate().unwrap();
        let generations = saved_generations(&opened);
        assert_eq!(generations.len(), 1, "one save's outlines, not a mix");
        let found = generations[0];
        let last_saved = last_saved.expect("the child finished a save first");
        // The save in progress either committed in full or not at all.
        assert!(
            found == last_saved || found == last_saved + 1.0,
            "found generation {found} after save {last_saved}"
        );
        assert_eq!(opened.ids().count(), KILL_LANDMASSES + 1);
    }
    assert!(
        killed_mid_transaction > 0,
        "no kill landed inside a save's transaction"
    );
}

/// The Map the `land_and_water` Command script builds: an island with a hole, an islet, and
/// a Lake.
fn land_and_water() -> Map {
    let text = include_str!("scripts/land_and_water.json");
    let script: Value = serde_json::from_str(text).unwrap();
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    for step in script["steps"].as_array().unwrap() {
        let command: Command = serde_json::from_value(step["step"].clone()).unwrap();
        map.execute(&command).unwrap();
    }
    map
}

#[test]
fn an_island_with_a_hole_and_a_lake_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "island.ssmap");
    let mut map = land_and_water();
    let lake = map
        .ids()
        .find(|&id| matches!(map.node(id), Some(Node::Object(o)) if o.kind.name() == "lake"))
        .unwrap();
    map.execute(&Command::SetFill {
        ids: vec![lake],
        fill: Color::rgba(0x20, 0x60, 0xa0, 0xc0),
    })
    .unwrap();
    let landmass = map.draw_order()[0];
    let Some(Node::Object(object)) = map.node(landmass) else {
        panic!("a Landmass")
    };
    let geometry = object.kind.geometry().unwrap();
    assert_eq!(geometry.0.len(), 2, "an island and an islet");
    assert_eq!(geometry.0[0].holes.len(), 1);

    MapFile::create(&file_path, &map, None).unwrap();
    let (mut file, mut opened) = MapFile::open(&file_path).unwrap();
    assert_eq!(view_json(&opened), view_json(&map));
    assert_eq!(opened.view(), map.view());

    // A fill change rewrites just that Object's row.
    opened
        .execute(&Command::SetFill {
            ids: vec![landmass],
            fill: Color::rgb(0x80, 0xa0, 0x60),
        })
        .unwrap();
    assert_eq!(file.save(&opened, None).unwrap().written, 1);
    drop(file);
    let (_, reopened) = MapFile::open(&file_path).unwrap();
    assert_eq!(reopened.view(), opened.view());
}

#[test]
fn outlines_saved_before_fills_existed_open_with_the_default_fill() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = path(&dir, "old.ssmap");
    let (map, _, landmass) = sample_map();
    MapFile::create(&file_path, &map, None).unwrap();
    sql(
        &file_path,
        "UPDATE nodes SET props = json_remove(props, '$.kind.fill') WHERE geometry IS NOT NULL",
    );
    let (_, opened) = MapFile::open(&file_path).unwrap();
    match opened.node(landmass) {
        Some(Node::Object(object)) => {
            assert_eq!(object.kind.fill(), Some(Landmass::DEFAULT_FILL));
        }
        _ => panic!("a Landmass"),
    }
}
