//! StarScript Map Maker's editor. The binary in `main.rs` only opens a window around
//! [`MapMakerApp`], so UI tests can drive the same app headlessly.

mod asset_browser;
mod canvas_controls;
mod canvas_view;
pub mod clipboard;
pub mod credits;
pub mod dialog;
pub mod document;
pub mod editor;
pub mod export_dialog;
pub mod export_region;
pub mod export_run;
mod file_actions;
pub mod frame_time;
pub mod handles;
pub mod home;
pub mod icons;
mod label_styles;
mod layer_tree;
pub mod menu;
pub mod native_dialog;
pub mod native_menu;
pub mod new_map;
pub mod overlay;
mod pack_page;
pub mod prop_widgets;
mod properties;
mod recent;
pub mod recovery;
pub mod shell;
pub mod tab;
mod tab_strip;
mod tessellation;
pub mod theme;
pub mod toast;
mod tool_dock;
mod tool_section;
pub mod tools;
pub mod update;
pub mod zoom;

use eframe::{egui, egui_wgpu};
use native_dialog::{Pick, Polled};
use ssmap_core::asset::{PackLibrary, PackRoots};
use ssmap_core::file::{JournalBase, MapFile, PREVIEW_SIZE, remove_journal};
use ssmap_core::{CREDITS_CHUNK_KEYWORD, Canvas, Map, MapTemplate, TemplateError};
use ssmap_render::{ExportJob, PngMeta, SpriteStore, TessellationCache};
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// What every window shares: settings, recent Maps, the installed Asset packs, crash recovery
/// and the native menu. Not tied to one window's Maps.
struct AppState {
    /// Where journals of Maps never saved go; `None` turns them off.
    recovery_dir: Option<PathBuf>,
    /// Whether the Canvas renderer is installed, so new tabs get a tessellation cache.
    gpu: bool,
    /// The installed Asset packs' art, shared with the Canvas renderer and picking.
    sprites: SpriteStore,
    credits: Credits,
    recent: RecentFiles,
    updates: UpdateCheck,
    /// Journals of Maps never saved left by a crash, oldest first, offered one at a time.
    unsaved_found: Vec<Found>,
    /// The journal of a Map never saved being offered now; recovering it opens a new tab.
    unsaved_prompt: Option<Found>,
    /// `None` without a GPU, as in UI tests; saves then store no preview.
    render_state: Option<egui_wgpu::RenderState>,
    /// Asked for a repaint as background work finishes.
    egui_ctx: egui::Context,
    /// Closing the window only hides it and leaves the app running (macOS); elsewhere closing
    /// the window quits.
    close_hides_window: bool,
    /// The frame-time overlay, off by default (F3 in debug and profiling builds).
    overlay: overlay::Overlay,
    /// Quit was asked for (⌘Q) and is handled next frame.
    quit_requested: bool,
    /// Commands whose key the egui handler leaves alone, because something else (the native
    /// menu on macOS) owns it. Each key is then handled once.
    shortcuts_off: HashSet<menu::Command>,
    /// The native menu bar, on macOS; `None` elsewhere and in UI tests, which use the egui bar.
    #[cfg(target_os = "macos")]
    native_menu: Option<native_menu::mac::MacMenu>,
}

/// What belongs to one window: its tabs, panels and the dialogs modal to it. The Home tab
/// lives only in the main window.
struct WindowState {
    /// The open Maps, one per tab, in the order of the tab row. May be empty.
    tabs: Vec<MapTab>,
    /// The index of the active tab: the one whose Map the window shows and every command acts
    /// on, reached through [`MapMakerApp::tab`]. Meaningless while `tabs` is empty.
    active: usize,
    /// Whether the Home tab shows instead of the active tab's Map. Always the case while no
    /// Map is open.
    home_active: bool,
    /// The tab strip's scroll position and the like.
    strip: tab_strip::State,
    /// The Home tab's list, search and thumbnails.
    home: home::Home,
    /// Panel widths and collapse state, the same for every tab in the window.
    shell: shell::Shell,
    /// ⇥ just toggled the panels: clear keyboard focus at the next frame.
    drop_focus: bool,
    new_map_dialog: NewMapDialog,
    export_dialog: ExportDialog,
    /// The PNG export in progress, stepped once per frame behind a modal dialog.
    export_run: Option<ExportRun>,
    open_dialog: Pick,
    save_dialog: Pick,
    /// The Map file a Home card's Delete asked about; the dialog confirms before it is trashed.
    delete_prompt: Option<PathBuf>,
    /// A tab's unsaved-changes question was answered Save for a Map never saved: what to do
    /// once the Save As dialog saves it. Closing that dialog instead cancels.
    awaiting_save: Option<(usize, Pending)>,
    /// Set once the user has chosen to close despite unsaved changes.
    close_allowed: bool,
    /// What the close or quit in progress is for.
    leaving: Leaving,
    /// The window was closed but the app keeps running, with no Map open.
    window_hidden: bool,
    /// The window title last sent, so it is only sent when it changes.
    title: String,
}

pub struct MapMakerApp {
    app: AppState,
    /// The one window, for now.
    win: WindowState,
}

impl MapMakerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::with_recovery_dir(cc, Recovery::default_unsaved_dir())
    }

    /// Starts on the Home tab with no Map open, keeping journals of Maps never saved in
    /// `recovery_dir` (`None` turns them off) and offering any a crash left there.
    pub fn with_recovery_dir(
        cc: &eframe::CreationContext<'_>,
        recovery_dir: Option<PathBuf>,
    ) -> Self {
        Self::build(cc, None, recovery_dir)
    }

    /// Starts with `map` open in a tab.
    pub fn with_map(
        cc: &eframe::CreationContext<'_>,
        map: Map,
        recovery_dir: Option<PathBuf>,
    ) -> Self {
        Self::build(cc, Some(map), recovery_dir)
    }

    fn build(
        cc: &eframe::CreationContext<'_>,
        map: Option<Map>,
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
        let mut unsaved_found = Recovery::new(recovery_dir.clone()).find_unsaved();
        unsaved_found.reverse();
        let tabs: Vec<MapTab> = map
            .into_iter()
            .map(|map| {
                let mut tab = MapTab::new(
                    map,
                    gpu.then(TessellationCache::new),
                    Recovery::new(recovery_dir.clone()),
                );
                tab.editor.set_sprites(sprites.clone());
                tab
            })
            .collect();
        // Without a GPU (UI tests) there is no real window, and muda only works from the
        // main thread, so those runs keep the egui menu bar.
        #[cfg(target_os = "macos")]
        let native_menu = gpu.then(|| native_menu::mac::MacMenu::new(&cc.egui_ctx));
        #[cfg(not(target_os = "macos"))]
        let native_menu: Option<()> = None;
        let app = AppState {
            recovery_dir,
            gpu,
            unsaved_prompt: None,
            sprites,
            credits,
            recent: RecentFiles::load(cc.storage),
            // The native menu owns the ⌘ keys; the egui handler leaves them alone.
            shortcuts_off: if native_menu.is_some() {
                native_menu::owned_commands().into_iter().collect()
            } else {
                HashSet::new()
            },
            #[cfg(target_os = "macos")]
            native_menu,
            updates: UpdateCheck::load(cc.storage),
            unsaved_found,
            render_state: cc.wgpu_render_state.clone(),
            egui_ctx: cc.egui_ctx.clone(),
            close_hides_window: cfg!(target_os = "macos"),
            overlay: overlay::Overlay::default(),
            quit_requested: false,
        };
        let win = WindowState {
            home_active: tabs.is_empty(),
            tabs,
            active: 0,
            strip: tab_strip::State::default(),
            home: home::Home::default(),
            awaiting_save: None,
            delete_prompt: None,
            shell: shell::Shell::load(cc.storage),
            drop_focus: false,
            new_map_dialog: NewMapDialog::default(),
            export_dialog: ExportDialog::default(),
            export_run: None,
            open_dialog: if gpu {
                Pick::default()
            } else {
                Pick::headless()
            },
            save_dialog: if gpu {
                Pick::default()
            } else {
                Pick::headless()
            },
            close_allowed: false,
            leaving: Leaving::Quit,
            window_hidden: false,
            title: String::new(),
        };
        Self { app, win }
    }

    pub fn editor(&self) -> &Editor {
        self.tab().editor()
    }

    /// The panels' widths and collapse state.
    pub fn shell(&self) -> &shell::Shell {
        &self.win.shell
    }

    pub fn document(&self) -> &Document {
        self.tab().document()
    }

    /// Whether the system Save As dialog is showing.
    pub fn save_dialog_open(&self) -> bool {
        self.win.save_dialog.is_open()
    }

    /// Recently opened or saved Map files, newest first.
    pub fn recent_files(&self) -> &[PathBuf] {
        self.app.recent.paths()
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
        &self.win.tabs
    }

    /// The index of the active tab.
    pub fn active_tab(&self) -> usize {
        self.win.active
    }

    /// Makes tab `index` the active one; out of range does nothing.
    pub fn focus_tab(&mut self, index: usize) {
        if index < self.win.tabs.len() {
            self.win.active = index;
            self.win.home_active = false;
        }
    }

    /// Whether the Home tab is showing: chosen, or because no Map is open.
    pub fn home_active(&self) -> bool {
        self.win.home_active || self.win.tabs.is_empty()
    }

    /// The tab being shown.
    fn current_slot(&self) -> tab_strip::Slot {
        if self.home_active() {
            tab_strip::Slot::Home
        } else {
            tab_strip::Slot::Map(self.win.active)
        }
    }

    fn show_slot(&mut self, slot: tab_strip::Slot) {
        match slot {
            tab_strip::Slot::Home => self.win.home_active = true,
            tab_strip::Slot::Map(index) => self.focus_tab(index),
        }
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
    fn remove_tab(&mut self, index: usize) {
        let mut tab = self.win.tabs.remove(index);
        tab.recovery.reset();
        if index < self.win.active || self.win.active >= self.win.tabs.len() {
            self.win.active = self.win.active.saturating_sub(1);
        }
    }

    /// The index of the tab showing the Map file at `path`, comparing canonical paths.
    fn tab_for(&self, path: &Path) -> Option<usize> {
        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or(path.to_path_buf());
        let wanted = canonical(path);
        self.win.tabs.iter().position(|tab| {
            tab.document
                .path()
                .is_some_and(|open| canonical(open) == wanted)
        })
    }

    /// A new tab for `map`, with this run's art and a Recovery journal of its own.
    fn new_tab(&self, map: Map, document: Document) -> MapTab {
        let recovery = Recovery::new(self.app.recovery_dir.clone());
        let mut tab = MapTab::new(map, self.app.gpu.then(TessellationCache::new), recovery);
        tab.document = document;
        tab.editor.set_sprites(self.app.sprites.clone());
        tab
    }

    /// Adds `tab` after the others and shows it. A blank tab on show takes it instead, so a
    /// Map opened or created from the start page opens in that tab.
    fn push_tab(&mut self, tab: MapTab) {
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

    fn push_blank(&mut self, tab: MapTab) {
        self.win.tabs.push(tab);
        self.win.active = self.win.tabs.len() - 1;
        self.win.home_active = false;
    }

    /// Whether the start page of a blank tab is what the window shows.
    fn start_page_showing(&self) -> bool {
        !self.home_active()
            && self
                .win
                .tabs
                .get(self.win.active)
                .is_some_and(MapTab::is_blank)
    }

    /// Whether a Map's editor is what the window shows: neither Home nor a start page.
    fn map_showing(&self) -> bool {
        !self.home_active() && !self.start_page_showing()
    }

    /// Shows a message on the active tab's status bar, or on stderr when no Map is open.
    fn notify_error(&mut self, message: String) {
        match self.win.tabs.get_mut(self.win.active) {
            Some(tab) => tab.editor.error(message),
            None => eprintln!("{message}"),
        }
    }

    /// The window title, as last sent to the window.
    pub fn title(&self) -> &str {
        &self.win.title
    }

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

    /// The active tab: the one whose Map the window shows and every command acts on. Panics
    /// while no Map is open.
    fn tab(&self) -> &MapTab {
        &self.win.tabs[self.win.active]
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
    fn replay_recovery(&mut self, found: Found) {
        let tab = &mut self.win.tabs[self.win.active];
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
        let tab = &mut self.win.tabs[self.win.active];
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

    /// Closing the window or quitting with unsaved changes asks about each tab that has some, in
    /// turn; a clean close deletes every Recovery journal. Where closing the window leaves the
    /// app running, only a quit request exits.
    fn handle_close(&mut self, ctx: &egui::Context) {
        let quit = std::mem::take(&mut self.app.quit_requested);
        let window = ctx.input(|input| input.viewport().close_requested());
        if (!quit && !window) || self.win.close_allowed {
            return;
        }
        // A question already on screen is the close in progress.
        let asking =
            self.win.awaiting_save.is_some() || self.win.tabs.iter().any(|t| t.pending.is_some());
        if quit {
            self.win.leaving = Leaving::Quit;
        } else if !asking {
            self.win.leaving = if self.app.close_hides_window {
                Leaving::Window
            } else {
                Leaving::Quit
            };
        }
        let unsaved = self.win.tabs.iter().any(MapTab::has_unsaved_changes);
        if window && (unsaved || self.win.leaving == Leaving::Window) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if unsaved {
            if !asking {
                self.advance_quit(ctx);
            }
        } else if quit || self.win.leaving == Leaving::Window {
            self.close(ctx);
        } else {
            for tab in &mut self.win.tabs {
                tab.recovery.reset();
            }
        }
    }

    /// Asks to quit the app, as ⌘Q does. Unsaved Maps are asked about first.
    pub fn request_quit(&mut self) {
        self.app.quit_requested = true;
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
        self.app.close_hides_window = hides;
    }

    /// Quitting: shows the next tab with unsaved changes that has not been decided about and
    /// asks what to do with it; closes the window once none is left.
    fn advance_quit(&mut self, ctx: &egui::Context) {
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
    fn cancel_quit(&mut self) {
        for tab in &mut self.win.tabs {
            tab.quit_decided = false;
        }
    }

    /// Leaves, discarding unsaved changes and their Recovery journals: quits the app, or closes
    /// the window and its tabs and leaves the app running.
    fn close(&mut self, ctx: &egui::Context) {
        for tab in &mut self.win.tabs {
            tab.recovery.reset();
        }
        match self.win.leaving {
            Leaving::Quit => {
                self.win.close_allowed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
    fn dialog_open(&self) -> bool {
        self.win
            .tabs
            .get(self.win.active)
            .is_some_and(|tab| tab.pending.is_some() || tab.recovery_prompt.is_some())
            || self.win.tabs.iter().any(|tab| tab.pending.is_some())
            || self.win.awaiting_save.is_some()
            || self.app.unsaved_prompt.is_some()
            || self.win.delete_prompt.is_some()
            || self.win.new_map_dialog.open
            || self.win.export_dialog.open
            || self.win.export_run.is_some()
            || self.win.open_dialog.is_open()
            || self.win.save_dialog.is_open()
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
        self.win.close_allowed
    }

    /// Saves to the open file, or asks where to save a Map that has never been saved.
    pub fn save(&mut self) -> bool {
        // A gesture still open would otherwise reach the journal after the save holding it.
        self.win.tabs[self.win.active].editor.end_gesture();
        self.win.tabs[self.win.active].editor.embed_used_assets();
        let preview = self.preview_png();
        let tab = &mut self.win.tabs[self.win.active];
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
        self.win.tabs[self.win.active].editor.end_gesture();
        self.win.tabs[self.win.active].editor.embed_used_assets();
        let preview = self.preview_png();
        let path = document::with_extension(path.to_path_buf());
        let tab = &mut self.win.tabs[self.win.active];
        let result = tab
            .document
            .save_as(&path, &tab.editor.map, preview.as_deref());
        self.saved(result)
    }

    fn saved(&mut self, result: Result<(), ssmap_core::file::FileError>) -> bool {
        let tab = &mut self.win.tabs[self.win.active];
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

    fn show_open_dialog(&mut self) {
        let directory = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|tab| tab.document.path())
            .and_then(Path::parent);
        self.win.open_dialog.open_map(&self.app.egui_ctx, directory);
    }

    fn show_save_dialog(&mut self) {
        let tab = &self.win.tabs[self.win.active];
        let directory = tab.document.path().and_then(Path::parent);
        self.win
            .save_dialog
            .save_map(&self.app.egui_ctx, directory, &tab.document.name());
    }

    /// The preview PNG stored with each save, rendered like the screen.
    fn preview_png(&mut self) -> Option<Vec<u8>> {
        let render_state = self.app.render_state.as_ref()?;
        let tab = &mut self.win.tabs[self.win.active];
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

    /// Stops the egui handler from reacting to `command`'s key, or starts again.
    pub fn set_shortcut_enabled(&mut self, command: menu::Command, enabled: bool) {
        if enabled {
            self.app.shortcuts_off.remove(&command);
        } else {
            self.app.shortcuts_off.insert(command);
        }
    }

    /// Loads the packs under `roots` afresh, for after one is installed or deleted. Every
    /// open Map, the Asset picker and the Canvas see the change on the next frame, without a
    /// restart. Returns what could not be loaded.
    pub fn reload_packs(&mut self, roots: &PackRoots) -> Vec<ssmap_core::asset::LoadProblem> {
        let (library, problems) = PackLibrary::load(roots);
        self.app.credits = Credits::new(&library);
        self.app.sprites.set_library(Arc::new(library));
        problems
    }

    /// The store the open Maps draw from, for tests.
    pub fn sprites(&self) -> &SpriteStore {
        &self.app.sprites
    }

    /// The Editor of the Map showing, for tests; `None` on Home.
    pub fn active_editor_mut(&mut self) -> Option<&mut Editor> {
        if !self.map_showing() {
            return None;
        }
        self.win
            .tabs
            .get_mut(self.win.active)
            .map(|tab| &mut tab.editor)
    }

    /// What the menus depend on, from the active tab.
    fn menu_state(&self) -> menu::State {
        let mut state = menu::State {
            updates_at_startup: self.app.updates.enabled() == Some(true),
            checking_for_updates: self.app.updates.is_checking(),
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
        let editor = &self.tab().editor;
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
        state.can_paste = editor.has_clipboard();
        state.can_select_all = !editor.active_layer_objects().is_empty();
        state.has_active_layer = editor
            .active_layer()
            .is_some_and(|id| editor.map.contains(id));
        state.can_combine = editor.can_combine();
        state.can_split = editor.can_split();
        state.can_shuffle = editor.can_shuffle();
        state.can_re_export = self.re_export_target().is_some() && self.win.export_run.is_none();
        state
    }

    /// The menu bar, drawn from [`menu::menus`].
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
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
    fn native_menu_shown(&self) -> bool {
        #[cfg(target_os = "macos")]
        return self.app.native_menu.is_some();
        #[cfg(not(target_os = "macos"))]
        false
    }

    /// Runs the native menu items chosen since the last frame.
    #[cfg(target_os = "macos")]
    fn native_menu_actions(&mut self, ctx: &egui::Context) {
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
    fn native_menu_sync(&mut self, ctx: &egui::Context) {
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

    fn open_export_dialog(&mut self) {
        let tab = &self.win.tabs[self.win.active];
        let directory = tab.document.path().and_then(Path::parent);
        self.win.export_dialog.open_with(
            &tab.editor.map.export_settings,
            directory.map(Path::to_path_buf),
        );
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
            C::Credits => self.app.credits.open = true,
            C::CheckForUpdates => self.app.updates.start(true),
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
        let tab = &mut self.win.tabs[self.win.active];
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
                    editor.paste(at);
                }
            }
            C::Duplicate => {
                editor.duplicate_selection();
            }
            C::SelectAll => editor.select_all(),
            C::Deselect => editor.deselect(),
            C::DeleteNodes => editor.delete_selected_nodes(),
            C::FinishNodeEdit => editor.leave_node_edit(),
            C::CancelGesture => editor.cancel_gesture(),
            C::ZoomIn => tab.canvas_view.request_zoom(zoom::Zoom::In),
            C::ZoomOut => tab.canvas_view.request_zoom(zoom::Zoom::Out),
            C::ZoomFit => tab.canvas_view.request_zoom(zoom::Zoom::Fit),
            C::ZoomSelection => tab.canvas_view.request_zoom(zoom::Zoom::Selection),
            C::ActualSize => tab.canvas_view.request_zoom(zoom::Zoom::Percent(1.0)),
            C::EmbedAssets => {
                editor.end_gesture();
                if !editor.embed_used_assets() {
                    editor.info("Nothing was embedded");
                }
            }
            C::UpdateFromPacks => {
                editor.end_gesture();
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

    /// File dialogs and the unsaved-changes question.
    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Polled::Chosen(path) = self.win.open_dialog.poll() {
            self.open_path(&path);
        }
        match self.win.save_dialog.poll() {
            Polled::Chosen(path) => {
                let saved = self.save_as_path(&path);
                match self.win.awaiting_save.take() {
                    Some((index, pending)) if saved && index == self.win.active => {
                        self.finish(pending, ctx);
                    }
                    Some(_) => self.cancel_quit(),
                    None => {}
                }
            }
            Polled::Cancelled if self.win.awaiting_save.is_some() => {
                // The Save As dialog was closed without saving: the question is cancelled.
                self.win.awaiting_save = None;
                self.cancel_quit();
            }
            _ => {}
        }

        if let Some(canvas) = self.win.new_map_dialog.show(ctx) {
            let template = self.win.new_map_dialog.chosen_template();
            self.new_map_from(template, canvas);
        }
        if let Some(tab) = self.win.tabs.get_mut(self.win.active) {
            match self.win.export_dialog.show(ctx, &tab.editor.map.canvas) {
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
            let trashed = file_actions::move_to_trash(&path);
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

    /// The folder the dialog's Export button writes into: the one typed in the dialog.
    fn export_from_dialog(&mut self) {
        let directory = PathBuf::from(self.win.export_dialog.directory.trim());
        let directory = if directory.as_os_str().is_empty() {
            self.document_directory().unwrap_or_default()
        } else {
            directory
        };
        let tab = &mut self.win.tabs[self.win.active];
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
            self.win.tabs[self.win.active]
                .editor
                .error("Export the Map once with Export PNG… first");
            return;
        }
        if self.tab().export_directory.is_none() {
            self.win.tabs[self.win.active].export_directory = self.document_directory();
        }
        self.start_export();
    }

    /// Starts exporting the active tab to the stored file name in `export_directory`. Only
    /// one export runs at a time.
    fn start_export(&mut self) {
        if self.win.export_run.is_some() {
            return;
        }
        if self.app.render_state.is_none() {
            self.win.tabs[self.win.active]
                .editor
                .error("Export needs a graphics adapter");
            return;
        }
        let Some(dest) = self.re_export_target() else {
            self.win.tabs[self.win.active]
                .editor
                .error("The export has no file name");
            return;
        };
        let tab = &mut self.win.tabs[self.win.active];
        let settings = tab.editor.map.export_settings.clone();
        let output = settings.credits.output(&tab.editor.credit_lines());
        let mut meta =
            PngMeta::default().with_software("StarScript Map Maker", env!("CARGO_PKG_VERSION"));
        if let Some(chunk) = &output.chunk {
            meta = meta.with_credits(CREDITS_CHUNK_KEYWORD, chunk);
        }
        match ExportJob::new(&tab.editor.map, &settings, meta, &dest) {
            Ok(job) => self.win.export_run = Some(ExportRun::new(job, dest, output.file)),
            Err(err) => tab.editor.error(format!("Export failed: {err}")),
        }
    }

    /// Steps the running export once and shows its progress dialog.
    fn export_progress(&mut self, ctx: &egui::Context) {
        let Some(run) = self.win.export_run.as_mut() else {
            return;
        };
        let Some(render_state) = self.app.render_state.as_ref() else {
            return;
        };
        let tab = &mut self.win.tabs[self.win.active];
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
                    self.win.export_run = None;
                    tab.editor.info("Export cancelled");
                } else {
                    // Keep stepping even when no input arrives.
                    ctx.request_repaint();
                }
            }
            Outcome::Finished(size) => {
                self.win.export_run = None;
                tab.editor.info(format!(
                    "Exported {} × {} px to {}",
                    size.width,
                    size.height,
                    dest.display()
                ));
            }
            Outcome::Failed(message) => {
                self.win.export_run = None;
                tab.editor.error(format!("Export failed: {message}"));
            }
        }
    }

    /// Asks what to do with the active tab's unsaved changes before it closes or the app quits.
    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
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
        self.win.tabs[self.win.active].pending = None;
        match choice {
            UnsavedChoice::Save if self.tab().document.path().is_none() => {
                // A Map never saved is saved through Save As.
                self.win.awaiting_save = Some((self.win.active, pending));
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
            Pending::CloseTab => self.remove_tab(self.win.active),
            Pending::Quit => {
                self.win.tabs[self.win.active].quit_decided = true;
                self.advance_quit(ctx);
            }
        }
    }

    /// Offers a Recovery journal left by a crash: the active tab's Map file's, or else one of
    /// a Map never saved, which opens in a tab of its own.
    fn recovery_dialog(&mut self, ctx: &egui::Context) {
        let tab_asking = self
            .win
            .tabs
            .get(self.win.active)
            .is_some_and(|tab| tab.recovery_prompt.is_some() || tab.pending.is_some());
        if !tab_asking && self.win.awaiting_save.is_none() && self.app.unsaved_prompt.is_none() {
            self.app.unsaved_prompt = self.app.unsaved_found.pop();
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
        let prompt = self.win.tabs[self.win.active]
            .recovery_prompt
            .take()
            .expect("shown above");
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
        if self.app.unsaved_prompt.is_none() {
            return;
        }
        let text = "A Map that was never saved wasn’t closed properly. Recover it?";
        let Some(choice) = recovery_modal(ctx, text, "Recover") else {
            return;
        };
        let found = self.app.unsaved_prompt.take().expect("shown above");
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

    /// Shows the active Map's file name in the window title, with a dot while it has unsaved
    /// changes.
    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.win.tabs.get(self.win.active) {
            Some(tab) => {
                let dirty = if tab.has_unsaved_changes() {
                    " •"
                } else {
                    ""
                };
                format!("{}{dirty} — StarScript Map Maker", tab.name())
            }
            None => "StarScript Map Maker".to_owned(),
        };
        if title != self.win.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.win.title = title;
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
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
            let slot = tab_strip::cycle(self.current_slot(), self.win.tabs.len(), forward);
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
        let editor = &mut self.win.tabs[self.win.active].editor;
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
            if consume(input, &off, menu::Command::CancelGesture) {
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
            // Shifted shortcuts first: egui ignores an extra Shift, so ⇧S would also read as S.
            let mut tools = ToolId::ALL;
            tools.sort_by_key(|tool| !tool.shortcut().0.shift);
            for tool in tools {
                let (modifiers, key) = tool.shortcut();
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

    /// The strip of tabs in the title-bar area: Home, one tab per open Map, and a +.
    fn tab_strip(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let infos: Vec<_> = self
            .win
            .tabs
            .iter()
            .map(|tab| tab_strip::TabInfo {
                name: tab.name(),
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
                    &mut self.win.strip,
                    CHROME_INSET,
                    CUSTOM_CHROME,
                )
            })
            .inner;
        for action in actions {
            match action {
                tab_strip::Action::Show(slot) => self.show_slot(slot),
                tab_strip::Action::Close(index) => self.close_tab(index),
                tab_strip::Action::NewMap => self.new_blank_tab(),
            }
        }
    }

    /// The Home tab: the Maps the app knows of, and what the user does with them. With
    /// `start_page` it is the lighter start page of a blank tab, where a Map opens in that tab.
    fn home_screen(&mut self, ui: &mut egui::Ui, start_page: bool) {
        let ctx = ui.ctx().clone();
        let tabs: Vec<home::OpenTab> = self
            .win
            .tabs
            .iter()
            .map(|tab| home::OpenTab {
                path: tab.document.path().map(Path::to_path_buf),
                name: tab.document.name(),
                canvas: (tab.editor.map.canvas.width, tab.editor.map.canvas.height),
                blank: tab.is_blank(),
            })
            .collect();
        self.win.home.entered();
        self.win.home.poll(&ctx);
        let wanted: Vec<PathBuf> = self
            .app
            .recent
            .paths()
            .iter()
            .cloned()
            .chain(tabs.iter().filter_map(|tab| tab.path.clone()))
            .collect();
        self.win.home.request(wanted.iter().map(PathBuf::as_path));
        let known = home::known_entries(self.app.recent.paths(), &tabs, |path| {
            self.win.home.info(path)
        });
        let open = home::open_entries(&tabs, |path| self.win.home.info(path));
        let data = home::Data {
            known: &known,
            open: &open,
            packs: self.app.credits.page().packs.len(),
            sprites: Some(&self.app.sprites),
        };
        self.app.credits.show(&ctx, None);
        self.app.updates.show(&ctx);
        let actions = if start_page {
            self.win.home.start_page(ui, &data)
        } else {
            self.win.home.show(ui, &data, &mut self.app.updates)
        };
        if self.win.home.loading() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        for action in actions {
            match action {
                home::Action::NewMap => self.win.new_map_dialog.open = true,
                home::Action::OpenDialog => self.show_open_dialog(),
                home::Action::Open(path) => {
                    self.open_path(&path);
                }
                home::Action::Focus(index) => self.focus_tab(index),
                home::Action::Remove(path) => {
                    self.app.recent.remove(&path);
                    self.win.home.forget(&path);
                }
                home::Action::Reveal(path) => {
                    if let Err(err) = file_actions::reveal(&path) {
                        self.notify_error(format!("Couldn’t show the file: {err}"));
                    }
                }
                home::Action::Delete(path) => self.win.delete_prompt = Some(path),
                home::Action::Credits => self.app.credits.open = true,
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.win.tabs.get(self.win.active) else {
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
                let real_world = self.win.shell.current_layout().real_world_coordinates;
                ui.label(text(shell::cursor_text(
                    &tab.editor.map.canvas,
                    point,
                    real_world,
                )));
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

/// What a close in progress ends in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Leaving {
    /// The app exits.
    Quit,
    /// The window closes and the app keeps running.
    Window,
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

impl eframe::App for MapMakerApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.recent.store(storage);
        self.win.shell.store(storage);
        self.app.updates.store(storage);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        self.native_menu_actions(ui.ctx());
        self.show(ui);
        // After the frame, so the menu shows what this frame changed.
        #[cfg(target_os = "macos")]
        self.native_menu_sync(ui.ctx());
    }
}

impl MapMakerApp {
    /// One frame of the editor.
    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.app.overlay.begin_frame(&ctx);
        self.app.overlay.show(&ctx);
        // ⌘+ and ⌘− zoom the Canvas only: keep egui's whole-UI zoom off and the scale at 1.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        if ctx.zoom_factor() != 1.0 {
            ctx.set_zoom_factor(1.0);
        }
        // What Copy or Cut took last frame goes to the system clipboard as text.
        if let Some(text) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|tab| tab.editor.take_clipboard_text())
        {
            ctx.copy_text(text);
        }
        // Dialogs take the keyboard while they are open.
        self.handle_close(&ctx);
        let dialog_open = self.dialog_open();
        if std::mem::take(&mut self.win.drop_focus) {
            // The ⇥ that toggled the panels also moved keyboard focus into one of them.
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        self.win.shell.set_window_width(ctx.content_rect().width());
        let panels_were_hidden = self.win.shell.panels_hidden();
        if !dialog_open && self.win.shell.overlay_open() {
            // Esc closes an open panel overlay before anything else sees it.
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.win.shell.close_overlays();
            }
        }
        if !dialog_open {
            let _input = ssmap_render::profile::stage(ssmap_render::profile::Stage::Input);
            self.open_dropped_files(&ctx);
            self.shortcuts(&ctx);
        }
        self.win.drop_focus = self.win.shell.panels_hidden() != panels_were_hidden;
        self.dialogs(&ctx);
        self.update_title(&ctx);
        let strip = egui::Frame::NONE.fill(theme::palette(&ctx).bg_chrome);
        egui::Panel::top("tabs")
            .exact_size(theme::metrics::TITLE_BAR_WITH_TABS)
            .frame(strip)
            .show(ui, |ui| self.tab_strip(ui, !dialog_open));
        if !self.native_menu_shown() {
            egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        }
        if self.home_active() || self.start_page_showing() {
            self.home_screen(ui, !self.home_active());
            self.journal(&ctx);
            return;
        }
        self.win.home.left();
        let tab = &mut self.win.tabs[self.win.active];
        self.app.credits.show(&ctx, Some(&mut tab.editor));
        tab.label_styles.show(&ctx, &mut tab.editor);
        self.app.updates.show(&ctx);
        egui::Panel::bottom("status")
            .exact_size(theme::metrics::STATUS_BAR)
            .show(ui, |ui| self.status_bar(ui));
        let tab = &mut self.win.tabs[self.win.active];
        let layout = self.win.shell.current_layout();
        // The window grew: an overlay for a panel that now shows in full has nothing to open.
        if layout.layers != shell::PanelMode::Rail {
            self.win.shell.set_layers_overlay(false);
        }
        if layout.properties != shell::PanelMode::Rail {
            self.win.shell.set_properties_overlay(false);
        }
        let mut layers_divider = None;
        let mut properties_divider = None;
        let mut layers_rail = None;
        let mut properties_rail = None;
        match layout.layers {
            shell::PanelMode::Hidden => {}
            shell::PanelMode::Rail => {
                let panel = egui::Panel::left("layers")
                    .resizable(false)
                    .exact_size(layout.layers.width())
                    .show(ui, |ui| {
                        let path = shell::selection_path(&tab.editor.map, tab.editor.selection());
                        let current = self.win.shell.sidebar_section();
                        if let Some(click) = shell::layers_rail(ui, &path, current) {
                            self.win.shell.click_layers_rail(click);
                        }
                    });
                layers_rail = Some(panel.response.rect);
            }
            shell::PanelMode::Full(width) => {
                let panel = egui::Panel::left("layers")
                    .resizable(false)
                    .exact_size(width)
                    .show(ui, |ui| {
                        layers_panel(ui, &mut self.win.shell, tab, layout.layers_footer, false)
                    });
                layers_divider = Some(panel.response.rect);
            }
        }
        match layout.properties {
            shell::PanelMode::Hidden => {}
            shell::PanelMode::Rail => {
                let panel = egui::Panel::right("properties")
                    .resizable(false)
                    .exact_size(layout.properties.width())
                    .show(ui, |ui| {
                        if shell::properties_rail(ui) {
                            let open = self.win.shell.properties_overlay();
                            self.win.shell.set_properties_overlay(!open);
                        }
                    });
                properties_rail = Some(panel.response.rect);
            }
            shell::PanelMode::Full(width) => {
                let panel = egui::Panel::right("properties")
                    .resizable(false)
                    .exact_size(width)
                    .show(ui, |ui| tab.properties.show(ui, &mut tab.editor));
                if tab.properties.take_manage_label_styles() {
                    tab.label_styles.open = true;
                }
                properties_divider = Some(panel.response.rect);
            }
        }
        let canvas = egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| tab.canvas_view.show(ui, &mut tab.editor));
        tool_dock::show(&ctx, canvas.response.rect, &mut tab.editor.active_tool);
        if let Some(action) = toast::show(&ctx, canvas.response.rect, tab.editor.toasts_mut()) {
            tab.editor.run_toast_action(action);
        }
        // After the Canvas, so the strips on its edges win the pointer over it.
        for (side, rect) in [
            (shell::Side::Left, layers_divider),
            (shell::Side::Right, properties_divider),
        ] {
            if let Some(rect) = rect {
                shell::divider(ui, &mut self.win.shell, side, rect);
            }
        }
        // A rail's panel opened over the Canvas: Layers from the left, Properties as a
        // floating inspector pinned to the top right.
        let canvas_rect = canvas.response.rect;
        let palette = theme::palette(&ctx);
        let overlay_frame = egui::Frame::new()
            .fill(palette.bg_panel)
            .stroke(egui::Stroke::new(1.0, palette.border_strong))
            .shadow(egui::Shadow {
                offset: [0, 4],
                blur: 16,
                spread: 0,
                color: theme::SCRIM,
            });
        let mut overlay_rect = None;
        if self.win.shell.layers_overlay() {
            let width = self.win.shell.width(shell::Side::Left);
            let area = egui::Area::new(egui::Id::new("layers overlay"))
                .order(egui::Order::Foreground)
                .fixed_pos(canvas_rect.min)
                .show(&ctx, |ui| {
                    overlay_frame.show(ui, |ui| {
                        ui.set_width(width);
                        ui.set_height(canvas_rect.height());
                        layers_panel(ui, &mut self.win.shell, tab, true, true);
                    });
                });
            overlay_rect = Some(area.response.rect);
        } else if self.win.shell.properties_overlay() {
            const MARGIN: f32 = 8.0;
            let width = self.win.shell.width(shell::Side::Right);
            let height = (canvas_rect.height() - 2.0 * MARGIN).max(0.0);
            let area = egui::Area::new(egui::Id::new("properties inspector"))
                .order(egui::Order::Foreground)
                .fixed_pos(canvas_rect.right_top() + egui::vec2(-width - MARGIN, MARGIN))
                .show(&ctx, |ui| {
                    overlay_frame.show(ui, |ui| {
                        ui.set_width(width);
                        ui.set_height(height);
                        tab.properties.show(ui, &mut tab.editor);
                    });
                });
            if tab.properties.take_manage_label_styles() {
                tab.label_styles.open = true;
            }
            overlay_rect = Some(area.response.rect);
        }
        if let Some(overlay_rect) = overlay_rect {
            // A press anywhere else closes it. The rails are left out, as they toggle it.
            let outside = ctx.input(|input| {
                input.pointer.any_pressed()
                    && input.pointer.interact_pos().is_some_and(|pos| {
                        !overlay_rect.contains(pos)
                            && !layers_rail.is_some_and(|rail| rail.contains(pos))
                            && !properties_rail.is_some_and(|rail| rail.contains(pos))
                    })
            });
            if outside {
                self.win.shell.close_overlays();
            }
        }
        self.journal(&ctx);
    }
}

/// The Layers panel: its header, the tree and the bottom bar with the collapse control. As an
/// overlay, collapsing closes it.
fn layers_panel(
    ui: &mut egui::Ui,
    shell: &mut shell::Shell,
    tab: &mut MapTab,
    footer: bool,
    overlay: bool,
) {
    if shell.sidebar_section() == shell::SidebarSection::History {
        shell::panel_header(ui, "History", |_| {});
        let (steps, applied) = tab.editor.history_steps();
        if let Some(target) = shell::history_list(ui, &steps, applied) {
            tab.editor.jump_to_history(target);
        }
        return;
    }
    shell::panel_header(ui, "Layers", |ui| {
        if icons::icon_button(ui, icons::Icon::Add, "New Layer (⇧⌘N)").clicked() {
            tab.editor.new_layer();
        }
    });
    if tab.layer_tree.show(ui, &mut tab.editor, footer) {
        if overlay {
            shell.close_overlays();
        } else {
            shell.set_layers_collapsed(true);
        }
    }
}

/// Runs `on_deleted` (closing the tab, forgetting the file) only if the Trash move succeeded.
fn after_trash(trashed: Result<(), String>, on_deleted: impl FnOnce()) -> Result<(), String> {
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

    fn args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

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
