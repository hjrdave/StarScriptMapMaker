//! Art boards: creating, resizing, renaming, reordering, deleting and promoting them, how
//! Objects follow, and how they are saved. Every edit is undone exactly by its inverses.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    ArtBoard, Background, CANVAS_BOARD_ID, Canvas, Color, Command, CommandError, DistanceUnit, Id,
    InvariantError, Map, Membership, MultiPolygon, Node, Op, Polygon, Scale,
};

fn square(x: f64, y: f64, size: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]],
        holes: vec![],
    }])
}

/// A Map with one Layer holding two Landmasses; returns the Map and the Landmasses.
fn map() -> (Map, Id, Id) {
    let mut map = Map::new(Canvas::new(100, 80, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    let mut land = |x| {
        let id = Id::new();
        map.execute(&Command::CreateLandmass {
            id,
            parent: layer,
            index: None,
            geometry: square(x, 10.0, 20.0),
            fill: None,
        })
        .unwrap();
        id
    };
    let (a, b) = (land(10.0), land(40.0));
    (map, a, b)
}

fn create(map: &mut Map, name: &str, origin: [f64; 2]) -> (Id, Vec<Op>) {
    let id = Id::new();
    let undo = map
        .execute(&Command::CreateBoard {
            id,
            name: name.into(),
            origin,
            width: 50,
            height: 40,
        })
        .unwrap();
    (id, undo)
}

fn membership(map: &Map, id: Id) -> Membership {
    match map.node(id) {
        Some(Node::Object(object)) => object.membership,
        _ => panic!("not an Object"),
    }
}

fn join(map: &mut Map, ids: &[Id], membership: Membership) {
    map.execute(&Command::SetMembership {
        ids: ids.to_vec(),
        membership,
    })
    .unwrap();
}

fn names(map: &Map) -> Vec<&str> {
    map.art_boards().iter().map(|b| b.name.as_str()).collect()
}

#[test]
fn a_new_map_has_only_the_canvas() {
    let (map, _, _) = map();
    assert!(map.art_boards().is_empty());
    assert_eq!(map.canvas().id, CANVAS_BOARD_ID);
    assert_eq!(map.canvas().name, "Canvas");
}

#[test]
fn creating_a_board_adds_it_on_top_with_the_canvas_background_and_undoes() {
    let (mut map, _, _) = map();
    let water = Background {
        water: Color::rgb(1, 2, 3),
        ..Background::default()
    };
    map.execute(&Command::SetBackground {
        background: water.clone(),
    })
    .unwrap();
    let before = map.view();
    let (first, undo_first) = create(&mut map, "", [200.0, 0.0]);
    let (second, _) = create(&mut map, "Staging", [300.0, 0.0]);
    assert_eq!(names(&map), ["Art board 1", "Staging"]);
    assert_eq!(map.art_boards()[0].id, first);
    assert_eq!(map.art_boards()[1].id, second);
    assert_eq!(map.art_boards()[0].background, water);
    assert_eq!(map.art_boards()[0].rect().max, [250.0, 40.0]);

    // Undo the later one by hand, then the first through its inverses.
    map.execute(&Command::DeleteBoard { id: second }).unwrap();
    map.revert(undo_first);
    assert_eq!(map.view(), before);
}

#[test]
fn creating_needs_a_fresh_id_and_a_real_size() {
    let (mut map, a, _) = map();
    let (board, _) = create(&mut map, "B", [0.0, 0.0]);
    for id in [board, a, CANVAS_BOARD_ID] {
        let result = map.execute(&Command::CreateBoard {
            id,
            name: "Again".into(),
            origin: [0.0, 0.0],
            width: 10,
            height: 10,
        });
        assert!(matches!(result, Err(CommandError::IdInUse(_))), "{id}");
    }
    let empty = map.execute(&Command::CreateBoard {
        id: Id::new(),
        name: "Empty".into(),
        origin: [0.0, 0.0],
        width: 0,
        height: 10,
    });
    assert!(matches!(empty, Err(CommandError::InvalidCanvasSize { .. })));
    let far = map.execute(&Command::CreateBoard {
        id: Id::new(),
        name: "Far".into(),
        origin: [f64::NAN, 0.0],
        width: 10,
        height: 10,
    });
    assert!(matches!(far, Err(CommandError::InvalidCanvasOrigin(_))));
    assert_eq!(map.art_boards().len(), 1);
}

#[test]
fn resizing_and_moving_a_board_changes_no_object() {
    let (mut map, a, _) = map();
    let (board, _) = create(&mut map, "B", [200.0, 0.0]);
    join(&mut map, &[a], Membership::Board(board));
    let layers = map.view().layers;
    let before = map.view();
    let undo = map
        .execute(&Command::ResizeBoard {
            id: board,
            origin: [-30.0, -20.0],
            width: 5,
            height: 6,
        })
        .unwrap();
    let resized = map.art_board(board).unwrap();
    assert_eq!(resized.rect().min, [-30.0, -20.0]);
    assert_eq!((resized.width, resized.height), (5, 6));
    assert_eq!(map.view().layers, layers);
    map.revert(undo);
    assert_eq!(map.view(), before);
}

#[test]
fn board_commands_work_on_the_canvas_too() {
    let (mut map, _, _) = map();
    let canvas = map.canvas().id;
    map.execute(&Command::ResizeBoard {
        id: canvas,
        origin: [5.0, 5.0],
        width: 30,
        height: 20,
    })
    .unwrap();
    assert_eq!(map.canvas().rect().min, [5.0, 5.0]);
    assert_eq!((map.canvas().width, map.canvas().height), (30, 20));
    map.execute(&Command::RenameBoard {
        id: canvas,
        name: "Main".into(),
    })
    .unwrap();
    assert_eq!(map.canvas().name, "Main");
    let water = Background {
        water: Color::rgb(9, 9, 9),
        ..Background::default()
    };
    map.execute(&Command::SetBoardBackground {
        id: canvas,
        background: water.clone(),
    })
    .unwrap();
    assert_eq!(map.canvas().background, water);
}

#[test]
fn renaming_and_recolouring_an_art_board_undo_exactly() {
    let (mut map, _, _) = map();
    let (board, _) = create(&mut map, "B", [0.0, 0.0]);
    let before = map.view();
    let rename = map
        .execute(&Command::RenameBoard {
            id: board,
            name: "Sketches".into(),
        })
        .unwrap();
    let water = Background {
        water: Color::rgb(7, 7, 7),
        ..Background::default()
    };
    let recolour = map
        .execute(&Command::SetBoardBackground {
            id: board,
            background: water.clone(),
        })
        .unwrap();
    assert_eq!(map.art_board(board).unwrap().name, "Sketches");
    assert_eq!(map.art_board(board).unwrap().background, water);
    map.revert(recolour);
    map.revert(rename);
    assert_eq!(map.view(), before);
}

#[test]
fn boards_can_be_reordered_but_the_canvas_stays_lowest() {
    let (mut map, _, _) = map();
    let (_, _) = create(&mut map, "One", [0.0, 0.0]);
    let (two, _) = create(&mut map, "Two", [0.0, 0.0]);
    let (_, _) = create(&mut map, "Three", [0.0, 0.0]);
    let before = map.view();
    let undo = map
        .execute(&Command::ReorderBoard { id: two, index: 0 })
        .unwrap();
    assert_eq!(names(&map), ["Two", "One", "Three"]);
    // Past the end means the top.
    let top = map
        .execute(&Command::ReorderBoard { id: two, index: 99 })
        .unwrap();
    assert_eq!(names(&map), ["One", "Three", "Two"]);
    map.revert(top);
    map.revert(undo);
    assert_eq!(map.view(), before);

    let canvas = map.canvas().id;
    assert!(matches!(
        map.execute(&Command::ReorderBoard {
            id: canvas,
            index: 1
        }),
        Err(CommandError::CannotReorderCanvas(_))
    ));
}

#[test]
fn deleting_a_board_keeps_its_objects_and_sets_them_off_the_boards() {
    let (mut map, a, b) = map();
    let (board, _) = create(&mut map, "B", [200.0, 0.0]);
    join(&mut map, &[a], Membership::Board(board));
    let before = map.view();
    let undo = map.execute(&Command::DeleteBoard { id: board }).unwrap();
    assert!(map.art_boards().is_empty());
    assert_eq!(membership(&map, a), Membership::OffCanvas);
    assert_eq!(membership(&map, b), Membership::Canvas);
    assert!(map.contains(a), "no Object is deleted");
    map.revert(undo);
    assert_eq!(map.view(), before);
    assert_eq!(membership(&map, a), Membership::Board(board));
}

#[test]
fn the_canvas_cannot_be_deleted_and_unknown_boards_are_refused() {
    let (mut map, a, _) = map();
    let canvas = map.canvas().id;
    assert!(matches!(
        map.execute(&Command::DeleteBoard { id: canvas }),
        Err(CommandError::CannotDeleteCanvas(_))
    ));
    for command in [
        Command::DeleteBoard { id: Id::new() },
        Command::RenameBoard {
            id: Id::new(),
            name: "x".into(),
        },
        Command::SetPrimaryBoard { id: Id::new() },
        Command::SetMembership {
            ids: vec![a],
            membership: Membership::Board(Id::new()),
        },
    ] {
        assert!(
            matches!(map.execute(&command), Err(CommandError::NoSuchBoard(_))),
            "{command:?}"
        );
    }
}

#[test]
fn the_op_will_not_remove_a_board_that_still_has_objects() {
    let (mut map, a, _) = map();
    let (board, _) = create(&mut map, "B", [200.0, 0.0]);
    join(&mut map, &[a], Membership::Board(board));
    assert!(map.apply(Op::RemoveBoard { id: board }).is_err());
    assert!(map.art_board(board).is_some());
}

#[test]
fn making_a_board_the_canvas_swaps_roles_and_every_object_keeps_its_board() {
    let (mut map, a, b) = map();
    let (board, _) = create(&mut map, "Staging", [200.0, 0.0]);
    join(&mut map, &[a], Membership::Board(board));
    let old = map.canvas().id;
    let before = map.view();
    let undo = map
        .execute(&Command::SetPrimaryBoard { id: board })
        .unwrap();

    // Staging is the Canvas now, and the old Canvas is an Art board in its place.
    assert_eq!(map.canvas().id, board);
    assert_eq!(map.canvas().name, "Staging");
    assert_eq!(map.canvas().rect().min, [200.0, 0.0]);
    assert_eq!(map.art_boards().len(), 1);
    assert_eq!(map.art_boards()[0].id, old);
    assert_eq!(map.art_boards()[0].name, "Canvas");
    assert_eq!(map.art_boards()[0].rect().max, [100.0, 80.0]);
    // `a` was on Staging, which is the Canvas now; `b` was on the old Canvas.
    assert_eq!(membership(&map, a), Membership::Canvas);
    assert_eq!(membership(&map, b), Membership::Board(old));
    map.validate().unwrap();

    map.revert(undo);
    assert_eq!(map.view(), before);
    assert_eq!(membership(&map, a), Membership::Board(board));
    assert_eq!(membership(&map, b), Membership::Canvas);
}

#[test]
fn off_canvas_objects_stay_off_when_the_canvas_changes() {
    let (mut map, a, _) = map();
    let (board, _) = create(&mut map, "B", [200.0, 0.0]);
    join(&mut map, &[a], Membership::OffCanvas);
    map.execute(&Command::SetPrimaryBoard { id: board })
        .unwrap();
    assert_eq!(membership(&map, a), Membership::OffCanvas);
}

#[test]
fn making_the_canvas_the_canvas_changes_nothing() {
    let (mut map, _, _) = map();
    let (_, _) = create(&mut map, "B", [200.0, 0.0]);
    let before = map.view();
    let canvas = map.canvas().id;
    let undo = map
        .execute(&Command::SetPrimaryBoard { id: canvas })
        .unwrap();
    assert!(undo.is_empty());
    assert_eq!(map.view(), before);
}

#[test]
fn pasting_objects_from_a_map_with_other_boards_puts_them_on_the_canvas() {
    let (mut source, a, _) = map();
    let (board, _) = create(&mut source, "B", [200.0, 0.0]);
    join(&mut source, &[a], Membership::Board(board));
    let layer = source.view().layers[0].id;
    let subtree = source.subtree(layer);

    let (mut target, _, _) = map();
    let new_ids: Vec<Id> = (0..3).map(|_| Id::new()).collect();
    target
        .execute(&Command::Paste {
            subtrees: vec![subtree],
            new_ids: new_ids.clone(),
            parent: None,
            index: None,
            offset: [0.0, 0.0],
        })
        .unwrap();
    target.validate().unwrap();
    // The pasted Landmass that was on the other Map's board joined the Canvas.
    let pasted = new_ids[1];
    assert_eq!(membership(&target, pasted), Membership::Canvas);
}

#[test]
fn boards_and_their_members_survive_save_and_reopen() {
    let (mut map, a, _) = map();
    let (board, _) = create(&mut map, "Staging", [200.0, 0.0]);
    join(&mut map, &[a], Membership::Board(board));
    map.execute(&Command::SetPrimaryBoard { id: board })
        .unwrap();
    let (second, _) = create(&mut map, "Sketches", [400.0, 0.0]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("boards.ssmap");
    let (_, _) = MapFile::create(&path, &map, None).unwrap();

    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.view(), map.view());
    assert_eq!(reopened.canvas().id, board);
    assert_eq!(names(&reopened), ["Canvas", "Sketches"]);
    assert_eq!(reopened.art_boards()[1].id, second);
    reopened.validate().unwrap();
}

#[test]
fn a_saved_board_change_is_rewritten_and_removing_all_boards_drops_the_key() {
    let (mut map, _, _) = map();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("boards.ssmap");
    let (mut file, _) = MapFile::create(&path, &map, None).unwrap();
    let (board, _) = create(&mut map, "Staging", [200.0, 0.0]);
    file.save(&map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(names(&reopened), ["Staging"]);

    map.execute(&Command::DeleteBoard { id: board }).unwrap();
    file.save(&map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert!(reopened.art_boards().is_empty());
}

#[test]
fn a_file_with_only_the_canvas_reads_as_one_board_and_writes_the_old_shape() {
    let (map, _, _) = map();
    // The Canvas JSON has no id or name while they are the defaults, so older files and
    // Maps without Art boards look the same.
    let canvas = serde_json::to_value(map.canvas()).unwrap();
    assert!(canvas.get("id").is_none());
    assert!(canvas.get("name").is_none());
    let view = serde_json::to_value(map.view()).unwrap();
    assert!(view.get("art_boards").is_none());

    let back: Canvas = serde_json::from_value(canvas).unwrap();
    assert_eq!(back.id, CANVAS_BOARD_ID);
    assert_eq!(back.name, "Canvas");
}

#[test]
fn a_view_with_a_repeated_or_unknown_board_is_refused() {
    let (map, a, _) = map();
    let mut view = map.view();
    let board = ArtBoard::new(Id::new(), "B", [0.0, 0.0], 10, 10, Background::default());
    view.art_boards = vec![board.clone(), board.clone()];
    assert!(matches!(
        Map::from_view(view.clone()),
        Err(InvariantError::DuplicateBoard(_))
    ));
    view.art_boards = vec![ArtBoard {
        id: view.canvas.id,
        ..board
    }];
    assert!(matches!(
        Map::from_view(view),
        Err(InvariantError::DuplicateBoard(_))
    ));

    // An Object naming a board the Map lacks fails validation.
    let mut map = map;
    let (board, _) = create(&mut map, "B", [0.0, 0.0]);
    join(&mut map, &[a], Membership::Board(board));
    let mut view = map.view();
    view.art_boards.clear();
    let broken = Map::from_view(view).unwrap();
    assert!(matches!(
        broken.validate(),
        Err(InvariantError::UnknownBoard { .. })
    ));
}

#[test]
fn membership_serialises_with_the_board_id() {
    let id = Id::new();
    let json = serde_json::to_value(Membership::Board(id)).unwrap();
    assert_eq!(json, serde_json::json!({ "board": id }));
    let back: Membership = serde_json::from_value(json).unwrap();
    assert_eq!(back, Membership::Board(id));
    let canvas: Membership = serde_json::from_str("\"canvas\"").unwrap();
    assert_eq!(canvas, Membership::Canvas);
    let off: Membership = serde_json::from_str("\"pasteboard\"").unwrap();
    assert_eq!(off, Membership::OffCanvas);
}
