//! Canvas resize and Object membership: resizing never moves, resizes, removes or re-homes an
//! Object, and only an explicit membership change does the last.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Command, CommandError, DistanceUnit, History, Id, Map, Membership, MultiPolygon, Node,
    Polygon, Scale,
};

fn square(x: f64, y: f64, size: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]],
        holes: vec![],
    }])
}

fn map_with_landmass(geometry: MultiPolygon) -> (Map, Id) {
    let mut map = Map::new(Canvas::new(100, 80, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    let id = Id::new();
    map.execute(&Command::CreateLandmass {
        id,
        parent: layer,
        index: None,
        geometry,
        fill: None,
    })
    .unwrap();
    (map, id)
}

fn resize(map: &mut Map, origin: [f64; 2], width: u32, height: u32) {
    map.execute(&Command::ResizeCanvas {
        origin,
        width,
        height,
    })
    .unwrap();
}

fn membership(map: &Map, id: Id) -> Membership {
    match map.node(id) {
        Some(Node::Object(object)) => object.membership,
        _ => panic!("not an Object"),
    }
}

#[test]
fn new_objects_belong_to_the_canvas() {
    let (map, id) = map_with_landmass(square(10.0, 10.0, 20.0));
    assert_eq!(membership(&map, id), Membership::Canvas);
}

#[test]
fn resizing_from_any_edge_never_changes_objects() {
    let (mut map, id) = map_with_landmass(square(10.0, 10.0, 20.0));
    let before = map.view().layers;

    // Grow past the left and top edges: the origin moves, the Object does not.
    resize(&mut map, [-50.0, -40.0], 150, 120);
    assert_eq!(map.canvas.rect().min, [-50.0, -40.0]);
    assert_eq!(map.canvas.rect().max, [100.0, 80.0]);
    assert_eq!(map.view().layers, before);

    // Shrink past the Object from the left/top, and then to nothing around it.
    resize(&mut map, [60.0, 60.0], 10, 10);
    assert_eq!(map.view().layers, before);

    // Grow back.
    resize(&mut map, [0.0, 0.0], 100, 80);
    assert_eq!(map.view().layers, before);
    assert_eq!(membership(&map, id), Membership::Canvas);
}

#[test]
fn an_object_left_outside_a_shrunken_canvas_stays_a_member() {
    let (mut map, id) = map_with_landmass(square(80.0, 60.0, 10.0));
    resize(&mut map, [0.0, 0.0], 20, 20);
    assert_eq!(membership(&map, id), Membership::Canvas);
    resize(&mut map, [0.0, 0.0], 100, 80);
    assert_eq!(membership(&map, id), Membership::Canvas);
}

#[test]
fn membership_changes_only_when_asked() {
    let (mut map, id) = map_with_landmass(square(10.0, 10.0, 20.0));
    map.execute(&Command::SetMembership {
        ids: vec![id],
        membership: Membership::OffCanvas,
    })
    .unwrap();
    assert_eq!(membership(&map, id), Membership::OffCanvas);
    // Resizing to cover the Object does not adopt it.
    resize(&mut map, [-100.0, -100.0], 400, 400);
    assert_eq!(membership(&map, id), Membership::OffCanvas);
}

#[test]
fn membership_needs_an_unlocked_object() {
    let (mut map, id) = map_with_landmass(square(0.0, 0.0, 5.0));
    let layer = map.view().layers[0].id;
    assert!(matches!(
        map.execute(&Command::SetMembership {
            ids: vec![layer],
            membership: Membership::OffCanvas,
        }),
        Err(CommandError::NotAnObject(_))
    ));
    map.execute(&Command::SetLocked { id, locked: true })
        .unwrap();
    assert!(matches!(
        map.execute(&Command::SetMembership {
            ids: vec![id],
            membership: Membership::OffCanvas,
        }),
        Err(CommandError::Locked(_))
    ));
    // A locked Object does not stop the Canvas resizing.
    resize(&mut map, [0.0, 0.0], 10, 10);
}

#[test]
fn rejects_an_empty_or_non_finite_canvas() {
    let (mut map, _) = map_with_landmass(square(0.0, 0.0, 5.0));
    let bad = |origin, width, height| Command::ResizeCanvas {
        origin,
        width,
        height,
    };
    assert!(matches!(
        map.execute(&bad([0.0, 0.0], 0, 10)),
        Err(CommandError::InvalidCanvasSize { .. })
    ));
    assert!(matches!(
        map.execute(&bad([f64::NAN, 0.0], 10, 10)),
        Err(CommandError::InvalidCanvasOrigin(_))
    ));
    assert_eq!((map.canvas.width, map.canvas.height), (100, 80));
}

#[test]
fn undo_and_redo_restore_the_canvas_and_touch_no_objects() {
    let (mut map, _) = map_with_landmass(square(10.0, 10.0, 20.0));
    let before = map.view();
    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::ResizeCanvas {
                origin: [-30.0, -20.0],
                width: 50,
                height: 40,
            },
            vec![],
        )
        .unwrap();
    let after = map.view();
    assert_eq!(after.layers, before.layers);
    assert_ne!(after.canvas, before.canvas);

    history.undo(&mut map).unwrap();
    assert_eq!(map.view(), before);
    history.redo(&mut map).unwrap();
    assert_eq!(map.view(), after);
}

#[test]
fn undo_restores_membership() {
    let (mut map, id) = map_with_landmass(square(10.0, 10.0, 20.0));
    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::SetMembership {
                ids: vec![id],
                membership: Membership::OffCanvas,
            },
            vec![],
        )
        .unwrap();
    history.undo(&mut map).unwrap();
    assert_eq!(membership(&map, id), Membership::Canvas);
    history.redo(&mut map).unwrap();
    assert_eq!(membership(&map, id), Membership::OffCanvas);
}

#[test]
fn origin_and_membership_round_trip_through_json() {
    let (mut map, id) = map_with_landmass(square(10.0, 10.0, 20.0));
    resize(&mut map, [-5.5, 7.0], 30, 30);
    map.execute(&Command::SetMembership {
        ids: vec![id],
        membership: Membership::OffCanvas,
    })
    .unwrap();
    let json = serde_json::to_string(&map.view()).unwrap();
    let back = Map::from_view(serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(back.view(), map.view());
}

#[test]
fn origin_and_membership_survive_a_save_and_reopen() {
    let (mut map, id) = map_with_landmass(square(10.0, 10.0, 20.0));
    resize(&mut map, [-5.5, 7.0], 30, 30);
    map.execute(&Command::SetMembership {
        ids: vec![id],
        membership: Membership::OffCanvas,
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ssmap");
    MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.view(), map.view());
}

#[test]
fn old_files_without_origin_or_membership_load_with_defaults() {
    let (map, id) = map_with_landmass(square(10.0, 10.0, 20.0));
    let json = serde_json::to_string(&map.view()).unwrap();
    assert!(!json.contains("origin") && !json.contains("membership"));
    let back = Map::from_view(serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(back.canvas.origin, [0.0, 0.0]);
    assert_eq!(membership(&back, id), Membership::Canvas);
}
