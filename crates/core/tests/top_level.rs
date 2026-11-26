//! The Commands that create Objects, with no Layer to put them in: they go to the top level
//! (ADR 0018).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, Id, Label, Map, MultiPolygon, PlacedStamp,
    Polygon, River, RiverPoint, Scale, Stamp,
};

fn empty_map() -> Map {
    Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)))
}

fn square(x: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[x, 0.0], [x + 50.0, 0.0], [x + 50.0, 50.0], [x, 50.0]],
        holes: Vec::new(),
    }])
}

fn stamp(x: f64) -> Box<Stamp> {
    Box::new(Stamp::new("base/tree".parse().unwrap(), [x, 10.0]))
}

/// One Command of each kind that creates an Object, putting `id` into `parent`.
fn creations(id: Id, parent: Option<Id>) -> Vec<Command> {
    vec![
        Command::CreateLandmass {
            id,
            parent,
            index: None,
            geometry: square(0.0),
            fill: None,
        },
        Command::CreateLake {
            id,
            parent,
            index: None,
            geometry: square(100.0),
            fill: None,
        },
        Command::CreateRiver {
            id,
            parent,
            index: None,
            river: Box::new(River::new(vec![
                RiverPoint::new([10.0, 10.0], 4.0),
                RiverPoint::new([300.0, 200.0], 12.0),
            ])),
        },
        Command::CreateLabel {
            id,
            parent,
            index: None,
            label: Box::new(Label {
                text: "Thule".into(),
                ..Label::default()
            }),
        },
        Command::CreateStamp {
            id,
            parent,
            index: None,
            stamp: stamp(5.0),
        },
        Command::PlaceStamps {
            parent,
            stamps: vec![PlacedStamp {
                id,
                stamp: *stamp(5.0),
            }],
        },
    ]
}

#[test]
fn every_kind_of_object_can_be_created_at_the_top_level_and_undone() {
    for command in creations(Id::new(), None) {
        let mut map = empty_map();
        let below = Id::new();
        map.execute(&Command::CreateLandmass {
            id: below,
            parent: None,
            index: None,
            geometry: square(400.0),
            fill: None,
        })
        .unwrap();
        let before = map.clone();
        let inverses = map.execute(&command).unwrap();
        let [first, id] = map.children(None) else {
            panic!("{command:?} left {:?} at the top level", map.children(None));
        };
        assert_eq!(*first, below, "{command:?} goes on top");
        assert_eq!(map.parent(*id), None);
        map.revert(inverses);
        assert_eq!(map.children(None), before.children(None));
    }
}

#[test]
fn an_index_places_a_top_level_object_among_its_siblings() {
    let mut map = empty_map();
    let ids: Vec<Id> = (0..2).map(|_| Id::new()).collect();
    for (i, &id) in ids.iter().enumerate() {
        map.execute(&Command::CreateStamp {
            id,
            parent: None,
            index: None,
            stamp: stamp(i as f64),
        })
        .unwrap();
    }
    let under = Id::new();
    map.execute(&Command::CreateStamp {
        id: under,
        parent: None,
        index: Some(0),
        stamp: stamp(9.0),
    })
    .unwrap();
    assert_eq!(map.children(None), [under, ids[0], ids[1]]);
}

#[test]
fn a_locked_or_missing_layer_still_refuses_new_objects() {
    let mut map = empty_map();
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    map.execute(&Command::SetLocked {
        id: layer,
        locked: true,
    })
    .unwrap();
    let missing = Id::new();
    for command in creations(Id::new(), Some(layer)) {
        assert_eq!(
            map.clone().execute(&command),
            Err(CommandError::Locked(layer))
        );
    }
    for command in creations(Id::new(), Some(missing)) {
        assert_eq!(
            map.clone().execute(&command),
            Err(CommandError::NotFound(missing))
        );
    }
}

#[test]
fn a_script_may_leave_out_the_parent_or_give_a_layer() {
    let id = Id::new();
    let layer = Id::new();
    let top: Command = serde_json::from_value(serde_json::json!({
        "command": "create_stamp",
        "id": id,
        "stamp": *stamp(1.0),
    }))
    .unwrap();
    assert!(matches!(top, Command::CreateStamp { parent: None, .. }));
    let inside: Command = serde_json::from_value(serde_json::json!({
        "command": "create_stamp",
        "id": id,
        "parent": layer,
        "stamp": *stamp(1.0),
    }))
    .unwrap();
    assert!(matches!(inside, Command::CreateStamp { parent: Some(p), .. } if p == layer));
    // Written back, a top-level Command has no parent at all.
    let written = serde_json::to_value(&top).unwrap();
    assert!(written.get("parent").is_none(), "{written}");
}
