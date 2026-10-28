//! Art boards: the board Ops each restore the Map with their inverse, the Commands are one Undo
//! step, and Objects keep their positions and membership through every board edit.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use ssmap_core::file::MapFile;
use ssmap_core::{
    Background, Board, Canvas, Command, CommandError, DistanceUnit, History, Id, Map, Membership,
    MultiPolygon, Node, Op, OpError, Polygon, Scale, Selection,
};

fn square(x: f64, y: f64, size: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![[x, y], [x + size, y], [x + size, y + size], [x, y + size]],
        holes: vec![],
    }])
}

fn new_map() -> Map {
    Map::new(Canvas::new(100, 80, Scale::new(1.0, DistanceUnit::Miles)))
}

fn board(id: Id, name: Option<&str>, x: f64) -> Board {
    let mut canvas = Canvas::new(50, 40, Scale::new(1.0, DistanceUnit::Miles));
    canvas.origin = [x, 0.0];
    Board {
        id,
        name: name.map(str::to_owned),
        canvas,
    }
}

fn create_board(map: &mut Map, x: f64) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateBoard {
        id,
        name: None,
        origin: [x, 0.0],
        width: 50,
        height: 40,
        index: None,
    })
    .unwrap();
    id
}

fn add_landmass(map: &mut Map) -> Id {
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
        geometry: square(10.0, 10.0, 20.0),
        fill: None,
    })
    .unwrap();
    id
}

fn membership(map: &Map, id: Id) -> Membership {
    match map.node(id) {
        Some(Node::Object(object)) => object.membership,
        _ => panic!("not an Object"),
    }
}

/// Applies `op` and then its inverse, and checks that the Map is as it was.
fn round_trip(map: &mut Map, op: Op) {
    let before = map.view();
    let inverse = map.apply(op).unwrap();
    assert_ne!(map.view(), before, "the Op changed nothing");
    map.apply(inverse).unwrap();
    assert_eq!(map.view(), before);
}

#[test]
fn a_new_map_has_one_board_which_is_the_canvas() {
    let map = new_map();
    assert_eq!(map.boards().len(), 1);
    assert_eq!(map.boards()[0].id, map.primary_board());
    assert_eq!(map.canvas().width, 100);
    assert_eq!(map.board_label(map.primary_board()).unwrap(), "Canvas");
}

#[test]
fn insert_board_and_its_inverse() {
    let mut map = new_map();
    round_trip(
        &mut map,
        Op::InsertBoard {
            index: 0,
            board: board(Id::new(), Some("Dungeon"), 200.0),
        },
    );
}

#[test]
fn remove_board_and_its_inverse() {
    let mut map = new_map();
    let first = create_board(&mut map, 200.0);
    create_board(&mut map, 400.0);
    round_trip(&mut map, Op::RemoveBoard { id: first });
}

#[test]
fn set_board_rect_and_its_inverse() {
    let mut map = new_map();
    let id = create_board(&mut map, 200.0);
    round_trip(
        &mut map,
        Op::SetBoardRect {
            id,
            origin: [-10.0, 5.0],
            width: 70,
            height: 30,
        },
    );
}

#[test]
fn set_board_name_and_its_inverse() {
    let mut map = new_map();
    let id = create_board(&mut map, 200.0);
    round_trip(
        &mut map,
        Op::SetBoardName {
            id,
            name: Some("Harbour".into()),
        },
    );
}

fn teal() -> Background {
    Background {
        water: ssmap_core::Color::rgb(0, 128, 128),
        ..Background::default()
    }
}

#[test]
fn set_board_background_and_its_inverse() {
    let mut map = new_map();
    let id = create_board(&mut map, 200.0);
    round_trip(
        &mut map,
        Op::SetBoardBackground {
            id,
            background: teal(),
        },
    );
    // The primary board is a board too.
    let primary = map.primary_board();
    round_trip(
        &mut map,
        Op::SetBoardBackground {
            id: primary,
            background: teal(),
        },
    );
}

#[test]
fn the_board_background_command_changes_one_board_in_one_undo_step() {
    let mut map = new_map();
    let id = create_board(&mut map, 200.0);
    let canvas_before = map.canvas().background.clone();
    let before = map.view();
    let undo = map
        .execute(&Command::SetBoardBackground {
            id,
            background: teal(),
        })
        .unwrap();
    assert_eq!(map.board(id).unwrap().canvas.background, teal());
    assert_eq!(
        map.canvas().background,
        canvas_before,
        "the Canvas is not touched"
    );
    assert_eq!(
        Command::SetBoardBackground {
            id,
            background: teal()
        }
        .label(),
        "Change Board Background"
    );
    map.revert(undo);
    assert_eq!(map.view(), before);

    let mut bad = teal();
    bad.texture.strength = 2.0;
    for command in [
        Command::SetBoardBackground {
            id,
            background: bad,
        },
        Command::SetBoardBackground {
            id: Id::new(),
            background: teal(),
        },
    ] {
        assert!(map.execute(&command).is_err(), "{command:?}");
    }
    assert_eq!(map.view(), before);
}

#[test]
fn move_board_and_its_inverse() {
    let mut map = new_map();
    let first = create_board(&mut map, 200.0);
    create_board(&mut map, 400.0);
    round_trip(
        &mut map,
        Op::MoveBoard {
            id: first,
            index: 2,
        },
    );
}

#[test]
fn set_primary_and_its_inverse() {
    let mut map = new_map();
    let id = create_board(&mut map, 200.0);
    round_trip(&mut map, Op::SetPrimary { id });
}

#[test]
fn board_ops_check_their_targets() {
    let mut map = new_map();
    let missing = Id::new();
    let primary = map.primary_board();
    let before = map.view();
    assert_eq!(
        map.apply(Op::RemoveBoard { id: missing }),
        Err(OpError::NoSuchBoard(missing))
    );
    assert_eq!(
        map.apply(Op::RemoveBoard { id: primary }),
        Err(OpError::RemovePrimaryBoard(primary))
    );
    assert_eq!(
        map.apply(Op::SetPrimary { id: missing }),
        Err(OpError::NoSuchBoard(missing))
    );
    assert!(
        map.apply(Op::InsertBoard {
            index: 0,
            board: board(primary, None, 0.0)
        })
        .is_err()
    );
    assert!(
        map.apply(Op::MoveBoard {
            id: primary,
            index: 1
        })
        .is_err()
    );
    assert_eq!(map.view(), before);
}

#[test]
fn every_board_command_is_one_undo_step() {
    let mut map = new_map();
    let land = add_landmass(&mut map);
    let mut history = History::default();
    let board_id = Id::new();
    let commands = [
        Command::CreateBoard {
            id: board_id,
            name: None,
            origin: [200.0, 0.0],
            width: 50,
            height: 40,
            index: None,
        },
        Command::SetMembership {
            ids: vec![land],
            membership: Membership::Board(board_id),
        },
        Command::ResizeBoard {
            id: board_id,
            origin: [210.0, 5.0],
            width: 60,
            height: 45,
        },
        Command::RenameBoard {
            id: board_id,
            name: Some("Harbour".into()),
        },
        Command::ReorderBoard {
            id: board_id,
            index: 0,
        },
        Command::SetPrimaryBoard { id: board_id },
        Command::DeleteBoard {
            id: map.primary_board(),
        },
    ];
    let mut views = vec![map.view()];
    for command in &commands {
        history
            .execute_step(&mut map, command, Selection::default())
            .unwrap();
        views.push(map.view());
    }
    for expected in views.iter().rev().skip(1) {
        history.undo(&mut map).unwrap();
        assert_eq!(&map.view(), expected);
    }
    for expected in views.iter().skip(1) {
        history.redo(&mut map).unwrap();
        assert_eq!(&map.view(), expected);
    }
}

#[test]
fn deleting_a_board_sets_its_objects_off_board_and_keeps_them() {
    let mut map = new_map();
    let land = add_landmass(&mut map);
    let id = create_board(&mut map, 200.0);
    map.execute(&Command::SetMembership {
        ids: vec![land],
        membership: Membership::Board(id),
    })
    .unwrap();
    let outline = map.view().layers;
    map.execute(&Command::DeleteBoard { id }).unwrap();
    assert!(map.board(id).is_none());
    assert_eq!(membership(&map, land), Membership::OffCanvas);
    assert!(map.contains(land));
    assert_eq!(map.view().layers.len(), outline.len());
}

#[test]
fn deleting_the_primary_board_is_refused() {
    let mut map = new_map();
    create_board(&mut map, 200.0);
    let before = map.view();
    let primary = map.primary_board();
    assert!(matches!(
        map.execute(&Command::DeleteBoard { id: primary }),
        Err(CommandError::DeletePrimaryBoard(_))
    ));
    assert_eq!(map.view(), before);
}

#[test]
fn a_new_primary_swaps_roles_and_objects_stay_on_their_boards() {
    let mut map = new_map();
    let on_canvas = add_landmass(&mut map);
    let on_board = add_landmass(&mut map);
    let off = add_landmass(&mut map);
    let old = map.primary_board();
    let id = create_board(&mut map, 200.0);
    map.execute(&Command::SetMembership {
        ids: vec![on_board],
        membership: Membership::Board(id),
    })
    .unwrap();
    map.execute(&Command::SetMembership {
        ids: vec![off],
        membership: Membership::OffCanvas,
    })
    .unwrap();

    map.execute(&Command::SetPrimaryBoard { id }).unwrap();

    assert_eq!(map.primary_board(), id);
    assert_eq!(map.canvas().origin, [200.0, 0.0]);
    assert_eq!(map.board_label(id).unwrap(), "Canvas");
    assert_eq!(map.board_label(old).unwrap(), "Art board 1");
    assert_eq!(membership(&map, on_board), Membership::Canvas);
    assert_eq!(membership(&map, on_canvas), Membership::Board(old));
    assert_eq!(membership(&map, off), Membership::OffCanvas);
}

#[test]
fn resizing_or_moving_a_board_never_changes_objects() {
    let mut map = new_map();
    let land = add_landmass(&mut map);
    let id = create_board(&mut map, 200.0);
    map.execute(&Command::SetMembership {
        ids: vec![land],
        membership: Membership::Board(id),
    })
    .unwrap();
    let before = map.view().layers;
    map.execute(&Command::ResizeBoard {
        id,
        origin: [-500.0, -500.0],
        width: 5,
        height: 5,
    })
    .unwrap();
    assert_eq!(map.view().layers, before);
    assert_eq!(membership(&map, land), Membership::Board(id));
}

#[test]
fn board_commands_reject_bad_input() {
    let mut map = new_map();
    let id = create_board(&mut map, 200.0);
    let missing = Id::new();
    let before = map.view();
    let bad = [
        Command::ResizeBoard {
            id,
            origin: [0.0, 0.0],
            width: 0,
            height: 10,
        },
        Command::ResizeBoard {
            id,
            origin: [f64::NAN, 0.0],
            width: 10,
            height: 10,
        },
        Command::ResizeBoard {
            id: missing,
            origin: [0.0, 0.0],
            width: 10,
            height: 10,
        },
        Command::CreateBoard {
            id,
            name: None,
            origin: [0.0, 0.0],
            width: 10,
            height: 10,
            index: None,
        },
        Command::RenameBoard {
            id: missing,
            name: None,
        },
        Command::DeleteBoard { id: missing },
        Command::SetPrimaryBoard { id: missing },
        Command::ReorderBoard {
            id: missing,
            index: 0,
        },
    ];
    for command in &bad {
        assert!(map.execute(command).is_err(), "{command:?}");
    }
    assert_eq!(map.view(), before);
}

#[test]
fn names_default_by_position_and_may_repeat() {
    let mut map = new_map();
    let a = create_board(&mut map, 200.0);
    let b = create_board(&mut map, 400.0);
    assert_eq!(map.board_label(a).unwrap(), "Art board 1");
    assert_eq!(map.board_label(b).unwrap(), "Art board 2");
    for id in [a, b] {
        map.execute(&Command::RenameBoard {
            id,
            name: Some("Same".into()),
        })
        .unwrap();
    }
    assert_eq!(map.board_label(a), map.board_label(b));
    map.execute(&Command::RenameBoard { id: a, name: None })
        .unwrap();
    assert_eq!(map.board_label(a).unwrap(), "Art board 1");
}

#[test]
fn boards_and_the_primary_survive_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("boards.ssmap");
    let mut map = new_map();
    let land = add_landmass(&mut map);
    let id = create_board(&mut map, 200.0);
    map.execute(&Command::SetMembership {
        ids: vec![land],
        membership: Membership::Board(id),
    })
    .unwrap();
    map.execute(&Command::RenameBoard {
        id,
        name: Some("Harbour".into()),
    })
    .unwrap();
    map.execute(&Command::SetPrimaryBoard { id }).unwrap();
    let (mut file, _) = MapFile::create(&path, &map, None).unwrap();
    file.save(&map, None).unwrap();
    drop(file);

    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.view(), map.view());
    assert_eq!(reopened.primary_board(), id);
    assert_eq!(membership(&reopened, land), Membership::Canvas);
}

#[test]
fn a_file_saved_before_boards_opens_as_one_board_and_saves_in_the_new_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.ssmap");
    let mut map = new_map();
    let land = add_landmass(&mut map);
    let (file, _) = MapFile::create(&path, &map, None).unwrap();
    drop(file);
    // What a file from before format 1.10 looks like: no `boards` key, an old version.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DELETE FROM meta WHERE key = 'boards';
         UPDATE meta SET value = '1.9' WHERE key = 'format_version';",
    )
    .unwrap();
    drop(conn);

    let (mut file, mut opened) = MapFile::open(&path).unwrap();
    assert_eq!(opened.boards().len(), 1);
    assert_eq!(opened.boards()[0].id, opened.primary_board());
    assert_eq!(opened.canvas(), map.canvas());
    assert_eq!(membership(&opened, land), Membership::Canvas);

    // Saving writes the new shape, even with nothing changed.
    let id = Id::new();
    opened
        .execute(&Command::CreateBoard {
            id,
            name: None,
            origin: [200.0, 0.0],
            width: 50,
            height: 40,
            index: None,
        })
        .unwrap();
    file.save(&opened, None).unwrap();
    drop(file);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let json: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'boards'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(json.contains("\"primary\""));
    let (_, again) = MapFile::open(&path).unwrap();
    assert_eq!(again.boards().len(), 2);
}

#[test]
fn a_file_saved_before_boards_that_is_not_changed_still_gets_the_new_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.ssmap");
    let map = new_map();
    let (file, _) = MapFile::create(&path, &map, None).unwrap();
    drop(file);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("DELETE FROM meta WHERE key = 'boards'", [])
        .unwrap();
    drop(conn);

    let (mut file, opened) = MapFile::open(&path).unwrap();
    file.save(&opened, None).unwrap();
    drop(file);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM meta WHERE key = 'boards'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
