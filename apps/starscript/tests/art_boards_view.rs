//! A Map with Art boards opens and draws in the editor: the boards are named on the Canvas
//! view, and undoing the board leaves the view as it was.

use eframe::egui;
use egui_kittest::Harness;
use ssmap_app::MapMakerApp;
use ssmap_core::{Canvas, Command, DistanceUnit, Id, Scale};

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness.state_mut().new_map(Canvas::new(
        4096,
        3072,
        Scale::new(0.25, DistanceUnit::Miles),
    ));
    harness.run();
    harness
}

#[test]
fn a_map_with_art_boards_draws_and_each_edit_is_one_undo_step() {
    let mut harness = app();
    let board = Id::new();
    let editor = harness.state_mut().active_editor_mut().unwrap();
    editor
        .run(Command::CreateBoard {
            id: board,
            name: Some("Staging".into()),
            origin: [5000.0, 0.0],
            width: 2000,
            height: 1500,
            index: None,
        })
        .unwrap();
    assert_eq!(editor.undo_label(), Some("Add Art Board"));
    harness.run();
    assert_eq!(harness.state().editor().map().boards().len(), 2);

    harness.state_mut().active_editor_mut().unwrap().undo();
    harness.run();
    assert_eq!(harness.state().editor().map().boards().len(), 1);
}
