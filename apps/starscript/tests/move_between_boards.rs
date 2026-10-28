//! Dragging Objects between boards: the board under the centre of what is dragged becomes
//! theirs, the gap between boards makes them off-board, and one drag is one Undo step.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use eframe::egui;
use egui_kittest::Harness;
use ssmap_app::MapMakerApp;
use ssmap_app::menu::Command as MenuCommand;
use ssmap_core::{
    Canvas, Command, DistanceUnit, Id, Map, Membership, MultiPolygon, Node, Polygon, Scale,
};

fn square(min: f64, size: f64) -> Vec<[f64; 2]> {
    vec![
        [min, min],
        [min + size, min],
        [min + size, min + size],
        [min, min + size],
    ]
}

/// A 1000 square Canvas with a 400 square island on it, and an Art board of the same size
/// 200 units to its right. Returns the Map, the island and the Art board.
fn map() -> (Map, Id, Id) {
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let (layer, island, board) = (Id::new(), Id::new(), Id::new());
    for command in [
        Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        },
        Command::CreateLandmass {
            id: island,
            parent: layer,
            index: None,
            geometry: MultiPolygon(vec![Polygon {
                exterior: square(0.0, 400.0),
                holes: Vec::new(),
            }]),
            fill: None,
        },
        Command::CreateBoard {
            id: board,
            name: None,
            origin: [1200.0, 0.0],
            width: 1000,
            height: 1000,
            index: None,
        },
    ] {
        map.execute(&command).unwrap();
    }
    (map, island, board)
}

fn membership(harness: &Harness<'_, MapMakerApp>, id: Id) -> Membership {
    match harness.state().editor().map().node(id) {
        Some(Node::Object(object)) => object.membership,
        _ => panic!("an Object"),
    }
}

fn drag(harness: &mut Harness<'_, MapMakerApp>, from: [f64; 2], to: [f64; 2]) {
    let screen = |harness: &Harness<'_, MapMakerApp>, point| {
        harness.state().canvas_to_screen(point).unwrap()
    };
    let (from, to) = (screen(harness, from), screen(harness, to));
    harness.hover_at(from);
    harness.run();
    harness.drag_at(from);
    harness.run();
    for step in 1..=4 {
        harness.hover_at(from + (to - from) * (step as f32 / 4.0));
        harness.run();
    }
    harness.drop_at(to);
    harness.run();
}

fn app() -> (Harness<'static, MapMakerApp>, Id, Id) {
    let (map, island, board) = map();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_map(cc, map, None));
    harness.run();
    harness.state_mut().run_command(MenuCommand::ZoomAll);
    harness.run();
    // Select the island.
    let at = harness.state().canvas_to_screen([50.0, 300.0]).unwrap();
    harness.hover_at(at);
    harness.run();
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run();
    assert_eq!(harness.state().editor().selection(), [island]);
    (harness, island, board)
}

#[test]
fn dragging_an_object_onto_an_art_board_makes_it_a_member_in_one_undo_step() {
    let (mut harness, island, board) = app();
    assert_eq!(membership(&harness, island), Membership::Canvas);
    // Its centre (200, 200) lands at (1650, 200).
    drag(&mut harness, [50.0, 300.0], [1500.0, 300.0]);
    assert_eq!(membership(&harness, island), Membership::Board(board));
    assert_eq!(harness.state().editor().undo_label(), Some("Move"));
    harness.state_mut().active_editor_mut().unwrap().undo();
    harness.run();
    assert_eq!(membership(&harness, island), Membership::Canvas);
}

#[test]
fn dropping_it_between_the_boards_sets_it_off_the_boards_and_back_on_the_canvas() {
    let (mut harness, island, board) = app();
    drag(&mut harness, [50.0, 300.0], [1500.0, 300.0]);
    assert_eq!(membership(&harness, island), Membership::Board(board));
    // Centre to (1150, 200): in the gap.
    drag(&mut harness, [1500.0, 300.0], [1000.0, 300.0]);
    assert_eq!(membership(&harness, island), Membership::OffCanvas);
    // Centre to (500, 200): on the Canvas again.
    drag(&mut harness, [1000.0, 300.0], [350.0, 300.0]);
    assert_eq!(membership(&harness, island), Membership::Canvas);
}

#[test]
fn resizing_a_board_over_an_object_does_not_adopt_it() {
    let (mut harness, island, _) = app();
    let primary = harness.state().editor().map().primary_board();
    harness
        .state_mut()
        .active_editor_mut()
        .unwrap()
        .run(Command::ResizeBoard {
            id: primary,
            origin: [-100.0, -100.0],
            width: 3000,
            height: 3000,
        })
        .unwrap();
    harness.run();
    assert_eq!(membership(&harness, island), Membership::Canvas);
}
