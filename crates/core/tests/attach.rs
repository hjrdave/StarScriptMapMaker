//! Attach to Layer Below: a Layer clips to the Landmasses in the Layer below it.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, History, Id, Map, MultiPolygon, Node, Polygon,
    Scale, Stamp,
};

fn new_map() -> Map {
    Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)))
}

fn layer(map: &mut Map, parent: Option<Id>) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateLayer {
        id,
        parent,
        index: None,
        name: "Layer".into(),
    })
    .unwrap();
    id
}

fn land(map: &mut Map, parent: Id) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateLandmass {
        id,
        parent,
        index: None,
        geometry: MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
            holes: Vec::new(),
        }]),
        fill: None,
    })
    .unwrap();
    id
}

fn stamp(map: &mut Map, parent: Id) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateStamp {
        id,
        parent,
        index: None,
        stamp: Box::new(Stamp::default()),
    })
    .unwrap();
    id
}

fn attach(map: &mut Map, id: Id, attached: bool) -> Result<(), CommandError> {
    map.execute(&Command::SetAttached { id, attached })
        .map(|_| ())
}

fn is_attached(map: &Map, id: Id) -> bool {
    matches!(map.node(id), Some(Node::Layer(l)) if l.attached)
}

#[test]
fn an_attached_layer_clips_to_the_landmasses_in_the_layer_below_at_any_depth() {
    let mut map = new_map();
    let below = layer(&mut map, None);
    let above = layer(&mut map, None);
    let (a, b) = (land(&mut map, below), land(&mut map, below));
    // A Group inside the Layer below counts too, and a Stamp in it does not.
    let group = layer(&mut map, Some(below));
    let deep = land(&mut map, group);
    stamp(&mut map, group);
    let covered = stamp(&mut map, above);

    assert!(map.clip_targets(covered).is_empty());
    attach(&mut map, above, true).unwrap();
    assert!(is_attached(&map, above));
    let mut targets = map.clip_targets(covered);
    targets.sort();
    let mut expected = vec![a, b, deep];
    expected.sort();
    assert_eq!(targets, expected);

    attach(&mut map, above, false).unwrap();
    assert!(map.clip_targets(covered).is_empty());
}

#[test]
fn the_layer_below_is_in_the_same_parent_and_the_lowest_has_none() {
    let mut map = new_map();
    let bottom = layer(&mut map, None);
    let top = layer(&mut map, None);
    assert_eq!(map.layer_below(top), Some(bottom));
    assert_eq!(map.layer_below(bottom), None);
    assert_eq!(
        attach(&mut map, bottom, true),
        Err(CommandError::NothingBelow(bottom))
    );
    // Inside a Group, "below" is the next one down in that Group, not outside it.
    let group = layer(&mut map, None);
    let (low, high) = (layer(&mut map, Some(group)), layer(&mut map, Some(group)));
    assert_eq!(map.layer_below(high), Some(low));
    assert_eq!(map.layer_below(low), None);
}

#[test]
fn a_layer_nested_in_an_attached_group_clips_the_same_way_and_a_nearer_setting_wins() {
    let mut map = new_map();
    let below = layer(&mut map, None);
    let a = land(&mut map, below);
    let group = layer(&mut map, None);
    let inner = layer(&mut map, Some(group));
    let covered = stamp(&mut map, inner);
    attach(&mut map, group, true).unwrap();
    assert_eq!(map.clip_targets(covered), vec![a]);

    // The inner Layer's own Clip replaces the Group's.
    let other = land(&mut map, below);
    map.execute(&Command::SetClip {
        id: inner,
        target: Some(other),
    })
    .unwrap();
    assert_eq!(map.clip_targets(covered), vec![other]);
}

#[test]
fn attaching_replaces_a_clip_to_one_landmass_in_the_same_step_and_undo_restores_it() {
    let mut map = new_map();
    let below = layer(&mut map, None);
    let a = land(&mut map, below);
    let above = layer(&mut map, None);
    map.execute(&Command::SetClip {
        id: above,
        target: Some(a),
    })
    .unwrap();
    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::SetAttached {
                id: above,
                attached: true,
            },
            vec![],
        )
        .unwrap();
    let Some(Node::Layer(l)) = map.node(above) else {
        panic!("a Layer")
    };
    assert!(l.attached && l.clip.is_none());
    assert_eq!(history.undo_label(), Some("Attach to Layer Below"));
    history.undo(&mut map).unwrap();
    let Some(Node::Layer(l)) = map.node(above) else {
        panic!("a Layer")
    };
    assert!(!l.attached && l.clip == Some(a));
}

#[test]
fn a_locked_layer_refuses_and_an_object_is_not_a_layer() {
    let mut map = new_map();
    let below = layer(&mut map, None);
    let above = layer(&mut map, None);
    let s = stamp(&mut map, above);
    map.execute(&Command::SetLocked {
        id: above,
        locked: true,
    })
    .unwrap();
    assert_eq!(
        attach(&mut map, above, true),
        Err(CommandError::Locked(above))
    );
    assert!(matches!(
        attach(&mut map, s, true),
        Err(CommandError::NotALayer(_) | CommandError::InLockedLayer { .. })
    ));
    let _ = below;
}

#[test]
fn an_attached_layer_survives_a_save_and_reopen_and_older_files_open_unattached() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("attached.ssmap");
    let mut map = new_map();
    let below = layer(&mut map, None);
    land(&mut map, below);
    let above = layer(&mut map, None);
    attach(&mut map, above, true).unwrap();
    MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert!(is_attached(&reopened, above));
    assert_eq!(reopened.view(), map.view());

    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "UPDATE nodes SET props = json_remove(props, '$.attached') WHERE kind = 'layer';
         UPDATE meta SET value = '1.11' WHERE key = 'format_version';",
    )
    .unwrap();
    drop(conn);
    let (_, old) = MapFile::open(&path).unwrap();
    assert!(!is_attached(&old, above));
}
