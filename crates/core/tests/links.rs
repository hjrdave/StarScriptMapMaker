//! Links: Layers and Objects that move and transform together (issue #263).

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, History, Id, Map, MultiPolygon, Node, Point,
    Polygon, Scale, Stamp, Subtree,
};

fn empty_map() -> Map {
    Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ))
}

fn layer(map: &mut Map, parent: Option<Id>, name: &str) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateLayer {
        id,
        parent,
        index: None,
        name: name.into(),
    })
    .unwrap();
    id
}

fn square(min: [f64; 2], size: f64) -> MultiPolygon {
    let [x, y] = min;
    MultiPolygon(vec![Polygon {
        exterior: vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]],
        holes: Vec::new(),
    }])
}

fn landmass(map: &mut Map, parent: Id, min: [f64; 2]) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateLandmass {
        id,
        parent,
        index: None,
        geometry: square(min, 10.0),
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

fn link(map: &mut Map, ids: &[Id]) -> Id {
    let id = Id::new();
    map.execute(&Command::Link {
        id,
        ids: ids.to_vec(),
    })
    .unwrap();
    id
}

fn corner(map: &Map, id: Id) -> Point {
    match map.node(id) {
        Some(Node::Object(o)) => o.kind.geometry().unwrap().0[0].exterior[0],
        _ => panic!("not an Object"),
    }
}

fn translate(map: &mut Map, ids: &[Id], offset: [f64; 2]) -> Result<(), CommandError> {
    map.execute(&Command::Translate {
        ids: ids.to_vec(),
        offset,
    })
    .map(|_| ())
}

/// Two Landmasses in different Groups, linked.
fn linked_pair() -> (Map, Id, Id, Id, Id) {
    let mut map = empty_map();
    let (g1, g2) = (layer(&mut map, None, "G1"), layer(&mut map, None, "G2"));
    let a = landmass(&mut map, g1, [0.0, 0.0]);
    let b = landmass(&mut map, g2, [100.0, 100.0]);
    link(&mut map, &[a, b]);
    (map, a, b, g1, g2)
}

#[test]
fn linking_stores_a_set_per_node_across_groups() {
    let (map, a, b, g1, g2) = linked_pair();
    assert_eq!(map.node(a).unwrap().link(), map.node(b).unwrap().link());
    assert!(map.node(a).unwrap().link().is_some());
    let mut both = vec![a, b];
    both.sort();
    assert_eq!(map.linked_with(a), both);
    assert_eq!(map.linked_with(b), both);
    assert!(map.is_linked(a));
    // Linking is not grouping: each keeps its own place.
    assert_eq!(map.parent(a), Some(g1));
    assert_eq!(map.parent(b), Some(g2));
    assert!(!map.is_linked(g1));
}

#[test]
fn linking_needs_two_nodes_and_merges_existing_sets() {
    let (mut map, a, b, g1, _) = linked_pair();
    let c = landmass(&mut map, g1, [50.0, 50.0]);
    assert_eq!(
        map.execute(&Command::Link {
            id: Id::new(),
            ids: vec![c, c]
        }),
        Err(CommandError::NothingToLink)
    );
    // Linking c to a brings b along.
    link(&mut map, &[a, c]);
    assert_eq!(map.linked_with(b).len(), 3);
    assert_eq!(map.linked_with(c).len(), 3);
}

#[test]
fn unlinking_leaves_a_set_of_one_that_links_nothing() {
    let (mut map, a, b, ..) = linked_pair();
    map.execute(&Command::Unlink { ids: vec![a] }).unwrap();
    assert!(map.node(a).unwrap().link().is_none());
    assert!(!map.is_linked(a) && !map.is_linked(b));
    // It moves alone.
    translate(&mut map, &[b], [5.0, 0.0]).unwrap();
    assert_eq!(corner(&map, a), [0.0, 0.0]);
    assert_eq!(corner(&map, b), [105.0, 100.0]);
}

#[test]
fn a_locked_node_cannot_be_linked_or_unlinked() {
    let (mut map, a, b, ..) = linked_pair();
    map.execute(&Command::SetLocked {
        id: b,
        locked: true,
    })
    .unwrap();
    assert_eq!(
        map.execute(&Command::Unlink { ids: vec![b] }),
        Err(CommandError::Locked(b))
    );
    assert_eq!(
        map.execute(&Command::Link {
            id: Id::new(),
            ids: vec![a, b]
        }),
        Err(CommandError::Locked(b))
    );
}

#[test]
fn moving_one_linked_node_moves_the_others_as_one_undo_step() {
    let (mut map, a, b, ..) = linked_pair();
    let before = map.view();
    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::Translate {
                ids: vec![a],
                offset: [7.0, 3.0],
            },
            vec![],
        )
        .unwrap();
    assert_eq!(corner(&map, a), [7.0, 3.0]);
    assert_eq!(corner(&map, b), [107.0, 103.0]);
    assert_eq!(history.undo_len(), 1);
    let moved = map.view();
    history.undo(&mut map).unwrap();
    assert_eq!(map.view(), before);
    history.redo(&mut map).unwrap();
    assert_eq!(map.view(), moved);
}

#[test]
fn transforming_applies_the_same_transform_to_the_linked_nodes() {
    let (mut map, a, b, ..) = linked_pair();
    map.execute(&Command::Transform {
        ids: vec![a],
        transform: ssmap_core::Affine([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]),
    })
    .unwrap();
    assert_eq!(corner(&map, a), [0.0, 0.0]);
    assert_eq!(corner(&map, b), [200.0, 200.0]);
}

#[test]
fn a_linked_layer_moves_the_objects_inside_it() {
    let mut map = empty_map();
    let (g1, g2) = (layer(&mut map, None, "G1"), layer(&mut map, None, "G2"));
    let a = landmass(&mut map, g1, [0.0, 0.0]);
    let inner = layer(&mut map, Some(g2), "Inner");
    let deep = landmass(&mut map, inner, [100.0, 0.0]);
    link(&mut map, &[a, g2]);
    translate(&mut map, &[a], [1.0, 1.0]).unwrap();
    assert_eq!(corner(&map, deep), [101.0, 1.0]);
}

#[test]
fn a_locked_linked_node_blocks_the_move_and_is_named() {
    let (mut map, a, b, _, g2) = linked_pair();
    let before = map.view();
    map.execute(&Command::SetLocked {
        id: b,
        locked: true,
    })
    .unwrap();
    let locked = map.view();
    assert_eq!(
        translate(&mut map, &[a], [5.0, 5.0]),
        Err(CommandError::LinkedLocked {
            target: a,
            locked: b
        })
    );
    assert_eq!(map.view(), locked, "nothing moved");

    // Locked through a Layer above counts, and names that Layer.
    map.execute(&Command::SetLocked {
        id: b,
        locked: false,
    })
    .unwrap();
    assert_eq!(map.view(), before);
    map.execute(&Command::SetLocked {
        id: g2,
        locked: true,
    })
    .unwrap();
    assert_eq!(
        translate(&mut map, &[a], [5.0, 5.0]),
        Err(CommandError::LinkedLocked {
            target: a,
            locked: g2
        })
    );
    assert_eq!(corner(&map, a), [0.0, 0.0]);
    let message = translate(&mut map, &[a], [1.0, 1.0])
        .unwrap_err()
        .to_string();
    assert!(message.contains(&g2.to_string()), "{message}");
}

#[test]
fn links_round_trip_through_the_map_file_and_undo_restores_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("links.ssmap");
    let (mut map, a, b, g1, _) = linked_pair();
    let s = stamp(&mut map, g1);
    link(&mut map, &[b, s]);

    MapFile::create(&path, &map, None).unwrap();
    let (_, mut reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.view(), map.view());
    assert_eq!(reopened.linked_with(a), map.linked_with(a));
    assert_eq!(reopened.linked_with(a).len(), 3);
    // The index is rebuilt on load, so a move still propagates.
    translate(&mut reopened, &[a], [1.0, 0.0]).unwrap();
    assert_eq!(corner(&reopened, b), [101.0, 100.0]);

    let mut history = History::default();
    history
        .execute_step(&mut map, &Command::Unlink { ids: vec![s] }, vec![])
        .unwrap();
    assert_eq!(map.linked_with(a).len(), 2);
    history.undo(&mut map).unwrap();
    assert_eq!(map.linked_with(a).len(), 3);
}

#[test]
fn deleting_a_linked_node_leaves_the_rest_linked_and_undo_brings_it_back() {
    let (mut map, a, b, ..) = linked_pair();
    let mut history = History::default();
    history
        .execute_step(&mut map, &Command::Delete { ids: vec![b] }, vec![])
        .unwrap();
    assert!(!map.is_linked(a));
    history.undo(&mut map).unwrap();
    assert!(map.is_linked(a) && map.is_linked(b));
}

#[test]
fn duplicating_one_of_a_set_copies_only_it_unlinked() {
    let (mut map, a, b, ..) = linked_pair();
    let copy = Id::new();
    map.execute(&Command::Duplicate {
        ids: vec![a],
        new_ids: vec![copy],
        offset: [0.0, 0.0],
    })
    .unwrap();
    assert!(map.node(copy).unwrap().link().is_none());
    assert_eq!(map.linked_with(a).len(), 2);
    assert!(map.linked_with(b).contains(&a));
    assert!(!map.linked_with(b).contains(&copy));
}

#[test]
fn duplicating_a_whole_set_links_the_copies_to_each_other() {
    let (mut map, a, b, ..) = linked_pair();
    let (ca, cb) = (Id::new(), Id::new());
    map.execute(&Command::Duplicate {
        ids: vec![a, b],
        new_ids: vec![ca, cb],
        offset: [0.0, 0.0],
    })
    .unwrap();
    let mut copies = vec![ca, cb];
    copies.sort();
    assert_eq!(map.linked_with(ca), copies);
    let old = map.node(a).unwrap().link();
    assert_ne!(map.node(ca).unwrap().link(), old, "a set of its own");
    assert_eq!(map.linked_with(a).len(), 2);
    // Moving a copy leaves the originals alone.
    translate(&mut map, &[ca], [9.0, 0.0]).unwrap();
    assert_eq!(corner(&map, a), [0.0, 0.0]);
    assert_eq!(corner(&map, cb), [109.0, 100.0]);
}

fn branch(map: &Map, id: Id) -> Subtree {
    Subtree {
        id,
        node: map.node(id).unwrap().clone(),
        children: map
            .children(Some(id))
            .iter()
            .map(|&c| branch(map, c))
            .collect(),
    }
}

#[test]
fn pasting_keeps_links_among_the_pasted_nodes_only() {
    let (mut map, a, b, g1, _) = linked_pair();
    let c = landmass(&mut map, g1, [300.0, 300.0]);
    link(&mut map, &[b, c]);
    let dest = layer(&mut map, None, "Dest");

    // a, b and c are one set. Pasting all three keeps them linked, apart from the originals.
    let all = vec![branch(&map, a), branch(&map, b), branch(&map, c)];
    let new_ids = vec![Id::new(), Id::new(), Id::new()];
    map.execute(&Command::Paste {
        parent: Some(dest),
        index: None,
        subtrees: all,
        new_ids: new_ids.clone(),
        offset: [0.0, 0.0],
    })
    .unwrap();
    assert_eq!(map.linked_with(new_ids[0]).len(), 3);
    assert!(
        map.linked_with(new_ids[0])
            .iter()
            .all(|id| new_ids.contains(id))
    );
    assert_eq!(map.linked_with(a).len(), 3);

    // Pasting only one of the set leaves the copy unlinked.
    let lone = Id::new();
    map.execute(&Command::Paste {
        parent: Some(dest),
        index: None,
        subtrees: vec![branch(&map, a)],
        new_ids: vec![lone],
        offset: [0.0, 0.0],
    })
    .unwrap();
    assert!(map.node(lone).unwrap().link().is_none());
}

#[test]
fn many_linked_nodes_move_together() {
    let mut map = empty_map();
    let root = layer(&mut map, None, "Root");
    let ids: Vec<Id> = (0..2000)
        .map(|i| landmass(&mut map, root, [f64::from(i) * 20.0, 0.0]))
        .collect();
    link(&mut map, &ids);
    translate(&mut map, &ids[..1], [0.0, 5.0]).unwrap();
    assert!(ids.iter().all(|&id| corner(&map, id)[1] == 5.0));
}
