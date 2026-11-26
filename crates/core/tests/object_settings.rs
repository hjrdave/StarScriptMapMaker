//! An Object's own visibility and opacity, which every Object has whether or not it sits in a
//! Layer (ADR 0018).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, Id, Map, MultiPolygon, Node, Polygon, Scale,
};

fn square(x: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[x, 0.0], [x + 50.0, 0.0], [x + 50.0, 50.0], [x, 50.0]],
        holes: Vec::new(),
    }])
}

/// A Map with two Landmasses at the top level.
fn two_landmasses() -> (Map, [Id; 2]) {
    let mut map = Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)));
    let ids = [Id::new(), Id::new()];
    for (i, &id) in ids.iter().enumerate() {
        map.execute(&Command::CreateLandmass {
            id,
            parent: None,
            index: None,
            geometry: square(i as f64 * 100.0),
            fill: None,
        })
        .unwrap();
    }
    (map, ids)
}

fn object(map: &Map, id: Id) -> &ssmap_core::Object {
    match map.node(id) {
        Some(Node::Object(object)) => object,
        other => panic!("{id} is {other:?}"),
    }
}

#[test]
fn a_hidden_object_does_not_draw_and_undo_shows_it() {
    let (mut map, [a, b]) = two_landmasses();
    assert!(object(&map, a).visible);
    let before = map.revision();
    let inverses = map
        .execute(&Command::SetVisible {
            id: a,
            visible: false,
        })
        .unwrap();
    assert!(map.is_hidden(a));
    assert!(!map.is_hidden(b));
    assert_eq!(map.draw_order(), [b]);
    // What draws changed, so derived data rebuilds rather than redrawing one Object.
    assert!(map.changes_since(before).unwrap().other);
    map.revert(inverses);
    assert!(!map.is_hidden(a));
    assert_eq!(map.draw_order(), [a, b]);
}

#[test]
fn a_hidden_layer_hides_a_visible_object_inside_it() {
    let (mut map, [a, _]) = two_landmasses();
    let group = Id::new();
    map.execute(&Command::CreateGroup {
        id: group,
        ids: vec![a],
        name: "Group".into(),
    })
    .unwrap();
    map.execute(&Command::SetVisible {
        id: group,
        visible: false,
    })
    .unwrap();
    assert!(object(&map, a).visible);
    assert!(map.is_hidden(a));
}

#[test]
fn an_objects_opacity_is_set_checked_and_undone() {
    let (mut map, [a, _]) = two_landmasses();
    assert_eq!(object(&map, a).opacity, 1.0);
    let inverses = map
        .execute(&Command::SetOpacity {
            id: a,
            opacity: 0.4,
        })
        .unwrap();
    assert_eq!(object(&map, a).opacity, 0.4);
    assert_eq!(
        map.clone().execute(&Command::SetOpacity {
            id: a,
            opacity: 1.5
        }),
        Err(CommandError::InvalidOpacity(1.5))
    );
    map.revert(inverses);
    assert_eq!(object(&map, a).opacity, 1.0);
}

#[test]
fn a_locked_object_refuses_visibility_and_opacity() {
    let (mut map, [a, _]) = two_landmasses();
    map.execute(&Command::SetLocked {
        id: a,
        locked: true,
    })
    .unwrap();
    assert_eq!(
        map.clone().execute(&Command::SetVisible {
            id: a,
            visible: false
        }),
        Err(CommandError::Locked(a))
    );
    assert_eq!(
        map.clone().execute(&Command::SetOpacity {
            id: a,
            opacity: 0.5
        }),
        Err(CommandError::Locked(a))
    );
}

#[test]
fn visibility_and_opacity_are_saved_and_left_out_at_their_defaults() {
    let (mut map, [a, b]) = two_landmasses();
    map.execute(&Command::SetVisible {
        id: a,
        visible: false,
    })
    .unwrap();
    map.execute(&Command::SetOpacity {
        id: a,
        opacity: 0.25,
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.ssmap");
    MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert!(!object(&reopened, a).visible);
    assert_eq!(object(&reopened, a).opacity, 0.25);
    assert!(object(&reopened, b).visible);
    assert_eq!(object(&reopened, b).opacity, 1.0);
    let props = |id: Id| serde_json::to_value(map.node(id).unwrap()).unwrap();
    assert_eq!(props(a)["visible"], false);
    assert_eq!(props(a)["opacity"], 0.25);
    assert!(props(b).get("visible").is_none(), "{}", props(b));
    assert!(props(b).get("opacity").is_none(), "{}", props(b));
}
