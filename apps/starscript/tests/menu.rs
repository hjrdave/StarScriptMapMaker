//! The egui menu bar, drawn from the menu model: items follow the active tab, and each Map
//! command is one Undo step (no window needed).

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ssmap_app::MapMakerApp;
use ssmap_app::menu::Command;
use ssmap_core::asset::{PackLibrary, PackRoots};
use ssmap_core::{Canvas, DistanceUnit, Scale};
use ssmap_render::SpriteStore;

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness
        .state_mut()
        .new_map(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)));
    harness.run();
    harness
}

/// The test pack installed, with one of its Stamps placed on the Map.
fn with_stamp(harness: &mut Harness<'static, MapMakerApp>) {
    let roots = PackRoots {
        built_in: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/core/tests/packs"),
        user: None,
    };
    let (library, problems) = PackLibrary::load(&roots);
    assert!(problems.is_empty(), "{problems:?}");
    let editor = harness.state_mut().active_editor_mut().unwrap();
    editor.set_sprites(SpriteStore::new(Arc::new(library)));
    editor.selected_asset = Some("test-pack/mountain-1".parse().unwrap());
    editor.place_stamp([10.0, 10.0]).expect("a Stamp is placed");
    harness.run();
}

fn undo_depth(harness: &mut Harness<'static, MapMakerApp>) -> usize {
    let editor = harness.state_mut().active_editor_mut().unwrap();
    let mut steps = 0;
    while editor.can_undo() {
        editor.undo();
        steps += 1;
    }
    steps
}

#[test]
fn the_toolbar_command_buttons_are_gone() {
    let harness = app();
    for label in [
        "Undo",
        "Redo",
        "New Layer",
        "Union",
        "Subtract",
        "Label styles",
    ] {
        assert!(harness.query_by_label(label).is_none(), "{label}");
    }
    for menu in [
        "File", "Edit", "View", "Map", "Layer", "Object", "Window", "Help",
    ] {
        assert!(harness.get_all_by_label(menu).next().is_some(), "{menu}");
    }
}

#[test]
fn edit_menu_titles_carry_the_undo_name_and_enable_with_the_state() {
    let mut harness = app();
    let undo = common::item_label("Undo", Command::Undo);
    common::open_menu(&mut harness, "Edit");
    // Nothing to undo yet: the item is there but disabled.
    assert!(harness.get_by_label(&undo).accesskit_node().is_disabled());
    harness.key_press(egui::Key::Escape);
    harness.run();

    common::new_layer(&mut harness);
    common::open_menu(&mut harness, "Edit");
    let label = harness.state().tabs()[0]
        .editor()
        .undo_label()
        .expect("New Layer is an Undo step")
        .to_owned();
    let title = common::item_label(&format!("Undo {label}"), Command::Undo);
    assert!(!harness.get_by_label(&title).accesskit_node().is_disabled());
}

#[test]
fn embed_assets_is_one_undo_step() {
    let mut harness = app();
    with_stamp(&mut harness);
    let before = harness.state().tabs()[0]
        .editor()
        .map
        .embedded()
        .assets
        .len();
    common::choose(&mut harness, "Map", "Embed Assets");
    let embedded = harness.state().tabs()[0]
        .editor()
        .map
        .embedded()
        .assets
        .len();
    assert!(embedded > before, "the Stamp's art was embedded");
    // One Undo undoes the whole embed.
    common::undo(&mut harness);
    let after = harness.state().tabs()[0]
        .editor()
        .map
        .embedded()
        .assets
        .len();
    assert_eq!(after, before);
    // Embedding again with nothing left says so instead of adding a step.
    common::choose(&mut harness, "Map", "Embed Assets");
    common::choose(&mut harness, "Map", "Embed Assets");
    let notice = harness.state().tabs()[0].editor().notice().cloned();
    assert_eq!(notice.unwrap().text, "Nothing was embedded");
}

#[test]
fn update_from_packs_with_nothing_newer_changes_nothing() {
    let mut harness = app();
    with_stamp(&mut harness);
    common::choose(&mut harness, "Map", "Embed Assets");
    let steps = undo_depth(&mut harness);
    // Put the embed back, then update: the pack's art is what is embedded, so no new step.
    let editor = harness.state_mut().active_editor_mut().unwrap();
    for _ in 0..steps {
        editor.redo();
    }
    common::choose(&mut harness, "Map", "Update from Packs");
    let editor = harness.state_mut().active_editor_mut().unwrap();
    assert!(!editor.can_redo());
    assert!(
        editor
            .notice()
            .is_some_and(|toast| toast.text.starts_with("Nothing was updated"))
    );
}

#[test]
fn the_window_menu_lists_the_tabs_with_the_active_one_ticked() {
    let mut harness = app();
    harness
        .state_mut()
        .new_map(Canvas::new(50, 50, Scale::new(1.0, DistanceUnit::Miles)));
    harness.run();
    common::open_menu(&mut harness, "Window");
    // One checkbox per tab; the newest tab is the active one.
    let ticks: Vec<_> = harness
        .get_all_by_role(egui::accesskit::Role::CheckBox)
        .map(|node| node.accesskit_node().toggled())
        .collect();
    assert_eq!(
        ticks,
        [
            Some(egui::accesskit::Toggled::False),
            Some(egui::accesskit::Toggled::True)
        ]
    );
}

#[test]
fn a_switched_off_shortcut_is_left_alone() {
    let mut harness = app();
    harness
        .state_mut()
        .set_shortcut_enabled(Command::NewLayer, false);
    let layers = harness.state().tabs()[0].editor().map.children(None).len();
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::N,
    );
    harness.run();
    assert_eq!(
        harness.state().tabs()[0].editor().map.children(None).len(),
        layers
    );
    harness
        .state_mut()
        .set_shortcut_enabled(Command::NewLayer, true);
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::N,
    );
    harness.run();
    assert_eq!(
        harness.state().tabs()[0].editor().map.children(None).len(),
        layers + 1
    );
}
