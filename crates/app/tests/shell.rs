//! The window shell: collapsing Layers and Properties, hiding both with Tab, divider drags and
//! the status bar (no window needed).

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ssmap_app::MapMakerApp;
use ssmap_app::shell::Side;
use ssmap_app::theme::metrics;

fn app() -> Harness<'static, MapMakerApp> {
    app_with_step(0.25)
}

fn app_with_step(step_dt: f32) -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_step_dt(step_dt)
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness
}

const BOTH: egui::Modifiers = egui::Modifiers {
    alt: true,
    command: true,
    ctrl: false,
    shift: false,
    mac_cmd: false,
};

#[test]
fn the_layers_shortcut_collapses_to_the_rail_and_expands_again() {
    let mut harness = app();
    assert!(harness.query_by_label("Collapse Layers (⌘⌥1)").is_some());
    harness.key_press_modifiers(BOTH, egui::Key::Num1);
    harness.run();
    assert!(harness.state().shell().layers_collapsed());
    assert!(harness.query_by_label("Collapse Layers (⌘⌥1)").is_none());
    assert!(harness.query_by_label("Expand Layers (⌘⌥1)").is_some());
    harness.key_press_modifiers(BOTH, egui::Key::Num1);
    harness.run();
    assert!(!harness.state().shell().layers_collapsed());
}

#[test]
fn the_header_button_collapses_layers_and_clicking_the_rail_expands_it() {
    let mut harness = app();
    harness.get_by_label("Collapse Layers (⌘⌥1)").click();
    harness.run();
    assert!(harness.state().shell().layers_collapsed());
    harness.get_by_label("Expand Layers (⌘⌥1)").click();
    harness.run();
    assert!(!harness.state().shell().layers_collapsed());
}

#[test]
fn the_properties_shortcut_hides_and_shows_properties() {
    let mut harness = app();
    assert!(harness.query_by_label("Properties").is_some());
    harness.key_press_modifiers(BOTH, egui::Key::Num2);
    harness.run();
    assert!(!harness.state().shell().properties_shown());
    assert!(harness.query_by_label("Properties").is_none());
    harness.key_press_modifiers(BOTH, egui::Key::Num2);
    harness.run();
    assert!(harness.state().shell().properties_shown());
    assert!(harness.query_by_label("Properties").is_some());
}

#[test]
fn tab_hides_both_panels_and_brings_back_what_was_collapsed() {
    let mut harness = app();
    harness.key_press_modifiers(BOTH, egui::Key::Num1);
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(harness.state().shell().panels_hidden());
    assert!(harness.query_by_label("Properties").is_none());
    assert!(harness.query_by_label("Expand Layers (⌘⌥1)").is_none());
    harness.key_press(egui::Key::Tab);
    harness.run();
    // Layers is back as the rail it was collapsed to, not as the full panel.
    assert!(harness.state().shell().layers_shown());
    assert!(harness.state().shell().layers_collapsed());
    assert!(harness.query_by_label("Properties").is_some());
}

#[test]
fn dragging_a_divider_resizes_within_its_range() {
    let mut harness = app();
    let y = 400.0;
    // The Layers column starts at the window's left edge.
    drag(&mut harness, metrics::LAYERS_PANEL_DEFAULT, 5000.0, y);
    assert_eq!(
        harness.state().shell().width(Side::Left),
        metrics::LAYERS_PANEL_MAX
    );
    drag(&mut harness, metrics::LAYERS_PANEL_MAX, 0.0, y);
    assert_eq!(
        harness.state().shell().width(Side::Left),
        metrics::LAYERS_PANEL_MIN
    );
    // Properties is anchored to the right edge of the 1280 pt window.
    let edge = 1280.0 - metrics::PROPERTIES_PANEL_DEFAULT;
    drag(&mut harness, edge, 0.0, y);
    assert_eq!(
        harness.state().shell().width(Side::Right),
        metrics::PROPERTIES_PANEL_MAX
    );
}

#[test]
fn double_clicking_a_divider_resets_its_width() {
    // Frames a few milliseconds apart, so two clicks make a double click.
    let mut harness = app_with_step(0.01);
    let y = 400.0;
    drag(&mut harness, metrics::LAYERS_PANEL_DEFAULT, 300.0, y);
    let width = harness.state().shell().width(Side::Left);
    assert_ne!(width, metrics::LAYERS_PANEL_DEFAULT);
    let at = egui::pos2(width, y);
    harness.hover_at(at);
    harness.run();
    for _ in 0..2 {
        harness.drag_at(at);
        harness.step();
        harness.drop_at(at);
        harness.step();
    }
    harness.run();
    assert_eq!(
        harness.state().shell().width(Side::Left),
        metrics::LAYERS_PANEL_DEFAULT
    );
}

/// Presses at `from` on the row `y`, moves to `to` over a few frames and releases.
fn drag(harness: &mut Harness<'_, MapMakerApp>, from: f32, to: f32, y: f32) {
    let (from, to) = (egui::pos2(from, y), egui::pos2(to, y));
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

#[test]
fn the_status_bar_shows_the_cursor_in_pixels_and_real_world_units() {
    let mut harness = app();
    harness.hover_at(egui::pos2(640.0, 400.0));
    harness.run();
    // The starter Map is 4096 × 3072 px at 0.25 mi per px.
    let node = harness.get_by_label_contains(" px · ");
    let label = node.accesskit_node().value().unwrap();
    assert!(label.starts_with("X ") && label.ends_with(" mi"), "{label}");
}

#[test]
fn the_status_bar_summarises_the_selection() {
    let mut harness = app();
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::N,
    );
    harness.run();
    assert!(harness.query_by_label_contains("Layer · ").is_some());
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.query_by_label_contains("Active Layer: ").is_some());
}
