//! StarScript Map Maker's editor. The binary in `main.rs` only opens a window around
//! [`MapMakerApp`], so UI tests can drive the same app headlessly.

mod asset_browser;
mod canvas_view;
pub mod credits;
pub mod document;
pub mod editor;
pub mod export_dialog;
pub mod export_region;
pub mod export_run;
pub mod frame_time;
pub mod handles;
pub mod icons;
mod label_styles;
mod layer_tree;
pub mod new_map;
pub mod prop_widgets;
mod properties;
mod recent;
pub mod recovery;
pub mod shell;
pub mod tab;
mod tab_strip;
mod tessellation;
pub mod theme;
pub mod tools;
pub mod update;

use eframe::{egui, egui_wgpu};
use egui_file_dialog::{DialogState, FileDialog, Filter};
use ssmap_core::asset::{PackLibrary, PackRoots};
use ssmap_core::file::{JournalBase, MapFile, PREVIEW_SIZE, remove_journal};
use ssmap_core::{CREDITS_CHUNK_KEYWORD, Canvas, DistanceUnit, Map, Scale};
use ssmap_render::{ExportJob, PngMeta, SpriteStore, TessellationCache};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::asset_browser::AssetBrowser;
use crate::credits::Credits;
use crate::document::Document;
use crate::editor::Editor;
use crate::export_dialog::{DialogAction, ExportDialog};
use crate::export_run::{ExportRun, Outcome};
use crate::new_map::NewMapDialog;
use crate::recent::RecentFiles;
use crate::recovery::{Found, Recovery};
use crate::tab::MapTab;
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

/// On macOS the tabs are drawn in the title bar, behind the window's own buttons, which they
/// leave room for. Elsewhere the native title bar stays and the strip sits below it.
const CUSTOM_CHROME: bool = cfg!(target_os = "macos");
const CHROME_INSET: f32 = if CUSTOM_CHROME {
    theme::metrics::TRAFFIC_LIGHTS_INSET
} else {
    0.0
};

/// The window options for the title bar: on macOS the content runs under a transparent title
/// bar with no title text, so the tab strip is drawn there.
pub fn window_chrome(viewport: egui::ViewportBuilder) -> egui::ViewportBuilder {
    if CUSTOM_CHROME {
        viewport
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false)
    } else {
        viewport
    }
}

pub struct MapMakerApp {
    /// The open Maps, one per tab, in the order of the tab row. May be empty.
    tabs: Vec<MapTab>,
    /// The index of the active tab: the one whose Map the window shows and every command acts
    /// on, reached through [`Self::tab`]. Meaningless while `tabs` is empty.
    active: usize,
    /// Whether the Home tab shows instead of the active tab's Map. Always the case while no
    /// Map is open.
    home_active: bool,
    /// The tab strip's scroll position and the like.
    strip: tab_strip::State,
    /// Where journals of Maps never saved go; `None` turns them off.
    recovery_dir: Option<PathBuf>,
    /// Whether the Canvas renderer is installed, so new tabs get a tessellation cache.
    gpu: bool,
    asset_browser: AssetBrowser,
    /// The installed Asset packs' art, shared with the Canvas renderer and picking.
    sprites: SpriteStore,
    credits: Credits,
    recent: RecentFiles,
    /// Panel widths and collapse state, the same for every tab.
    shell: shell::Shell,
    /// ⇥ just toggled the panels: clear keyboard focus at the next frame.
    drop_focus: bool,
    updates: UpdateCheck,
    new_map_dialog: NewMapDialog,
    export_dialog: ExportDialog,
    /// The PNG export in progress, stepped once per frame behind a modal dialog.
    export_run: Option<ExportRun>,
    open_dialog: FileDialog,
    save_dialog: FileDialog,
    /// Journals of Maps never saved left by a crash, oldest first, offered one at a time.
    unsaved_found: Vec<Found>,
    /// The journal of a Map never saved being offered now; recovering it opens a new tab.
    unsaved_prompt: Option<Found>,
    /// A tab's unsaved-changes question was answered Save for a Map never saved: what to do
    /// once the Save As dialog saves it. Closing that dialog instead cancels.
    awaiting_save: Option<(usize, Pending)>,
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
        app.tabs[0].editor.default_layers = layers;
        if let Some(problem) = problem {
            app.tabs[0].editor.error(problem);
        }
        app
    }

    pub fn with_map(
        cc: &eframe::CreationContext<'_>,
        map: Map,
        recovery_dir: Option<PathBuf>,
    ) -> Self {
        theme::install(&cc.egui_ctx);
        let (library, problems) = PackLibrary::load(&PackRoots::installed());
        for problem in &problems {
            eprintln!("Asset pack problem: {problem}");
        }
        let credits = Credits::new(&library);
        let sprites = SpriteStore::new(Arc::new(library));
        let ctx = cc.egui_ctx.clone();
        sprites.set_repaint(move || ctx.request_repaint());
        let gpu = match cc.wgpu_render_state.as_ref() {
            Some(render_state) => canvas_view::install(render_state, sprites.clone()),
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
        let mut unsaved_found = Recovery::new(recovery_dir.clone()).find_unsaved();
        unsaved_found.reverse();
        let mut tab = MapTab::new(
            map,
            gpu.then(TessellationCache::new),
            Recovery::new(recovery_dir.clone()),
        );
        tab.editor.set_sprites(sprites.clone());
        Self {
            tabs: vec![tab],
            active: 0,
            home_active: false,
            strip: tab_strip::State::default(),
            recovery_dir,
            gpu,
            awaiting_save: None,
            unsaved_prompt: None,
            asset_browser: AssetBrowser::default(),
            sprites,
            credits,
            recent: RecentFiles::load(cc.storage),
            shell: shell::Shell::load(cc.storage),
            drop_focus: false,
            updates: UpdateCheck::load(cc.storage),
            new_map_dialog: NewMapDialog::default(),
            export_dialog: ExportDialog::default(),
            export_run: None,
            open_dialog: map_files().title("Open Map"),
            save_dialog: map_files().title("Save Map As"),
            unsaved_found,
            close_allowed: false,
            render_state: cc.wgpu_render_state.clone(),
            egui_ctx: cc.egui_ctx.clone(),
            title: String::new(),
        }
    }

    pub fn editor(&self) -> &Editor {
        self.tab().editor()
    }

    /// The panels' widths and collapse state.
    pub fn shell(&self) -> &shell::Shell {
        &self.shell
    }

    pub fn document(&self) -> &Document {
        self.tab().document()
    }

    /// Recently opened or saved Map files, newest first.
    pub fn recent_files(&self) -> &[PathBuf] {
        self.recent.paths()
    }

    /// Whether a Label's text box is open on the Canvas.
    pub fn editing_label_text(&self) -> bool {
        self.tab().canvas_view.editing_text()
    }

    /// The screen position of a Canvas point in the Canvas panel, once it has been drawn.
    pub fn canvas_to_screen(&self, point: [f64; 2]) -> Option<egui::Pos2> {
        self.tab().canvas_view.to_screen(point)
    }

    /// The Recovery journal being written for the active tab's Map, once there is one.
    pub fn recovery_journal(&self) -> Option<&Path> {
        self.tab().recovery_journal()
    }

    /// Whether edits are waiting to be written to the active tab's Recovery journal.
    pub fn has_unjournaled_changes(&self) -> bool {
        self.tab().recovery.has_pending()
    }

    /// The open Maps, in tab order.
    pub fn tabs(&self) -> &[MapTab] {
        &self.tabs
    }

    /// The index of the active tab.
    pub fn active_tab(&self) -> usize {
        self.active
    }

    /// Makes tab `index` the active one; out of range does nothing.
    pub fn focus_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
            self.home_active = false;
        }
    }

    /// Whether the Home tab is showing: chosen, or because no Map is open.
    pub fn home_active(&self) -> bool {
        self.home_active || self.tabs.is_empty()
    }

    /// The tab being shown.
    fn current_slot(&self) -> tab_strip::Slot {
        if self.home_active() {
            tab_strip::Slot::Home
        } else {
            tab_strip::Slot::Map(self.active)
        }
    }

    fn show_slot(&mut self, slot: tab_strip::Slot) {
        match slot {
            tab_strip::Slot::Home => self.home_active = true,
            tab_strip::Slot::Map(index) => self.focus_tab(index),
        }
    }

    /// Shows the tab ⌘`n` reaches (1 is Home, 9 the last tab); `n` out of range does nothing.
    pub fn show_tab_number(&mut self, n: u8) {
        if let Some(slot) = tab_strip::slot_for_number(n, self.tabs.len()) {
            self.show_slot(slot);
        }
    }

    /// Closes tab `index`, first asking about unsaved changes (the question is answered in the
    /// dialog on the next frame).
    pub fn close_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        if tab.has_unsaved_changes() {
            tab.pending = Some(Pending::CloseTab);
            self.active = index;
            self.home_active = false;
        } else {
            self.remove_tab(index);
        }
    }

    /// Drops tab `index` and its Recovery journal, keeping the same Map active if it can.
    fn remove_tab(&mut self, index: usize) {
        let mut tab = self.tabs.remove(index);
        tab.recovery.reset();
        if index < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
    }

    /// The index of the tab showing the Map file at `path`, comparing canonical paths.
    fn tab_for(&self, path: &Path) -> Option<usize> {
        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or(path.to_path_buf());
        let wanted = canonical(path);
        self.tabs.iter().position(|tab| {
            tab.document
                .path()
                .is_some_and(|open| canonical(open) == wanted)
        })
    }

    /// A new tab for `map`, with this run's art and a Recovery journal of its own.
    fn new_tab(&self, map: Map, document: Document) -> MapTab {
        let recovery = Recovery::new(self.recovery_dir.clone());
        let mut tab = MapTab::new(map, self.gpu.then(TessellationCache::new), recovery);
        tab.document = document;
        tab.editor.set_sprites(self.sprites.clone());
        tab
    }

    /// Adds `tab` after the others and shows it.
    fn push_tab(&mut self, tab: MapTab) {
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.home_active = false;
    }

    /// Shows a message on the active tab's status bar, or on stderr when no Map is open.
    fn notify_error(&mut self, message: String) {
        match self.tabs.get_mut(self.active) {
            Some(tab) => tab.editor.error(message),
            None => eprintln!("{message}"),
        }
    }

    /// The window title, as last sent to the window.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Opens a Map file in a new tab and shows it, or shows the tab that already has that
    /// file open.
    pub fn open_path(&mut self, path: &Path) -> bool {
        if let Some(index) = self.tab_for(path) {
            self.active = index;
            return true;
        }
        match Document::open(path) {
            Ok((document, map)) => {
                let mut tab = self.new_tab(map, document);
                let ctx = self.egui_ctx.clone();
                tab.editor
                    .start_background_roughness(move || ctx.request_repaint());
                self.recent.add(path);
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

    /// The active tab: the one whose Map the window shows and every command acts on. Panics
    /// while no Map is open.
    fn tab(&self) -> &MapTab {
        &self.tabs[self.active]
    }

    /// Opens a new Map from the starter template in a new tab and shows it.
    pub fn new_map(&mut self, canvas: Canvas) {
        let (map, layers, problem) = new_map::start_map(canvas);
        let mut tab = self.new_tab(map, Document::default());
        tab.editor.default_layers = layers;
        match problem {
            Some(problem) => tab.editor.error(problem),
            None => tab.editor.info("Created a new Map"),
        }
        self.push_tab(tab);
    }

    /// Applies a journal's changes to the Map file it was written against.
    fn replay_recovery(&mut self, found: Found) {
        let tab = &mut self.tabs[self.active];
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
    fn save_recovered_copy(&mut self, found: Found) {
        let tab = &mut self.tabs[self.active];
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
                self.recent.add(&copy_path);
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
    fn recover_unsaved(&mut self, found: Found) {
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
    fn journal(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        // What falls due within a frame is written now: egui repaints at once for so short a
        // wait anyway.
        let frame = Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
        for tab in &mut self.tabs {
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

    /// Closing the window with unsaved changes asks about each tab that has some, in turn; a
    /// clean close deletes every Recovery journal.
    fn handle_close(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) || self.close_allowed {
            return;
        }
        if self.tabs.iter().any(MapTab::has_unsaved_changes) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            // A question already on screen is the quit in progress.
            let asking =
                self.awaiting_save.is_some() || self.tabs.iter().any(|t| t.pending.is_some());
            if !asking {
                self.advance_quit(ctx);
            }
        } else {
            for tab in &mut self.tabs {
                tab.recovery.reset();
            }
        }
    }

    /// Quitting: shows the next tab with unsaved changes that has not been decided about and
    /// asks what to do with it; closes the window once none is left.
    fn advance_quit(&mut self, ctx: &egui::Context) {
        let next = self
            .tabs
            .iter()
            .position(|tab| tab.has_unsaved_changes() && !tab.quit_decided);
        match next {
            Some(index) => {
                self.active = index;
                self.home_active = false;
                self.tabs[index].pending = Some(Pending::Quit);
            }
            None => self.close(ctx),
        }
    }

    /// Stops a quit: every tab stays open and is asked about again next time.
    fn cancel_quit(&mut self) {
        for tab in &mut self.tabs {
            tab.quit_decided = false;
        }
    }

    /// Closes the window, discarding unsaved changes and their Recovery journals.
    fn close(&mut self, ctx: &egui::Context) {
        self.close_allowed = true;
        for tab in &mut self.tabs {
            tab.recovery.reset();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Whether a dialog has the keyboard, so shortcuts and the tab row wait.
    fn dialog_open(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|tab| tab.pending.is_some() || tab.recovery_prompt.is_some())
            || self.tabs.iter().any(|tab| tab.pending.is_some())
            || self.awaiting_save.is_some()
            || self.unsaved_prompt.is_some()
            || self.new_map_dialog.open
            || self.export_dialog.open
            || self.export_run.is_some()
            || *self.open_dialog.state() == DialogState::Open
            || *self.save_dialog.state() == DialogState::Open
    }

    /// Opens each `.ssmap` file dropped onto the window in a tab of its own.
    fn open_dropped_files(&mut self, ctx: &egui::Context) {
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

    /// Whether the user chose to close the window.
    pub fn is_closing(&self) -> bool {
        self.close_allowed
    }

    /// Saves to the open file, or asks where to save a Map that has never been saved.
    pub fn save(&mut self) -> bool {
        // A gesture still open would otherwise reach the journal after the save holding it.
        self.tabs[self.active].editor.end_gesture();
        self.tabs[self.active].editor.embed_used_assets();
        let preview = self.preview_png();
        let tab = &mut self.tabs[self.active];
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
        self.tabs[self.active].editor.end_gesture();
        self.tabs[self.active].editor.embed_used_assets();
        let preview = self.preview_png();
        let path = document::with_extension(path.to_path_buf());
        let tab = &mut self.tabs[self.active];
        let result = tab
            .document
            .save_as(&path, &tab.editor.map, preview.as_deref());
        self.saved(result)
    }

    fn saved(&mut self, result: Result<(), ssmap_core::file::FileError>) -> bool {
        let tab = &mut self.tabs[self.active];
        match result {
            Ok(()) => {
                // The save holds every change so far; the journal starts over.
                tab.editor.take_applied();
                tab.recovery.reset();
                if let Some(path) = tab.document.path() {
                    self.recent.add(path);
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

    fn show_open_dialog(&mut self) {
        if let Some(dir) = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.document.path())
            .and_then(Path::parent)
        {
            self.open_dialog.config_mut().initial_directory = dir.to_path_buf();
        }
        self.open_dialog.pick_file();
    }

    fn show_save_dialog(&mut self) {
        let config = self.save_dialog.config_mut();
        let tab = &self.tabs[self.active];
        if let Some(dir) = tab.document.path().and_then(Path::parent) {
            config.initial_directory = dir.to_path_buf();
        }
        config.default_file_name = tab.document.name();
        self.save_dialog.save_file();
    }

    /// The preview PNG stored with each save, rendered like the screen.
    fn preview_png(&mut self) -> Option<Vec<u8>> {
        let render_state = self.render_state.as_ref()?;
        let tab = &mut self.tabs[self.active];
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
                        self.open_path(&path);
                    } else {
                        self.recent.remove(&path);
                        self.notify_error(format!("“{}” no longer exists", path.display()));
                    }
                }
            });
            ui.separator();
            let has_tab = !self.home_active();
            if ui.add_enabled(has_tab, egui::Button::new("Save")).clicked() {
                self.save();
            }
            if ui
                .add_enabled(has_tab, egui::Button::new("Save As…"))
                .clicked()
            {
                self.show_save_dialog();
            }
            if ui
                .add_enabled(has_tab, egui::Button::new("Close Tab"))
                .clicked()
            {
                self.close_tab(self.active);
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(has_tab, egui::Button::new("Export PNG…"))
                .clicked()
            {
                let tab = &self.tabs[self.active];
                let directory = tab.document.path().and_then(Path::parent);
                self.export_dialog.open_with(
                    &tab.editor.map.export_settings,
                    directory.map(Path::to_path_buf),
                );
                ui.close();
            }
            let again = has_tab && self.re_export_target().is_some() && self.export_run.is_none();
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
            self.open_path(&path);
        }
        self.save_dialog.update(ctx);
        if let Some(path) = self.save_dialog.take_picked() {
            let saved = self.save_as_path(&path);
            match self.awaiting_save.take() {
                Some((index, pending)) if saved && index == self.active => {
                    self.finish(pending, ctx);
                }
                Some(_) => self.cancel_quit(),
                None => {}
            }
        } else if self.awaiting_save.is_some() && *self.save_dialog.state() != DialogState::Open {
            // The Save As dialog was closed without saving: the question is cancelled.
            self.awaiting_save = None;
            self.cancel_quit();
        }

        if let Some(canvas) = self.new_map_dialog.show(ctx) {
            self.new_map(canvas);
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            match self.export_dialog.show(ctx, &tab.editor.map.canvas) {
                Some(DialogAction::Save(settings)) => tab.editor.map.export_settings = settings,
                Some(DialogAction::Export(settings)) => {
                    tab.editor.map.export_settings = settings;
                    self.export_from_dialog();
                }
                None => {}
            }
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
        let tab = &mut self.tabs[self.active];
        let settings = &mut tab.editor.map.export_settings;
        if settings.file_name.as_deref().is_none_or(str::is_empty) {
            settings.file_name = Some(format!("{}.png", tab.document.name()));
        }
        tab.export_directory = Some(directory);
        self.start_export();
    }

    fn document_directory(&self) -> Option<PathBuf> {
        self.tab()
            .document
            .path()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    /// Where Re-export would write, from the stored settings; `None` before a first export.
    fn re_export_target(&self) -> Option<PathBuf> {
        let tab = self.tab();
        let directory = tab
            .export_directory
            .clone()
            .or_else(|| self.document_directory())?;
        export_run::export_path(&tab.editor.map.export_settings, &directory)
    }

    /// Runs an export of the active tab from the stored settings without opening the dialog.
    fn re_export(&mut self) {
        if self.re_export_target().is_none() {
            self.tabs[self.active]
                .editor
                .error("Export the Map once with Export PNG… first");
            return;
        }
        if self.tab().export_directory.is_none() {
            self.tabs[self.active].export_directory = self.document_directory();
        }
        self.start_export();
    }

    /// Starts exporting the active tab to the stored file name in `export_directory`. Only
    /// one export runs at a time.
    fn start_export(&mut self) {
        if self.export_run.is_some() {
            return;
        }
        if self.render_state.is_none() {
            self.tabs[self.active]
                .editor
                .error("Export needs a graphics adapter");
            return;
        }
        let Some(dest) = self.re_export_target() else {
            self.tabs[self.active]
                .editor
                .error("The export has no file name");
            return;
        };
        let tab = &mut self.tabs[self.active];
        let settings = tab.editor.map.export_settings.clone();
        let output = settings.credits.output(&tab.editor.credit_lines());
        let mut meta =
            PngMeta::default().with_software("StarScript Map Maker", env!("CARGO_PKG_VERSION"));
        if let Some(chunk) = &output.chunk {
            meta = meta.with_credits(CREDITS_CHUNK_KEYWORD, chunk);
        }
        match ExportJob::new(&tab.editor.map, &settings, meta, &dest) {
            Ok(job) => self.export_run = Some(ExportRun::new(job, dest, output.file)),
            Err(err) => tab.editor.error(format!("Export failed: {err}")),
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
        let tab = &mut self.tabs[self.active];
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
                if export_run::show_progress(ctx, &dest, progress) {
                    run.cancel();
                    self.export_run = None;
                    tab.editor.info("Export cancelled");
                } else {
                    // Keep stepping even when no input arrives.
                    ctx.request_repaint();
                }
            }
            Outcome::Finished(size) => {
                self.export_run = None;
                tab.editor.info(format!(
                    "Exported {} × {} px to {}",
                    size.width,
                    size.height,
                    dest.display()
                ));
            }
            Outcome::Failed(message) => {
                self.export_run = None;
                tab.editor.error(format!("Export failed: {message}"));
            }
        }
    }

    /// Asks what to do with the active tab's unsaved changes before it closes or the app quits.
    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let Some(pending) = tab.pending.clone() else {
            return;
        };
        let name = tab.document.name();
        let mut choice = None;
        egui::Modal::new(egui::Id::new("unsaved_changes")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label(format!("Save the changes to “{name}” before closing?"));
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(UnsavedChoice::Save);
                }
                if ui.button("Don’t Save").clicked() {
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
        self.tabs[self.active].pending = None;
        match choice {
            UnsavedChoice::Save if self.tab().document.path().is_none() => {
                // A Map never saved is saved through Save As.
                self.awaiting_save = Some((self.active, pending));
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
    fn finish(&mut self, pending: Pending, ctx: &egui::Context) {
        match pending {
            Pending::CloseTab => self.remove_tab(self.active),
            Pending::Quit => {
                self.tabs[self.active].quit_decided = true;
                self.advance_quit(ctx);
            }
        }
    }

    /// Offers a Recovery journal left by a crash: the active tab's Map file's, or else one of
    /// a Map never saved, which opens in a tab of its own.
    fn recovery_dialog(&mut self, ctx: &egui::Context) {
        let tab_asking = self
            .tabs
            .get(self.active)
            .is_some_and(|tab| tab.recovery_prompt.is_some() || tab.pending.is_some());
        if !tab_asking && self.awaiting_save.is_none() && self.unsaved_prompt.is_none() {
            self.unsaved_prompt = self.unsaved_found.pop();
        }
        let Some(tab) = self.tabs.get(self.active) else {
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
        let prompt = self.tabs[self.active]
            .recovery_prompt
            .take()
            .expect("shown above");
        match (prompt, recovered) {
            (RecoveryPrompt::Replay(found), true) => self.replay_recovery(found),
            (RecoveryPrompt::Copy { found, .. }, true) => self.save_recovered_copy(found),
            (RecoveryPrompt::Replay(found) | RecoveryPrompt::Copy { found, .. }, false) => {
                if let Err(err) = remove_journal(&found.path) {
                    self.notify_error(format!("Couldn’t delete the unsaved changes: {err}"));
                }
            }
        }
    }

    /// Offers the journal of a Map never saved that a crash left: recovering it opens a tab.
    fn unsaved_journal_dialog(&mut self, ctx: &egui::Context) {
        if self.unsaved_prompt.is_none() {
            return;
        }
        let mut recovered = None;
        egui::Modal::new(egui::Id::new("recovery")).show(ctx, |ui| {
            ui.heading("Recover unsaved changes");
            ui.label("A Map that was never saved wasn’t closed properly. Recover it?");
            ui.horizontal(|ui| {
                if ui.button("Recover").clicked() {
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
        let found = self.unsaved_prompt.take().expect("shown above");
        if recovered {
            self.recover_unsaved(found);
        } else if let Err(err) = remove_journal(&found.path) {
            self.notify_error(format!("Couldn’t delete the unsaved changes: {err}"));
        }
    }

    /// Shows the active Map's file name in the window title, with a dot while it has unsaved
    /// changes.
    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.tabs.get(self.active) {
            Some(tab) => {
                let dirty = if tab.has_unsaved_changes() {
                    " •"
                } else {
                    ""
                };
                format!("{}{dirty} — StarScript Map Maker", tab.document.name())
            }
            None => "StarScript Map Maker".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Text fields keep their own undo and keys. The Label text box keeps Escape too,
        // which egui takes its focus away for before this runs.
        if ctx.egui_wants_keyboard_input()
            || self
                .tabs
                .get(self.active)
                .is_some_and(|tab| tab.canvas_view.editing_text())
        {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        let shortcut = |modifiers, key| KeyboardShortcut::new(modifiers, key);
        // The panel shortcuts come first: egui ignores extra Alt, so Cmd+Alt+1 would
        // otherwise also match Cmd+1 (Home) below.
        ctx.input_mut(|input| {
            let both = Modifiers::COMMAND | Modifiers::ALT;
            if input.consume_shortcut(&shortcut(both, Key::Num1)) {
                self.shell.toggle_layers();
            }
            if input.consume_shortcut(&shortcut(both, Key::Num2)) {
                self.shell.toggle_properties();
            }
            if input.consume_key(Modifiers::NONE, Key::Tab) {
                self.shell.toggle_panels();
            }
        });
        let (new_map, number, step) = ctx.input_mut(|input| {
            let new_map = input.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::T));
            let number = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ]
            .iter()
            .zip(1u8..)
            .find(|(key, _)| input.consume_shortcut(&shortcut(Modifiers::COMMAND, **key)))
            .map(|(_, n)| n);
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
            self.new_map_dialog.open = true;
        }
        if let Some(n) = number {
            self.show_tab_number(n);
        }
        if let Some(forward) = step {
            let slot = tab_strip::cycle(self.current_slot(), self.tabs.len(), forward);
            self.show_slot(slot);
        }
        let (open, save, save_as, close_tab) = ctx.input_mut(|input| {
            // Shift+S first: plain Cmd+S would also match it.
            let save_as =
                input.consume_shortcut(&shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::S));
            let save = input.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::S));
            let open = input.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::O));
            let close_tab = input.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::W));
            (open, save, save_as, close_tab)
        });
        if open {
            self.show_open_dialog();
        }
        if close_tab && self.current_slot().can_close() {
            self.close_tab(self.active);
            return;
        }
        // Home has no Map to save or edit.
        if self.home_active() {
            return;
        }
        if save_as {
            self.show_save_dialog();
        } else if save {
            self.save();
        }
        let editor = &mut self.tabs[self.active].editor;
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
            if self.home_active() {
                return;
            }
            let Some(tab) = self.tabs.get_mut(self.active) else {
                return;
            };
            let editor = &mut tab.editor;
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
            ui.toggle_value(&mut tab.label_styles.open, "Label styles");
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

    /// The strip of tabs in the title-bar area: Home, one tab per open Map, and a +.
    fn tab_strip(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let infos: Vec<_> = self
            .tabs
            .iter()
            .map(|tab| tab_strip::TabInfo {
                name: tab.document.name(),
                path: tab.document.path().map(|path| path.display().to_string()),
                unsaved: tab.has_unsaved_changes(),
            })
            .collect();
        let current = self.current_slot();
        let actions = ui
            .add_enabled_ui(enabled, |ui| {
                tab_strip::show(
                    ui,
                    &infos,
                    current,
                    &mut self.strip,
                    CHROME_INSET,
                    CUSTOM_CHROME,
                )
            })
            .inner;
        for action in actions {
            match action {
                tab_strip::Action::Show(slot) => self.show_slot(slot),
                tab_strip::Action::Close(index) => self.close_tab(index),
                tab_strip::Action::NewMap => self.new_map_dialog.open = true,
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let palette = theme::palette(ui.ctx());
        let text = |text: String| {
            egui::RichText::new(text)
                .size(theme::CAPTION)
                .color(palette.text_secondary)
        };
        ui.horizontal_centered(|ui| {
            ui.add_space(theme::SPACING[3]);
            let ctx = ui.ctx().clone();
            let ppp = f64::from(ctx.pixels_per_point());
            if let Some(zoom) = tab.canvas_view.zoom(ppp) {
                ui.label(text(format!("{:.0}%", zoom * 100.0)));
            }
            let point = ctx
                .pointer_hover_pos()
                .and_then(|pointer| tab.canvas_view.canvas_point(pointer));
            if let Some(point) = point {
                ui.separator();
                ui.label(text(shell::cursor_text(&tab.editor.map.canvas, point)));
            }
            let summary = shell::selection_summary(
                &tab.editor.map,
                tab.editor.selection(),
                tab.editor.active_layer(),
            );
            if !summary.is_empty() {
                ui.separator();
                ui.label(text(summary));
            }
            // Notices move out of the status bar in #166.
            if let Some(notice) = tab.editor.notice() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(theme::SPACING[3]);
                    let color = if notice.is_error {
                        ui.visuals().error_fg_color
                    } else {
                        palette.text_primary
                    };
                    ui.label(
                        egui::RichText::new(&notice.text)
                            .size(theme::CAPTION)
                            .color(color),
                    );
                });
                ctx.request_repaint_after(
                    editor::NOTICE_DURATION.saturating_sub(notice.shown_at.elapsed()),
                );
            }
        });
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

/// What happens to a tab once the user has decided about its unsaved changes.
#[derive(Clone)]
enum Pending {
    /// Close the tab.
    CloseTab,
    /// Quit the app, which asks about every tab with unsaved changes in turn.
    Quit,
}

enum RecoveryPrompt {
    /// The tab's Map file's journal, written against its current save.
    Replay(Found),
    /// The tab's Map file's journal, written against another save of it.
    Copy { found: Found, reason: String },
}

enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}

impl eframe::App for MapMakerApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.recent.store(storage);
        self.shell.store(storage);
        self.updates.store(storage);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Dialogs take the keyboard while they are open.
        self.handle_close(&ctx);
        let dialog_open = self.dialog_open();
        if std::mem::take(&mut self.drop_focus) {
            // The ⇥ that toggled the panels also moved keyboard focus into one of them.
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        let panels_were_hidden = self.shell.panels_hidden();
        if !dialog_open {
            self.open_dropped_files(&ctx);
            self.shortcuts(&ctx);
        }
        self.drop_focus = self.shell.panels_hidden() != panels_were_hidden;
        self.dialogs(&ctx);
        self.update_title(&ctx);
        let strip = egui::Frame::NONE.fill(theme::palette(&ctx).bg_chrome);
        egui::Panel::top("tabs")
            .exact_size(theme::metrics::TITLE_BAR_WITH_TABS)
            .frame(strip)
            .show(ui, |ui| self.tab_strip(ui, !dialog_open));
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        if self.home_active() {
            // Until the Home screen exists this is a placeholder.
            self.updates.show(&ctx);
            egui::CentralPanel::default().show(ui, |ui| {
                if self.tabs.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.weak("No Map is open. Use File → New Map… or Open…");
                    });
                }
            });
            return;
        }
        let tab = &mut self.tabs[self.active];
        self.credits.show(&ctx, &mut tab.editor);
        tab.label_styles.show(&ctx, &mut tab.editor);
        self.updates.show(&ctx);
        egui::Panel::bottom("status")
            .exact_size(theme::metrics::STATUS_BAR)
            .show(ui, |ui| self.status_bar(ui));
        let tab = &mut self.tabs[self.active];
        egui::Panel::bottom("assets")
            .resizable(true)
            .default_size(150.0)
            .show(ui, |ui| self.asset_browser.show(ui, &mut tab.editor));
        let mut layers_divider = None;
        let mut properties_divider = None;
        if self.shell.layers_shown() {
            let collapsed = self.shell.layers_collapsed();
            let panel = egui::Panel::left("layers")
                .resizable(false)
                .exact_size(self.shell.layers_layout_width())
                .show(ui, |ui| {
                    if collapsed {
                        let path = shell::selection_path(&tab.editor.map, tab.editor.selection());
                        if shell::layers_rail(ui, &path) {
                            self.shell.set_layers_collapsed(false);
                        }
                        return;
                    }
                    shell::panel_header(ui, "Layers", |ui| {
                        if icons::icon_button(
                            ui,
                            icons::Icon::CollapseLeft,
                            "Collapse Layers (⌘⌥1)",
                        )
                        .clicked()
                        {
                            self.shell.set_layers_collapsed(true);
                        }
                        // Right to left: this lands just left of Collapse.
                        if icons::icon_button(ui, icons::Icon::Add, "New Layer (⇧⌘N)").clicked()
                        {
                            tab.editor.new_layer();
                        }
                    });
                    tab.layer_tree.show(ui, &mut tab.editor);
                });
            if !collapsed {
                layers_divider = Some(panel.response.rect);
            }
        }
        if self.shell.properties_shown() {
            let panel = egui::Panel::right("properties")
                .resizable(false)
                .exact_size(self.shell.width(shell::Side::Right))
                .show(ui, |ui| tab.properties.show(ui, &mut tab.editor));
            properties_divider = Some(panel.response.rect);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| tab.canvas_view.show(ui, &mut tab.editor));
        // After the Canvas, so the strips on its edges win the pointer over it.
        for (side, rect) in [
            (shell::Side::Left, layers_divider),
            (shell::Side::Right, properties_divider),
        ] {
            if let Some(rect) = rect {
                shell::divider(ui, &mut self.shell, side, rect);
            }
        }
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
