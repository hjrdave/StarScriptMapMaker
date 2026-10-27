//! Duplicate: copies of Layers and Objects go just above the originals, as one Command.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, Id, Map, Node, ObjectKind, Scale, Stamp,
};

fn new_map() -> Map {
    Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)))
}

fn layer(map: &mut Map, name: &str) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateLayer {
        id,
        parent: None,
        index: None,
        name: name.into(),
    })
    .unwrap();
    id
}

fn stamp(map: &mut Map, parent: Id, x: f64) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateStamp {
        id,
        parent,
        index: None,
        stamp: Box::new(Stamp::new("pack/asset".parse().unwrap(), [x, 0.0])),
    })
    .unwrap();
    id
}

fn duplicate(
    map: &mut Map,
    ids: &[Id],
    count: usize,
    offset: [f64; 2],
) -> Result<Vec<Id>, CommandError> {
    let new_ids: Vec<Id> = (0..count).map(|_| Id::new()).collect();
    map.execute(&Command::Duplicate {
        ids: ids.to_vec(),
        new_ids: new_ids.clone(),
        offset,
    })?;
    Ok(new_ids)
}

fn name(map: &Map, id: Id) -> String {
    match map.node(id).unwrap() {
        Node::Layer(layer) => layer.name.clone(),
        Node::Object(object) => object.name.clone().unwrap_or_default(),
    }
}

#[test]
fn a_copy_sits_directly_above_its_original_with_a_unique_name() {
    let mut map = new_map();
    let a = layer(&mut map, "Pine cluster");
    let b = layer(&mut map, "Other");
    let first = duplicate(&mut map, &[a], 1, [0.0; 2]).unwrap();
    assert_eq!(map.children(None), &[a, first[0], b]);
    assert_eq!(name(&map, first[0]), "Pine cluster copy");
    let second = duplicate(&mut map, &[a], 1, [0.0; 2]).unwrap();
    assert_eq!(map.children(None), &[a, second[0], first[0], b]);
    assert_eq!(name(&map, second[0]), "Pine cluster copy 2");
    // A copy of a copy doesn't pile up "copy"s.
    let third = duplicate(&mut map, &[first[0]], 1, [0.0; 2]).unwrap();
    assert_eq!(name(&map, third[0]), "Pine cluster copy 3");
}

#[test]
fn a_group_is_copied_with_everything_inside_and_keeps_its_settings() {
    let mut map = new_map();
    let group = layer(&mut map, "Forest");
    let (s1, s2) = (stamp(&mut map, group, 1.0), stamp(&mut map, group, 2.0));
    map.execute(&Command::SetOpacity {
        id: group,
        opacity: 0.5,
    })
    .unwrap();
    map.execute(&Command::SetDepthSort {
        id: group,
        depth_sort: true,
    })
    .unwrap();
    map.execute(&Command::SetLocked {
        id: group,
        locked: true,
    })
    .unwrap();
    // Naming a nested Layer too: the group, then both Stamps.
    let copies = duplicate(&mut map, &[group, s1], 3, [0.0; 2]).unwrap();
    let Some(Node::Layer(copy)) = map.node(copies[0]) else {
        panic!("a Layer");
    };
    assert_eq!(copy.opacity, 0.5);
    assert!(copy.depth_sort);
    assert!(!copy.locked, "the copy of a locked Layer is unlocked");
    assert_eq!(map.children(Some(copies[0])), &copies[1..]);
    assert_eq!(map.children(Some(group)), &[s1, s2]);
}

#[test]
fn several_objects_keep_their_order_and_move_with_the_offset() {
    let mut map = new_map();
    let l = layer(&mut map, "Trees");
    let (a, b, c) = (
        stamp(&mut map, l, 1.0),
        stamp(&mut map, l, 2.0),
        stamp(&mut map, l, 3.0),
    );
    let copies = duplicate(&mut map, &[a, c], 2, [10.0, 5.0]).unwrap();
    assert_eq!(map.children(Some(l)), &[a, copies[0], b, c, copies[1]]);
    let Some(Node::Object(object)) = map.node(copies[1]) else {
        panic!("an Object");
    };
    let ObjectKind::Stamp(copy) = &object.kind else {
        panic!("a Stamp");
    };
    assert_eq!(copy.position, [13.0, 5.0]);
}

#[test]
fn the_new_ids_must_match_what_is_copied() {
    let mut map = new_map();
    let l = layer(&mut map, "A");
    let s = stamp(&mut map, l, 0.0);
    assert!(matches!(
        duplicate(&mut map, &[l], 1, [0.0; 2]),
        Err(CommandError::DuplicateIds {
            expected: 2,
            got: 1
        })
    ));
    // An id inside a listed Layer comes with it rather than being copied twice.
    assert!(duplicate(&mut map, &[l, s], 2, [0.0; 2]).is_ok());
    let taken = vec![s, Id::new()];
    assert!(matches!(
        map.execute(&Command::Duplicate {
            ids: vec![l],
            new_ids: taken,
            offset: [0.0; 2]
        }),
        Err(CommandError::IdInUse(_))
    ));
}

#[test]
fn a_clip_follows_a_copied_landmass() {
    let mut map = new_map();
    let l = layer(&mut map, "Land");
    let land = Id::new();
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: l,
        index: None,
        geometry: ssmap_core::land::stroke_region(&[[10.0, 10.0], [60.0, 10.0]], 5.0),
        fill: None,
    })
    .unwrap();
    let clipped = layer(&mut map, "Clipped");
    map.execute(&Command::SetClip {
        id: clipped,
        target: Some(land),
    })
    .unwrap();
    let copies = duplicate(&mut map, &[l, clipped], 3, [0.0; 2]).unwrap();
    let Some(Node::Layer(copy)) = map.node(copies[2]) else {
        panic!("a Layer");
    };
    assert_eq!(copy.clip, Some(copies[1]));
}

#[test]
fn a_whole_duplicate_is_one_undo_step() {
    let mut map = new_map();
    let l = layer(&mut map, "A");
    stamp(&mut map, l, 0.0);
    let before = map.view();
    let inverses = map
        .execute(&Command::Duplicate {
            ids: vec![l],
            new_ids: vec![Id::new(), Id::new()],
            offset: [0.0; 2],
        })
        .unwrap();
    assert_ne!(map.view(), before);
    map.revert(inverses);
    assert_eq!(map.view(), before);
}
