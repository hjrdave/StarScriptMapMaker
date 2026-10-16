//! The menu bar as data: every item with its title, shortcut and whether it is enabled.
//!
//! No egui types here. The egui menu bar draws [`menus`], and the native macOS menu is built
//! from the same list, so the two cannot drift apart. [`shortcut`] is also what the keyboard
//! handler reads, so the label on an item is the key that runs it.

use crate::tab_strip;

/// Everything a menu item can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    NewMap,
    NewTab,
    Open,
    CloseTab,
    Save,
    SaveAs,
    ExportPng,
    ReExport,
    Undo,
    Redo,
    Delete,
    Copy,
    Cut,
    Paste,
    Duplicate,
    SelectAll,
    Deselect,
    DeleteNodes,
    FinishNodeEdit,
    CancelGesture,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    ZoomSelection,
    ActualSize,
    ToggleLayers,
    ToggleProperties,
    HidePanels,
    Home,
    /// ⌘`n` for `n` in 2 to 9.
    Tab(u8),
    LabelStyles,
    EmbedAssets,
    UpdateFromPacks,
    NewLayer,
    DeleteLayer,
    Union,
    Subtract,
    SplitApart,
    ShuffleVariant,
    /// The Window menu's entry for the Map in this tab.
    ShowTab(usize),
    Credits,
    CheckForUpdates,
    ToggleUpdateCheck,
}

/// The keys the menu uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    A,
    C,
    E,
    J,
    N,
    O,
    S,
    T,
    V,
    W,
    X,
    Z,
    Plus,
    Minus,
    /// The digit key `0` to `9`.
    Num(u8),
    Tab,
    Enter,
    Escape,
    Backspace,
}

/// A key and its modifiers. `command` is ⌘ on macOS and Ctrl elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl Shortcut {
    const fn new(command: bool, shift: bool, alt: bool, key: Key) -> Self {
        Self {
            command,
            shift,
            alt,
            key,
        }
    }

    /// The label shown beside the item: `⇧⌘Z` on macOS, `Ctrl+Shift+Z` elsewhere.
    pub fn label(self, mac: bool) -> String {
        let key = match (self.key, mac) {
            (Key::A, _) => "A".to_owned(),
            (Key::C, _) => "C".to_owned(),
            (Key::V, _) => "V".to_owned(),
            (Key::X, _) => "X".to_owned(),
            (Key::E, _) => "E".to_owned(),
            (Key::J, _) => "J".to_owned(),
            (Key::N, _) => "N".to_owned(),
            (Key::O, _) => "O".to_owned(),
            (Key::S, _) => "S".to_owned(),
            (Key::T, _) => "T".to_owned(),
            (Key::W, _) => "W".to_owned(),
            (Key::Z, _) => "Z".to_owned(),
            (Key::Plus, _) => "+".to_owned(),
            (Key::Minus, true) => "−".to_owned(),
            (Key::Minus, false) => "-".to_owned(),
            (Key::Num(n), _) => n.to_string(),
            (Key::Tab, true) => "⇥".to_owned(),
            (Key::Tab, false) => "Tab".to_owned(),
            (Key::Enter, true) => "⏎".to_owned(),
            (Key::Enter, false) => "Enter".to_owned(),
            (Key::Escape, _) => "Esc".to_owned(),
            (Key::Backspace, true) => "⌫".to_owned(),
            (Key::Backspace, false) => "Backspace".to_owned(),
        };
        if mac {
            // Apple's order: ⌥ ⇧ ⌘.
            let alt = if self.alt { "⌥" } else { "" };
            let shift = if self.shift { "⇧" } else { "" };
            let command = if self.command { "⌘" } else { "" };
            format!("{alt}{shift}{command}{key}")
        } else {
            let mut parts = Vec::new();
            if self.command {
                parts.push("Ctrl");
            }
            if self.alt {
                parts.push("Alt");
            }
            if self.shift {
                parts.push("Shift");
            }
            parts.push(&key);
            parts.join("+")
        }
    }
}

/// The key that runs `command`, if it has one.
pub fn shortcut(command: Command) -> Option<Shortcut> {
    use Command::*;
    let (c, s, a) = (true, true, true);
    Some(match command {
        NewMap => Shortcut::new(c, !s, !a, Key::N),
        NewTab => Shortcut::new(c, !s, !a, Key::T),
        Open => Shortcut::new(c, !s, !a, Key::O),
        CloseTab => Shortcut::new(c, !s, !a, Key::W),
        Save => Shortcut::new(c, !s, !a, Key::S),
        SaveAs => Shortcut::new(c, s, !a, Key::S),
        ExportPng => Shortcut::new(c, !s, !a, Key::E),
        Undo => Shortcut::new(c, !s, !a, Key::Z),
        Redo => Shortcut::new(c, s, !a, Key::Z),
        Delete | DeleteNodes => Shortcut::new(!c, !s, !a, Key::Backspace),
        Copy => Shortcut::new(c, !s, !a, Key::C),
        Cut => Shortcut::new(c, !s, !a, Key::X),
        Paste => Shortcut::new(c, !s, !a, Key::V),
        Duplicate => Shortcut::new(c, !s, !a, Key::J),
        SelectAll => Shortcut::new(c, !s, !a, Key::A),
        Deselect => Shortcut::new(c, s, !a, Key::A),
        FinishNodeEdit => Shortcut::new(!c, !s, !a, Key::Enter),
        CancelGesture => Shortcut::new(!c, !s, !a, Key::Escape),
        ZoomIn => Shortcut::new(c, !s, !a, Key::Plus),
        ZoomOut => Shortcut::new(c, !s, !a, Key::Minus),
        ZoomFit => Shortcut::new(!c, s, !a, Key::Num(1)),
        ZoomSelection => Shortcut::new(!c, s, !a, Key::Num(2)),
        ActualSize => Shortcut::new(!c, s, !a, Key::Num(0)),
        ToggleLayers => Shortcut::new(c, !s, a, Key::Num(1)),
        ToggleProperties => Shortcut::new(c, !s, a, Key::Num(2)),
        HidePanels => Shortcut::new(!c, !s, !a, Key::Tab),
        Home => Shortcut::new(c, !s, !a, Key::Num(1)),
        Tab(n @ 2..=9) => Shortcut::new(c, !s, !a, Key::Num(n)),
        NewLayer => Shortcut::new(c, s, !a, Key::N),
        _ => return None,
    })
}

/// One tab in the Window menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabState {
    pub name: String,
    pub active: bool,
}

/// What the menus depend on: the active tab's state, read afresh every frame.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// A Map is showing (not Home).
    pub has_map: bool,
    pub node_edit: bool,
    pub nodes_selected: bool,
    pub gesture_open: bool,
    pub undo_label: Option<String>,
    pub can_undo: bool,
    pub redo_label: Option<String>,
    pub can_redo: bool,
    pub has_selection: bool,
    /// Copy or Cut has taken something Paste can put back.
    pub can_paste: bool,
    /// The active Layer has Objects to select.
    pub can_select_all: bool,
    pub has_active_layer: bool,
    pub can_combine: bool,
    pub can_split: bool,
    pub can_shuffle: bool,
    pub can_re_export: bool,
    /// Open Maps, in tab order.
    pub tabs: Vec<TabState>,
    pub updates_at_startup: bool,
    pub checking_for_updates: bool,
}

/// One entry of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Item(Item),
    Separator,
    /// The Open Recent submenu, whose entries come from the recent files list.
    OpenRecent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub command: Command,
    pub title: String,
    pub shortcut: Option<Shortcut>,
    pub enabled: bool,
    /// `Some` for an item with a checkmark: whether it is ticked.
    pub checked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    pub title: &'static str,
    pub entries: Vec<Entry>,
}

impl Menu {
    /// The items, without separators.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.entries.iter().filter_map(|entry| match entry {
            Entry::Item(item) => Some(item),
            _ => None,
        })
    }

    pub fn item(&self, command: Command) -> Option<&Item> {
        self.items().find(|item| item.command == command)
    }
}

fn item(command: Command, title: impl Into<String>, enabled: bool) -> Entry {
    Entry::Item(Item {
        command,
        title: title.into(),
        shortcut: shortcut(command),
        enabled,
        checked: None,
    })
}

fn checked(command: Command, title: impl Into<String>, enabled: bool, on: bool) -> Entry {
    Entry::Item(Item {
        checked: Some(on),
        ..match item(command, title, enabled) {
            Entry::Item(item) => item,
            _ => unreachable!(),
        }
    })
}

/// "Undo" or "Undo Add Layer".
fn with_label(verb: &str, label: Option<&str>) -> String {
    match label {
        Some(label) if !label.is_empty() => format!("{verb} {label}"),
        _ => verb.to_owned(),
    }
}

/// Every menu, in order, for the given state.
pub fn menus(state: &State) -> Vec<Menu> {
    use Command::*;
    use Entry::Separator;
    let map = state.has_map;
    let file = vec![
        item(NewMap, "New Map…", true),
        item(NewTab, "New Tab", true),
        item(Open, "Open…", true),
        Entry::OpenRecent,
        Separator,
        item(CloseTab, "Close Tab", map),
        item(Save, "Save", map),
        item(SaveAs, "Save As…", map),
        Separator,
        item(ExportPng, "Export PNG…", map),
        item(ReExport, "Re-export", map && state.can_re_export),
    ];
    let mut edit = vec![
        item(
            Undo,
            with_label("Undo", state.undo_label.as_deref()),
            map && state.can_undo,
        ),
        item(
            Redo,
            with_label("Redo", state.redo_label.as_deref()),
            map && state.can_redo,
        ),
        Separator,
        item(Cut, "Cut", map && state.has_selection),
        item(Copy, "Copy", map && state.has_selection),
        item(Paste, "Paste", map && state.can_paste),
        item(Duplicate, "Duplicate", map && state.has_selection),
        item(Delete, "Delete", map && state.has_selection),
        item(SelectAll, "Select All", map && state.can_select_all),
        item(Deselect, "Deselect", map && state.has_selection),
    ];
    if map && state.node_edit {
        edit.extend([
            Separator,
            item(DeleteNodes, "Delete Nodes", state.nodes_selected),
            item(FinishNodeEdit, "Finish Node Editing", true),
            item(CancelGesture, "Cancel Gesture", state.gesture_open),
        ]);
    }
    let mut view = vec![
        item(ZoomIn, "Zoom In", map),
        item(ZoomOut, "Zoom Out", map),
        item(ZoomFit, "Zoom to Fit", map),
        item(
            ZoomSelection,
            "Zoom to Selection",
            map && state.has_selection,
        ),
        item(ActualSize, "Actual Size", map),
        Separator,
        item(ToggleLayers, "Toggle Layers", map),
        item(ToggleProperties, "Toggle Properties", map),
        item(HidePanels, "Hide Panels", map),
        Separator,
        item(Home, "Home", true),
    ];
    for n in 2..=9u8 {
        let exists = tab_strip::slot_for_number(n, state.tabs.len()).is_some();
        view.push(item(Tab(n), format!("Tab {n}"), exists));
    }
    let tab_items = state
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| checked(ShowTab(index), tab.name.clone(), true, tab.active))
        .collect();
    vec![
        Menu {
            title: "File",
            entries: file,
        },
        Menu {
            title: "Edit",
            entries: edit,
        },
        Menu {
            title: "View",
            entries: view,
        },
        Menu {
            title: "Map",
            entries: vec![
                item(LabelStyles, "Label Styles…", map),
                Separator,
                item(EmbedAssets, "Embed Assets", map),
                item(UpdateFromPacks, "Update from Packs", map),
            ],
        },
        Menu {
            title: "Layer",
            entries: vec![
                item(NewLayer, "New Layer", map),
                item(DeleteLayer, "Delete", map && state.has_active_layer),
            ],
        },
        Menu {
            title: "Object",
            entries: vec![
                item(Union, "Union", map && state.can_combine),
                item(Subtract, "Subtract", map && state.can_combine),
                item(SplitApart, "Split Apart", map && state.can_split),
                item(ShuffleVariant, "Shuffle Variant", map && state.can_shuffle),
            ],
        },
        Menu {
            title: "Window",
            entries: tab_items,
        },
        Menu {
            title: "Help",
            entries: vec![
                item(Credits, "Credits…", true),
                Separator,
                item(
                    CheckForUpdates,
                    "Check for Updates Now",
                    !state.checking_for_updates,
                ),
                checked(
                    ToggleUpdateCheck,
                    "Check for updates at startup",
                    true,
                    state.updates_at_startup,
                ),
            ],
        },
    ]
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

    fn find(menus: &[Menu], title: &str, command: Command) -> Item {
        menus
            .iter()
            .find(|menu| menu.title == title)
            .and_then(|menu| menu.item(command))
            .unwrap_or_else(|| panic!("{title} has {command:?}"))
            .clone()
    }

    #[test]
    fn on_home_only_the_map_free_items_are_enabled() {
        let menus = menus(&State::default());
        for (title, command) in [
            ("File", Command::NewMap),
            ("File", Command::Open),
            ("View", Command::Home),
            ("Help", Command::Credits),
        ] {
            assert!(find(&menus, title, command).enabled, "{command:?}");
        }
        for (title, command) in [
            ("File", Command::Save),
            ("File", Command::ExportPng),
            ("Map", Command::EmbedAssets),
            ("Map", Command::UpdateFromPacks),
            ("Layer", Command::NewLayer),
            ("View", Command::ZoomIn),
        ] {
            assert!(!find(&menus, title, command).enabled, "{command:?}");
        }
    }

    #[test]
    fn edit_and_object_items_follow_the_state() {
        let menus_for = |state: &State| menus(state);
        let m = menus_for(&open());
        for (title, command) in [
            ("Edit", Command::Undo),
            ("Edit", Command::Redo),
            ("Edit", Command::Delete),
            ("Edit", Command::SelectAll),
            ("Edit", Command::Deselect),
            ("View", Command::ZoomSelection),
            ("Layer", Command::DeleteLayer),
            ("File", Command::ReExport),
            ("Object", Command::Union),
            ("Object", Command::Subtract),
            ("Object", Command::SplitApart),
            ("Object", Command::ShuffleVariant),
        ] {
            assert!(!find(&m, title, command).enabled, "{command:?}");
        }
        let m = menus_for(&State {
            has_selection: true,
            can_paste: true,
            can_select_all: true,
            has_active_layer: true,
            can_combine: true,
            can_split: true,
            can_shuffle: true,
            can_re_export: true,
            ..open()
        });
        for (title, command) in [
            ("Edit", Command::Delete),
            ("Edit", Command::SelectAll),
            ("Edit", Command::Deselect),
            ("View", Command::ZoomSelection),
            ("Layer", Command::DeleteLayer),
            ("File", Command::ReExport),
            ("Object", Command::Union),
            ("Object", Command::Subtract),
            ("Object", Command::SplitApart),
            ("Object", Command::ShuffleVariant),
        ] {
            assert!(find(&m, title, command).enabled, "{command:?}");
        }
    }

    #[test]
    fn undo_and_redo_carry_the_command_names() {
        let m = menus(&State {
            can_undo: true,
            undo_label: Some("Add Layer".into()),
            can_redo: true,
            redo_label: Some("Move".into()),
            ..open()
        });
        assert_eq!(find(&m, "Edit", Command::Undo).title, "Undo Add Layer");
        assert_eq!(find(&m, "Edit", Command::Redo).title, "Redo Move");
        assert!(find(&m, "Edit", Command::Undo).enabled);
        let m = menus(&open());
        assert_eq!(find(&m, "Edit", Command::Undo).title, "Undo");
        assert_eq!(find(&m, "Edit", Command::Redo).title, "Redo");
    }

    #[test]
    fn node_edit_items_show_only_in_node_edit_mode() {
        let m = menus(&open());
        let edit = m.iter().find(|menu| menu.title == "Edit").unwrap();
        assert!(edit.item(Command::DeleteNodes).is_none());
        let m = menus(&State {
            node_edit: true,
            gesture_open: true,
            ..open()
        });
        assert!(!find(&m, "Edit", Command::DeleteNodes).enabled);
        assert!(find(&m, "Edit", Command::FinishNodeEdit).enabled);
        assert!(find(&m, "Edit", Command::CancelGesture).enabled);
    }

    #[test]
    fn map_menu_lists_label_styles_embed_and_update() {
        let m = menus(&open());
        let map = m.iter().find(|menu| menu.title == "Map").unwrap();
        let titles: Vec<_> = map.items().map(|item| item.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Label Styles…", "Embed Assets", "Update from Packs"]
        );
        assert!(map.items().all(|item| item.enabled));
    }

    #[test]
    fn window_menu_ticks_the_active_tab() {
        let m = menus(&State {
            tabs: vec![
                TabState {
                    name: "a".into(),
                    active: false,
                },
                TabState {
                    name: "b".into(),
                    active: true,
                },
            ],
            ..open()
        });
        assert_eq!(find(&m, "Window", Command::ShowTab(0)).checked, Some(false));
        assert_eq!(find(&m, "Window", Command::ShowTab(1)).checked, Some(true));
        // Two Maps: Tab 2 and 3 exist, and 9 always reaches the last one.
        assert!(find(&m, "View", Command::Tab(3)).enabled);
        assert!(!find(&m, "View", Command::Tab(4)).enabled);
    }

    #[test]
    fn shortcut_labels_follow_the_platform() {
        let redo = shortcut(Command::Redo).unwrap();
        assert_eq!(redo.label(true), "⇧⌘Z");
        assert_eq!(redo.label(false), "Ctrl+Shift+Z");
        let panels = shortcut(Command::ToggleLayers).unwrap();
        assert_eq!(panels.label(true), "⌥⌘1");
        assert_eq!(panels.label(false), "Ctrl+Alt+1");
        assert_eq!(shortcut(Command::ZoomFit).unwrap().label(false), "Shift+1");
        assert_eq!(shortcut(Command::HidePanels).unwrap().label(true), "⇥");
    }

    #[test]
    fn no_two_commands_share_a_shortcut_except_the_delete_pair() {
        let mut seen: Vec<(Shortcut, Command)> = Vec::new();
        let commands = menus(&State {
            node_edit: true,
            has_map: true,
            ..State::default()
        })
        .into_iter()
        .flat_map(|menu| menu.entries)
        .filter_map(|entry| match entry {
            Entry::Item(item) => Some(item.command),
            _ => None,
        });
        for command in commands {
            let Some(key) = shortcut(command) else {
                continue;
            };
            if let Some((_, other)) = seen.iter().find(|(known, _)| *known == key) {
                assert!(
                    matches!((command, *other), (Command::DeleteNodes, Command::Delete)),
                    "{command:?} and {other:?} share {key:?}"
                );
            }
            seen.push((key, command));
        }
    }
}
