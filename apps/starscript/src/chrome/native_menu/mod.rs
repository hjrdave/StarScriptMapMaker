//! The native macOS menu bar, built from the menu model in [`crate::menu`].
//!
//! This file is the part that needs no Mac: it turns [`menu::menus`] into what the native
//! menu shows (titles, accelerators, enabled state) and says which shortcuts it owns. The
//! `muda` code that puts it on screen is in `mac`, which only exists on macOS.
//!
//! The native menu owns the shortcuts that use ⌘. The egui handler skips those, so each key
//! is handled once. Keys without ⌘ (Backspace, Enter, Esc, ⇥, ⇧1) stay with egui, because as
//! menu key equivalents they would take the key away from text fields.

use crate::menu::{self, Command, Entry, Key, Shortcut, State, TabState};

#[cfg(target_os = "macos")]
pub mod dock;
#[cfg(target_os = "macos")]
pub mod mac;

/// A native key equivalent: ⌘ plus the keys listed. `key` is the character the key types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Accelerator {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeItem {
    pub command: Command,
    pub title: String,
    pub accelerator: Option<Accelerator>,
    pub enabled: bool,
    /// `Some` for an item with a checkmark: whether it is ticked.
    pub checked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NativeEntry {
    Item(NativeItem),
    Separator,
    /// The Open Recent submenu.
    OpenRecent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeMenu {
    pub title: &'static str,
    pub entries: Vec<NativeEntry>,
}

impl NativeMenu {
    pub(crate) fn items(&self) -> impl Iterator<Item = &NativeItem> {
        self.entries.iter().filter_map(|entry| match entry {
            NativeEntry::Item(item) => Some(item),
            _ => None,
        })
    }

    #[cfg(test)]
    pub(crate) fn item(&self, command: Command) -> Option<&NativeItem> {
        self.items().find(|item| item.command == command)
    }
}

/// The native key equivalent for `shortcut`, or `None` if the native menu leaves the key to
/// egui: it has no ⌘, or it is not a character key.
pub(crate) fn accelerator(shortcut: Shortcut) -> Option<Accelerator> {
    if !shortcut.command {
        return None;
    }
    let key = match shortcut.key {
        Key::A => "a".to_owned(),
        Key::E => "e".to_owned(),
        Key::C => "c".to_owned(),
        Key::V => "v".to_owned(),
        Key::X => "x".to_owned(),
        Key::G => "g".to_owned(),
        Key::J => "j".to_owned(),
        Key::N => "n".to_owned(),
        Key::O => "o".to_owned(),
        Key::S => "s".to_owned(),
        Key::T => "t".to_owned(),
        Key::W => "w".to_owned(),
        Key::Z => "z".to_owned(),
        Key::Plus => "+".to_owned(),
        Key::Minus => "-".to_owned(),
        Key::Num(n) => n.to_string(),
        Key::Tab | Key::Enter | Key::Escape | Key::Backspace => return None,
    };
    Some(Accelerator {
        command: true,
        shift: shortcut.shift,
        alt: shortcut.alt,
        key,
    })
}

/// Whether a text field has its own use for the key of `command`. While one has the keyboard
/// the native menu gives these keys up, so ⌘Z and ⌘A edit the text.
fn text_editing_key(command: Command) -> bool {
    matches!(
        command,
        Command::Undo
            | Command::Redo
            | Command::SelectAll
            | Command::Deselect
            | Command::Copy
            | Command::Cut
            | Command::Paste
    )
}

/// The native menus for `state`. `text_focus` is whether a text field has the keyboard.
pub(crate) fn menus(state: &State, text_focus: bool) -> Vec<NativeMenu> {
    menu::menus(state)
        .into_iter()
        .map(|menu| NativeMenu {
            title: menu.title,
            entries: menu
                .entries
                .into_iter()
                .map(|entry| match entry {
                    Entry::Separator => NativeEntry::Separator,
                    Entry::OpenRecent => NativeEntry::OpenRecent,
                    Entry::Item(item) => NativeEntry::Item(NativeItem {
                        accelerator: item
                            .shortcut
                            .and_then(accelerator)
                            .filter(|_| !(text_focus && text_editing_key(item.command))),
                        command: item.command,
                        title: item.title,
                        enabled: item.enabled,
                        checked: item.checked,
                    }),
                })
                .collect(),
        })
        .collect()
}

/// The items at the top of the StarScript Map Maker app menu, ahead of the standard Hide and
/// Quit items. They run the same commands as their twins in the Help menu.
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn app_items(state: &State) -> Vec<NativeItem> {
    vec![
        NativeItem {
            command: Command::Credits,
            title: "About StarScript Map Maker".to_owned(),
            accelerator: None,
            enabled: true,
            checked: None,
        },
        NativeItem {
            command: Command::CheckForUpdates,
            title: "Check for Updates…".to_owned(),
            accelerator: None,
            enabled: !state.checking_for_updates,
            checked: None,
        },
    ]
}

/// Every command whose key the native menu owns, so the egui handler can skip them.
pub(crate) fn owned_commands() -> Vec<Command> {
    let everything = State {
        has_map: true,
        node_edit: true,
        tabs: vec![TabState::default(); 9],
        ..State::default()
    };
    menus(&everything, false)
        .iter()
        .flat_map(NativeMenu::items)
        .filter(|item| item.accelerator.is_some())
        .map(|item| item.command)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> State {
        State {
            has_map: true,
            tabs: vec![TabState {
                name: "a.ssmap".into(),
                active: true,
            }],
            ..State::default()
        }
    }

    fn find(menus: &[NativeMenu], title: &str, command: Command) -> NativeItem {
        menus
            .iter()
            .find(|menu| menu.title == title)
            .and_then(|menu| menu.item(command))
            .unwrap_or_else(|| panic!("{title} has {command:?}"))
            .clone()
    }

    #[test]
    fn has_the_same_menus_titles_and_enabled_state_as_the_model() {
        let state = State {
            can_undo: true,
            undo_label: Some("Add Layer".into()),
            has_selection: true,
            ..open()
        };
        let native = menus(&state, false);
        let model = menu::menus(&state);
        assert_eq!(native.len(), model.len());
        for (native, model) in native.iter().zip(&model) {
            assert_eq!(native.title, model.title);
            assert_eq!(native.entries.len(), model.entries.len());
            for (n, m) in native.items().zip(model.items()) {
                assert_eq!(n.command, m.command);
                assert_eq!(n.title, m.title);
                assert_eq!(n.enabled, m.enabled);
                assert_eq!(n.checked, m.checked);
            }
        }
        assert_eq!(find(&native, "Edit", Command::Undo).title, "Undo Add Layer");
    }

    #[test]
    fn items_disable_on_home_and_undo_follows_the_tab() {
        let home = menus(&State::default(), false);
        assert!(!find(&home, "File", Command::Save).enabled);
        assert!(find(&home, "File", Command::NewMap).enabled);
        let m = menus(
            &State {
                can_undo: true,
                undo_label: Some("Move".into()),
                ..open()
            },
            false,
        );
        let undo = find(&m, "Edit", Command::Undo);
        assert!(undo.enabled);
        assert_eq!(undo.title, "Undo Move");
    }

    #[test]
    fn command_shortcuts_become_accelerators() {
        let m = menus(&open(), false);
        let save = find(&m, "File", Command::Save).accelerator.unwrap();
        assert_eq!(
            save,
            Accelerator {
                command: true,
                shift: false,
                alt: false,
                key: "s".into()
            }
        );
        let redo = find(&m, "Edit", Command::Redo).accelerator.unwrap();
        assert!(redo.command && redo.shift && !redo.alt);
        assert_eq!(redo.key, "z");
        let layers = find(&m, "View", Command::ToggleLayers).accelerator.unwrap();
        assert!(layers.command && layers.alt && !layers.shift);
        assert_eq!(layers.key, "1");
        assert_eq!(
            find(&m, "View", Command::ZoomIn).accelerator.unwrap().key,
            "+"
        );
        assert_eq!(
            find(&m, "View", Command::ZoomOut).accelerator.unwrap().key,
            "-"
        );
    }

    #[test]
    fn keys_without_command_stay_with_egui() {
        let m = menus(
            &State {
                node_edit: true,
                ..open()
            },
            false,
        );
        for (title, command) in [
            ("Edit", Command::Delete),
            ("Edit", Command::DeleteNodes),
            ("Edit", Command::FinishNodeEdit),
            ("Edit", Command::CancelGesture),
            ("View", Command::ZoomFit),
            ("View", Command::ActualSize),
            ("View", Command::HidePanels),
        ] {
            assert_eq!(find(&m, title, command).accelerator, None, "{command:?}");
        }
        let owned = owned_commands();
        assert!(owned.contains(&Command::Save));
        assert!(owned.contains(&Command::Tab(9)));
        assert!(!owned.contains(&Command::Delete));
        assert!(!owned.contains(&Command::HidePanels));
    }

    #[test]
    fn a_text_field_keeps_its_undo_and_select_all_keys() {
        let m = menus(&open(), true);
        for (title, command) in [
            ("Edit", Command::Undo),
            ("Edit", Command::Redo),
            ("Edit", Command::SelectAll),
            ("Edit", Command::Deselect),
        ] {
            assert_eq!(find(&m, title, command).accelerator, None, "{command:?}");
        }
        // Other keys still work with a text field focused.
        assert!(find(&m, "File", Command::Save).accelerator.is_some());
        // Ownership does not change with focus: the egui handler skips these either way.
        assert!(owned_commands().contains(&Command::Undo));
    }

    #[test]
    fn app_menu_has_about_and_check_for_updates() {
        let items = app_items(&State::default());
        assert_eq!(items[0].command, Command::Credits);
        assert_eq!(items[0].title, "About StarScript Map Maker");
        assert_eq!(items[1].command, Command::CheckForUpdates);
        assert_eq!(items[1].title, "Check for Updates…");
        assert!(items[1].enabled);
        let busy = app_items(&State {
            checking_for_updates: true,
            ..State::default()
        });
        assert!(!busy[1].enabled);
    }

    #[test]
    fn no_two_native_items_share_an_accelerator() {
        let everything = State {
            has_map: true,
            node_edit: true,
            tabs: vec![TabState::default(); 9],
            ..State::default()
        };
        let mut seen: Vec<(Accelerator, Command)> = Vec::new();
        for menu in menus(&everything, false) {
            for item in menu.items() {
                let Some(accelerator) = item.accelerator.clone() else {
                    continue;
                };
                if let Some((_, other)) = seen.iter().find(|(known, _)| *known == accelerator) {
                    panic!("{:?} and {other:?} share {accelerator:?}", item.command);
                }
                seen.push((accelerator, item.command));
            }
        }
    }
}
