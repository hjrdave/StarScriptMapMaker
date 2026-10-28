//! `Map::changes_since`: what changed after a revision, so derived data can patch itself.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::{
    Canvas, Changes, Command, DistanceUnit, Id, Map, MultiPolygon, Polygon, Scale, Stamp,
};

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
        "nothing edited, nothing else"
    );
}

#[test]
fn setting_a_stamp_is_a_edit_of_that_stamp() {
    let (mut map, _, ids) = with_stamps(3);
    let before = map.revision();
    map.execute(&Command::SetStamp {
        id: ids[1],
        stamp: stamp_at(50.0),
    })
    .unwrap();
    let changes = map.changes_since(before).unwrap();
    assert_eq!(changes.edited, [ids[1]]);
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
    assert_eq!(changes.edited, [ids[2], ids[0]]);
}

#[test]
fn removing_a_node_is_other() {
    let (mut map, _, ids) = with_stamps(2);
    let before = map.revision();
    map.execute(&Command::Delete { ids: vec![ids[0]] }).unwrap();
    assert!(map.changes_since(before).unwrap().other);
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
    assert_eq!(changes.edited, [ids[0]]);
    assert!(!changes.other);
}

#[test]
fn moving_an_outline_is_an_edit_of_that_landmass() {
    let (mut map, layer, _) = with_stamps(1);
    let land = Id::new();
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: layer,
        index: None,
        geometry: MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [40.0, 0.0], [40.0, 40.0], [0.0, 40.0]],
            holes: vec![],
        }]),
        fill: None,
    })
    .unwrap();
    let before = map.revision();
    map.execute(&Command::Translate {
        ids: vec![land],
        offset: [5.0, 5.0],
    })
    .unwrap();
    let changes = map.changes_since(before).unwrap();
    assert_eq!(changes.edited, [land]);
    assert!(!changes.other);
}

#[test]
fn placed_stamps_are_inserted_and_an_inserted_node_edited_later_is_only_inserted() {
    let (mut map, layer, ids) = with_stamps(2);
    let before = map.revision();
    let fresh = Id::new();
    map.execute(&Command::CreateStamp {
        id: fresh,
        parent: layer,
        index: None,
        stamp: stamp_at(80.0),
    })
    .unwrap();
    map.execute(&Command::SetStamp {
        id: fresh,
        stamp: stamp_at(81.0),
    })
    .unwrap();
    map.execute(&Command::SetStamp {
        id: ids[0],
        stamp: stamp_at(82.0),
    })
    .unwrap();
    let changes = map.changes_since(before).unwrap();
    assert_eq!(changes.inserted, [fresh]);
    assert_eq!(changes.edited, [ids[0]]);
    assert!(!changes.other && !changes.defaults);
}

#[test]
fn replacing_the_defaults_is_reported_as_such() {
    let (mut map, _, _) = with_stamps(1);
    let before = map.revision();
    map.execute(&Command::SetDefaultRoughness {
        roughness: ssmap_core::Roughness::new(5.0, 10.0, 1),
    })
    .unwrap();
    let changes = map.changes_since(before).unwrap();
    assert!(changes.defaults);
    assert!(!changes.other);
    assert!(changes.edited.is_empty() && changes.inserted.is_empty());
}
