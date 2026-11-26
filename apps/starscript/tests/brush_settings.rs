//! Brush settings are saved with the Map (ADR 0020): a change marks the Tab unsaved, makes no
//! Undo step, and comes back when the file is opened again. Each Tab has its own.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::tools::ToolId;
use ssmap_core::{BrushSettings, Canvas, DistanceUnit, Scale};

fn canvas() -> Canvas {
    Canvas::new(1024, 768, Scale::new(0.25, DistanceUnit::Miles))
}

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_step_dt(0.25)
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness.state_mut().new_map(canvas());
    harness.run();
    harness
}

fn pick(harness: &mut Harness<'static, MapMakerApp>, tool: ToolId) {
    harness
        .get_by_label(&format!("{} ({})", tool.label(), tool.shortcut_text()))
        .click();
    harness.run();
}

fn unsaved(harness: &Harness<'static, MapMakerApp>) -> bool {
    let app = harness.state();
    app.tabs()[app.active_tab()].has_unsaved_changes()
}

#[test]
fn a_changed_brush_setting_marks_the_tab_unsaved_and_comes_back_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("brushes.ssmap");
    let mut harness = app();
    assert_eq!(
        harness.state().editor().map().brush_settings,
        BrushSettings::default()
    );
    assert!(harness.state_mut().save_as_path(&path));
    harness.run();
    assert!(!unsaved(&harness));

    pick(&mut harness, ToolId::LandBrush);
    let start = harness.state().editor().land_settings(false).size;
    harness.get_by_label("Increase Size").click();
    harness.run();
    let editor = harness.state().editor();
    assert_eq!(editor.land_settings(false).size, start + 1.0);
    // Not undoable: the Undo menu has nothing new.
    assert_eq!(editor.undo_label(), None);
    assert!(unsaved(&harness));

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    harness.run();
    assert!(!unsaved(&harness));

    // Opened again, the Land Brush's Paint size is the one saved; Erase kept its own.
    let index = harness.state().active_tab();
    harness.state_mut().close_tab(index);
    harness.run();
    assert!(harness.state_mut().open_path(&path));
    harness.run();
    let editor = harness.state().editor();
    assert_eq!(editor.land_settings(false).size, start + 1.0);
    assert_eq!(editor.land_settings(true).size, start);
    assert!(!unsaved(&harness));
}

#[test]
fn each_tab_keeps_its_own_brush_settings() {
    let mut harness = app();
    pick(&mut harness, ToolId::Paint);
    harness.get_by_label("Increase Size").click();
    harness.run();
    let changed = harness.state().editor().paint_settings(false).size;

    harness.state_mut().new_map(canvas());
    harness.run();
    assert_eq!(
        harness.state().editor().map().brush_settings,
        BrushSettings::default()
    );
    harness.state_mut().focus_tab(0);
    harness.run();
    assert_eq!(harness.state().editor().paint_settings(false).size, changed);
}
