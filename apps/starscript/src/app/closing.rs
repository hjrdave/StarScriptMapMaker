//! Closing tabs, windows and the app, and the unsaved-changes question that comes first.

use super::*;
use crate::dialog;

impl MapMakerApp {
    /// Closing the window or quitting with unsaved changes asks about each tab that has some, in
    /// turn; a clean close deletes every Recovery journal. Where closing the window leaves the
    /// app running, only a quit request exits.
    pub(super) fn handle_close(&mut self, ctx: &egui::Context) {
        let window =
            self.win.close.take_request() || ctx.input(|input| input.viewport().close_requested());
        let quit = if self.win.main {
            self.app.quit.take_request()
        } else {
            self.win.close.quit_asked()
        };
        if (!quit && !window) || self.win.close.is_allowed() {
            return;
        }
        // Popped-out windows go first, each asking about its own unsaved tabs. Quitting waits
        // for them, and stops if the user cancels in one.
        if self.win.main
            && !self.popped.is_empty()
            && (quit || (window && !self.app.quit.close_hides_window()))
        {
            for popped in &mut self.popped {
                popped.close.ask_quit();
            }
            self.app.quit.request();
            if window {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            return;
        }
        // A question already on screen is the close in progress.
        let asking =
            self.win.close.is_awaiting_save() || self.win.tabs.iter().any(|t| t.pending.is_some());
        if !self.win.main {
            // A popped-out window only ever closes itself; the app goes on.
            self.win.close.set_leaving(Leaving::Window);
        } else if quit {
            self.win.close.set_leaving(Leaving::Quit);
        } else if !asking {
            self.win
                .close
                .set_leaving(if self.app.quit.close_hides_window() {
                    Leaving::Window
                } else {
                    Leaving::Quit
                });
        }
        let unsaved = self.win.tabs.iter().any(MapTab::has_unsaved_changes);
        if window && (unsaved || self.win.close.leaving() == Leaving::Window) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if unsaved {
            if !asking {
                self.advance_quit(ctx);
            }
        } else if quit || self.win.close.leaving() == Leaving::Window {
            self.close(ctx);
        } else {
            for tab in &mut self.win.tabs {
                tab.recovery.reset();
            }
        }
    }

    /// Asks to quit the app, as ⌘Q does. Unsaved Maps are asked about first.
    pub fn request_quit(&mut self) {
        self.app.quit.request();
    }

    /// Whether the window was closed while the app keeps running.
    pub fn window_hidden(&self) -> bool {
        self.win.window_hidden
    }

    /// Shows the window again after it was closed, with only the Home tab.
    pub fn reopen_window(&mut self, ctx: &egui::Context) {
        if !self.win.window_hidden {
            return;
        }
        self.win.window_hidden = false;
        self.win.home_active = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// Test hook: whether closing the window leaves the app running, as on macOS.
    pub fn set_close_hides_window(&mut self, hides: bool) {
        self.app.quit.set_close_hides_window(hides);
    }

    /// Quitting: shows the next tab with unsaved changes that has not been decided about and
    /// asks what to do with it; closes the window once none is left.
    pub(super) fn advance_quit(&mut self, ctx: &egui::Context) {
        let next = self
            .win
            .tabs
            .iter()
            .position(|tab| tab.has_unsaved_changes() && !tab.quit_decided);
        match next {
            Some(index) => {
                self.win.active = index;
                self.win.home_active = false;
                self.win.tabs[index].pending = Some(Pending::Quit);
            }
            None => self.close(ctx),
        }
    }

    /// Stops a quit: every tab stays open and is asked about again next time.
    pub(super) fn cancel_quit(&mut self) {
        for tab in &mut self.win.tabs {
            tab.quit_decided = false;
        }
        self.win.close.cancel_quit();
        self.app.quit.cancel();
    }

    /// Leaves, discarding unsaved changes and their Recovery journals: quits the app, or closes
    /// the window and its tabs and leaves the app running.
    pub(super) fn close(&mut self, ctx: &egui::Context) {
        for tab in &mut self.win.tabs {
            tab.recovery.reset();
        }
        match self.win.close.leaving() {
            Leaving::Quit => {
                self.win.close.allow();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Leaving::Window if !self.win.main => {
                // No tabs left: the window is dropped after its frame.
                self.win.tabs.clear();
            }
            Leaving::Window => {
                // The Maps' memory is released, not kept loaded behind a hidden window.
                self.win.tabs.clear();
                self.win.active = 0;
                self.win.home_active = true;
                self.win.window_hidden = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
        }
    }

    /// Whether a dialog has the keyboard, so shortcuts and the tab row wait.
    pub(super) fn dialog_open(&self) -> bool {
        self.win
            .tabs
            .get(self.win.active)
            .is_some_and(|tab| tab.pending.is_some() || tab.recovery_prompt.is_some())
            || self.win.tabs.iter().any(|tab| tab.pending.is_some())
            || self.win.close.is_awaiting_save()
            || (self.win.main && self.app.unsaved.is_asking())
            || self.win.delete_prompt.is_some()
            || self.win.pack_delete_prompt.is_some()
            || self.win.paste_prompt.is_some()
            || self.win.new_map_dialog.open
            || self.win.export.is_busy()
            || self.win.open_dialog.is_open()
            || self.win.save_dialog.is_open()
    }

    /// Whether the user chose to close the window.
    pub fn is_closing(&self) -> bool {
        self.win.close.is_allowed()
    }

    /// Asks what to do with the active tab's unsaved changes before it closes or the app quits.
    pub(super) fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.win.tabs.get(self.win.active) else {
            return;
        };
        let Some(pending) = tab.pending.clone() else {
            return;
        };
        let name = tab.document.name();
        let mut choice = None;
        let shown = dialog::show(
            ctx,
            "unsaved_changes",
            &format!("Save changes to “{name}”?"),
            360.0,
            |ui| {
                ui.label("Your changes are lost if you don’t save them.");
                dialog::buttons(ui, |ui| {
                    let save = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
                    if dialog::primary_button_with_shortcut(ui, "Save", &save).clicked() {
                        choice = Some(UnsavedChoice::Save);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(UnsavedChoice::Cancel);
                    }
                    if ui.button("Don’t Save").clicked() {
                        choice = Some(UnsavedChoice::Discard);
                    }
                });
            },
        );
        if choice.is_none() {
            if shown.cancelled {
                choice = Some(UnsavedChoice::Cancel);
            } else if dialog::enter_confirms(ctx) {
                choice = Some(UnsavedChoice::Save);
            }
        }
        let Some(choice) = choice else {
            return;
        };
        if let Some(tab) = self.tab_mut() {
            tab.pending = None;
        }
        match choice {
            UnsavedChoice::Save if self.tab().is_some_and(|tab| tab.document.path().is_none()) => {
                // A Map never saved is saved through Save As.
                self.win.close.await_save(self.win.active, pending);
                self.show_save_dialog();
            }
            UnsavedChoice::Save => {
                if self.save() {
                    self.finish(pending, ctx);
                } else {
                    self.cancel_quit();
                }
            }
            UnsavedChoice::Discard => self.finish(pending, ctx),
            UnsavedChoice::Cancel => self.cancel_quit(),
        }
    }

    /// The answer to an unsaved-changes question lets `pending` go ahead.
    pub(super) fn finish(&mut self, pending: Pending, ctx: &egui::Context) {
        match pending {
            Pending::CloseTab => self.remove_tab(self.win.active),
            Pending::Quit => {
                if let Some(tab) = self.tab_mut() {
                    tab.quit_decided = true;
                }
                self.advance_quit(ctx);
            }
        }
    }
}

enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}
