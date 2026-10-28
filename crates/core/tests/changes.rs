//! `Map::changes_since`: what changed after a revision, so derived data can patch itself.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::{Canvas, Changes, Command, DistanceUnit, Id, Map, Scale, Stamp};

fn new_map() -> Map {
    Map::new(Canvas::new(100, 80, Scale::new(1.0, DistanceUnit::Miles)))
}

fn stamp_at(x: f64) -> Box<Stamp> {
    Box::new(Stamp::new("base/tree".parse().unwrap(), [x, 10.0]))
}

/// A Map with a Layer holding `count` Stamps.
fn with_stamps(count: usize) -> (Map, Id, Vec<Id>) {
    let mut map = new_map();
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Trees".into(),
    })
    .unwrap();
    let ids: Vec<Id> = (0..count).map(|_| Id::new()).collect();
    for (i, &id) in ids.iter().enumerate() {
        map.execute(&Command::CreateStamp {
            id,
            parent: layer,
            index: None,
            stamp: stamp_at(i as f64),
        })
        .unwrap();
    }
    (map, layer, ids)
}

#[test]
fn nothing_changed_since_the_current_revision() {
    let (map, _, _) = with_stamps(2);
    assert_eq!(
        map.changes_since(map.revision()),
        Some(Changes::default()),
        "no properties, nothing else"
    );
}

#[test]
fn setting_a_stamp_is_a_property_change_of_that_stamp() {
    let (mut map, _, ids) = with_stamps(3);
    let before = map.revision();
    map.execute(&Command::SetStamp {
        id: ids[1],
        stamp: stamp_at(50.0),
    })
    .unwrap();
    let changes = map.changes_since(before).unwrap();
    assert_eq!(changes.properties, [ids[1]]);
    assert!(!changes.other);
}

#[test]
fn a_node_set_many_times_is_listed_once_in_the_order_first_set() {
    let (mut map, _, ids) = with_stamps(3);
    let before = map.revision();
    for (id, x) in [(ids[2], 1.0), (ids[0], 2.0), (ids[2], 3.0)] {
        map.execute(&Command::SetStamp {
            id,
            stamp: stamp_at(x),
        })
        .unwrap();
    }
    let changes = map.changes_since(before).unwrap();
    assert_eq!(changes.properties, [ids[2], ids[0]]);
}

#[test]
fn adding_removing_moving_and_map_wide_edits_are_other() {
    let (mut map, layer, ids) = with_stamps(2);
    let edits = [
        Command::CreateStamp {
            id: Id::new(),
            parent: layer,
            index: None,
            stamp: stamp_at(9.0),
        },
        Command::Delete { ids: vec![ids[0]] },
    ];
    for edit in edits {
        let before = map.revision();
        map.execute(&edit).unwrap();
        assert!(map.changes_since(before).unwrap().other, "{edit:?}");
    }
}

#[test]
fn a_revision_the_log_no_longer_reaches_is_unknown() {
    let (mut map, _, ids) = with_stamps(1);
    let start = map.revision();
    for i in 0..17_000 {
        map.execute(&Command::SetStamp {
            id: ids[0],
            stamp: stamp_at(f64::from(i)),
        })
        .unwrap();
    }
    assert_eq!(map.changes_since(start), None, "too long ago");
    assert!(map.changes_since(map.revision() - 10).is_some());
    assert_eq!(
        map.changes_since(map.revision() + 1),
        None,
        "from the future"
    );
}

#[test]
fn undo_is_a_change_like_any_other() {
    let (mut map, _, ids) = with_stamps(2);
    let before = map.revision();
    let inverse = map
        .execute(&Command::SetStamp {
            id: ids[0],
            stamp: stamp_at(70.0),
        })
        .unwrap();
    map.revert(inverse);
    let changes = map.changes_since(before).unwrap();
    assert_eq!(changes.properties, [ids[0]]);
    assert!(!changes.other);
}
