//! Tabs and windows: which tab shows, opening, closing and moving tabs between windows.

use super::*;

impl MapMakerApp {
    /// The tab being shown.
    pub(super) fn current_slot(&self) -> tab_strip::Slot {
        if self.home_active() {
            tab_strip::Slot::Home
        } else {
            tab_strip::Slot::Map(self.win.active)
        }
    }

    pub(super) fn show_slot(&mut self, slot: tab_strip::Slot) {
        match slot {
            // Home lives only in the main window.
            tab_strip::Slot::Home => self.win.home_active = self.win.main,
            tab_strip::Slot::Map(index) => self.focus_tab(index),
        }
    }

    pub(super) fn placement(&self) -> tab_strip::Placement {
        if self.win.main {
            tab_strip::Placement::Main
        } else {
            tab_strip::Placement::Popped
        }
    }

    /// How many windows are open: the main one and those tabs were popped out into.
    pub fn window_count(&self) -> usize {
        1 + self.popped.len()
    }

    /// The tabs of popped-out window `window` (0 is the first one popped out).
    pub fn popped_tabs(&self, window: usize) -> &[MapTab] {
        &self.popped[window].tabs
    }

    /// Runs `f` with popped-out window `window` standing in for the main one, so it acts on
    /// that window's tabs as the menus and keys of that window do.
    pub fn in_popped_window<R>(&mut self, window: usize, f: impl FnOnce(&mut Self) -> R) -> R {
        std::mem::swap(&mut self.win, &mut self.popped[window]);
        let result = f(self);
        std::mem::swap(&mut self.win, &mut self.popped[window]);
        result
    }

    /// Pops tab `index` out of this window into a new window of its own. Does nothing for a
    /// tab that is still a start page, or the only tab of a popped-out window.
    pub fn move_tab_to_new_window(&mut self, index: usize) {
        let movable = self
            .win
            .tabs
            .get(index)
            .is_some_and(|tab| !tab.is_blank() && tab.pending.is_none())
            && (self.win.main || self.win.tabs.len() > 1);
        if !movable {
            return;
        }
        let tab = self.take_tab(index);
        self.popped_count += 1;
        let mut win = self.new_window_state(egui::ViewportId::from_hash_of((
            "popped window",
            self.popped_count,
        )));
        win.shell = self.win.shell.clone();
        win.tabs.push(tab);
        self.popped.push(win);
        self.app.egui_ctx.request_repaint();
    }

    /// Puts tab `index` of a popped-out window back in the main window, once this window's
    /// frame is over. Does nothing in the main window.
    pub fn move_tab_to_main_window(&mut self, index: usize) {
        if self.win.main
            || self
                .win
                .tabs
                .get(index)
                .is_none_or(|tab| tab.pending.is_some())
        {
            return;
        }
        let tab = self.take_tab(index);
        self.app.returning.push(tab);
        self.app.egui_ctx.request_repaint();
    }

    /// Takes tab `index` out of this window with its Recovery journal and everything else, and
    /// keeps the same tab active if it can.
    pub(super) fn take_tab(&mut self, index: usize) -> MapTab {
        self.win.take_tab(index)
    }

    /// Test hook: lets go of tab `index` of window `window` (0 is the main one, then the popped
    /// out ones) with the pointer at `at` on the screen, as the end of a drag does.
    pub fn drop_tab(&mut self, window: usize, index: usize, at: egui::Pos2) {
        let from = if window == 0 {
            self.win.viewport
        } else {
            self.popped[window - 1].viewport
        };
        self.app.tab_drops.push(TabDrop { from, index, at });
        self.app.egui_ctx.request_repaint();
    }

    /// Test hook: where window `window`'s tab strip is on the screen (0 is the main window).
    #[doc(hidden)]
    pub fn strip_rect(&self, window: usize) -> egui::Rect {
        if window == 0 {
            self.win.strip_on_screen()
        } else {
            self.popped[window - 1].strip_on_screen()
        }
    }

    /// Test hook: where the Map tabs of window `window` start, and how wide each is.
    #[doc(hidden)]
    pub fn tab_layout(&self, window: usize) -> (f32, f32) {
        let state = if window == 0 {
            &self.win
        } else {
            &self.popped[window - 1]
        };
        let geometry = state.strip.geometry;
        (
            geometry.tabs_left + state.screen_origin.x,
            geometry.tab_width,
        )
    }

    /// A window with no tabs and no dialogs open.
    pub(super) fn new_window_state(&self, viewport: egui::ViewportId) -> WindowState {
        let headless = !self.app.gpu;
        let pick = || {
            if headless {
                Pick::headless()
            } else {
                Pick::default()
            }
        };
        WindowState {
            tabs: Vec::new(),
            active: 0,
            home_active: false,
            strip: tab_strip::State::default(),
            screen_origin: egui::Pos2::ZERO,
            content_size: egui::Vec2::ZERO,
            canvas_zone: None,
            layers_zone: None,
            home: home::Home::default(),
            shell: shell::Shell::default(),
            drop_focus: false,
            new_map_dialog: NewMapDialog::default(),
            export: ExportFlow::default(),
            open_dialog: pick(),
            save_dialog: pick(),
            pack_dialog: pick(),
            pack_install: pack_install::PackInstall::default(),
            delete_prompt: None,
            pack_delete_prompt: None,
            pack_trash: self.win.pack_trash,
            paste_prompt: None,
            awaiting_save: None,
            close_allowed: false,
            leaving: Leaving::Window,
            window_hidden: false,
            title: String::new(),
            main: false,
            viewport,
            quit_asked: false,
            close_requested: false,
        }
    }

    /// Asks to close this window, as its close button does: unsaved Maps are asked about first.
    pub fn request_window_close(&mut self) {
        self.win.close_requested = true;
    }

    /// Shows the tab ⌘`n` reaches (1 is Home, 9 the last tab); `n` out of range does nothing.
    pub fn show_tab_number(&mut self, n: u8) {
        if let Some(slot) = tab_strip::slot_for_number(n, self.win.tabs.len()) {
            self.show_slot(slot);
        }
    }

    /// Closes tab `index`, first asking about unsaved changes (the question is answered in the
    /// dialog on the next frame).
    pub fn close_tab(&mut self, index: usize) {
        let Some(tab) = self.win.tabs.get_mut(index) else {
            return;
        };
        if tab.has_unsaved_changes() {
            tab.pending = Some(Pending::CloseTab);
            self.win.active = index;
            self.win.home_active = false;
        } else {
            self.remove_tab(index);
        }
    }

    /// Drops tab `index` and its Recovery journal, keeping the same Map active if it can.
    pub(super) fn remove_tab(&mut self, index: usize) {
        let mut tab = self.win.tabs.remove(index);
        tab.recovery.reset();
        if index < self.win.active || self.win.active >= self.win.tabs.len() {
            self.win.active = self.win.active.saturating_sub(1);
        }
    }

    /// The index of the tab showing the Map file at `path`, comparing canonical paths.
    pub(super) fn tab_for(&self, path: &Path) -> Option<usize> {
        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or(path.to_path_buf());
        let wanted = canonical(path);
        self.win.tabs.iter().position(|tab| {
            tab.document
                .path()
                .is_some_and(|open| canonical(open) == wanted)
        })
    }

    /// A new tab for `map`, with this run's art and a Recovery journal of its own.
    pub(super) fn new_tab(&self, map: Map, document: Document) -> MapTab {
        let recovery = Recovery::new(self.app.recovery_dir.clone());
        let mut tab = MapTab::new(map, self.app.gpu.then(TessellationCache::new), recovery);
        tab.document = document;
        tab.editor.set_sprites(self.app.sprites.clone());
        if self.app.gpu {
            let ctx = self.app.egui_ctx.clone();
            tab.editor
                .compute_shore_rings_in_background(move || ctx.request_repaint());
        }
        tab
    }

    /// Adds `tab` after the others and shows it. A blank tab on show takes it instead, so a
    /// Map opened or created from the start page opens in that tab.
    pub(super) fn push_tab(&mut self, tab: MapTab) {
        if self.start_page_showing() {
            let mut blank = std::mem::replace(&mut self.win.tabs[self.win.active], tab);
            blank.recovery.reset();
        } else {
            self.win.tabs.push(tab);
            self.win.active = self.win.tabs.len() - 1;
        }
        self.win.home_active = false;
    }

    /// Opens a blank tab with the start page and shows it.
    pub fn new_blank_tab(&mut self) {
        let recovery = Recovery::new(self.app.recovery_dir.clone());
        self.push_blank(MapTab::blank(
            self.app.gpu.then(TessellationCache::new),
            recovery,
        ));
    }

    pub(super) fn push_blank(&mut self, tab: MapTab) {
        self.win.tabs.push(tab);
        self.win.active = self.win.tabs.len() - 1;
        self.win.home_active = false;
    }

    /// Whether the start page of a blank tab is what the window shows.
    pub(super) fn start_page_showing(&self) -> bool {
        !self.home_active()
            && self
                .win
                .tabs
                .get(self.win.active)
                .is_some_and(MapTab::is_blank)
    }

    /// Whether a Map's editor is what the window shows: neither Home nor a start page.
    pub(super) fn map_showing(&self) -> bool {
        !self.win.tabs.is_empty() && !self.home_active() && !self.start_page_showing()
    }

    /// Shows a message on the active tab's status bar, or on stderr when no Map is open.
    pub(super) fn notify_error(&mut self, message: String) {
        match self.win.tabs.get_mut(self.win.active) {
            Some(tab) => tab.editor.error(message),
            None => eprintln!("{message}"),
        }
    }

    /// The window title, as last sent to the window.
    pub fn title(&self) -> &str {
        &self.win.title
    }
}
