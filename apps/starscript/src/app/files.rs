//! Map files: opening, saving, the Recovery journal and what a crash left behind.

use super::*;
use crate::{dialog, document, new_map, recovery};

impl MapMakerApp {
    /// Opens a Map file in a new tab and shows it, or shows the tab that already has that
    /// file open.
    pub fn open_path(&mut self, path: &Path) -> bool {
        if let Some(index) = self.tab_for(path) {
            self.focus_tab(index);
            return true;
        }
        match Document::open(path) {
            Ok((document, map)) => {
                let mut tab = self.new_tab(map, document);
                let ctx = self.app.egui_ctx.clone();
                tab.editor
                    .start_background_roughness(move || ctx.request_repaint());
                self.app.recent.add(path);
                self.win.home.forget(path);
                tab.editor.info(format!("Opened “{}”", tab.document.name()));
                find_recovery(&mut tab, path);
                self.push_tab(tab);
                true
            }
            Err(err) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.notify_error(format!("Couldn’t open “{name}”: {err}"));
                false
            }
        }
    }

    /// Opens a new Map from the starter template in a new tab and shows it.
    pub fn new_map(&mut self, canvas: Canvas) {
        self.new_map_from(MapTemplate::starter(), canvas);
    }

    /// Opens a new Map from `template` in a new tab and shows it.
    pub fn new_map_from(&mut self, template: Result<MapTemplate, TemplateError>, canvas: Canvas) {
        let (map, layers, problem) = new_map::start_map_from(template, canvas);
        let mut tab = self.new_tab(map, Document::default());
        tab.editor.default_layers = layers;
        match problem {
            Some(problem) => tab.editor.error(problem),
            None => tab.editor.info("Created a new Map"),
        }
        self.push_tab(tab);
    }

    /// Applies a journal's changes to the Map file it was written against.
    pub(super) fn replay_recovery(&mut self, found: Found) {
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        let ops = found.recovered.ops.clone();
        match found.recovered.clone().replay(&mut tab.editor.map) {
            Ok(()) => {
                if let Err(err) =
                    tab.recovery
                        .resume(&tab.document, &tab.editor.map, &found.path, &ops)
                {
                    tab.editor
                        .error(format!("Couldn’t write the Recovery journal: {err}"));
                    return;
                }
                tab.editor.info(format!(
                    "Recovered the unsaved changes to “{}”",
                    tab.document.name()
                ));
            }
            Err(err) => {
                tab.recovery_prompt = Some(RecoveryPrompt::Copy {
                    found,
                    reason: format!("The unsaved changes no longer apply to it ({err})."),
                });
            }
        }
    }

    /// Saves a journal's changes, applied as far as they go, as a copy beside the active
    /// tab's Map file, which is left as it is.
    pub(super) fn save_recovered_copy(&mut self, found: Found) {
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        let Some(path) = tab.document.path().map(Path::to_path_buf) else {
            return;
        };
        let mut copy = tab.editor.map.clone();
        let skipped = found.recovered.replay_lenient(&mut copy);
        let copy_path = recovery::recovered_copy_path(&path);
        let copy_name = copy_path.file_name().unwrap_or_default().to_string_lossy();
        match MapFile::create(&copy_path, &copy, None) {
            Ok(_) => {
                let _ = remove_journal(&found.path);
                self.app.recent.add(&copy_path);
                self.win.home.forget(&copy_path);
                let skipped = match skipped {
                    0 => String::new(),
                    1 => " (1 change no longer applied)".into(),
                    n => format!(" ({n} changes no longer applied)"),
                };
                tab.editor.info(format!(
                    "Saved the recovered changes as “{copy_name}”{skipped}"
                ));
            }
            Err(err) => tab
                .editor
                .error(format!("Couldn’t save “{copy_name}”: {err}")),
        }
    }

    /// Opens a Map never saved from the journal a crash left, in a new tab.
    pub(super) fn recover_unsaved(&mut self, found: Found) {
        let JournalBase::Unsaved { map: view } = found.recovered.base.clone() else {
            return;
        };
        let mut map = match Map::from_view(*view) {
            Ok(map) => map,
            Err(err) => {
                self.notify_error(format!("Couldn’t recover the unsaved Map: {err}"));
                return;
            }
        };
        found.recovered.replay_lenient(&mut map);
        let mut tab = self.new_tab(map, Document::recovered());
        if let Err(err) = tab
            .recovery
            .resume(&tab.document, &tab.editor.map, &found.path, &[])
        {
            tab.editor
                .error(format!("Couldn’t write the Recovery journal: {err}"));
        } else {
            tab.editor.info("Recovered an unsaved Map");
        }
        self.push_tab(tab);
    }

    /// Writes the Ops applied this frame, in every tab, to its Recovery journal once they are
    /// due, and wakes up in time for that.
    pub(super) fn journal(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        // What falls due within a frame is written now: egui repaints at once for so short a
        // wait anyway.
        let frame = Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
        for tab in &mut self.win.tabs {
            tab.recovery.record(tab.editor.take_applied(), now);
            if let Err(err) = tab
                .recovery
                .flush_if_due(now + frame, &tab.document, &tab.editor.map)
            {
                tab.editor
                    .error(format!("Couldn’t write the Recovery journal: {err}"));
            }
            if let Some(wait) = tab.recovery.due_in(now) {
                ctx.request_repaint_after(wait);
            }
        }
    }

    /// Opens each `.ssmap` file dropped onto the window in a tab of its own.
    pub(super) fn open_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            let path = file.path();
            if path
                .extension()
                .is_some_and(|extension| extension == document::EXTENSION)
            {
                self.open_path(path);
            }
        }
    }

    /// Saves to the open file, or asks where to save a Map that has never been saved.
    pub fn save(&mut self) -> bool {
        // A gesture still open would otherwise reach the journal after the save holding it.
        let Some(tab) = self.tab_mut() else {
            return false;
        };
        tab.editor.end_open_gesture();
        tab.editor.embed_used_assets();
        let preview = self.preview_png();
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return false;
        };
        match tab.document.save(&tab.editor.map, preview.as_deref()) {
            Some(result) => self.saved(result),
            None => {
                self.show_save_dialog();
                false
            }
        }
    }

    /// Saves a copy at `path` that becomes the active tab's file.
    pub fn save_as_path(&mut self, path: &Path) -> bool {
        let Some(tab) = self.tab_mut() else {
            return false;
        };
        tab.editor.end_open_gesture();
        tab.editor.embed_used_assets();
        let preview = self.preview_png();
        let path = document::with_extension(path.to_path_buf());
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return false;
        };
        let result = tab
            .document
            .save_as(&path, &tab.editor.map, preview.as_deref());
        self.saved(result)
    }

    fn saved(&mut self, result: Result<(), ssmap_core::file::FileError>) -> bool {
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return false;
        };
        match result {
            Ok(()) => {
                // The save holds every change so far; the journal starts over.
                tab.editor.take_applied();
                tab.recovery.reset();
                if let Some(path) = tab.document.path() {
                    self.app.recent.add(path);
                    self.win.home.forget(path);
                }
                tab.editor.info(format!("Saved “{}”", tab.document.name()));
                true
            }
            Err(err) => {
                let name = tab.document.name();
                tab.editor.error(format!("Couldn’t save “{name}”: {err}"));
                false
            }
        }
    }

    pub(super) fn show_open_dialog(&mut self) {
        let directory = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|tab| tab.document.path())
            .and_then(Path::parent);
        self.win.open_dialog.open_map(&self.app.egui_ctx, directory);
    }

    pub(super) fn show_save_dialog(&mut self) {
        let Some(tab) = self.win.tabs.get(self.win.active) else {
            return;
        };
        let directory = tab.document.path().and_then(Path::parent);
        self.win
            .save_dialog
            .save_map(&self.app.egui_ctx, directory, &tab.document.name());
    }

    /// The preview PNG stored with each save, rendered like the screen.
    fn preview_png(&mut self) -> Option<Vec<u8>> {
        let render_state = self.app.render_state.as_ref()?;
        let tab = self.win.tabs.get_mut(self.win.active)?;
        let image = canvas_view::render_whole_canvas(
            render_state,
            &mut tab.editor,
            tab.canvas_view.cache(),
            PREVIEW_SIZE,
        )?;
        let mut png = Vec::new();
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut png),
            &image.pixels,
            image.width,
            image.height,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
        Some(png)
    }

    /// Offers a Recovery journal left by a crash: the active tab's Map file's, or else one of
    /// a Map never saved, which opens in a tab of its own.
    pub(super) fn recovery_dialog(&mut self, ctx: &egui::Context) {
        let tab_asking = self
            .win
            .tabs
            .get(self.win.active)
            .is_some_and(|tab| tab.recovery_prompt.is_some() || tab.pending.is_some());
        // The offer of a Map never saved is shared, and shown in the main window only.
        if self.win.main && !tab_asking && !self.win.close.is_awaiting_save() {
            self.app.unsaved.offer_next();
        }
        let Some(tab) = self.win.tabs.get(self.win.active) else {
            self.unsaved_journal_dialog(ctx);
            return;
        };
        let Some(prompt) = &tab.recovery_prompt else {
            self.unsaved_journal_dialog(ctx);
            return;
        };
        let name = tab.document.name();
        let (text, recover) = match prompt {
            RecoveryPrompt::Replay(_) => (
                format!(
                    "“{name}” wasn’t closed properly. Recover the changes made since it was \
                     last saved?"
                ),
                "Recover Changes",
            ),
            RecoveryPrompt::Copy { reason, .. } => (
                format!(
                    "“{name}” wasn’t closed properly, but its unsaved changes can’t be \
                     recovered into it. {reason} Save them as a separate copy? “{name}” is \
                     left as it is."
                ),
                "Save Recovered Copy",
            ),
        };
        let Some(choice) = recovery_modal(ctx, &text, recover) else {
            return;
        };
        let Some(prompt) = self.tab_mut().and_then(|tab| tab.recovery_prompt.take()) else {
            return;
        };
        match (prompt, choice) {
            (_, RecoveryChoice::Later) => {}
            (RecoveryPrompt::Replay(found), RecoveryChoice::Recover) => {
                self.replay_recovery(found);
            }
            (RecoveryPrompt::Copy { found, .. }, RecoveryChoice::Recover) => {
                self.save_recovered_copy(found);
            }
            (
                RecoveryPrompt::Replay(found) | RecoveryPrompt::Copy { found, .. },
                RecoveryChoice::Discard,
            ) => {
                if let Err(err) = remove_journal(&found.path) {
                    self.notify_error(format!("Couldn’t delete the unsaved changes: {err}"));
                }
            }
        }
    }

    /// Offers the journal of a Map never saved that a crash left: recovering it opens a tab.
    fn unsaved_journal_dialog(&mut self, ctx: &egui::Context) {
        if !self.win.main || !self.app.unsaved.is_asking() {
            return;
        }
        let text = "A Map that was never saved wasn’t closed properly. Recover it?";
        let Some(choice) = recovery_modal(ctx, text, "Recover") else {
            return;
        };
        let Some(found) = self.app.unsaved.take_prompt() else {
            return;
        };
        match choice {
            RecoveryChoice::Recover => self.recover_unsaved(found),
            RecoveryChoice::Discard => {
                if let Err(err) = remove_journal(&found.path) {
                    self.notify_error(format!("Couldn’t delete the unsaved changes: {err}"));
                }
            }
            RecoveryChoice::Later => {}
        }
    }
}

/// Why a Map file's journal is offered only as a copy, when its save generation differs.
const CHANGED_SINCE: &str = "The file was replaced or saved elsewhere since they were made.";

/// Offers the Recovery journal a crash left beside the Map file just opened in `tab`: to
/// replay if it was written against this save, or else only as a separate copy.
fn find_recovery(tab: &mut MapTab, path: &Path) {
    match Recovery::find_beside(path) {
        Ok(Some(found)) => {
            let generation = tab.document.save_generation().unwrap_or_default();
            tab.recovery_prompt = Some(if found.recovered.matches(generation) {
                RecoveryPrompt::Replay(found)
            } else {
                RecoveryPrompt::Copy {
                    found,
                    reason: CHANGED_SINCE.into(),
                }
            });
        }
        Ok(None) => {}
        Err(err) => tab.editor.error(format!(
            "Couldn’t read the unsaved changes to “{}”: {err}",
            tab.document.name()
        )),
    }
}

pub(crate) enum RecoveryPrompt {
    /// The tab's Map file's journal, written against its current save.
    Replay(Found),
    /// The tab's Map file's journal, written against another save of it.
    Copy { found: Found, reason: String },
}

/// The answer to a Recovery question.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RecoveryChoice {
    Recover,
    Discard,
    /// Esc: leave the unsaved changes on disk and ask again next time.
    Later,
}

/// Shows the Recovery dialog with `text` and a `recover` button.
fn recovery_modal(ctx: &egui::Context, text: &str, recover: &str) -> Option<RecoveryChoice> {
    let mut choice = None;
    let shown = dialog::show(ctx, "recovery", "Recover unsaved changes", 420.0, |ui| {
        ui.label(text);
        dialog::buttons(ui, |ui| {
            if dialog::primary_button(ui, true, recover).clicked() {
                choice = Some(RecoveryChoice::Recover);
            }
            if ui.button("Discard").clicked() {
                choice = Some(RecoveryChoice::Discard);
            }
        });
    });
    if choice.is_none() {
        if shown.cancelled {
            choice = Some(RecoveryChoice::Later);
        } else if dialog::enter_confirms(ctx) {
            choice = Some(RecoveryChoice::Recover);
        }
    }
    choice
}

/// Runs `on_deleted` (closing the tab, forgetting the file) only if the Trash move succeeded.
pub(super) fn after_trash(
    trashed: Result<(), String>,
    on_deleted: impl FnOnce(),
) -> Result<(), String> {
    trashed.map(|()| on_deleted())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_trash_keeps_the_tab_and_recent_entry() {
        let mut cleaned = false;
        let result = after_trash(Err("denied".into()), || cleaned = true);
        assert_eq!(result, Err("denied".to_owned()));
        assert!(!cleaned);
    }

    #[test]
    fn confirmed_delete_closes_the_tab_and_forgets_the_file() {
        let mut cleaned = false;
        assert_eq!(after_trash(Ok(()), || cleaned = true), Ok(()));
        assert!(cleaned);
    }
}
