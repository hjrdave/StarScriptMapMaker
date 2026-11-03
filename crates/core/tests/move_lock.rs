//! `Object::move_locked`: new shapes start locked for movement but stay editable (ADR 0004).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, History, Id, Map, MultiPolygon, Node, Polygon,
    Scale, Stamp,
};

fn square() -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        holes: vec![],
    }])
}

fn map_with_layer() -> (Map, Id) {
    let mut map = Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    (map, layer)
}

fn move_locked(map: &Map, id: Id) -> bool {
    match map.node(id) {
        Some(Node::Object(object)) => object.move_locked,
        _ => panic!("not an Object"),
    }
}

fn translate(map: &mut Map, id: Id) -> Result<(), CommandError> {
    map.execute(&Command::Translate {
        ids: vec![id],
        offset: [1.0, 1.0],
    })
    .map(|_| ())
}

#[test]
fn land_and_water_start_locked_for_movement_but_stamps_do_not() {
    let (mut map, layer) = map_with_layer();
    let (land, lake, stamp) = (Id::new(), Id::new(), Id::new());
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: layer,
        index: None,
        geometry: square(),
        fill: None,
    })
    .unwrap();
    map.execute(&Command::CreateLake {
        id: lake,
        parent: layer,
        index: None,
        geometry: square(),
        fill: None,
    })
    .unwrap();
    map.execute(&Command::CreateStamp {
        id: stamp,
        parent: layer,
        index: None,
        stamp: Box::new(Stamp::default()),
    })
    .unwrap();
    assert!(move_locked(&map, land) && move_locked(&map, lake));
    assert!(!move_locked(&map, stamp));
    assert_eq!(
        translate(&mut map, land),
        Err(CommandError::MoveLocked(land))
    );
}

#[test]
fn a_move_locked_object_can_still_be_edited_and_unlocking_allows_the_move() {
    let (mut map, layer) = map_with_layer();
    let land = Id::new();
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: layer,
        index: None,
        geometry: square(),
        fill: None,
    })
    .unwrap();
    map.execute(&Command::RenameObject {
        id: land,
        name: "Isle".into(),
    })
    .unwrap();
    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::SetMoveLocked {
                id: land,
                locked: false,
            },
            vec![],
        )
        .unwrap();
    translate(&mut map, land).unwrap();
    history.undo(&mut map).unwrap();
    assert!(move_locked(&map, land));
}

#[test]
fn a_linked_move_locked_object_blocks_the_whole_set() {
    let (mut map, layer) = map_with_layer();
    let (a, b) = (Id::new(), Id::new());
    for id in [a, b] {
        map.execute(&Command::CreateLandmass {
            id,
            parent: layer,
            index: None,
            geometry: square(),
            fill: None,
        })
        .unwrap();
    }
    map.execute(&Command::Link {
        id: Id::new(),
        ids: vec![a, b],
    })
    .unwrap();
    map.execute(&Command::SetMoveLocked {
        id: a,
        locked: false,
    })
    .unwrap();
    assert_eq!(translate(&mut map, a), Err(CommandError::MoveLocked(b)));
}

/// A Landmass in `parent` that is free to move on its own: its own pin is off.
fn free_land(map: &mut Map, parent: Id) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateLandmass {
        id,
        parent,
        index: None,
        geometry: square(),
        fill: None,
    })
    .unwrap();
    map.execute(&Command::SetMoveLocked { id, locked: false })
        .unwrap();
    id
}

fn pin(map: &mut Map, id: Id, pinned: bool) -> Result<(), CommandError> {
    map.execute(&Command::SetMoveLocked { id, locked: pinned })
        .map(|_| ())
}

#[test]
fn a_pinned_layer_pins_everything_inside_it_and_unpinning_frees_it() {
    let (mut map, layer) = map_with_layer();
    let land = free_land(&mut map, layer);
    assert!(translate(&mut map, land).is_ok());
    assert!(!map.is_pinned(land));

    pin(&mut map, layer, true).unwrap();
    assert!(map.is_pinned(land));
    assert_eq!(map.pinned_ancestor(land), Some(layer));
    // The Landmass has no pin of its own, and still refuses to move.
    assert!(!move_locked(&map, land));
    assert_eq!(
        translate(&mut map, land),
        Err(CommandError::MoveLocked(land))
    );

    pin(&mut map, layer, false).unwrap();
    assert!(!map.is_pinned(land) && map.pinned_ancestor(land).is_none());
    assert!(translate(&mut map, land).is_ok());
}

#[test]
fn a_pin_reaches_through_nested_groups_and_an_object_keeps_its_own() {
    let (mut map, outer) = map_with_layer();
    let inner = Id::new();
    map.execute(&Command::CreateLayer {
        id: inner,
        parent: Some(outer),
        index: None,
        name: "Inner".into(),
    })
    .unwrap();
    let deep = free_land(&mut map, inner);
    pin(&mut map, outer, true).unwrap();
    assert!(map.is_pinned(deep) && map.is_pinned(inner));
    assert_eq!(map.pinned_ancestor(deep), Some(outer));

    // Unpinning the Group leaves an Object's own pin alone.
    pin(&mut map, deep, true).unwrap();
    pin(&mut map, outer, false).unwrap();
    assert!(map.is_pinned(deep), "the Landmass's own pin stays");
    assert!(!map.is_pinned(inner));
}

#[test]
fn pinning_a_layer_is_one_undo_step_and_a_locked_layer_refuses_it() {
    let (mut map, layer) = map_with_layer();
    let land = free_land(&mut map, layer);
    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::SetMoveLocked {
                id: layer,
                locked: true,
            },
            vec![],
        )
        .unwrap();
    assert!(map.is_pinned(land));
    history.undo(&mut map).unwrap();
    assert!(!map.is_pinned(land));
    history.redo(&mut map).unwrap();
    assert!(map.is_pinned(land));

    // A locked Layer refuses every Command, the pin included.
    pin(&mut map, layer, false).unwrap();
    map.execute(&Command::SetLocked {
        id: layer,
        locked: true,
    })
    .unwrap();
    assert_eq!(pin(&mut map, layer, true), Err(CommandError::Locked(layer)));
}

#[test]
fn a_layers_pin_survives_a_save_and_reopen_and_older_files_have_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pinned.ssmap");
    let (mut map, layer) = map_with_layer();
    let land = free_land(&mut map, layer);
    pin(&mut map, layer, true).unwrap();
    ssmap_core::file::MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = ssmap_core::file::MapFile::open(&path).unwrap();
    assert!(reopened.is_pinned(land));
    assert_eq!(reopened.view(), map.view());

    // A Layer saved before format 1.12 has no `pinned` in its props and opens unpinned.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "UPDATE nodes SET props = json_remove(props, '$.pinned') WHERE kind = 'layer';
         UPDATE meta SET value = '1.11' WHERE key = 'format_version';",
    )
    .unwrap();
    drop(conn);
    let (_, old) = ssmap_core::file::MapFile::open(&path).unwrap();
    assert!(!old.is_pinned(land));
}

#[test]
fn the_refusal_says_the_object_is_pinned_to_the_canvas() {
    let id = Id::new();
    assert_eq!(
        CommandError::MoveLocked(id).to_string(),
        format!("{id} is pinned to the Canvas")
    );
}
