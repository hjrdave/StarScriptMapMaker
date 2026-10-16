//! The floating tool dock: clicking selects a tool, S is Stamp (C selects nothing), and
//! the dock stays inside the Canvas when Layers collapses.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::tools::ToolId;
use ssmap_core::{Canvas, DistanceUnit, Scale};

const WIDTH: f32 = 1280.0;

fn app(size: egui::Vec2) -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_step_dt(0.25)
        .with_size(size)
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

fn button_label(tool: ToolId) -> String {
    format!("{} ({})", tool.label(), tool.shortcut_text())
}

/// The dock's extent: the left edge of the first button and the right edge of the last.
fn dock_span(harness: &Harness<'static, MapMakerApp>) -> (f32, f32) {
    let left = harness
        .get_by_label(&button_label(ToolId::Select))
        .rect()
        .min
        .x;
    let right = harness
        .get_by_label(&button_label(ToolId::ExportRegion))
        .rect()
        .max
        .x;
    (left, right)
}

#[test]
fn clicking_a_dock_button_sets_the_active_tool() {
    let mut harness = app(egui::vec2(WIDTH, 800.0));
    for tool in ToolId::ALL {
        harness.get_by_label(&button_label(tool)).click();
        harness.run();
        assert_eq!(harness.state().editor().active_tool, tool);
    }
}

#[test]
fn s_selects_stamp_and_c_does_not_select_a_tool() {
    let mut harness = app(egui::vec2(WIDTH, 800.0));
    harness.key_press(egui::Key::V);
    harness.run();
    assert_eq!(harness.state().editor().active_tool, ToolId::Select);
    harness.key_press(egui::Key::C);
    harness.run();
    assert_eq!(harness.state().editor().active_tool, ToolId::Select);
    harness.key_press(egui::Key::S);
    harness.run();
    assert_eq!(harness.state().editor().active_tool, ToolId::Stamp);
}

#[test]
fn all_ten_tools_show_at_the_minimum_window_size() {
    let harness = app(egui::vec2(1024.0, 640.0));
    for tool in ToolId::ALL {
        assert!(harness.query_by_label(&button_label(tool)).is_some());
    }
}

#[test]
fn the_dock_stays_inside_the_canvas_and_recentres_when_layers_collapses() {
    let mut harness = app(egui::vec2(WIDTH, 800.0));
    let layers = harness.state().shell().current_layout().layers.width();
    let properties = harness.state().shell().width(ssmap_app::shell::Side::Right);
    let (left, right) = dock_span(&harness);
    assert!(left >= layers && right <= WIDTH - properties);
    let before = (left + right) / 2.0;

    harness.get_by_label("Collapse Layers (⌘⌥1)").click();
    harness.run();
    let layers = harness.state().shell().current_layout().layers.width();
    assert!(layers < 100.0);
    let (left, right) = dock_span(&harness);
    assert!(left >= layers && right <= WIDTH - properties);
    assert!(((left + right) / 2.0 - before).abs() > 1.0);
}
