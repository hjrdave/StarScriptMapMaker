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

#[test]
fn layers_have_no_move_lock() {
    let (mut map, layer) = map_with_layer();
    assert_eq!(
        map.execute(&Command::SetMoveLocked {
            id: layer,
            locked: true
        })
        .map(|_| ()),
        Err(CommandError::NotAnObject(layer))
    );
}
