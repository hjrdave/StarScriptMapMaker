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
        parent: Some(parent),
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
        parent: Some(layer),
        index: None,
        geometry: square(),
        fill: None,
    })
    .unwrap();
    let lake = Id::new();
    map.execute(&Command::CreateLake {
        id: lake,
        parent: Some(layer),
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
        parent: Some(layer),
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
            parent: Some(layer),
            index: None,
            stamp: Box::new(Stamp::default()),
        })
        .unwrap();
    map.revert(inverse);
    assert!(map.node(id).is_none());
    // The same Command run again after the undo names it the same way.
    map.execute(&Command::CreateStamp {
        id,
        parent: Some(layer),
        index: None,
        stamp: Box::new(Stamp::default()),
    })
    .unwrap();
    assert_eq!(name(&map, id), "Stamp 001");
}

/// `next_number` worked out the slow way: one above the highest `<prefix> NNN` among every
/// node's name, or 1.
fn next_number_by_scan(map: &Map, prefix: &str) -> u32 {
    map.ids()
        .filter_map(|id| {
            let name = name(map, id);
            let digits = name.strip_prefix(prefix)?.strip_prefix(' ')?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            digits.parse::<u32>().ok()
        })
        .max()
        .map_or(1, |highest| highest + 1)
}

/// The kept index agrees with a scan of every name, for every prefix the steps use.
fn assert_numbers_match(map: &Map) {
    for prefix in [
        "Stamp",
        "Layer",
        "Landmass",
        "Big Hill",
        "Hill",
        "",
        "Stamp 001",
    ] {
        assert_eq!(
            map.next_number(prefix),
            next_number_by_scan(map, prefix),
            "prefix {prefix:?}"
        );
    }
}

fn run(map: &mut Map, command: Command) -> Vec<ssmap_core::Op> {
    let inverses = map.execute(&command).unwrap();
    assert_numbers_match(map);
    inverses
}

#[test]
fn the_number_index_follows_creates_renames_deletes_and_undo() {
    let (mut map, layer) = map_with_layer();
    assert_numbers_match(&map);
    let stamps: Vec<Id> = (0..5).map(|_| stamp(&mut map, layer)).collect();
    assert_numbers_match(&map);
    assert_eq!(map.next_number("Stamp"), 6);

    // Renamed out of the series, into another, to a name with a space in its prefix, to
    // digits that aren't a number of the series, and cleared.
    run(
        &mut map,
        Command::RenameObject {
            id: stamps[4],
            name: "Hill 012".into(),
        },
    );
    assert_eq!(map.next_number("Stamp"), 5);
    assert_eq!(map.next_number("Hill"), 13);
    run(
        &mut map,
        Command::RenameObject {
            id: stamps[3],
            name: "Big Hill 040".into(),
        },
    );
    assert_eq!(map.next_number("Big Hill"), 41);
    assert_eq!(map.next_number("Hill"), 13, "Big Hill is another prefix");
    run(
        &mut map,
        Command::RenameObject {
            id: stamps[2],
            name: "Stamp 0x1".into(),
        },
    );
    run(
        &mut map,
        Command::RenameObject {
            id: stamps[1],
            name: "Stamp 001 002".into(),
        },
    );
    assert_eq!(map.next_number("Stamp 001"), 3);
    run(
        &mut map,
        Command::RenameObject {
            id: stamps[0],
            name: String::new(),
        },
    );
    assert_eq!(map.next_number("Stamp"), 1, "no Stamp is numbered now");
    run(
        &mut map,
        Command::RenameLayer {
            id: layer,
            name: "Layer 009".into(),
        },
    );
    assert_eq!(map.next_number("Layer"), 10);

    // Two nodes with the same number: deleting one keeps the number taken.
    let twin = stamp(&mut map, layer);
    run(
        &mut map,
        Command::RenameObject {
            id: twin,
            name: "Hill 012".into(),
        },
    );
    let undo_delete = run(
        &mut map,
        Command::Delete {
            ids: vec![stamps[4]],
        },
    );
    assert_eq!(map.next_number("Hill"), 13);
    run(&mut map, Command::Delete { ids: vec![twin] });
    assert_eq!(map.next_number("Hill"), 1);

    // Undo puts the deleted one, and its number, back.
    map.revert(undo_delete);
    assert_numbers_match(&map);
    assert_eq!(map.next_number("Hill"), 13);

    // Deleting the Layer takes everything inside it.
    run(&mut map, Command::Delete { ids: vec![layer] });
    for prefix in ["Stamp", "Hill", "Big Hill", "Layer"] {
        assert_eq!(map.next_number(prefix), 1, "{prefix}");
    }
}

#[test]
fn placed_stamps_and_a_map_built_from_a_view_keep_the_index() {
    let (mut map, layer) = map_with_layer();
    let placed: Vec<PlacedStamp> = (0..300)
        .map(|_| PlacedStamp {
            id: Id::new(),
            stamp: Stamp::default(),
        })
        .collect();
    let undo = run(
        &mut map,
        Command::PlaceStamps {
            parent: Some(layer),
            stamps: placed,
        },
    );
    assert_eq!(map.next_number("Stamp"), 301);
    // A Map loaded from a view (as from a file) counts the same.
    let loaded = Map::from_view(map.view()).unwrap();
    assert_numbers_match(&loaded);
    assert_eq!(loaded.next_number("Stamp"), 301);
    map.revert(undo);
    assert_numbers_match(&map);
    assert_eq!(map.next_number("Stamp"), 1);
}
