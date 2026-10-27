//! The History section of the left sidebar: it lists the undo history, a click jumps to a
//! step, and ⌘⌥5 shows it.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::shell::SidebarSection;
use ssmap_core::{Canvas, DistanceUnit, Scale};

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_step_dt(0.25)
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

const BOTH: egui::Modifiers = egui::Modifiers {
    alt: true,
    command: true,
    ctrl: false,
    shift: false,
    mac_cmd: false,
};

fn layer_count(harness: &Harness<'_, MapMakerApp>) -> usize {
    harness.state().tabs()[harness.state().active_tab()]
        .editor()
        .map()
        .children(None)
        .len()
}

fn show_history(harness: &mut Harness<'_, MapMakerApp>) {
    harness.key_press_modifiers(BOTH, egui::Key::Num5);
    harness.run();
}

#[test]
fn the_shortcut_shows_the_history_section() {
    let mut harness = app();
    assert_eq!(
        harness.state().shell().sidebar_section(),
        SidebarSection::Layers
    );
    show_history(&mut harness);
    assert_eq!(
        harness.state().shell().sidebar_section(),
        SidebarSection::History
    );
    assert!(harness.query_by_label("Original").is_some());
    // Only one section shows at a time.
    assert!(harness.query_by_label("New Layer (⇧⌘N)").is_none());
}

#[test]
fn the_shortcut_expands_a_collapsed_sidebar() {
    let mut harness = app();
    harness.key_press_modifiers(BOTH, egui::Key::Num1);
    harness.run();
    assert!(harness.state().shell().layers_collapsed());
    show_history(&mut harness);
    assert!(!harness.state().shell().layers_collapsed());
    assert!(harness.query_by_label("Original").is_some());
}

#[test]
fn the_section_lists_the_steps_and_a_click_jumps_to_one() {
    let mut harness = app();
    let before = layer_count(&harness);
    for _ in 0..3 {
        harness.state_mut().active_editor_mut().unwrap().new_layer();
    }
    harness.run();
    show_history(&mut harness);
    let (steps, applied) = harness.state().tabs()[0].editor().history_steps();
    assert_eq!((steps.len(), applied), (3, 3));
    let name = steps[0].clone();
    assert_eq!(harness.query_all_by_label(&name).count(), 3);
    assert_eq!(layer_count(&harness), before + 3);

    // Jump back to the first step; the later two stay available to redo.
    harness.get_all_by_label(&name).next().unwrap().click();
    harness.run();
    assert_eq!(layer_count(&harness), before + 1);
    let editor = harness.state().tabs()[0].editor();
    assert_eq!(editor.history_steps().1, 1);
    assert!(editor.can_redo());

    // And forward again.
    harness.get_all_by_label(&name).last().unwrap().click();
    harness.run();
    assert_eq!(layer_count(&harness), before + 3);

    // Original is the point before every step.
    harness.get_by_label("Original").click();
    harness.run();
    assert_eq!(layer_count(&harness), before);
    assert!(!harness.state().tabs()[0].editor().can_undo());
}
