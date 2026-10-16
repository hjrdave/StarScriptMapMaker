//! Helpers the UI tests share.
#![allow(dead_code)]

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::menu::{self, Command};

/// The text an item's button has for assistive tools: its title and its shortcut label.
pub fn item_label(title: &str, command: Command) -> String {
    match menu::shortcut(command) {
        Some(shortcut) => format!("{title} {}", shortcut.label(cfg!(target_os = "macos"))),
        None => title.to_owned(),
    }
}

/// Opens the menu bar's `menu` and clicks the item with the label `item`.
pub fn choose<S>(harness: &mut Harness<'_, S>, menu: &str, item: &str) {
    open_menu(harness, menu);
    harness.get_by_label(item).click();
    harness.run();
}

/// Opens a menu of the bar. Some panels have a title like a menu's ("Map"); the bar is first.
pub fn open_menu<S>(harness: &mut Harness<'_, S>, menu: &str) {
    harness
        .get_all_by_label(menu)
        .next()
        .unwrap_or_else(|| panic!("a {menu} menu"))
        .click();
    harness.run();
}

/// Edit ▸ Undo, whose title carries the name of the step ("Undo New Layer").
pub fn undo<S>(harness: &mut Harness<'_, S>) {
    open_menu(harness, "Edit");
    let shortcut = menu::shortcut(Command::Undo).unwrap();
    let suffix = format!(" {}", shortcut.label(cfg!(target_os = "macos")));
    harness.get_by_label_contains(&suffix).click();
    harness.run();
}

/// Layer ▸ New Layer.
pub fn new_layer<S>(harness: &mut Harness<'_, S>) {
    choose(
        harness,
        "Layer",
        &item_label("New Layer", Command::NewLayer),
    );
}
