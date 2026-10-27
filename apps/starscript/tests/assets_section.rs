//! The Assets section of the left sidebar: the Asset picker, ⌘⌥4, picking an Asset switches to
//! the Stamp tool, and the empty state when no packs are installed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::path::PathBuf;

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::shell::SidebarSection;
use ssmap_app::tools::ToolId;
use ssmap_core::asset::PackRoots;
use ssmap_core::{Canvas, DistanceUnit, Scale};

const BOTH: egui::Modifiers = egui::Modifiers {
    alt: true,
    command: true,
    ctrl: false,
    shift: false,
    mac_cmd: false,
};

fn roots(with_pack: bool) -> PackRoots {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    PackRoots {
        built_in: if with_pack {
            manifest.join("../../crates/core/tests/packs")
        } else {
            manifest.join("no-such-packs")
        },
        user: None,
    }
}

/// The app with a Map open and the packs of `roots` installed, showing the Assets section.
fn app(with_pack: bool) -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_step_dt(0.25)
        .with_size(egui::vec2(1440.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness
        .state_mut()
        .new_map(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    harness.state_mut().reload_packs(&roots(with_pack));
    let store = harness.state().sprites().clone();
    harness
        .state_mut()
        .active_editor_mut()
        .unwrap()
        .set_sprites(store);
    harness.run();
    harness.key_press_modifiers(BOTH, egui::Key::Num4);
    harness.run();
    harness
}

fn editor<'a>(harness: &'a Harness<'_, MapMakerApp>) -> &'a ssmap_app::editor::Editor {
    harness.state().tabs()[harness.state().active_tab()].editor()
}

#[test]
fn the_shortcut_and_the_strip_show_the_assets_section() {
    let mut harness = app(true);
    assert_eq!(
        harness.state().shell().sidebar_section(),
        SidebarSection::Assets
    );
    assert!(harness.query_by_label("Tree (test-pack/tree)").is_some());
    assert!(
        harness
            .get_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .is_some(),
        "the search field"
    );
    // Only one section shows at a time.
    assert!(harness.query_by_label("New Layer (⇧⌘N)").is_none());

    // The strip order is Layers, Assets, History.
    assert_eq!(
        SidebarSection::ALL,
        [
            SidebarSection::Layers,
            SidebarSection::Assets,
            SidebarSection::History
        ]
    );
    assert_eq!(SidebarSection::Assets.label(), "Assets (⌘⌥4)");

    harness.key_press_modifiers(BOTH, egui::Key::Num1);
    harness.run();
    harness.key_press_modifiers(BOTH, egui::Key::Num4);
    harness.run();
    assert!(harness.query_by_label("Tree (test-pack/tree)").is_some());
}

#[test]
fn clicking_an_asset_picks_it_and_switches_to_the_stamp_tool() {
    let mut harness = app(true);
    assert_eq!(editor(&harness).active_tool, ToolId::Select);
    assert_eq!(editor(&harness).selected_asset, None);
    harness.get_by_label("Tree (test-pack/tree)").click();
    harness.run();
    assert_eq!(
        editor(&harness).selected_asset,
        Some("test-pack/tree".parse().unwrap())
    );
    assert_eq!(editor(&harness).active_tool, ToolId::Stamp);
    // The picked Asset's pack · author · licence.
    assert!(
        harness
            .query_by_label_contains("Test pack · StarScript Map Maker contributors · CC0-1.0")
            .is_some()
    );
}

#[test]
fn an_empty_search_offers_a_way_out() {
    let mut harness = app(true);
    harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .expect("the search field")
        .click();
    harness.run();
    harness
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .expect("the search field")
        .type_text("zzz");
    harness.run();
    assert!(harness.query_by_label("No Assets match “zzz”").is_some());
    harness.get_by_label("Clear search").click();
    harness.run();
    assert!(harness.query_by_label("Tree (test-pack/tree)").is_some());
}

#[test]
fn without_packs_the_section_says_so_and_opens_the_asset_packs_page() {
    let mut harness = app(false);
    assert!(
        harness
            .query_by_label("No Asset packs are installed.")
            .is_some()
    );
    assert!(!harness.state().home_active());
    harness.get_by_label("Open Asset packs").click();
    harness.run();
    assert!(harness.state().home_active());
    assert!(harness.query_by_label("Open Asset packs").is_none());
}

#[test]
fn properties_links_back_to_the_assets_section() {
    let mut harness = app(true);
    harness.key_press_modifiers(BOTH, egui::Key::Num5);
    harness.run();
    assert_eq!(
        harness.state().shell().sidebar_section(),
        SidebarSection::History
    );
    harness.state_mut().active_editor_mut().unwrap().active_tool = ToolId::Stamp;
    harness.run();
    harness.get_by_label("Choose Asset…").click();
    harness.run();
    assert_eq!(
        harness.state().shell().sidebar_section(),
        SidebarSection::Assets
    );
}
