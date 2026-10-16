//! Groups: Layers nested in Layers, with Lock and Hide inherited (issue #261).

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, History, Id, Map, Node, Scale, Stamp,
};

fn empty_map() -> Map {
    Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)))
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

fn group(map: &mut Map, ids: &[Id]) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateGroup {
        id,
        ids: ids.to_vec(),
        name: "Group".into(),
    })
    .unwrap();
    id
}

fn set_locked(map: &mut Map, id: Id, locked: bool) {
    map.execute(&Command::SetLocked { id, locked }).unwrap();
}

#[test]
fn groups_nest_inside_groups() {
    let mut map = empty_map();
    let a = layer(&mut map, None, "A");
    let b = layer(&mut map, None, "B");
    let inner = group(&mut map, &[a, b]);
    let s = stamp(&mut map, a);
    let outer = group(&mut map, &[inner]);

    assert_eq!(map.children(None), [outer]);
    assert_eq!(map.children(Some(outer)), [inner]);
    assert_eq!(map.children(Some(inner)), [a, b]);
    assert_eq!(map.parent(s), Some(a));
    assert!(map.is_ancestor_or_self(outer, s));
    map.validate().unwrap();
}

#[test]
fn a_group_takes_the_place_of_its_topmost_member_and_keeps_their_order() {
    let mut map = empty_map();
    let bottom = layer(&mut map, None, "Bottom");
    let a = layer(&mut map, None, "A");
    let middle = layer(&mut map, None, "Middle");
    let b = layer(&mut map, None, "B");
    let top = layer(&mut map, None, "Top");
    // Listed out of order on purpose.
    let g = group(&mut map, &[b, a]);
    assert_eq!(map.children(None), [bottom, middle, g, top]);
    assert_eq!(map.children(Some(g)), [a, b]);
}

#[test]
fn grouping_needs_siblings_and_unlocked_members() {
    let mut map = empty_map();
    let a = layer(&mut map, None, "A");
    let b = layer(&mut map, None, "B");
    let inside = layer(&mut map, Some(a), "Inside");
    let create = |ids: Vec<Id>| Command::CreateGroup {
        id: Id::new(),
        ids,
        name: "G".into(),
    };
    assert!(matches!(
        map.execute(&create(vec![b, inside])),
        Err(CommandError::NotSiblings(_))
    ));
    assert_eq!(
        map.execute(&create(vec![])),
        Err(CommandError::NothingToGroup)
    );
    assert_eq!(
        map.execute(&create(vec![a, a])),
        Err(CommandError::DuplicateTarget(a))
    );
    set_locked(&mut map, b, true);
    assert!(map.execute(&create(vec![a, b])).is_err());
    map.validate().unwrap();
}

#[test]
fn locking_a_group_locks_everything_inside_but_leaves_it_selectable() {
    let mut map = empty_map();
    let a = layer(&mut map, None, "A");
    let s = stamp(&mut map, a);
    let g = group(&mut map, &[a]);
    set_locked(&mut map, g, true);

    assert!(map.is_locked(a) && map.is_locked(s));
    assert!(map.locked_by_parent(a) && map.locked_by_parent(s));
    assert!(!map.locked_by_parent(g), "the Group is locked itself");
    assert_eq!(map.locked_ancestor(s), Some(g));
    // Locked stamps are still found by picking, flagged as locked.
    assert!(map.node(s).is_some());

    // Edits and moves are refused, and a child can't be unlocked on its own.
    assert!(matches!(
        map.execute(&Command::RenameLayer {
            id: a,
            name: "X".into()
        }),
        Err(CommandError::InLockedLayer { .. })
    ));
    assert!(
        map.execute(&Command::Translate {
            ids: vec![s],
            offset: [1.0, 1.0]
        })
        .is_err()
    );
    assert!(
        map.execute(&Command::SetLocked {
            id: a,
            locked: false
        })
        .is_err()
    );
    assert!(
        map.execute(&Command::Move {
            id: s,
            parent: None,
            index: 0
        })
        .is_err()
    );
    set_locked(&mut map, g, false);
    assert!(!map.is_locked(s));
}

#[test]
fn a_locked_group_refuses_new_content() {
    let mut map = empty_map();
    let outside = layer(&mut map, None, "Outside");
    let moved = stamp(&mut map, outside);
    let g = layer(&mut map, None, "G");
    let nested = layer(&mut map, Some(g), "Nested");
    set_locked(&mut map, g, true);

    assert!(!map.accepts_children(g));
    assert!(!map.accepts_children(nested), "locked through its parent");
    assert!(map.accepts_children(outside));
    assert!(!map.accepts_children(moved), "an Object holds nothing");

    for parent in [g, nested] {
        assert!(
            map.execute(&Command::Move {
                id: moved,
                parent: Some(parent),
                index: 0
            })
            .is_err()
        );
        assert!(
            map.execute(&Command::CreateLayer {
                id: Id::new(),
                parent: Some(parent),
                index: None,
                name: "New".into()
            })
            .is_err()
        );
    }
    assert_eq!(map.parent(moved), Some(outside));
}

#[test]
fn hiding_a_group_hides_everything_inside() {
    let mut map = empty_map();
    let a = layer(&mut map, None, "A");
    let s = stamp(&mut map, a);
    let g = group(&mut map, &[a]);
    assert_eq!(map.draw_order(), [s]);
    map.execute(&Command::SetVisible {
        id: g,
        visible: false,
    })
    .unwrap();
    assert!(map.is_hidden(g) && map.is_hidden(a) && map.is_hidden(s));
    assert!(map.draw_order().is_empty());
    // The child's own flag is untouched, so showing the Group brings it back.
    assert!(matches!(map.node(a), Some(Node::Layer(l)) if l.visible));
    map.execute(&Command::SetVisible {
        id: g,
        visible: true,
    })
    .unwrap();
    assert_eq!(map.draw_order(), [s]);
}

#[test]
fn deleting_a_group_deletes_its_contents() {
    let mut map = empty_map();
    let a = layer(&mut map, None, "A");
    let s = stamp(&mut map, a);
    let g = group(&mut map, &[a]);
    map.execute(&Command::Delete { ids: vec![g] }).unwrap();
    assert!(!map.contains(g) && !map.contains(a) && !map.contains(s));
}

#[test]
fn ungrouping_moves_the_contents_up_in_place() {
    let mut map = empty_map();
    let below = layer(&mut map, None, "Below");
    let a = layer(&mut map, None, "A");
    let b = layer(&mut map, None, "B");
    let above = layer(&mut map, None, "Above");
    let g = group(&mut map, &[a, b]);
    map.execute(&Command::Ungroup { ids: vec![g] }).unwrap();
    assert_eq!(map.children(None), [below, a, b, above]);
    assert!(!map.contains(g));
    map.validate().unwrap();
}

#[test]
fn ungrouping_nested_groups_together_flattens_them() {
    let mut map = empty_map();
    let a = layer(&mut map, None, "A");
    let b = layer(&mut map, None, "B");
    let inner = group(&mut map, &[a]);
    let outer = group(&mut map, &[inner, b]);
    map.execute(&Command::Ungroup {
        ids: vec![outer, inner],
    })
    .unwrap();
    assert_eq!(map.children(None), [a, b]);
    map.validate().unwrap();
}

#[test]
fn a_locked_group_cannot_be_ungrouped() {
    let mut map = empty_map();
    let a = layer(&mut map, None, "A");
    let g = group(&mut map, &[a]);
    set_locked(&mut map, g, true);
    assert!(map.execute(&Command::Ungroup { ids: vec![g] }).is_err());
    assert_eq!(map.children(Some(g)), [a]);
}

/// Runs `command` as one Undo step and checks undo and redo restore the exact tree.
fn one_undo_step(map: &mut Map, command: &Command) {
    let before = map.view();
    let mut history = History::default();
    history.execute_step(map, command, vec![]).unwrap();
    let after = map.view();
    assert_ne!(before, after);
    assert_eq!(history.undo_len(), 1);
    history.undo(map).unwrap();
    assert_eq!(map.view(), before, "undo restores the tree");
    map.validate().unwrap();
    history.redo(map).unwrap();
    assert_eq!(map.view(), after, "redo repeats it");
    map.validate().unwrap();
}

#[test]
fn create_ungroup_reparent_and_delete_are_each_one_undo_step() {
    let mut map = empty_map();
    let a = layer(&mut map, None, "A");
    let b = layer(&mut map, None, "B");
    let s = stamp(&mut map, a);
    let dest = layer(&mut map, None, "Dest");

    let g = Id::new();
    one_undo_step(
        &mut map,
        &Command::CreateGroup {
            id: g,
            ids: vec![a, b],
            name: "G".into(),
        },
    );
    one_undo_step(&mut map, &Command::Ungroup { ids: vec![g] });
    one_undo_step(
        &mut map,
        &Command::Move {
            id: s,
            parent: Some(dest),
            index: 0,
        },
    );
    one_undo_step(&mut map, &Command::Delete { ids: vec![a] });
}

#[test]
fn groups_round_trip_through_the_map_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("groups.ssmap");
    let mut map = empty_map();
    let a = layer(&mut map, None, "A");
    let b = layer(&mut map, None, "B");
    let s = stamp(&mut map, a);
    let inner = group(&mut map, &[a]);
    let outer = group(&mut map, &[inner, b]);
    map.execute(&Command::SetVisible {
        id: inner,
        visible: false,
    })
    .unwrap();
    set_locked(&mut map, outer, true);

    MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.view(), map.view());
    assert_eq!(reopened.children(Some(outer)), [inner, b]);
    assert_eq!(reopened.parent(s), Some(a));
    assert!(reopened.locked_by_parent(s) && reopened.is_hidden(s));
    reopened.validate().unwrap();
}

#[test]
fn a_big_tree_groups_and_ungroups() {
    let mut map = empty_map();
    let root = layer(&mut map, None, "Root");
    let ids: Vec<Id> = (0..2000).map(|_| stamp(&mut map, root)).collect();
    let g = group(&mut map, &ids);
    assert_eq!(map.children(Some(g)), ids);
    map.execute(&Command::Ungroup { ids: vec![g] }).unwrap();
    assert_eq!(map.children(Some(root)), ids);
}
