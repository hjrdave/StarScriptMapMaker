//! Paste: copied branches go into a Layer as one Command, from this Map or another.

use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, Id, Map, Node, ObjectKind, Scale, Stamp, Subtree,
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

/// The branches rooted at `ids`, as a copy would hold them.
fn copied(map: &Map, ids: &[Id]) -> Vec<Subtree> {
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
    ids.iter().map(|&id| branch(map, id)).collect()
}

fn count(subtrees: &[Subtree]) -> usize {
    subtrees.iter().map(|s| 1 + count(&s.children)).sum()
}

fn paste(
    map: &mut Map,
    parent: Option<Id>,
    index: Option<usize>,
    subtrees: Vec<Subtree>,
    offset: [f64; 2],
) -> Result<Vec<Id>, CommandError> {
    let new_ids: Vec<Id> = (0..count(&subtrees)).map(|_| Id::new()).collect();
    map.execute(&Command::Paste {
        parent,
        index,
        subtrees,
        new_ids: new_ids.clone(),
        offset,
    })?;
    Ok(new_ids)
}

fn position(map: &Map, id: Id) -> [f64; 2] {
    match map.node(id) {
        Some(Node::Object(o)) => match &o.kind {
            ObjectKind::Stamp(s) => s.position,
            _ => panic!("not a Stamp"),
        },
        _ => panic!("not an Object"),
    }
}

#[test]
fn a_pasted_branch_keeps_its_structure_and_moves_as_a_whole() {
    let mut source = new_map();
    let group = layer(&mut source, "Pines");
    let a = stamp(&mut source, group, 10.0);
    let b = stamp(&mut source, group, 20.0);
    let branches = copied(&source, &[group]);

    let mut target = new_map();
    let _below = layer(&mut target, "Below");
    let ids = paste(&mut target, None, None, branches, [5.0, 7.0]).unwrap();
    assert_eq!(target.children(None).last(), Some(&ids[0]));
    assert_eq!(target.children(Some(ids[0])), &[ids[1], ids[2]]);
    assert_eq!(position(&target, ids[1]), [15.0, 7.0]);
    assert_eq!(position(&target, ids[2]), [25.0, 7.0]);
    // The source is untouched.
    assert_eq!(position(&source, a), [10.0, 0.0]);
    assert_eq!(position(&source, b), [20.0, 0.0]);
    target.validate().unwrap();
}

#[test]
fn a_name_already_beside_the_paste_gets_copy_added() {
    let mut map = new_map();
    let group = layer(&mut map, "Pine cluster");
    let branches = copied(&map, &[group]);
    let first = paste(&mut map, None, None, branches.clone(), [0.0; 2]).unwrap();
    let second = paste(&mut map, None, None, branches, [0.0; 2]).unwrap();
    let name = |id| match map.node(id).unwrap() {
        Node::Layer(l) => l.name.clone(),
        _ => unreachable!(),
    };
    assert_eq!(name(first[0]), "Pine cluster copy");
    assert_eq!(name(second[0]), "Pine cluster copy 2");
}

#[test]
fn paste_goes_at_the_index_and_is_one_undo_step() {
    let mut map = new_map();
    let bottom = layer(&mut map, "Bottom");
    let top = layer(&mut map, "Top");
    let other = layer(&mut map, "Other");
    let branches = copied(&map, &[other]);
    let before = map.view();
    let new_ids = vec![Id::new()];
    let inverses = map
        .execute(&Command::Paste {
            parent: None,
            index: Some(1),
            subtrees: branches,
            new_ids: new_ids.clone(),
            offset: [0.0; 2],
        })
        .unwrap();
    assert_eq!(map.children(None), &[bottom, new_ids[0], top, other]);
    map.revert(inverses);
    assert_eq!(map.view(), before);
}

#[test]
fn a_clip_follows_a_pasted_landmass_and_is_dropped_when_the_landmass_is_gone() {
    let mut source = new_map();
    let group = layer(&mut source, "Clipped");
    // A Clip to a Landmass that is not in the target Map.
    let mut branches = copied(&source, &[group]);
    if let Node::Layer(l) = &mut branches[0].node {
        l.clip = Some(Id::new());
    }
    let mut target = new_map();
    let ids = paste(&mut target, None, None, branches, [0.0; 2]).unwrap();
    match target.node(ids[0]).unwrap() {
        Node::Layer(l) => assert_eq!(l.clip, None),
        _ => unreachable!(),
    }
}

#[test]
fn paste_into_a_locked_layer_or_with_wrong_ids_fails() {
    let mut map = new_map();
    let group = layer(&mut map, "Locked");
    let branches = copied(&map, &[group]);
    map.execute(&Command::SetLocked {
        id: group,
        locked: true,
    })
    .unwrap();
    assert!(matches!(
        paste(&mut map, Some(group), None, branches.clone(), [0.0; 2]),
        Err(CommandError::Locked(_))
    ));
    assert!(matches!(
        map.execute(&Command::Paste {
            parent: None,
            index: None,
            subtrees: branches,
            new_ids: Vec::new(),
            offset: [0.0; 2],
        }),
        Err(CommandError::DuplicateIds { .. })
    ));
}
