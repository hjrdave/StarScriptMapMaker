//! New Objects are named for their kind and the next free number: "Stamp 001", "Stamp 002".
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::{
    Canvas, Command, DistanceUnit, Id, Map, MultiPolygon, Node, PlacedStamp, Polygon, Scale, Stamp,
};

fn map_with_layer() -> (Map, Id) {
    let mut map = Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Layer 001".into(),
    })
    .unwrap();
    (map, layer)
}

fn name(map: &Map, id: Id) -> String {
    match map.node(id) {
        Some(Node::Object(object)) => object.name.clone().unwrap_or_default(),
        Some(Node::Layer(layer)) => layer.name.clone(),
        None => panic!("no such node"),
    }
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

fn square() -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        holes: Vec::new(),
    }])
}

#[test]
fn stamps_count_up_from_001_and_each_kind_counts_on_its_own() {
    let (mut map, layer) = map_with_layer();
    let (a, b) = (stamp(&mut map, layer), stamp(&mut map, layer));
    assert_eq!(
        (name(&map, a).as_str(), name(&map, b).as_str()),
        ("Stamp 001", "Stamp 002")
    );
    let land = Id::new();
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: layer,
        index: None,
        geometry: square(),
        fill: None,
    })
    .unwrap();
    let lake = Id::new();
    map.execute(&Command::CreateLake {
        id: lake,
        parent: layer,
        index: None,
        geometry: square(),
        fill: None,
    })
    .unwrap();
    assert_eq!(name(&map, land), "Landmass 001");
    assert_eq!(name(&map, lake), "Lake 001");
}

#[test]
fn the_next_number_is_one_above_the_highest_and_a_deleted_middle_one_is_not_refilled() {
    let (mut map, layer) = map_with_layer();
    let ids: Vec<Id> = (0..3).map(|_| stamp(&mut map, layer)).collect();
    map.execute(&Command::Delete { ids: vec![ids[1]] }).unwrap();
    let fourth = stamp(&mut map, layer);
    assert_eq!(name(&map, fourth), "Stamp 004");
    // A user's own name does not count, and numbers past 999 keep counting.
    map.execute(&Command::RenameObject {
        id: ids[2],
        name: "Stamp 1200".into(),
    })
    .unwrap();
    map.execute(&Command::RenameObject {
        id: ids[0],
        name: "Castle".into(),
    })
    .unwrap();
    let next = stamp(&mut map, layer);
    assert_eq!(name(&map, next), "Stamp 1201");
}

#[test]
fn placed_stamps_are_numbered_one_after_another() {
    let (mut map, layer) = map_with_layer();
    stamp(&mut map, layer);
    let placed: Vec<PlacedStamp> = (0..3)
        .map(|_| PlacedStamp {
            id: Id::new(),
            stamp: Stamp::default(),
        })
        .collect();
    let ids: Vec<Id> = placed.iter().map(|p| p.id).collect();
    map.execute(&Command::PlaceStamps {
        parent: layer,
        stamps: placed,
    })
    .unwrap();
    let names: Vec<String> = ids.iter().map(|&id| name(&map, id)).collect();
    assert_eq!(names, ["Stamp 002", "Stamp 003", "Stamp 004"]);
}

#[test]
fn a_name_is_stored_so_undo_and_redo_keep_it() {
    let (mut map, layer) = map_with_layer();
    let id = Id::new();
    let inverse = map
        .execute(&Command::CreateStamp {
            id,
            parent: layer,
            index: None,
            stamp: Box::new(Stamp::default()),
        })
        .unwrap();
    map.revert(inverse);
    assert!(map.node(id).is_none());
    // The same Command run again after the undo names it the same way.
    map.execute(&Command::CreateStamp {
        id,
        parent: layer,
        index: None,
        stamp: Box::new(Stamp::default()),
    })
    .unwrap();
    assert_eq!(name(&map, id), "Stamp 001");
}
