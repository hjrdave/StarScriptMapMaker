//! StarScript Map Maker's editor. The binary in `main.rs` only opens a window around
//! [`MapMakerApp`], so UI tests can drive the same app headlessly.

mod asset_browser;
mod canvas_view;
pub mod credits;
pub mod document;
pub mod editor;
pub mod export_dialog;
pub mod export_run;
pub mod frame_time;
pub mod handles;
mod label_styles;
mod layer_tree;
pub mod new_map;
mod properties;
mod recent;
pub mod recovery;
mod tessellation;
pub mod tools;
pub mod update;

use eframe::{egui, egui_wgpu};
use egui_file_dialog::{DialogState, FileDialog, Filter};
use ssmap_core::asset::{PackLibrary, PackRoots};
use ssmap_core::file::{JournalBase, MapFile, PREVIEW_SIZE, remove_journal};
use ssmap_core::{CREDITS_CHUNK_KEYWORD, Canvas, DistanceUnit, Map, Scale};
use ssmap_render::{ExportJob, PngMeta, SpriteStore};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::asset_browser::AssetBrowser;
use crate::canvas_view::CanvasView;
use crate::credits::Credits;
use crate::document::Document;
use crate::editor::Editor;
use crate::export_dialog::{DialogAction, ExportDialog};
use crate::export_run::{ExportRun, Outcome};
use crate::label_styles::LabelStylesPanel;
use crate::layer_tree::LayerTree;
use crate::new_map::NewMapDialog;
use crate::properties::PropertiesPanel;
use crate::recent::RecentFiles;
use crate::recovery::{Found, Recovery};
use crate::tools::ToolId;
use crate::update::UpdateCheck;

/// How much `[` and `]` shrink or grow the land brush.
const BRUSH_SIZE_STEP: f64 = 1.25;

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
    label_styles: LabelStylesPanel,
    asset_browser: AssetBrowser,
    /// The installed Asset packs' art, shared with the Canvas renderer and picking.
    sprites: SpriteStore,
    canvas_view: CanvasView,
    credits: Credits,
    recent: RecentFiles,
    updates: UpdateCheck,
    new_map_dialog: NewMapDialog,
    export_dialog: ExportDialog,
    /// The PNG export in progress, stepped once per frame behind a modal dialog.
    export_run: Option<ExportRun>,
    /// The folder of the last export, for Re-export when the Map has no file yet.
    export_directory: Option<PathBuf>,
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
    /// Asked for a repaint as background work finishes.
    egui_ctx: egui::Context,
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
        // Every session starts with a Map from the starter template at the default size.
        let canvas = Canvas::new(4096, 3072, Scale::new(0.25, DistanceUnit::Miles));
        let (map, layers, problem) = new_map::start_map(canvas);
        let mut app = Self::with_map(cc, map, recovery_dir);
        app.editor.default_layers = layers;
        if let Some(problem) = problem {
            app.editor.error(problem);
        }
        app
    }

    pub fn with_map(
        cc: &eframe::CreationContext<'_>,
        map: Map,
        recovery_dir: Option<PathBuf>,
    ) -> Self {
        let (library, problems) = PackLibrary::load(&PackRoots::installed());
        for problem in &problems {
            eprintln!("Asset pack problem: {problem}");
        }
        let credits = Credits::new(&library);
        let sprites = SpriteStore::new(Arc::new(library));
        let ctx = cc.egui_ctx.clone();
        sprites.set_repaint(move || ctx.request_repaint());
        let cache = match cc.wgpu_render_state.as_ref() {
            Some(render_state) => canvas_view::install(render_state, sprites.clone()),
            None => None,
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
        let mut editor = Editor::new(map);
        editor.set_sprites(sprites.clone());
        Self {
            editor,
            document: Document::default(),
            layer_tree: LayerTree::default(),
            properties: PropertiesPanel::default(),
            label_styles: LabelStylesPanel::default(),
            asset_browser: AssetBrowser::default(),
            sprites,
            canvas_view: CanvasView::new(cache),
            credits,
            recent: RecentFiles::load(cc.storage),
            updates: UpdateCheck::load(cc.storage),
            new_map_dialog: NewMapDialog::default(),
            export_dialog: ExportDialog::default(),
            export_run: None,
            export_directory: None,
            open_dialog: map_files().title("Open Map"),
            save_dialog: map_files().title("Save Map As"),
            pending: None,
            recovery,
            recovery_prompt: None,
            unsaved_found,
            close_allowed: false,
            render_state: cc.wgpu_render_state.clone(),
            egui_ctx: cc.egui_ctx.clone(),
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

    /// Whether a Label's text box is open on the Canvas.
    pub fn editing_label_text(&self) -> bool {
        self.canvas_view.editing_text()
    }

    /// The screen position of a Canvas point in the Canvas panel, once it has been drawn.
    pub fn canvas_to_screen(&self, point: [f64; 2]) -> Option<egui::Pos2> {
        self.canvas_view.to_screen(point)
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
                let ctx = self.egui_ctx.clone();
                self.editor
                    .start_background_roughness(move || ctx.request_repaint());
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
        let selected_asset = self.editor.selected_asset.take();
        self.editor = Editor::new(map);
        self.editor.set_sprites(self.sprites.clone());
        self.editor.selected_asset = selected_asset;
        self.layer_tree = LayerTree::default();
        self.properties = PropertiesPanel::default();
        self.canvas_view.refit();
    }

    /// Replaces the open Map with a new one from the starter template, without asking about
    /// unsaved changes.
    pub fn new_map(&mut self, canvas: Canvas) {
        let (map, layers, problem) = new_map::start_map(canvas);
        self.recovery.reset();
        self.recovery_prompt = None;
        self.replace(Document::default(), map);
        self.editor.default_layers = layers;
        match problem {
            Some(problem) => self.editor.error(problem),
            None => self.editor.info("Created a new Map"),
        }
    }

    /// Starts a new Map, first asking whether to discard unsaved changes.
    fn request_new(&mut self, canvas: Canvas) {
        if self.document.has_unsaved_changes(&self.editor.map) {
            self.pending = Some(Pending::New(canvas));
        } else {
            self.new_map(canvas);
        }
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
        let mut map = match Map::from_view(*view) {
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
        self.editor.embed_used_assets();
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
        self.editor.embed_used_assets();
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
    fn preview_png(&mut self) -> Option<Vec<u8>> {
        let render_state = self.render_state.as_ref()?;
        let image = canvas_view::render_whole_canvas(render_state, &mut self.editor, PREVIEW_SIZE)?;
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
            if ui.button("New Map…").clicked() {
                self.new_map_dialog.open = true;
                ui.close();
            }
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
            ui.separator();
            if ui.button("Export PNG…").clicked() {
                let directory = self.document.path().and_then(Path::parent);
                self.export_dialog.open_with(
                    &self.editor.map.export_settings,
                    directory.map(Path::to_path_buf),
                );
                ui.close();
            }
            let again = self.re_export_target().is_some() && self.export_run.is_none();
            if ui
                .add_enabled(again, egui::Button::new("Re-export"))
                .clicked()
            {
                self.re_export();
                ui.close();
            }
        });
        ui.menu_button("Help", |ui| {
            if ui.button("Credits…").clicked() {
                self.credits.open = true;
                ui.close();
            }
            ui.separator();
            let mut automatic = self.updates.enabled() == Some(true);
            if ui
                .checkbox(&mut automatic, "Check for updates at startup")
                .changed()
            {
                self.updates.set_enabled(automatic);
            }
            let idle = !self.updates.is_checking();
            if ui
                .add_enabled(idle, egui::Button::new("Check for Updates Now"))
                .clicked()
            {
                self.updates.start(true);
                ui.close();
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

        if let Some(canvas) = self.new_map_dialog.show(ctx) {
            self.request_new(canvas);
        }
        match self.export_dialog.show(ctx, &self.editor.map.canvas) {
            Some(DialogAction::Save(settings)) => self.editor.map.export_settings = settings,
            Some(DialogAction::Export(settings)) => {
                self.editor.map.export_settings = settings;
                self.export_from_dialog();
            }
            None => {}
        }
        self.export_progress(ctx);
        self.recovery_dialog(ctx);
        self.unsaved_dialog(ctx);
    }

    /// The folder the dialog's Export button writes into: the one typed in the dialog.
    fn export_from_dialog(&mut self) {
        let directory = PathBuf::from(self.export_dialog.directory.trim());
        let directory = if directory.as_os_str().is_empty() {
            self.document_directory().unwrap_or_default()
        } else {
            directory
        };
        let settings = &mut self.editor.map.export_settings;
        if settings.file_name.as_deref().is_none_or(str::is_empty) {
            settings.file_name = Some(format!("{}.png", self.document.name()));
        }
        self.export_directory = Some(directory);
        self.start_export();
    }

    fn document_directory(&self) -> Option<PathBuf> {
        self.document
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    /// Where Re-export would write, from the stored settings; `None` before a first export.
    fn re_export_target(&self) -> Option<PathBuf> {
        let directory = self
            .export_directory
            .clone()
            .or_else(|| self.document_directory())?;
        export_run::export_path(&self.editor.map.export_settings, &directory)
    }

    /// Runs an export from the stored settings without opening the dialog.
    fn re_export(&mut self) {
        if self.re_export_target().is_none() {
            self.editor
                .error("Export the Map once with Export PNG… first");
            return;
        }
        if self.export_directory.is_none() {
            self.export_directory = self.document_directory();
        }
        self.start_export();
    }

    /// Starts exporting to the stored file name in `export_directory`.
    fn start_export(&mut self) {
        if self.export_run.is_some() {
            return;
        }
        if self.render_state.is_none() {
            self.editor.error("Export needs a graphics adapter");
            return;
        }
        let Some(dest) = self.re_export_target() else {
            self.editor.error("The export has no file name");
            return;
        };
        let settings = self.editor.map.export_settings.clone();
        let output = settings.credits.output(&self.editor.credit_lines());
        let mut meta =
            PngMeta::default().with_software("StarScript Map Maker", env!("CARGO_PKG_VERSION"));
        if let Some(chunk) = &output.chunk {
            meta = meta.with_credits(CREDITS_CHUNK_KEYWORD, chunk);
        }
        match ExportJob::new(&self.editor.map, &settings, meta, &dest) {
            Ok(job) => self.export_run = Some(ExportRun::new(job, dest, output.file)),
            Err(err) => self.editor.error(format!("Export failed: {err}")),
        }
    }

    /// Steps the running export once and shows its progress dialog.
    fn export_progress(&mut self, ctx: &egui::Context) {
        let Some(run) = self.export_run.as_mut() else {
            return;
        };
        let Some(render_state) = self.render_state.as_ref() else {
            return;
        };
        let editor = &mut self.editor;
        let outcome = run.frame(
            |job| match canvas_view::step_export(render_state, editor, job) {
                Some(step) => step.map_err(|err| err.to_string()),
                None => Err("no graphics adapter".to_owned()),
            },
        );
        let dest = run.dest().to_path_buf();
        match outcome {
            Outcome::Running(progress) => {
                if export_run::show_progress(ctx, &dest, progress) {
                    run.cancel();
                    self.export_run = None;
                    self.editor.info("Export cancelled");
                } else {
                    // Keep stepping even when no input arrives.
                    ctx.request_repaint();
                }
            }
            Outcome::Finished(size) => {
                self.export_run = None;
                self.editor.info(format!(
                    "Exported {} × {} px to {}",
                    size.width,
                    size.height,
                    dest.display()
                ));
            }
            Outcome::Failed(message) => {
                self.export_run = None;
                self.editor.error(format!("Export failed: {message}"));
            }
        }
    }

    /// Asks what to do with unsaved changes before opening another Map or closing.
    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        let has_file = self.document.path().is_some();
        let name = self.document.name();
        let question = match pending {
            Pending::Open(_) | Pending::New(_) => {
                format!("Discard the unsaved changes to “{name}”?")
            }
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
                Pending::New(canvas) => self.new_map(canvas),
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
        // Text fields keep their own undo and keys. The Label text box keeps Escape too,
        // which egui takes its focus away for before this runs.
        if ctx.egui_wants_keyboard_input() || self.canvas_view.editing_text() {
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
                if editor.node_edit().is_some() {
                    editor.delete_selected_nodes();
                } else {
                    editor.delete_selection();
                }
            }
            if input.consume_key(Modifiers::NONE, Key::Escape) {
                if editor.gesture_open() {
                    editor.cancel_gesture();
                } else if editor.node_edit().is_some() {
                    editor.leave_node_edit();
                } else if editor.tool_busy {
                    editor.cancel_tool = true;
                } else {
                    editor.select(Vec::new());
                }
            }
            for tool in ToolId::ALL {
                if input.consume_key(Modifiers::NONE, tool.shortcut()) {
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
            if tools::uses_brush_size(editor.active_tool) {
                ui.add(
                    egui::Slider::new(&mut editor.brush_size, tools::BRUSH_SIZES)
                        .logarithmic(true)
                        .max_decimals(0)
                        .text("Size"),
                )
                .on_hover_text(format!(
                    "Brush diameter in Canvas units ([ and ]). Hold {} to erase.",
                    tools::ERASE_MODIFIER
                ));
            }
            if editor.active_tool == ToolId::Scatter {
                scatter_settings(ui, &mut editor.scatter);
            }
            if editor.can_shuffle()
                && ui
                    .button("Shuffle Variant")
                    .on_hover_text("Re-roll each selected Stamp within its Variant group")
                    .clicked()
            {
                editor.shuffle_variants();
            }
            ui.toggle_value(&mut self.label_styles.open, "Label styles");
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
            ui.separator();
            if ui
                .add_enabled(editor.can_combine(), egui::Button::new("Union"))
                .on_hover_text("Merge the selected Landmasses into the topmost one")
                .clicked()
            {
                editor.union_selection();
            }
            if ui
                .add_enabled(editor.can_combine(), egui::Button::new("Subtract"))
                .on_hover_text("Cut the topmost selected Landmass out of the others")
                .clicked()
            {
                editor.subtract_selection();
            }
            if ui
                .add_enabled(editor.can_split(), egui::Button::new("Split Apart"))
                .on_hover_text("Make each part of the selected Landmasses a Landmass of its own")
                .clicked()
            {
                editor.split_selection();
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
    New(Canvas),
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
        self.updates.store(storage);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Dialogs take the keyboard while they are open.
        self.handle_close(&ctx);
        let dialog_open = self.pending.is_some()
            || self.new_map_dialog.open
            || self.export_dialog.open
            || self.export_run.is_some()
            || self.recovery_prompt.is_some()
            || *self.open_dialog.state() == DialogState::Open
            || *self.save_dialog.state() == DialogState::Open;
        if !dialog_open {
            self.shortcuts(&ctx);
        }
        self.dialogs(&ctx);
        self.update_title(&ctx);
        self.credits.show(&ctx, &mut self.editor);
        self.label_styles.show(&ctx, &mut self.editor);
        self.updates.show(&ctx);
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::bottom("assets")
            .resizable(true)
            .default_size(150.0)
            .show(ui, |ui| self.asset_browser.show(ui, &mut self.editor));
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

/// The scatter brush's settings, in a menu beside the toolbar.
fn scatter_settings(ui: &mut egui::Ui, brush: &mut ssmap_core::scatter::ScatterBrush) {
    use ssmap_core::scatter::Placement;
    ui.menu_button("Scatter settings", |ui| {
        ui.add(egui::Slider::new(&mut brush.density, 0.5..=100.0).text("Density"))
            .on_hover_text("Stamps per 100 by 100 Canvas units");
        ui.add(egui::Slider::new(&mut brush.spacing, 0.0..=200.0).text("Spacing"))
            .on_hover_text("The least distance between two Stamps in a stroke");
        ui.add(egui::Slider::new(&mut brush.size_jitter, 0.0..=0.9).text("Size jitter"));
        ui.add(egui::Slider::new(&mut brush.rotation_jitter, 0.0..=180.0).text("Rotation jitter"))
            .on_hover_text("Degrees either way");
        ui.checkbox(&mut brush.random_flip, "Random flip");
        ui.checkbox(&mut brush.random_variant, "Random variant");
        ui.separator();
        ui.radio_value(&mut brush.placement, Placement::Land, "Land only");
        ui.radio_value(&mut brush.placement, Placement::Water, "Water only");
        ui.radio_value(&mut brush.placement, Placement::Anywhere, "Anywhere");
    });
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
