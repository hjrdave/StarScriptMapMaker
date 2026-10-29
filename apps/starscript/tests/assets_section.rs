//! The Asset picker dialog: ⌘⌥4, the Assets menu item and the Stamp section open it, picking
//! arms the Stamp tool and closes it, and Esc closes it without picking. The sidebar's icon
//! rail has no Assets entry.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
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

#[test]
fn the_icon_strip_lists_layers_boards_and_history_in_that_order() {
    assert_eq!(
        SidebarSection::ALL,
        [
            SidebarSection::Layers,
            SidebarSection::Boards,
            SidebarSection::History
        ]
    );
    let harness = app();
    assert!(harness.query_by_label("Assets (⌘⌥4)").is_none());
}

#[test]
fn the_shortcut_opens_the_picker_and_leaves_the_sidebar_alone() {
    let mut harness = app();
    assert!(!harness.state().assets_open());
    harness.key_press_modifiers(BOTH, egui::Key::Num4);
    harness.run();
    assert!(harness.state().assets_open());
    assert_eq!(
        harness.state().shell().sidebar_section(),
        SidebarSection::Layers
    );
    // With the built-in packs installed it shows the search field, not the empty state.
    assert!(harness.query_by_label("Open Asset packs").is_none());
}

#[test]
fn the_stamp_section_opens_the_picker() {
    let mut harness = app();
    harness.state_mut().active_editor_mut().unwrap().active_tool = ToolId::Stamp;
    harness.run();
    assert!(!harness.state().assets_open());
    harness.get_by_label("Choose Asset").click();
    harness.run();
    assert!(harness.state().assets_open());
}

#[test]
fn escape_closes_the_picker_without_picking() {
    let mut harness = app();
    harness.key_press_modifiers(BOTH, egui::Key::Num4);
    harness.run();
    assert!(harness.state().assets_open());
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().assets_open());
    let editor = harness.state_mut().active_editor_mut().unwrap();
    assert_ne!(editor.active_tool, ToolId::Stamp);
}

#[test]
fn picking_an_asset_arms_the_stamp_tool_and_closes_the_picker() {
    let mut harness = app();
    harness.key_press_modifiers(BOTH, egui::Key::Num4);
    harness.run();
    // Thumbnails are labelled "<name> (<pack>/<id>)"; take the first one.
    harness
        .get_all_by_role(egui::accesskit::Role::Button)
        .find(|node| {
            node.accesskit_node()
                .label()
                .is_some_and(|l| l.contains(" (") && l.contains('/'))
        })
        .expect("a thumbnail")
        .click();
    harness.run();
    assert!(!harness.state().assets_open());
    let editor = harness.state_mut().active_editor_mut().unwrap();
    assert_eq!(editor.active_tool, ToolId::Stamp);
    assert!(editor.selected_asset.is_some());
}
