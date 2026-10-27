//! The menu bar, menu commands and keyboard shortcuts.

use super::*;
use crate::{tools, zoom};

/// How much `[` and `]` shrink or grow the land brush.
const BRUSH_SIZE_STEP: f64 = 1.25;

impl MapMakerApp {
    /// Stops the egui handler from reacting to `command`'s key, or starts again.
    pub fn set_shortcut_enabled(&mut self, command: menu::Command, enabled: bool) {
        if enabled {
            self.app.shortcuts_off.remove(&command);
        } else {
            self.app.shortcuts_off.insert(command);
        }
    }

    /// What the menus depend on, from the active tab.
    pub(super) fn menu_state(&self) -> menu::State {
        let mut state = menu::State {
            updates_at_startup: self.app.updates.enabled() == Some(true),
            checking_for_updates: self.app.updates.is_checking(),
            popped: !self.win.main,
            tabs: self
                .win
                .tabs
                .iter()
                .enumerate()
                .map(|(index, tab)| menu::TabState {
                    name: tab.name(),
                    active: !self.home_active() && index == self.win.active,
                })
                .collect(),
            ..menu::State::default()
        };
        if !self.map_showing() {
            return state;
        }
        let Some(tab) = self.tab() else {
            return state;
        };
        let editor = &tab.editor;
        state.has_map = true;
        state.node_edit = editor.node_edit().is_some();
        state.nodes_selected = editor
            .node_edit()
            .is_some_and(|edit| !edit.nodes.is_empty());
        state.gesture_open = editor.gesture_open();
        state.can_undo = editor.can_undo();
        state.undo_label = editor.undo_label().map(str::to_owned);
        state.can_redo = editor.can_redo();
        state.redo_label = editor.redo_label().map(str::to_owned);
        state.has_selection = !editor.selection().is_empty();
        state.can_paste = self.app.clipboard.is_some() || editor.has_clipboard();
        state.can_select_all = !editor.active_layer_objects().is_empty();
        state.has_active_layer = editor
            .active_layer()
            .is_some_and(|id| editor.map().contains(id));
        state.can_combine = editor.can_combine();
        state.can_split = editor.can_split();
        state.can_shuffle = editor.can_shuffle();
        state.can_re_export = self.re_export_target().is_some() && !self.win.export.is_running();
        state
    }

    /// The menu bar, drawn from [`menu::menus`].
    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let menus = menu::menus(&self.menu_state());
        let mut chosen = None;
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in &menus {
                ui.menu_button(menu.title, |ui| {
                    for entry in &menu.entries {
                        match entry {
                            menu::Entry::Separator => {
                                ui.separator();
                            }
                            menu::Entry::OpenRecent => {
                                ui.menu_button("Open Recent", |ui| {
                                    if let Some(path) = self.app.recent.menu(ui) {
                                        ui.close();
                                        self.open_recent(&path);
                                    }
                                });
                            }
                            menu::Entry::Item(item) => {
                                if menu_item(ui, item) {
                                    chosen = Some(item.command);
                                    ui.close();
                                }
                            }
                        }
                    }
                });
            }
        });
        if let Some(command) = chosen {
            self.run_command(command);
        }
    }

    /// Whether the native menu bar stands in for the egui one.
    pub(super) fn native_menu_shown(&self) -> bool {
        #[cfg(target_os = "macos")]
        return self.app.native_menu.is_some();
        #[cfg(not(target_os = "macos"))]
        false
    }

    /// Runs the native menu items chosen since the last frame.
    #[cfg(target_os = "macos")]
    pub(super) fn native_menu_actions(&mut self, ctx: &egui::Context) {
        use native_menu::mac::Action;
        let Some(mut native) = self.app.native_menu.take() else {
            return;
        };
        if native_menu::dock::take_click() {
            self.reopen_window(ctx);
        }
        let dialog_open = self.dialog_open();
        for action in native.take_actions() {
            match action {
                Action::Quit => self.request_quit(),
                // A dialog has the keyboard, as it does for the shortcuts.
                _ if dialog_open => {}
                Action::Command(command) => self.run_command(command),
                Action::OpenRecent(path) => self.open_recent(&path),
            }
        }
        self.app.native_menu = Some(native);
    }

    /// Brings the native menu bar up to date with the active tab.
    #[cfg(target_os = "macos")]
    pub(super) fn native_menu_sync(&mut self, ctx: &egui::Context) {
        let Some(mut native) = self.app.native_menu.take() else {
            return;
        };
        let state = self.menu_state();
        let text_focus = ctx.egui_wants_keyboard_input()
            || self
                .win
                .tabs
                .get(self.win.active)
                .is_some_and(|tab| tab.canvas_view.editing_text());
        native.sync(native_menu::mac::Spec {
            app: native_menu::app_items(&state),
            menus: native_menu::menus(&state, text_focus),
            recent: self.app.recent.paths().to_vec(),
        });
        self.app.native_menu = Some(native);
    }

    fn open_recent(&mut self, path: &Path) {
        if path.is_file() {
            self.open_path(path);
        } else {
            self.app.recent.remove(path);
            self.win.home.forget(path);
            self.notify_error(format!("“{}” no longer exists", path.display()));
        }
    }

    /// Runs a menu command: from a click on the menu bar, or from its key.
    pub fn run_command(&mut self, command: menu::Command) {
        use menu::Command as C;
        match command {
            C::NewMap => self.win.new_map_dialog.open = true,
            C::NewTab => self.new_blank_tab(),
            C::Open => self.show_open_dialog(),
            C::Home => self.show_slot(tab_strip::Slot::Home),
            C::ToggleLayers => self.win.shell.toggle_layers(),
            C::ToggleProperties => self.win.shell.toggle_properties(),
            C::ShowHistory => self.win.shell.show_section(shell::SidebarSection::History),
            C::HidePanels => self.win.shell.toggle_panels(),
            C::Tab(n) => self.show_tab_number(n),
            C::ShowTab(index) => self.focus_tab(index),
            C::MoveTabToNewWindow if self.map_showing() => {
                self.move_tab_to_new_window(self.win.active);
            }
            C::MoveTabToMainWindow if self.map_showing() => {
                self.move_tab_to_main_window(self.win.active);
            }
            C::MoveTabToNewWindow | C::MoveTabToMainWindow => {}
            C::Credits => {
                self.app.credits.open = true;
                self.show_main_window();
            }
            C::CheckForUpdates => {
                self.app.updates.start(true);
                self.show_main_window();
            }
            C::ToggleUpdateCheck => {
                let on = self.app.updates.enabled() == Some(true);
                self.app.updates.set_enabled(!on);
            }
            C::CloseTab => {
                if self.current_slot().can_close() {
                    self.close_tab(self.win.active);
                }
            }
            _ => self.run_map_command(command),
        }
    }

    /// Brings the main window forward, where the dialogs shared by every window show.
    fn show_main_window(&self) {
        if !self.win.main {
            self.app
                .egui_ctx
                .send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
    }

    /// The commands that act on the Map showing; they do nothing on Home or a start page.
    fn run_map_command(&mut self, command: menu::Command) {
        use menu::Command as C;
        if !self.map_showing() {
            return;
        }
        match command {
            C::Save => {
                self.save();
            }
            C::SaveAs => self.show_save_dialog(),
            C::ExportPng => self.open_export_dialog(),
            C::ReExport => self.re_export(),
            _ => {}
        }
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        let editor = &mut tab.editor;
        match command {
            C::LabelStyles => tab.label_styles.open = !tab.label_styles.open,
            C::Undo => editor.undo(),
            C::Redo => editor.redo(),
            C::Delete => editor.delete_selection(),
            C::Copy => {
                editor.copy_selection();
            }
            C::Cut => editor.cut_selection(),
            C::Paste => {
                let pointer = self.app.egui_ctx.input(|i| i.pointer.latest_pos());
                if let Some(at) = tab.canvas_view.paste_point(pointer) {
                    if let Some(clipboard) = self.app.clipboard.clone() {
                        editor.set_clipboard(clipboard);
                    }
                    match editor.paste_gaps() {
                        Some(gaps) if !gaps.is_empty() => {
                            self.win.paste_prompt = Some(PastePrompt { gaps, at });
                        }
                        _ => {
                            editor.paste(at);
                        }
                    }
                }
            }
            C::Duplicate => {
                editor.duplicate_selection();
            }
            C::SelectAll => editor.select_all(),
            C::Deselect => editor.deselect(),
            C::DeleteNodes => editor.delete_selected_nodes(),
            C::FinishNodeEdit => editor.leave_node_edit(),
            C::CancelGesture => {
                editor.cancel_open_gesture();
            }
            C::ZoomIn => tab.canvas_view.request_zoom(zoom::Zoom::In),
            C::ZoomOut => tab.canvas_view.request_zoom(zoom::Zoom::Out),
            C::ZoomFit => tab.canvas_view.request_zoom(zoom::Zoom::Fit),
            C::ZoomSelection => tab.canvas_view.request_zoom(zoom::Zoom::Selection),
            C::ActualSize => tab.canvas_view.request_zoom(zoom::Zoom::Percent(1.0)),
            C::EmbedAssets => {
                editor.end_open_gesture();
                if !editor.embed_used_assets() {
                    editor.info("Nothing was embedded");
                }
            }
            C::UpdateFromPacks => {
                editor.end_open_gesture();
                if !editor.update_from_packs() {
                    editor.info("Nothing was updated: no installed pack has newer art");
                }
            }
            C::NewLayer => {
                editor.new_layer();
            }
            C::NewGroup => {
                editor.new_group();
            }
            C::Group => {
                editor.group_selection();
            }
            C::Ungroup => {
                editor.ungroup_selection();
            }
            C::DeleteLayer => editor.delete_active_layer(),
            C::Union => editor.union_selection(),
            C::Subtract => editor.subtract_selection(),
            C::SplitApart => editor.split_selection(),
            C::ShuffleVariant => {
                editor.shuffle_variants();
            }
            _ => {}
        }
    }

    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        // Text fields keep their own undo and keys. The Label text box keeps Escape too,
        // which egui takes its focus away for before this runs.
        if ctx.egui_wants_keyboard_input()
            || self
                .win
                .tabs
                .get(self.win.active)
                .is_some_and(|tab| tab.canvas_view.editing_text())
        {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        let shortcut = |modifiers, key| KeyboardShortcut::new(modifiers, key);
        let off = self.app.shortcuts_off.clone();
        // The panel shortcuts come first: egui ignores extra Alt, so Cmd+Alt+1 would
        // otherwise also match Cmd+1 (Home) below.
        let panels = ctx.input_mut(|input| {
            [
                menu::Command::ToggleLayers,
                menu::Command::ToggleProperties,
                menu::Command::ShowHistory,
                menu::Command::HidePanels,
            ]
            .into_iter()
            .filter(|&command| consume(input, &off, command))
            .collect::<Vec<_>>()
        });
        for command in panels {
            self.run_command(command);
        }
        let (new_map, number, step) = ctx.input_mut(|input| {
            let new_map = consume(input, &off, menu::Command::NewTab);
            let number = (1u8..=9).find(|&n| {
                let command = if n == 1 {
                    menu::Command::Home
                } else {
                    menu::Command::Tab(n)
                };
                consume(input, &off, command)
            });
            // Shift first: plain Ctrl+Tab would also match it.
            let step = if input
                .consume_shortcut(&shortcut(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab))
            {
                Some(false)
            } else if input.consume_shortcut(&shortcut(Modifiers::CTRL, Key::Tab)) {
                Some(true)
            } else {
                None
            };
            (new_map, number, step)
        });
        if new_map {
            self.new_blank_tab();
        }
        if let Some(n) = number {
            self.show_tab_number(n);
        }
        if let Some(forward) = step {
            let mut slot = tab_strip::cycle(self.current_slot(), self.win.tabs.len(), forward);
            if slot == tab_strip::Slot::Home && !self.win.main {
                // No Home here: go on to the next one.
                slot = tab_strip::cycle(slot, self.win.tabs.len(), forward);
            }
            self.show_slot(slot);
        }
        let (new_map, open, save, save_as, close_tab) = ctx.input_mut(|input| {
            // Cmd+Shift+N is New Layer, below.
            let new_map = !input.modifiers.shift && consume(input, &off, menu::Command::NewMap);
            // Shift+S first: plain Cmd+S would also match it.
            let save_as = consume(input, &off, menu::Command::SaveAs);
            let save = consume(input, &off, menu::Command::Save);
            let open = consume(input, &off, menu::Command::Open);
            let close_tab = consume(input, &off, menu::Command::CloseTab);
            (new_map, open, save, save_as, close_tab)
        });
        if new_map {
            self.win.new_map_dialog.open = true;
        }
        if open {
            self.show_open_dialog();
        }
        if close_tab && self.current_slot().can_close() {
            self.close_tab(self.win.active);
            return;
        }
        // Home and a start page have no Map to save or edit.
        if !self.map_showing() {
            return;
        }
        if save_as {
            self.show_save_dialog();
        } else if save {
            self.save();
        }
        let commands = ctx.input_mut(|input| {
            // More specific first: egui ignores an extra Shift or Alt, so ⌘A would also read
            // as ⇧⌘A, and ⇧0 as plain 0.
            let mut commands = Vec::new();
            for command in [
                menu::Command::ExportPng,
                menu::Command::Deselect,
                menu::Command::SelectAll,
                menu::Command::ActualSize,
                menu::Command::ZoomFit,
                menu::Command::ZoomSelection,
                menu::Command::ZoomIn,
                menu::Command::ZoomOut,
            ] {
                if consume(input, &off, command) {
                    commands.push(command);
                }
            }
            // ⌘= is the same key as ⌘+ on most keyboards. The native menu only matches ⌘⇧=
            // for Zoom In, so the app takes the bare keys itself, menu or not. When the
            // menu does take a key it never reaches here, so nothing zooms twice.
            commands.extend(canvas_zoom_keys(input));
            commands.extend(clipboard_keys(input, &off));
            commands
        });
        for command in commands {
            self.run_command(command);
        }
        let Some(tab) = self.tab_mut() else {
            return;
        };
        let editor = &mut tab.editor;
        ctx.input_mut(|input| {
            if editor.node_edit().is_some() && consume(input, &off, menu::Command::FinishNodeEdit) {
                editor.leave_node_edit();
            }
            if consume(input, &off, menu::Command::Redo)
                || input.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::Y))
            {
                editor.redo();
            } else if consume(input, &off, menu::Command::Undo) {
                editor.undo();
            }
            if consume(input, &off, menu::Command::Duplicate) {
                editor.duplicate_selection();
            }
            if consume(input, &off, menu::Command::NewLayer) {
                editor.new_layer();
            }
            if consume(input, &off, menu::Command::Ungroup) {
                editor.ungroup_selection();
            } else if consume(input, &off, menu::Command::Group) {
                editor.group_selection();
            }
            if consume(input, &off, menu::Command::Delete)
                || (!off.contains(&menu::Command::Delete)
                    && input.consume_key(Modifiers::NONE, Key::Delete))
            {
                if editor.node_edit().is_some() {
                    editor.delete_selected_nodes();
                } else {
                    editor.delete_selection();
                }
            }
            if consume(input, &off, menu::Command::CancelGesture) && !editor.cancel_open_gesture() {
                if editor.node_edit().is_some() {
                    editor.leave_node_edit();
                } else if editor.tool_busy {
                    editor.cancel_tool = true;
                } else {
                    editor.select(Vec::new());
                }
            }
            // Shifted shortcuts first: egui ignores an extra Shift, so ⇧S would also read as S.
            let mut tools = ToolId::ALL;
            tools.sort_by_key(|tool| {
                !tool
                    .shortcut()
                    .is_some_and(|(modifiers, _)| modifiers.shift)
            });
            for tool in tools {
                let Some((modifiers, key)) = tool.shortcut() else {
                    continue;
                };
                if input.consume_key(modifiers, key) {
                    editor.active_tool = tool;
                }
            }
            if tools::uses_brush_size(editor.active_tool) {
                let step = if input.consume_key(Modifiers::NONE, Key::OpenBracket) {
                    1.0 / BRUSH_SIZE_STEP
                } else if input.consume_key(Modifiers::NONE, Key::CloseBracket) {
                    BRUSH_SIZE_STEP
                } else {
                    1.0
                };
                editor.brush_size = (editor.brush_size * step)
                    .clamp(*tools::BRUSH_SIZES.start(), *tools::BRUSH_SIZES.end());
            }
        });
    }
}

/// Whether to show ⌘ in shortcut labels rather than Ctrl.
const MAC: bool = cfg!(target_os = "macos");

/// Draws one menu item; returns whether it was chosen.
fn menu_item(ui: &mut egui::Ui, item: &menu::Item) -> bool {
    match item.checked {
        Some(on) => {
            let mut value = on;
            ui.add_enabled(item.enabled, egui::Checkbox::new(&mut value, &item.title))
                .changed()
        }
        None => {
            let mut button = egui::Button::new(&item.title);
            if let Some(shortcut) = item.shortcut {
                button = button.shortcut_text(shortcut.label(MAC));
            }
            ui.add_enabled(item.enabled, button).clicked()
        }
    }
}

/// The egui key for a menu key.
fn egui_key(key: menu::Key) -> egui::Key {
    use menu::Key as K;
    match key {
        K::A => egui::Key::A,
        K::C => egui::Key::C,
        K::V => egui::Key::V,
        K::X => egui::Key::X,
        K::E => egui::Key::E,
        K::G => egui::Key::G,
        K::J => egui::Key::J,
        K::N => egui::Key::N,
        K::O => egui::Key::O,
        K::S => egui::Key::S,
        K::T => egui::Key::T,
        K::W => egui::Key::W,
        K::Z => egui::Key::Z,
        K::Plus => egui::Key::Plus,
        K::Minus => egui::Key::Minus,
        K::Num(0) => egui::Key::Num0,
        K::Num(1) => egui::Key::Num1,
        K::Num(2) => egui::Key::Num2,
        K::Num(3) => egui::Key::Num3,
        K::Num(4) => egui::Key::Num4,
        K::Num(5) => egui::Key::Num5,
        K::Num(6) => egui::Key::Num6,
        K::Num(7) => egui::Key::Num7,
        K::Num(8) => egui::Key::Num8,
        K::Num(_) => egui::Key::Num9,
        K::Tab => egui::Key::Tab,
        K::Enter => egui::Key::Enter,
        K::Escape => egui::Key::Escape,
        K::Backspace => egui::Key::Backspace,
    }
}

/// The Canvas zoom keys the native menu does not match: ⌘= and ⌘+ in, ⌘− out.
fn canvas_zoom_keys(input: &mut egui::InputState) -> Option<menu::Command> {
    use egui::{Key, Modifiers};
    if input.consume_key(Modifiers::COMMAND, Key::Equals)
        || input.consume_key(Modifiers::COMMAND, Key::Plus)
    {
        Some(menu::Command::ZoomIn)
    } else if input.consume_key(Modifiers::COMMAND, Key::Minus) {
        Some(menu::Command::ZoomOut)
    } else {
        None
    }
}

/// The Copy, Cut and Paste presses. egui also reports ⌘C, ⌘X and ⌘V as events of their own,
/// which are taken here too (unless the native menu owns the key) so one press runs once.
fn clipboard_keys(
    input: &mut egui::InputState,
    off: &HashSet<menu::Command>,
) -> Vec<menu::Command> {
    use egui::Event;
    use menu::Command as C;
    let mut pressed = Vec::new();
    for command in [C::Copy, C::Cut, C::Paste] {
        let mut hit = consume(input, off, command);
        if !off.contains(&command) {
            let before = input.events.len();
            input.events.retain(|event| {
                !matches!(
                    (command, event),
                    (C::Copy, Event::Copy) | (C::Cut, Event::Cut) | (C::Paste, Event::Paste(_))
                )
            });
            hit |= input.events.len() != before;
        }
        if hit {
            pressed.push(command);
        }
    }
    pressed
}

/// Takes the key press that runs `command`, unless its shortcut is switched off. The key is
/// read from the menu model, so the label and the handling are one binding.
fn consume(
    input: &mut egui::InputState,
    off: &HashSet<menu::Command>,
    command: menu::Command,
) -> bool {
    let Some(shortcut) = menu::shortcut(command).filter(|_| !off.contains(&command)) else {
        return false;
    };
    let mut modifiers = egui::Modifiers::NONE;
    modifiers.command = shortcut.command;
    modifiers.shift = shortcut.shift;
    modifiers.alt = shortcut.alt;
    input.consume_shortcut(&egui::KeyboardShortcut::new(
        modifiers,
        egui_key(shortcut.key),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_equals_minus_and_plus_zoom_the_canvas() {
        let press = |key| {
            let ctx = egui::Context::default();
            let mut raw = egui::RawInput::default();
            raw.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            });
            let mut found = None;
            let mut output = ctx.run_ui(raw, |ui| {
                found = ui.ctx().input_mut(canvas_zoom_keys);
            });
            output.textures_delta.clear();
            found
        };
        assert_eq!(press(egui::Key::Equals), Some(menu::Command::ZoomIn));
        assert_eq!(press(egui::Key::Plus), Some(menu::Command::ZoomIn));
        assert_eq!(press(egui::Key::Minus), Some(menu::Command::ZoomOut));
        assert_eq!(press(egui::Key::A), None);
    }
}
