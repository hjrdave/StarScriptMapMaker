//! The Asset picker is a modal over the window: ⌘⌥4, the Assets menu item and the Stamp tool's
//! Asset card open it, picking an Asset arms the Stamp tool and closes it, and Esc, Close or a
//! click outside close it without picking. The left sidebar has no Assets section.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::shell::SidebarSection;
use ssmap_app::tools::ToolId;
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

fn open_with_the_shortcut(harness: &mut Harness<'static, MapMakerApp>) {
    harness.key_press_modifiers(BOTH, egui::Key::Num4);
    harness.run();
    assert!(harness.state().asset_picker_open());
}

#[test]
fn the_icon_strip_lists_layers_art_boards_and_history_only() {
    assert_eq!(
        SidebarSection::ALL,
        [
            SidebarSection::Layers,
            SidebarSection::Boards,
            SidebarSection::History
        ]
    );
}

#[test]
fn a_layout_saved_with_the_assets_section_loads_as_layers() {
    let saved: SidebarSection = serde_json::from_str("\"Assets\"").unwrap();
    assert_eq!(saved, SidebarSection::Layers);
}

#[test]
fn the_shortcut_opens_the_picker_and_leaves_the_sidebar_as_it_was() {
    let mut harness = app();
    assert!(!harness.state().asset_picker_open());
    let section = harness.state().shell().sidebar_section();
    open_with_the_shortcut(&mut harness);
    harness.get_by_label("Choose an Asset");
    harness.get_by_label("Close");
    assert_eq!(harness.state().shell().sidebar_section(), section);
}

#[test]
fn the_stamp_tools_asset_card_opens_the_picker() {
    let mut harness = app();
    harness.state_mut().active_editor_mut().unwrap().active_tool = ToolId::Stamp;
    harness.run();
    assert!(!harness.state().asset_picker_open());
    harness.get_by_label("Choose Asset").click();
    harness.run();
    assert!(harness.state().asset_picker_open());
}

#[test]
fn picking_an_asset_arms_the_stamp_tool_and_closes_the_picker() {
    let mut harness = app();
    open_with_the_shortcut(&mut harness);
    assert!(harness.state().editor().selected_asset.is_none());
    // A thumbnail is named "Name (pack/asset)".
    harness
        .get_all_by_label_contains("/")
        .next()
        .expect("a thumbnail")
        .click();
    harness.run();
    assert!(!harness.state().asset_picker_open());
    let editor = harness.state().editor();
    assert!(editor.selected_asset.is_some());
    assert_eq!(editor.active_tool, ToolId::Stamp);
}

#[test]
fn escape_and_close_leave_the_picker_without_picking() {
    let mut harness = app();
    open_with_the_shortcut(&mut harness);
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().asset_picker_open());

    open_with_the_shortcut(&mut harness);
    harness.get_by_label("Close").click();
    harness.run();
    assert!(!harness.state().asset_picker_open());
    assert!(harness.state().editor().selected_asset.is_none());
}
