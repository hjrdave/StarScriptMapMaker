//! The app itself: [`MapMakerApp`], its state, and the `eframe::App` impl. The methods are
//! spread over the sibling modules by what they do.

mod close_flow;
mod closing;
mod dialogs;
mod drag_drop;
mod export_flow;
mod files;
mod frame;
mod menus;
mod packs;
mod panels;
mod tabs;
mod unsaved_offer;

use eframe::{egui, egui_wgpu};
use native_dialog::{Pick, Polled};
use ssmap_core::asset::{PackLibrary, PackRoots, TrashError};
use ssmap_core::file::{JournalBase, MapFile, PREVIEW_SIZE, remove_journal};
use ssmap_core::{CREDITS_CHUNK_KEYWORD, Canvas, Map, MapTemplate, TemplateError};
use ssmap_render::{ExportJob, PngMeta, SpriteStore, TessellationCache};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::credits::Credits;
use crate::document::Document;
use crate::editor::Editor;
use crate::export_dialog::DialogAction;
use crate::export_run::{ExportRun, Outcome};
use crate::new_map::NewMapDialog;
use crate::recent::RecentFiles;
use crate::recovery::{Found, Recovery};
use crate::tab::MapTab;
use crate::tools::ToolId;
use crate::update::UpdateCheck;
use crate::{
    canvas_view, clipboard, file_actions, home, menu, native_dialog, native_menu, overlay,
    pack_install, shell, tab_strip, theme,
};

pub(crate) use self::close_flow::Pending;
pub(crate) use self::files::RecoveryPrompt;

use self::close_flow::{CloseFlow, Leaving, QuitState};
use self::drag_drop::{PayloadDrop, TabDrop};
use self::export_flow::ExportFlow;
use self::files::after_trash;
use self::panels::layers_panel;
use self::unsaved_offer::UnsavedOffer;

/// What every window shares: settings, recent Maps, the installed Asset packs, crash recovery
/// and the native menu. Not tied to one window's Maps.
struct AppState {
    /// Where journals of Maps never saved go; `None` turns them off.
    recovery_dir: Option<PathBuf>,
    /// Whether the Canvas renderer is installed, so new tabs get a tessellation cache.
    gpu: bool,
    /// Shows the Open, Save and folder dialogs; tests swap it for one that shows nothing.
    launch_dialog: native_dialog::Launch,
    /// The installed Asset packs' art, shared with the Canvas renderer and picking.
    sprites: SpriteStore,
    credits: Credits,
    /// Where packs are loaded from and added to.
    pack_roots: PackRoots,
    recent: RecentFiles,
    updates: UpdateCheck,
    /// Journals of Maps never saved left by a crash, offered one at a time.
    unsaved: UnsavedOffer,
    /// `None` without a GPU, as in UI tests; saves then store no preview.
    render_state: Option<egui_wgpu::RenderState>,
    /// Asked for a repaint as background work finishes.
    egui_ctx: egui::Context,
    /// The frame-time overlay, off by default (F3 in debug and profiling builds).
    overlay: overlay::Overlay,
    /// A quit asked for (⌘Q), and whether closing the window quits.
    quit: QuitState,
    /// Commands whose key the egui handler leaves alone, because something else (the native
    /// menu on macOS) owns it. Each key is then handled once.
    shortcuts_off: HashSet<menu::Command>,
    /// What Copy or Cut last took in any tab, so Paste works in another tab or Map.
    clipboard: Option<clipboard::Clipboard>,
    /// Tabs sent back to the main window from a popped-out one, taken in once that window's
    /// frame is over.
    returning: Vec<MapTab>,
    /// Tabs dragged and dropped this frame, settled once every window's frame is over.
    tab_drops: Vec<TabDrop>,
    /// Assets and Layers let go this frame, settled like `tab_drops`.
    payload_drops: Vec<PayloadDrop>,
    /// Where on the screen the pointer is while an Asset or Layer is dragged: windows under it
    /// show a drop highlight.
    drag_hover: Option<egui::Pos2>,
    /// The window and tab index whose Layers panel a Layer is being dragged out of.
    drag_source: Option<(egui::ViewportId, usize)>,
    /// Some window showed a drop highlight last frame, so the drag shows the copy cursor.
    drag_copy: bool,
    /// Some window shows a drop highlight this frame.
    drag_over_target: bool,
    /// The native menu bar, on macOS; `None` elsewhere and in UI tests, which use the egui bar.
    #[cfg(target_os = "macos")]
    native_menu: Option<native_menu::mac::MacMenu>,
}

/// A Paste waiting on the answer to "what's missing, proceed?".
#[derive(Clone)]
struct PastePrompt {
    gaps: ssmap_core::PasteGaps,
    /// Where the paste lands, in Canvas units: the mouse when it was asked for.
    at: [f64; 2],
}

/// What belongs to one window: its tabs, panels and the dialogs modal to it. The Home tab
/// lives only in the main window.
struct WindowState {
    /// The open Maps, one per tab, in the order of the tab row. May be empty.
    tabs: Vec<MapTab>,
    /// The index of the active tab: the one whose Map the window shows and every command acts
    /// on, reached through [`MapMakerApp::tab`]. Meaningless while `tabs` is empty.
    active: usize,
    /// Problems to tell the user about while no Map is open to show them on; Home lists them
    /// until dismissed.
    notices: Vec<String>,
    /// Whether the Home tab shows instead of the active tab's Map. Always the case while no
    /// Map is open.
    home_active: bool,
    /// The tab strip's scroll position and the like.
    strip: tab_strip::State,
    /// Where this window's content starts on the screen, to tell which window a dropped tab
    /// is over.
    screen_origin: egui::Pos2,
    /// The size of this window's content, to tell whether a dropped tab is over the window at
    /// all.
    content_size: egui::Vec2,
    /// Where the Canvas and the Layers panel were drawn (in the window's own coordinates), which
    /// an Asset or Layer dragged from another window can be dropped on. `None` while the window
    /// shows no Map.
    canvas_zone: Option<egui::Rect>,
    layers_zone: Option<egui::Rect>,
    /// The Home tab's list, search and thumbnails.
    home: home::Home,
    /// Panel widths and collapse state, the same for every tab in the window.
    shell: shell::Shell,
    /// ⇥ just toggled the panels: clear keyboard focus at the next frame.
    drop_focus: bool,
    new_map_dialog: NewMapDialog,
    export: ExportFlow,
    open_dialog: Pick,
    save_dialog: Pick,
    /// The folder dialog of Add pack….
    pack_dialog: Pick,
    /// The pack being added, or the message about the last one.
    pack_install: pack_install::PackInstall,
    /// The Map file a Home card's Delete asked about; the dialog confirms before it is trashed.
    delete_prompt: Option<PathBuf>,
    /// The id of the Asset pack a card's Delete asked about; the dialog confirms first.
    pack_delete_prompt: Option<String>,
    /// Moves a pack folder to the Trash; tests swap it for one that touches nothing.
    pack_trash: fn(&Path) -> Result<(), TrashError>,
    /// A Paste into a Map that lacks something the copied layers need: the popup asks first.
    paste_prompt: Option<PastePrompt>,
    /// This window's close or quit in progress.
    close: CloseFlow,
    /// The window was closed but the app keeps running, with no Map open.
    window_hidden: bool,
    /// The window title last sent, so it is only sent when it changes.
    title: String,
    /// The main window, as opposed to one a tab was popped out into.
    main: bool,
    /// The viewport this window is shown in.
    viewport: egui::ViewportId,
}

pub struct MapMakerApp {
    app: AppState,
    /// The main window.
    win: WindowState,
    /// The windows tabs were popped out into, each with its own tabs and dialogs. While one of
    /// them is shown its state is swapped into `win`, so everything that acts on "the window"
    /// acts on it.
    popped: Vec<WindowState>,
    /// How many windows were popped out so far, for their viewport ids.
    popped_count: u64,
}

impl WindowState {
    /// Where the Canvas is on the screen.
    fn canvas_on_screen(&self) -> Option<egui::Rect> {
        self.canvas_zone
            .map(|zone| zone.translate(self.screen_origin.to_vec2()))
    }

    /// Where the Layers panel is on the screen.
    fn layers_on_screen(&self) -> Option<egui::Rect> {
        self.layers_zone
            .map(|zone| zone.translate(self.screen_origin.to_vec2()))
    }

    /// Takes tab `index` out with its Recovery journal and everything else, and keeps the same
    /// tab active if it can.
    fn take_tab(&mut self, index: usize) -> MapTab {
        let tab = self.tabs.remove(index);
        if index < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        tab
    }

    /// Whether tab `index` can leave: not a start page or a tab being saved.
    fn can_release(&self, index: usize) -> bool {
        self.tabs
            .get(index)
            .is_some_and(|tab| !tab.is_blank() && tab.pending.is_none())
    }

    /// The whole window content on the screen.
    fn content_on_screen(&self) -> egui::Rect {
        egui::Rect::from_min_size(self.screen_origin, self.content_size)
    }

    /// The strip on the screen.
    fn strip_on_screen(&self) -> egui::Rect {
        self.strip
            .geometry
            .strip
            .translate(self.screen_origin.to_vec2())
    }
}

impl MapMakerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::build(
            cc,
            None,
            Recovery::default_unsaved_dir(),
            native_dialog::launch_native,
        )
    }

    /// Starts on the Home tab with no Map open, keeping journals of Maps never saved in
    /// `recovery_dir` (`None` turns them off) and offering any a crash left there. File
    /// dialogs count as open but never show, for tests without a window.
    pub fn with_recovery_dir(
        cc: &eframe::CreationContext<'_>,
        recovery_dir: Option<PathBuf>,
    ) -> Self {
        Self::build(cc, None, recovery_dir, native_dialog::launch_headless)
    }

    /// Starts with `map` open in a tab. File dialogs count as open but never show, for tests
    /// without a window.
    pub fn with_map(
        cc: &eframe::CreationContext<'_>,
        map: Map,
        recovery_dir: Option<PathBuf>,
    ) -> Self {
        Self::build(cc, Some(map), recovery_dir, native_dialog::launch_headless)
    }

    fn build(
        cc: &eframe::CreationContext<'_>,
        map: Option<Map>,
        recovery_dir: Option<PathBuf>,
        launch_dialog: native_dialog::Launch,
    ) -> Self {
        theme::install(&cc.egui_ctx);
        let (library, problems) = PackLibrary::load(&PackRoots::installed());
        let notices: Vec<String> = problems.iter().map(packs::problem_notice).collect();
        let credits = Credits::new(&library);
        let sprites = SpriteStore::new(Arc::new(library));
        let ctx = cc.egui_ctx.clone();
        sprites.set_repaint(move || ctx.request_repaint());
        let gpu = match cc.wgpu_render_state.as_ref() {
            Some(render_state) => canvas_view::install(render_state, sprites.clone()),
            None => false,
        };
        let unsaved = UnsavedOffer::new(Recovery::new(recovery_dir.clone()).find_unsaved());
        let tabs: Vec<MapTab> = map
            .into_iter()
            .map(|map| {
                let mut tab = MapTab::new(
                    map,
                    gpu.then(TessellationCache::new),
                    Recovery::new(recovery_dir.clone()),
                );
                tab.editor.set_sprites(sprites.clone());
                if gpu {
                    let ctx = cc.egui_ctx.clone();
                    tab.editor
                        .compute_shore_rings_in_background(move || ctx.request_repaint());
                }
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
            launch_dialog,
            sprites,
            credits,
            pack_roots: PackRoots::installed(),
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
            unsaved,
            render_state: cc.wgpu_render_state.clone(),
            egui_ctx: cc.egui_ctx.clone(),
            quit: QuitState::new(cfg!(target_os = "macos")),
            overlay: overlay::Overlay::default(),
            clipboard: None,
            returning: Vec::new(),
            tab_drops: Vec::new(),
            payload_drops: Vec::new(),
            drag_hover: None,
            drag_source: None,
            drag_copy: false,
            drag_over_target: false,
        };
        let win = WindowState {
            home_active: tabs.is_empty(),
            tabs,
            active: 0,
            notices,
            strip: tab_strip::State::default(),
            screen_origin: egui::Pos2::ZERO,
            content_size: egui::Vec2::ZERO,
            canvas_zone: None,
            layers_zone: None,
            home: home::Home::default(),
            delete_prompt: None,
            pack_delete_prompt: None,
            pack_trash: file_actions::move_to_trash,
            paste_prompt: None,
            shell: shell::Shell::load(cc.storage),
            drop_focus: false,
            new_map_dialog: NewMapDialog::default(),
            export: ExportFlow::new(launch_dialog),
            open_dialog: Pick::new(launch_dialog),
            save_dialog: Pick::new(launch_dialog),
            pack_dialog: Pick::new(launch_dialog),
            pack_install: pack_install::PackInstall::default(),
            close: CloseFlow::new(Leaving::Quit),
            window_hidden: false,
            title: String::new(),
            main: true,
            viewport: egui::ViewportId::ROOT,
        };
        Self {
            app,
            win,
            popped: Vec::new(),
            popped_count: 0,
        }
    }

    /// The active tab's Editor.
    ///
    /// # Panics
    ///
    /// Panics while no Map is open (Home showing with no tabs).
    #[allow(clippy::expect_used, reason = "no Map is open")]
    pub fn editor(&self) -> &Editor {
        self.tab().expect("no Map is open").editor()
    }

    /// The panels' widths and collapse state.
    pub fn shell(&self) -> &shell::Shell {
        &self.win.shell
    }

    /// The active tab's Document.
    ///
    /// # Panics
    ///
    /// Panics while no Map is open (Home showing with no tabs).
    #[allow(clippy::expect_used, reason = "no Map is open")]
    pub fn document(&self) -> &Document {
        self.tab().expect("no Map is open").document()
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
        self.tab().is_some_and(|tab| tab.canvas_view.editing_text())
    }

    /// The screen position of a Canvas point in the Canvas panel, once it has been drawn.
    pub fn canvas_to_screen(&self, point: [f64; 2]) -> Option<egui::Pos2> {
        self.tab()?.canvas_view.to_screen(point)
    }

    /// The Recovery journal being written for the active tab's Map, once there is one.
    pub fn recovery_journal(&self) -> Option<&Path> {
        self.tab()?.recovery_journal()
    }

    /// Whether edits are waiting to be written to the active tab's Recovery journal.
    pub fn has_unjournaled_changes(&self) -> bool {
        self.tab().is_some_and(|tab| tab.recovery.has_pending())
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
        self.win.main && (self.win.home_active || self.win.tabs.is_empty())
    }

    /// The active tab: the one whose Map the window shows and every command acts on. `None`
    /// while no Map is open.
    fn tab(&self) -> Option<&MapTab> {
        self.win.tabs.get(self.win.active)
    }

    /// The active tab, mutably; `None` while no Map is open.
    fn tab_mut(&mut self) -> Option<&mut MapTab> {
        self.win.tabs.get_mut(self.win.active)
    }
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
        self.app.drag_copy = std::mem::take(&mut self.app.drag_over_target);
        self.show(ui);
        self.show_popped(ui.ctx());
        // After the frame, so the menu shows what this frame changed.
        #[cfg(target_os = "macos")]
        self.native_menu_sync(ui.ctx());
    }
}
