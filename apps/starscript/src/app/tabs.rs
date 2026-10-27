//! Tabs: which tab shows, opening and closing them.

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
            tab_strip::Slot::Home => self.win.home_active = true,
            tab_strip::Slot::Map(index) => self.focus_tab(index),
        }
    }

    /// Settles a tab let go at `at` after a drag: dropped on the strip it moves to that place
    /// in the row, anywhere else it stays where it is. A start page or a tab being saved
    /// stays too.
    pub fn drop_tab(&mut self, index: usize, at: egui::Pos2) {
        let movable = self
            .win
            .tabs
            .get(index)
            .is_some_and(|tab| !tab.is_blank() && tab.pending.is_none());
        if !movable || !self.win.strip.geometry.strip.contains(at) {
            return;
        }
        let mut to = self
            .win
            .strip
            .geometry
            .drop_index(at.x, self.win.tabs.len());
        if to > index {
            to -= 1;
        }
        if to != index {
            let tab = self.win.tabs.remove(index);
            self.win.tabs.insert(to, tab);
            self.win.active = to;
            self.win.home_active = false;
        }
    }

    /// Test hook: where the tab strip is in the window, and where the Map tabs start and how
    /// wide each is.
    #[doc(hidden)]
    pub fn tab_layout(&self) -> (egui::Rect, f32, f32) {
        let geometry = self.win.strip.geometry;
        (geometry.strip, geometry.tabs_left, geometry.tab_width)
    }

    /// Asks to close this window, as its close button does: unsaved Maps are asked about first.
    pub fn request_window_close(&mut self) {
        self.win.close.request();
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
        let deleted = tab.recovery.reset();
        if index < self.win.active || self.win.active >= self.win.tabs.len() {
            self.win.active = self.win.active.saturating_sub(1);
        }
        self.notify_journal_delete(deleted);
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
        let blank = if self.start_page_showing() {
            self.tab_mut()
        } else {
            None
        };
        if let Some(slot) = blank {
            let deleted = std::mem::replace(slot, tab).recovery.reset();
            self.notify_journal_delete(deleted);
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

    /// Shows a message on the active tab's status bar, or on Home when no Map is open.
    pub(super) fn notify_error(&mut self, message: String) {
        match self.win.tabs.get_mut(self.win.active) {
            Some(tab) => tab.editor.error(message),
            None => self.win.notices.push(message),
        }
    }

    /// Tells the user when a Recovery journal could not be deleted: it is offered again on the
    /// next launch.
    pub(super) fn notify_journal_delete(&mut self, deleted: std::io::Result<()>) {
        if let Err(err) = deleted {
            self.notify_error(format!("Couldn’t delete the unsaved changes: {err}"));
        }
    }

    /// The messages Home lists while no Map is open, for tests.
    pub fn home_notices(&self) -> &[String] {
        &self.win.notices
    }

    /// The window title, as last sent to the window.
    pub fn title(&self) -> &str {
        &self.win.title
    }
}
