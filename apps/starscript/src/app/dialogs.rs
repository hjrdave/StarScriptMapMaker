//! The dialogs a window shows over its content, and the PNG export they start.

use super::*;
use crate::{dialog, export_run, paste_dialog, tools};

impl MapMakerApp {
    /// File dialogs and the unsaved-changes question.
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        if let Polled::Chosen(folder) = self.win.pack_dialog.poll() {
            self.add_pack(ctx, folder);
        }
        self.finish_pack_install();
        if let Polled::Chosen(path) = self.win.open_dialog.poll() {
            self.open_path(&path);
        }
        match self.win.save_dialog.poll() {
            Polled::Chosen(path) => {
                let saved = self.save_as_path(&path);
                match self.win.close.take_awaiting_save() {
                    Some((index, pending)) if saved && index == self.win.active => {
                        self.finish(pending, ctx);
                    }
                    Some(_) => self.cancel_quit(),
                    None => {}
                }
            }
            Polled::Cancelled if self.win.close.is_awaiting_save() => {
                // The Save As dialog was closed without saving: the question is cancelled.
                self.win.close.take_awaiting_save();
                self.cancel_quit();
            }
            _ => {}
        }

        if let Some(canvas) = self.win.new_map_dialog.show(ctx) {
            let template = self.win.new_map_dialog.chosen_template();
            self.new_map_from(template, canvas);
        }
        if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
            match self.win.export.show_dialog(ctx, &tab.editor.map.canvas) {
                Some(DialogAction::Save(settings)) => tab.editor.map.export_settings = settings,
                Some(DialogAction::Export(settings)) => {
                    tab.editor.map.export_settings = settings;
                    self.export_from_dialog();
                }
                Some(DialogAction::DrawRegion(settings)) => {
                    tab.editor.map.export_settings = settings;
                    tab.editor.active_tool = tools::ToolId::ExportRegion;
                }
                None => {}
            }
        }
        self.export_progress(ctx);
        self.recovery_dialog(ctx);
        self.unsaved_dialog(ctx);
        self.delete_dialog(ctx);
        self.pack_delete_dialog(ctx);
        self.paste_dialog(ctx);
    }

    /// Lists what the copied layers need that this Map lacks and asks whether to paste anyway.
    /// Esc and Cancel paste nothing.
    fn paste_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.win.paste_prompt.clone() else {
            return;
        };
        let Some(choice) = paste_dialog::show(ctx, &prompt.gaps) else {
            return;
        };
        self.win.paste_prompt = None;
        if choice == paste_dialog::Choice::Paste
            && let Some(tab) = self.win.tabs.get_mut(self.win.active)
        {
            tab.editor.paste(prompt.at);
        }
    }

    /// Confirms moving a Map file to the Trash. Cancel is the default: Enter and Esc cancel.
    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = self.win.delete_prompt.clone() else {
            return;
        };
        let name = path.file_stem().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let open_tab = self.tab_for(&path);
        let mut delete = false;
        let mut cancel = false;
        let shown = dialog::show(
            ctx,
            "delete_map",
            &format!("Delete “{name}”?"),
            380.0,
            |ui| {
                ui.label(format!(
                    "The file is moved to the {}, so you can put it back from there.",
                    file_actions::TRASH_NAME
                ));
                if open_tab.is_some() {
                    ui.add_space(6.0);
                    ui.label("It is open in a tab, which closes without saving.");
                }
                dialog::buttons(ui, |ui| {
                    let danger = theme::palette(ui.ctx()).danger;
                    let button =
                        egui::Button::new(egui::RichText::new("Delete").strong()).fill(danger);
                    if ui.add(button).clicked() {
                        delete = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            },
        );
        if shown.cancelled || dialog::enter_confirms(ctx) {
            cancel = true;
        }
        if delete {
            self.win.delete_prompt = None;
            // Trash first: if it fails, the open tab and its unsaved edits stay.
            let trashed = file_actions::move_to_trash(&path).map_err(|err| err.to_string());
            let result = after_trash(trashed, || {
                if let Some(index) = open_tab {
                    self.remove_tab(index);
                }
                self.app.recent.remove(&path);
                self.win.home.forget(&path);
            });
            if let Err(err) = result {
                self.notify_error(format!("Couldn’t delete “{name}”: {err}"));
            }
        } else if cancel {
            self.win.delete_prompt = None;
        }
    }

    pub(super) fn open_export_dialog(&mut self) {
        let Some(tab) = self.win.tabs.get(self.win.active) else {
            return;
        };
        let directory = tab.document.path().and_then(Path::parent);
        self.win.export.open_dialog(
            &tab.editor.map.export_settings,
            directory.map(Path::to_path_buf),
        );
    }

    /// The folder the dialog's Export button writes into: the one typed in the dialog.
    fn export_from_dialog(&mut self) {
        let directory = self.win.export.typed_directory();
        let directory = if directory.as_os_str().is_empty() {
            self.document_directory().unwrap_or_default()
        } else {
            directory
        };
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        let settings = &mut tab.editor.map.export_settings;
        if settings.file_name.as_deref().is_none_or(str::is_empty) {
            settings.file_name = Some(format!("{}.png", tab.document.name()));
        }
        tab.export_directory = Some(directory);
        self.start_export();
    }

    fn document_directory(&self) -> Option<PathBuf> {
        self.tab()?
            .document
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    /// Where Re-export would write, from the stored settings; `None` before a first export.
    pub(super) fn re_export_target(&self) -> Option<PathBuf> {
        let tab = self.tab()?;
        let directory = tab
            .export_directory
            .clone()
            .or_else(|| self.document_directory())?;
        export_run::export_path(&tab.editor.map.export_settings, &directory)
    }

    /// Runs an export of the active tab from the stored settings without opening the dialog.
    pub(super) fn re_export(&mut self) {
        if self.re_export_target().is_none() {
            if let Some(tab) = self.tab_mut() {
                tab.editor
                    .error("Export the Map once with Export PNG… first");
            }
            return;
        }
        let directory = self.document_directory();
        if let Some(tab) = self.tab_mut().filter(|tab| tab.export_directory.is_none()) {
            tab.export_directory = directory;
        }
        self.start_export();
    }

    /// Starts exporting the active tab to the stored file name in `export_directory`. Only
    /// one export runs at a time.
    fn start_export(&mut self) {
        if self.win.export.is_running() {
            return;
        }
        if self.app.render_state.is_none() {
            if let Some(tab) = self.tab_mut() {
                tab.editor.error("Export needs a graphics adapter");
            }
            return;
        }
        let Some(dest) = self.re_export_target() else {
            if let Some(tab) = self.tab_mut() {
                tab.editor.error("The export has no file name");
            }
            return;
        };
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        let settings = tab.editor.map.export_settings.clone();
        let output = settings.credits.output(&tab.editor.credit_lines());
        let mut meta =
            PngMeta::default().with_software("StarScript Map Maker", env!("CARGO_PKG_VERSION"));
        if let Some(chunk) = &output.chunk {
            meta = meta.with_credits(CREDITS_CHUNK_KEYWORD, chunk);
        }
        match ExportJob::new(&tab.editor.map, &settings, meta, &dest) {
            Ok(job) => self
                .win
                .export
                .start(ExportRun::new(job, dest, output.file)),
            Err(err) => tab.editor.error(format!("Export failed: {err}")),
        }
    }

    /// Steps the running export once and shows its progress dialog.
    fn export_progress(&mut self, ctx: &egui::Context) {
        let Some(run) = self.win.export.run_mut() else {
            return;
        };
        let Some(render_state) = self.app.render_state.as_ref() else {
            return;
        };
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        let editor = &mut tab.editor;
        let cache = tab.canvas_view.cache();
        let outcome =
            run.frame(
                |job| match canvas_view::step_export(render_state, editor, cache, job) {
                    Some(step) => step.map_err(|err| err.to_string()),
                    None => Err("no graphics adapter".to_owned()),
                },
            );
        let dest = run.dest().to_path_buf();
        match outcome {
            Outcome::Running(progress) => {
                if export_run::show_progress(ctx, &dest, progress, run.time_left()) {
                    run.cancel();
                    self.win.export.end();
                    tab.editor.info("Export cancelled");
                } else {
                    // Keep stepping even when no input arrives.
                    ctx.request_repaint();
                }
            }
            Outcome::Finished(size) => {
                self.win.export.end();
                tab.editor.info(format!(
                    "Exported {} × {} px to {}",
                    size.width,
                    size.height,
                    dest.display()
                ));
            }
            Outcome::Failed(message) => {
                self.win.export.end();
                tab.editor.error(format!("Export failed: {message}"));
            }
        }
    }
}
