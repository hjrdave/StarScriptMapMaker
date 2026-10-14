//! StarScript Map Maker's editor. The binary in `main.rs` only opens a window around
//! [`MapMakerApp`], so UI tests can drive the same app headlessly.

mod canvas_view;
pub mod document;
pub mod editor;
pub mod handles;
mod layer_tree;
mod properties;
mod recent;
pub mod recovery;
pub mod tools;

use eframe::{egui, egui_wgpu};
use egui_file_dialog::{DialogState, FileDialog, Filter};
use ssmap_core::file::{JournalBase, MapFile, PREVIEW_SIZE, remove_journal};
use ssmap_core::{Canvas, DistanceUnit, Map, Scale};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::canvas_view::CanvasView;
use crate::document::Document;
use crate::editor::Editor;
use crate::layer_tree::LayerTree;
use crate::properties::PropertiesPanel;
use crate::recent::RecentFiles;
use crate::recovery::{Found, Recovery};
use crate::tools::ToolId;

/// The Map file to open at launch, from the command-line arguments after the program name.
///
/// Windows (the installer's file association) and Linux (the `.desktop` file's `%F`) pass a
/// double-clicked file this way. Flags are skipped, including the `-psn_…` process serial
/// number older macOS versions add.
pub fn launch_file(args: impl IntoIterator<Item = OsString>) -> Option<PathBuf> {
    let path = PathBuf::from(
        args.into_iter()
            .find(|arg| !arg.to_string_lossy().starts_with('-'))?,
    );
    // Absolute, so Recent Files still finds it from another working directory.
    Some(std::path::absolute(&path).unwrap_or(path))
}

pub struct MapMakerApp {
    editor: Editor,
    document: Document,
    layer_tree: LayerTree,
    properties: PropertiesPanel,
    canvas_view: CanvasView,
    recent: RecentFiles,
    open_dialog: FileDialog,
    save_dialog: FileDialog,
    /// What to do once the user decides what to do with unsaved changes.
    pending: Option<Pending>,
    recovery: Recovery,
    /// A Recovery journal left by a crash, awaiting the user's decision.
    recovery_prompt: Option<RecoveryPrompt>,
    /// Journals of Maps never saved left by a crash, oldest first, offered one at a time
    /// while the open Map has no unsaved changes.
    unsaved_found: Vec<Found>,
    /// Set once the user has chosen to close despite unsaved changes.
    close_allowed: bool,
    /// `None` without a GPU, as in UI tests; saves then store no preview.
    render_state: Option<egui_wgpu::RenderState>,
    /// The window title last sent, so it is only sent when it changes.
    title: String,
}

impl MapMakerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::with_recovery_dir(cc, Recovery::default_unsaved_dir())
    }

    /// Starts with a blank Map, keeping journals of Maps never saved in `recovery_dir`
    /// (`None` turns them off) and offering any a crash left there.
    pub fn with_recovery_dir(
        cc: &eframe::CreationContext<'_>,
        recovery_dir: Option<PathBuf>,
    ) -> Self {
        // Until the New Map dialog exists, every session starts with a blank 4096x3072 Map.
        let map = Map::new(Canvas::new(
            4096,
            3072,
            Scale::new(0.25, DistanceUnit::Miles),
        ));
        Self::with_map(cc, map, recovery_dir)
    }

    pub fn with_map(
        cc: &eframe::CreationContext<'_>,
        map: Map,
        recovery_dir: Option<PathBuf>,
    ) -> Self {
        let gpu = match cc.wgpu_render_state.as_ref() {
            Some(render_state) => {
                canvas_view::install(render_state);
                true
            }
            None => false,
        };
        let map_files = || -> FileDialog {
            FileDialog::new()
                .add_file_filter(
                    "Map files",
                    Filter::new(|path: &Path| {
                        path.extension()
                            .is_some_and(|ext| ext == document::EXTENSION)
                    }),
                )
                .default_file_filter("Map files")
                .add_save_extension("Map file", document::EXTENSION)
                .default_save_extension("Map file")
        };
        let recovery = Recovery::new(recovery_dir);
        let mut unsaved_found = recovery.find_unsaved();
        unsaved_found.reverse();
        Self {
            editor: Editor::new(map),
            document: Document::default(),
            layer_tree: LayerTree::default(),
            properties: PropertiesPanel::default(),
            canvas_view: CanvasView::new(gpu),
            recent: RecentFiles::load(cc.storage),
            open_dialog: map_files().title("Open Map"),
            save_dialog: map_files().title("Save Map As"),
            pending: None,
            recovery,
            recovery_prompt: None,
            unsaved_found,
            close_allowed: false,
            render_state: cc.wgpu_render_state.clone(),
            title: String::new(),
        }
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Recently opened or saved Map files, newest first.
    pub fn recent_files(&self) -> &[PathBuf] {
        self.recent.paths()
    }

    /// The Recovery journal being written for the open Map, once there is one.
    pub fn recovery_journal(&self) -> Option<&Path> {
        self.recovery.journal_path()
    }

    /// Whether edits are waiting to be written to the Recovery journal.
    pub fn has_unjournaled_changes(&self) -> bool {
        self.recovery.has_pending()
    }

    /// The window title, as last sent to the window.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Opens a Map file in place of the current Map, without asking about unsaved changes.
    pub fn open_path(&mut self, path: &Path) -> bool {
        match Document::open(path) {
            Ok((document, map)) => {
                self.recovery.reset();
                self.recovery_prompt = None;
                self.replace(document, map);
                self.recent.add(path);
                self.editor
                    .info(format!("Opened “{}”", self.document.name()));
                self.find_recovery(path);
                true
            }
            Err(err) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.editor.error(format!("Couldn’t open “{name}”: {err}"));
                false
            }
        }
    }

    fn replace(&mut self, document: Document, map: Map) {
        self.document = document;
        self.editor = Editor::new(map);
        self.layer_tree = LayerTree::default();
        self.properties = PropertiesPanel::default();
        self.canvas_view.refit();
    }

    /// Opens a Map file, first asking whether to discard unsaved changes.
    fn request_open(&mut self, path: PathBuf) {
        if self.document.has_unsaved_changes(&self.editor.map) {
            self.pending = Some(Pending::Open(path));
        } else {
            self.open_path(&path);
        }
    }

    /// Offers the Recovery journal a crash left beside the Map file just opened: to replay
    /// if it was written against this save, or else only as a separate copy.
    fn find_recovery(&mut self, path: &Path) {
        match Recovery::find_beside(path) {
            Ok(Some(found)) => {
                let generation = self.document.save_generation().unwrap_or_default();
                self.recovery_prompt = Some(if found.recovered.matches(generation) {
                    RecoveryPrompt::Replay(found)
                } else {
                    RecoveryPrompt::Copy {
                        found,
                        reason: CHANGED_SINCE.into(),
                    }
                });
            }
            Ok(None) => {}
            Err(err) => self.editor.error(format!(
                "Couldn’t read the unsaved changes to “{}”: {err}",
                self.document.name()
            )),
        }
    }

    /// Applies a journal's changes to the Map file it was written against.
    fn replay_recovery(&mut self, found: Found) {
        let ops = found.recovered.ops.clone();
        match found.recovered.clone().replay(&mut self.editor.map) {
            Ok(()) => {
                if let Err(err) =
                    self.recovery
                        .resume(&self.document, &self.editor.map, &found.path, &ops)
                {
                    self.editor
                        .error(format!("Couldn’t write the Recovery journal: {err}"));
                    return;
                }
                self.editor.info(format!(
                    "Recovered the unsaved changes to “{}”",
                    self.document.name()
                ));
            }
            Err(err) => {
                self.recovery_prompt = Some(RecoveryPrompt::Copy {
                    found,
                    reason: format!("The unsaved changes no longer apply to it ({err})."),
                });
            }
        }
    }

    /// Saves a journal's changes, applied as far as they go, as a copy beside the open Map
    /// file, which is left as it is.
    fn save_recovered_copy(&mut self, found: Found) {
        let Some(path) = self.document.path().map(Path::to_path_buf) else {
            return;
        };
        let mut copy = self.editor.map.clone();
        let skipped = found.recovered.replay_lenient(&mut copy);
        let copy_path = recovery::recovered_copy_path(&path);
        let copy_name = copy_path.file_name().unwrap_or_default().to_string_lossy();
        match MapFile::create(&copy_path, &copy, None) {
            Ok(_) => {
                let _ = remove_journal(&found.path);
                self.recent.add(&copy_path);
                let skipped = match skipped {
                    0 => String::new(),
                    1 => " (1 change no longer applied)".into(),
                    n => format!(" ({n} changes no longer applied)"),
                };
                self.editor.info(format!(
                    "Saved the recovered changes as “{copy_name}”{skipped}"
                ));
            }
            Err(err) => self
                .editor
                .error(format!("Couldn’t save “{copy_name}”: {err}")),
        }
    }

    /// Opens a Map never saved from the journal a crash left.
    fn recover_unsaved(&mut self, found: Found) {
        let JournalBase::Unsaved { map: view } = found.recovered.base.clone() else {
            return;
        };
        let mut map = match Map::from_view(view) {
            Ok(map) => map,
            Err(err) => {
                self.editor
                    .error(format!("Couldn’t recover the unsaved Map: {err}"));
                return;
            }
        };
        found.recovered.replay_lenient(&mut map);
        self.replace(Document::recovered(), map);
        if let Err(err) = self
            .recovery
            .resume(&self.document, &self.editor.map, &found.path, &[])
        {
            self.editor
                .error(format!("Couldn’t write the Recovery journal: {err}"));
            return;
        }
        self.editor.info("Recovered an unsaved Map");
    }

    /// Writes the Ops applied this frame to the Recovery journal once they are due, and
    /// wakes up in time for that.
    fn journal(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.recovery.record(self.editor.take_applied(), now);
        // What falls due within a frame is written now: egui repaints at once for so short a
        // wait anyway.
        let frame = Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
        if let Err(err) = self
            .recovery
            .flush_if_due(now + frame, &self.document, &self.editor.map)
        {
            self.editor
                .error(format!("Couldn’t write the Recovery journal: {err}"));
        }
        if let Some(wait) = self.recovery.due_in(now) {
            ctx.request_repaint_after(wait);
        }
    }

    /// A close with unsaved changes asks first; a clean close deletes the Recovery journal.
    fn handle_close(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) || self.close_allowed {
            return;
        }
        if self.document.has_unsaved_changes(&self.editor.map) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Close);
        } else {
            self.recovery.reset();
        }
    }

    /// Closes the window, discarding unsaved changes and their Recovery journal.
    fn close(&mut self, ctx: &egui::Context) {
        self.close_allowed = true;
        self.recovery.reset();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Whether the user chose to close the window.
    pub fn is_closing(&self) -> bool {
        self.close_allowed
    }

    /// Saves to the open file, or asks where to save a Map that has never been saved.
    pub fn save(&mut self) -> bool {
        // A gesture still open would otherwise reach the journal after the save holding it.
        self.editor.end_gesture();
        let preview = self.preview_png();
        match self.document.save(&self.editor.map, preview.as_deref()) {
            Some(result) => self.saved(result),
            None => {
                self.show_save_dialog();
                false
            }
        }
    }

    /// Saves a copy at `path` that becomes the open file.
    pub fn save_as_path(&mut self, path: &Path) -> bool {
        self.editor.end_gesture();
        let preview = self.preview_png();
        let path = document::with_extension(path.to_path_buf());
        let result = self
            .document
            .save_as(&path, &self.editor.map, preview.as_deref());
        self.saved(result)
    }

    fn saved(&mut self, result: Result<(), ssmap_core::file::FileError>) -> bool {
        match result {
            Ok(()) => {
                // The save holds every change so far; the journal starts over.
                self.editor.take_applied();
                self.recovery.reset();
                if let Some(path) = self.document.path().map(Path::to_path_buf) {
                    self.recent.add(&path);
                }
                self.editor
                    .info(format!("Saved “{}”", self.document.name()));
                true
            }
            Err(err) => {
                let name = self.document.name();
                self.editor.error(format!("Couldn’t save “{name}”: {err}"));
                false
            }
        }
    }

    fn show_open_dialog(&mut self) {
        if let Some(dir) = self.document.path().and_then(Path::parent) {
            self.open_dialog.config_mut().initial_directory = dir.to_path_buf();
        }
        self.open_dialog.pick_file();
    }

    fn show_save_dialog(&mut self) {
        let config = self.save_dialog.config_mut();
        if let Some(dir) = self.document.path().and_then(Path::parent) {
            config.initial_directory = dir.to_path_buf();
        }
        config.default_file_name = self.document.name();
        self.save_dialog.save_file();
    }

    /// The preview PNG stored with each save, rendered like the screen.
    fn preview_png(&self) -> Option<Vec<u8>> {
        let render_state = self.render_state.as_ref()?;
        let image = canvas_view::render_whole_canvas(render_state, &self.editor.map, PREVIEW_SIZE)?;
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

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui.button("Open…").clicked() {
                self.show_open_dialog();
            }
            ui.menu_button("Open Recent", |ui| {
                if let Some(path) = self.recent.menu(ui) {
                    ui.close();
                    if path.is_file() {
                        self.request_open(path);
                    } else {
                        self.recent.remove(&path);
                        self.editor
                            .error(format!("“{}” no longer exists", path.display()));
                    }
                }
            });
            ui.separator();
            if ui.button("Save").clicked() {
                self.save();
            }
            if ui.button("Save As…").clicked() {
                self.show_save_dialog();
            }
        });
    }

    /// File dialogs and the unsaved-changes question.
    fn dialogs(&mut self, ctx: &egui::Context) {
        self.open_dialog.update(ctx);
        if let Some(path) = self.open_dialog.take_picked() {
            self.request_open(path);
        }
        self.save_dialog.update(ctx);
        if let Some(path) = self.save_dialog.take_picked() {
            self.save_as_path(&path);
        }

        self.recovery_dialog(ctx);
        self.unsaved_dialog(ctx);
    }

    /// Asks what to do with unsaved changes before opening another Map or closing.
    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        let has_file = self.document.path().is_some();
        let name = self.document.name();
        let question = match pending {
            Pending::Open(_) => format!("Discard the unsaved changes to “{name}”?"),
            Pending::Close => format!("Save the changes to “{name}” before closing?"),
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("unsaved_changes")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label(question);
            ui.horizontal(|ui| {
                if has_file && ui.button("Save").clicked() {
                    choice = Some(UnsavedChoice::Save);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(UnsavedChoice::Discard);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(UnsavedChoice::Cancel);
                }
            });
        });
        let Some(choice) = choice else {
            return;
        };
        self.pending = None;
        let proceed = match choice {
            UnsavedChoice::Save => self.save(),
            UnsavedChoice::Discard => true,
            UnsavedChoice::Cancel => false,
        };
        if proceed {
            match pending {
                Pending::Open(path) => {
                    self.open_path(&path);
                }
                Pending::Close => self.close(ctx),
            }
        }
    }

    /// Offers a Recovery journal left by a crash: the open Map file's, or else one of a Map
    /// never saved while the open Map has no unsaved changes.
    fn recovery_dialog(&mut self, ctx: &egui::Context) {
        if self.recovery_prompt.is_none()
            && self.pending.is_none()
            && !self.document.has_unsaved_changes(&self.editor.map)
        {
            self.recovery_prompt = self.unsaved_found.pop().map(RecoveryPrompt::Unsaved);
        }
        let Some(prompt) = &self.recovery_prompt else {
            return;
        };
        let name = self.document.name();
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
            RecoveryPrompt::Unsaved(_) => (
                "A Map that was never saved wasn’t closed properly. Recover it?".into(),
                "Recover",
            ),
        };
        let mut recovered = None;
        egui::Modal::new(egui::Id::new("recovery")).show(ctx, |ui| {
            ui.heading("Recover unsaved changes");
            ui.label(text);
            ui.horizontal(|ui| {
                if ui.button(recover).clicked() {
                    recovered = Some(true);
                }
                if ui.button("Discard Changes").clicked() {
                    recovered = Some(false);
                }
            });
        });
        let Some(recovered) = recovered else {
            return;
        };
        let prompt = self.recovery_prompt.take().expect("shown above");
        match (prompt, recovered) {
            (RecoveryPrompt::Replay(found), true) => self.replay_recovery(found),
            (RecoveryPrompt::Copy { found, .. }, true) => self.save_recovered_copy(found),
            (RecoveryPrompt::Unsaved(found), true) => self.recover_unsaved(found),
            (
                RecoveryPrompt::Replay(found)
                | RecoveryPrompt::Copy { found, .. }
                | RecoveryPrompt::Unsaved(found),
                false,
            ) => {
                if let Err(err) = remove_journal(&found.path) {
                    self.editor
                        .error(format!("Couldn’t delete the unsaved changes: {err}"));
                }
            }
        }
    }

    /// Shows the file name in the window title, with a dot while there are unsaved changes.
    fn update_title(&mut self, ctx: &egui::Context) {
        let dirty = if self.document.has_unsaved_changes(&self.editor.map) {
            " •"
        } else {
            ""
        };
        let title = format!("{}{dirty} — StarScript Map Maker", self.document.name());
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Text fields keep their own undo and keys.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        let shortcut = |modifiers, key| KeyboardShortcut::new(modifiers, key);
        let (open, save, save_as) = ctx.input_mut(|input| {
            // Shift+S first: plain Cmd+S would also match it.
            let save_as =
                input.consume_shortcut(&shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::S));
            let save = input.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::S));
            let open = input.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::O));
            (open, save, save_as)
        });
        if open {
            self.show_open_dialog();
        }
        if save_as {
            self.show_save_dialog();
        } else if save {
            self.save();
        }
        let editor = &mut self.editor;
        ctx.input_mut(|input| {
            if input.consume_shortcut(&shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
                || input.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::Y))
            {
                editor.redo();
            } else if input.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::Z)) {
                editor.undo();
            }
            if input.consume_shortcut(&shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::N)) {
                editor.new_layer();
            }
            if input.consume_key(Modifiers::NONE, Key::Delete)
                || input.consume_key(Modifiers::NONE, Key::Backspace)
            {
                editor.delete_selection();
            }
            if input.consume_key(Modifiers::NONE, Key::Escape) {
                if editor.gesture_open() {
                    editor.cancel_gesture();
                } else {
                    editor.select(Vec::new());
                }
            }
            for tool in ToolId::ALL {
                if input.consume_key(Modifiers::NONE, tool.shortcut()) {
                    editor.active_tool = tool;
                }
            }
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.file_menu(ui);
            ui.separator();
            let editor = &mut self.editor;
            for tool in ToolId::ALL {
                let selected = editor.active_tool == tool;
                if ui
                    .selectable_label(selected, tool.label())
                    .on_hover_text(format!("{} ({:?})", tool.label(), tool.shortcut()))
                    .clicked()
                {
                    editor.active_tool = tool;
                }
            }
            ui.separator();
            let undo_hint = editor.undo_label().map(|l| format!("Undo {l}"));
            let undo = ui.add_enabled(editor.can_undo(), egui::Button::new("Undo"));
            if undo.on_hover_text(undo_hint.unwrap_or_default()).clicked() {
                editor.undo();
            }
            let redo_hint = editor.redo_label().map(|l| format!("Redo {l}"));
            let redo = ui.add_enabled(editor.can_redo(), egui::Button::new("Redo"));
            if redo.on_hover_text(redo_hint.unwrap_or_default()).clicked() {
                editor.redo();
            }
            ui.separator();
            if ui.button("New Layer").clicked() {
                editor.new_layer();
            }
            let has_selection = !editor.selection().is_empty();
            if ui
                .add_enabled(has_selection, egui::Button::new("Delete"))
                .clicked()
            {
                editor.delete_selection();
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(notice) = self.editor.notice() {
                let color = if notice.is_error {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().text_color()
                };
                ui.colored_label(color, &notice.text);
                ui.ctx().request_repaint_after(
                    editor::NOTICE_DURATION.saturating_sub(notice.shown_at.elapsed()),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ppp = f64::from(ui.ctx().pixels_per_point());
                if let Some(zoom) = self.canvas_view.zoom(ppp) {
                    ui.label(format!("{:.0}%", zoom * 100.0));
                }
                if let Some(layer) = self.editor.active_layer() {
                    ui.weak(format!(
                        "Active Layer: {}",
                        layer_tree::node_name(&self.editor.map, layer)
                    ));
                }
            });
        });
    }
}

/// Why a Map file's journal is offered only as a copy, when its save generation differs.
const CHANGED_SINCE: &str = "The file was replaced or saved elsewhere since they were made.";

#[derive(Clone)]
enum Pending {
    Open(PathBuf),
    Close,
}

enum RecoveryPrompt {
    /// The open Map file's journal, written against its current save.
    Replay(Found),
    /// The open Map file's journal, written against another save of it.
    Copy { found: Found, reason: String },
    /// The journal of a Map never saved.
    Unsaved(Found),
}

enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}

impl eframe::App for MapMakerApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.recent.store(storage);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Dialogs take the keyboard while they are open.
        self.handle_close(&ctx);
        let dialog_open = self.pending.is_some()
            || self.recovery_prompt.is_some()
            || *self.open_dialog.state() == DialogState::Open
            || *self.save_dialog.state() == DialogState::Open;
        if !dialog_open {
            self.shortcuts(&ctx);
        }
        self.dialogs(&ctx);
        self.update_title(&ctx);
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("layers")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| self.layer_tree.show(ui, &mut self.editor));
        egui::Panel::right("properties")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| self.properties.show(ui, &mut self.editor));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas_view.show(ui, &mut self.editor));
        self.journal(&ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn launch_file_is_the_first_non_flag_argument() {
        assert_eq!(launch_file(args(&[])), None);
        assert_eq!(launch_file(args(&["-psn_0_12345"])), None);
        let file = launch_file(args(&["-psn_0_12345", "maps/Isle.ssmap", "other.ssmap"]));
        let file = file.unwrap();
        assert!(file.is_absolute());
        assert!(file.ends_with("maps/Isle.ssmap"));
    }
}
