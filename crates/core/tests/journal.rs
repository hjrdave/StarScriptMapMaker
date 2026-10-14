//! The Recovery journal (ADR 0005).

use std::io::Write;

use ssmap_core::file::{
    Journal, JournalBase, JournalError, MapFile, Recovered, recovery_path, remove_journal,
};
use ssmap_core::{Canvas, DistanceUnit, Id, Layer, Map, Node, Op, Property, Scale, Subtree};
use uuid::Uuid;

fn empty_map() -> Map {
    Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)))
}

fn insert_layer(id: Id, name: &str) -> Op {
    Op::Insert {
        parent: None,
        index: 0,
        subtree: Subtree {
            id,
            node: Node::Layer(Layer::new(name)),
            children: Vec::new(),
        },
    }
}

fn rename(id: Id, name: &str) -> Op {
    Op::SetProperty {
        id,
        property: Property::Name(name.into()),
    }
}

#[test]
fn the_journal_sits_beside_the_map_file() {
    assert_eq!(
        recovery_path("/maps/Coast.ssmap".as_ref()),
        std::path::Path::new("/maps/Coast.ssmap.recovery")
    );
}

#[test]
fn appended_ops_read_back_in_order_and_replay() {
    let dir = tempfile::tempdir().unwrap();
    let map_path = dir.path().join("a.ssmap");
    let (file, _) = MapFile::create(&map_path, &empty_map(), None).unwrap();
    let base = JournalBase::File {
        generation: file.save_generation(),
    };
    let path = recovery_path(&map_path);
    let layer = Id::new();
    let mut journal = Journal::create(&path, &base, &[insert_layer(layer, "Land")]).unwrap();
    journal.append(&[rename(layer, "Coast")]).unwrap();
    journal.append(&[]).unwrap();
    journal.append(&[rename(layer, "Sea")]).unwrap();
    drop(journal);

    let recovered = Recovered::read(&path).unwrap().unwrap();
    assert!(recovered.matches(file.save_generation()));
    assert!(!recovered.matches(Uuid::now_v7()));
    assert_eq!(recovered.ops.len(), 3);
    assert!(!recovered.torn);
    let mut map = empty_map();
    recovered.replay(&mut map).unwrap();
    match map.node(layer) {
        Some(Node::Layer(layer)) => assert_eq!(layer.name, "Sea"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_torn_last_line_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("torn.recovery");
    let base = JournalBase::Unsaved {
        map: Box::new(empty_map().view()),
    };
    let layer = Id::new();
    drop(Journal::create(&path, &base, &[insert_layer(layer, "Land")]).unwrap());
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(br#"[{"op":"set_property","id":"#).unwrap();

    let recovered = Recovered::read(&path).unwrap().unwrap();
    assert!(recovered.torn);
    assert_eq!(recovered.base, base);
    assert_eq!(recovered.ops, vec![insert_layer(layer, "Land")]);
}

#[test]
fn a_failed_replay_leaves_the_map_unchanged_and_lenient_replay_skips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.recovery");
    let base = JournalBase::File {
        generation: Uuid::nil(),
    };
    let layer = Id::new();
    let ops = [insert_layer(layer, "Land"), rename(Id::new(), "Gone")];
    drop(Journal::create(&path, &base, &ops).unwrap());

    let mut map = empty_map();
    let recovered = Recovered::read(&path).unwrap().unwrap();
    assert!(recovered.clone().replay(&mut map).is_err());
    assert!(map.children(None).is_empty());
    assert_eq!(recovered.replay_lenient(&mut map), 1);
    assert_eq!(map.children(None), &[layer]);
}

#[test]
fn a_live_journal_is_in_use_and_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("live.recovery");
    let base = JournalBase::File {
        generation: Uuid::nil(),
    };
    let journal = Journal::create(&path, &base, &[]).unwrap();
    assert!(matches!(Recovered::read(&path), Err(JournalError::InUse)));
    assert!(matches!(
        Journal::create(&path, &base, &[]),
        Err(JournalError::InUse)
    ));
    journal.delete().unwrap();
    assert!(!path.exists());
    assert!(Recovered::read(&path).unwrap().is_none());
    remove_journal(&path).unwrap();
}

#[test]
fn other_files_are_not_journals() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.recovery");
    std::fs::write(&path, "hello\n").unwrap();
    assert!(matches!(
        Recovered::read(&path),
        Err(JournalError::NotAJournal)
    ));
}
