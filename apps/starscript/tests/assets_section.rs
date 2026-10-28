//! The Assets section of the left sidebar: ⌘⌥4 shows it, it holds the Asset picker, and the
//! Stamp tool's section in Properties points to it.

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

#[test]
fn the_icon_strip_lists_layers_boards_assets_and_history_in_that_order() {
    assert_eq!(
        SidebarSection::ALL,
        [
            SidebarSection::Layers,
            SidebarSection::Boards,
            SidebarSection::Assets,
            SidebarSection::History
        ]
    );
}

#[test]
fn the_shortcut_shows_the_assets_section_and_only_that() {
    let mut harness = app();
    harness.key_press_modifiers(BOTH, egui::Key::Num4);
    harness.run();
    assert_eq!(
        harness.state().shell().sidebar_section(),
        SidebarSection::Assets
    );
    assert!(harness.query_by_label("New Layer (⇧⌘N)").is_none());
    // With the built-in packs installed it shows the search field, not the empty state.
    assert!(harness.query_by_label("Open Asset packs").is_none());
}

#[test]
fn the_stamp_section_opens_the_assets_section() {
    let mut harness = app();
    harness.state_mut().active_editor_mut().unwrap().active_tool = ssmap_app::tools::ToolId::Stamp;
    harness.run();
    assert_eq!(
        harness.state().shell().sidebar_section(),
        SidebarSection::Layers
    );
    harness.get_by_label("Choose Asset…").click();
    harness.run();
    assert_eq!(
        harness.state().shell().sidebar_section(),
        SidebarSection::Assets
    );
}
