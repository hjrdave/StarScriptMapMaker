//! `Command::RenameObject`: trimming, Lock, and undo (issue #376).

use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, History, Id, Map, Node, Scale, Stamp,
};

fn empty_map() -> Map {
    Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)))
}

/// A Map with one Layer holding one Stamp; returns `(map, layer, stamp)`.
fn map_with_stamp() -> (Map, Id, Id) {
    let mut map = empty_map();
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    let stamp = Id::new();
    map.execute(&Command::CreateStamp {
        id: stamp,
        parent: layer,
        index: None,
        stamp: Box::new(Stamp::default()),
    })
    .unwrap();
    (map, layer, stamp)
}

fn object_name(map: &Map, id: Id) -> Option<String> {
    match map.node(id) {
        Some(Node::Object(object)) => object.name.clone(),
        _ => panic!("not an Object"),
    }
}

fn rename(id: Id, name: &str) -> Command {
    Command::RenameObject {
        id,
        name: name.into(),
    }
}

#[test]
fn renames_an_object() {
    let (mut map, _, stamp) = map_with_stamp();
    map.execute(&rename(stamp, "Castle")).unwrap();
    assert_eq!(object_name(&map, stamp).as_deref(), Some("Castle"));
}

#[test]
fn trims_the_name() {
    let (mut map, _, stamp) = map_with_stamp();
    map.execute(&rename(stamp, "  Castle \t")).unwrap();
    assert_eq!(object_name(&map, stamp).as_deref(), Some("Castle"));
}

#[test]
fn a_layer_is_not_an_object() {
    let (mut map, layer, _) = map_with_stamp();
    assert!(matches!(
        map.execute(&rename(layer, "Nope")),
        Err(CommandError::NotAnObject(id)) if id == layer
    ));
}

#[test]
fn a_locked_object_is_rejected() {
    let (mut map, _, stamp) = map_with_stamp();
    map.execute(&Command::SetLocked {
        id: stamp,
        locked: true,
    })
    .unwrap();
    assert!(matches!(
        map.execute(&rename(stamp, "Castle")),
        Err(CommandError::Locked(id)) if id == stamp
    ));
    assert_eq!(object_name(&map, stamp), None);
}

#[test]
fn undo_restores_the_old_name() {
    let (mut map, _, stamp) = map_with_stamp();
    let mut history = History::default();
    history
        .execute_step(&mut map, &rename(stamp, "Castle"), vec![])
        .unwrap();
    history
        .execute_step(&mut map, &rename(stamp, "Keep"), vec![])
        .unwrap();
    assert_eq!(object_name(&map, stamp).as_deref(), Some("Keep"));

    history.undo(&mut map).unwrap();
    assert_eq!(object_name(&map, stamp).as_deref(), Some("Castle"));
    history.undo(&mut map).unwrap();
    assert_eq!(object_name(&map, stamp), None);
    history.redo(&mut map).unwrap();
    assert_eq!(object_name(&map, stamp).as_deref(), Some("Castle"));
}
