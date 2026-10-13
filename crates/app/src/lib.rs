//! StarScript Map Maker's editor. The binary in `main.rs` only opens a window around
//! [`MapMakerApp`], so UI tests can drive the same app headlessly.

mod canvas_view;
pub mod document;
pub mod editor;
pub mod handles;
mod layer_tree;
mod properties;
mod recent;
pub mod tools;

use eframe::{egui, egui_wgpu};
use egui_file_dialog::{DialogState, FileDialog, Filter};
use ssmap_core::file::PREVIEW_SIZE;
use ssmap_core::{Canvas, DistanceUnit, Map, Scale};
use std::path::{Path, PathBuf};

use crate::canvas_view::CanvasView;
use crate::document::Document;
use crate::editor::Editor;
use crate::layer_tree::LayerTree;
use crate::properties::PropertiesPanel;
use crate::recent::RecentFiles;
use crate::tools::ToolId;

pub struct MapMakerApp {
    editor: Editor,
    document: Document,
    layer_tree: LayerTree,
    properties: PropertiesPanel,
    canvas_view: CanvasView,
    recent: RecentFiles,
    open_dialog: FileDialog,
    save_dialog: FileDialog,
    /// A file to open once the user decides what to do with unsaved changes.
    pending_open: Option<PathBuf>,
    /// `None` without a GPU, as in UI tests; saves then store no preview.
    render_state: Option<egui_wgpu::RenderState>,
    /// The window title last sent, so it is only sent when it changes.
    title: String,
}

impl MapMakerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Until the New Map dialog exists, every session starts with a blank 4096x3072 Map.
        let map = Map::new(Canvas::new(
            4096,
            3072,
            Scale::new(0.25, DistanceUnit::Miles),
        ));
        Self::with_map(cc, map)
    }

    pub fn with_map(cc: &eframe::CreationContext<'_>, map: Map) -> Self {
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
        Self {
            editor: Editor::new(map),
            document: Document::default(),
            layer_tree: LayerTree::default(),
            properties: PropertiesPanel::default(),
            canvas_view: CanvasView::new(gpu),
            recent: RecentFiles::load(cc.storage),
            open_dialog: map_files().title("Open Map"),
            save_dialog: map_files().title("Save Map As"),
            pending_open: None,
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

    /// The window title, as last sent to the window.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Opens a Map file in place of the current Map, without asking about unsaved changes.
    pub fn open_path(&mut self, path: &Path) -> bool {
        match Document::open(path) {
            Ok((document, map)) => {
                self.document = document;
                self.editor = Editor::new(map);
                self.layer_tree = LayerTree::default();
                self.properties = PropertiesPanel::default();
                self.canvas_view.refit();
                self.recent.add(path);
                self.editor
                    .info(format!("Opened “{}”", self.document.name()));
                true
            }
            Err(err) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.editor.error(format!("Couldn’t open “{name}”: {err}"));
                false
            }
        }
    }

    /// Opens a Map file, first asking whether to discard unsaved changes.
    fn request_open(&mut self, path: PathBuf) {
        if self.document.has_unsaved_changes(&self.editor.map) {
            self.pending_open = Some(path);
        } else {
            self.open_path(&path);
        }
    }

    /// Saves to the open file, or asks where to save a Map that has never been saved.
    pub fn save(&mut self) -> bool {
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

        let Some(path) = self.pending_open.clone() else {
            return;
        };
        let has_file = self.document.path().is_some();
        let name = self.document.name();
        let mut choice = None;
        egui::Modal::new(egui::Id::new("unsaved_changes")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label(format!("Discard the unsaved changes to “{name}”?"));
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
        match choice {
            Some(UnsavedChoice::Save) => {
                self.pending_open = None;
                if self.save() {
                    self.open_path(&path);
                }
            }
            Some(UnsavedChoice::Discard) => {
                self.pending_open = None;
                self.open_path(&path);
            }
            Some(UnsavedChoice::Cancel) => self.pending_open = None,
            None => {}
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
        let dialog_open = self.pending_open.is_some()
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
    }
}
