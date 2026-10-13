//! StarScript Map Maker's editor. The binary in `main.rs` only opens a window around
//! [`MapMakerApp`], so UI tests can drive the same app headlessly.

mod canvas_view;
pub mod editor;
pub mod handles;
mod layer_tree;
mod properties;
pub mod tools;

use eframe::egui;
use ssmap_core::{Canvas, DistanceUnit, Map, Scale};

use crate::canvas_view::CanvasView;
use crate::editor::Editor;
use crate::layer_tree::LayerTree;
use crate::properties::PropertiesPanel;
use crate::tools::ToolId;

pub struct MapMakerApp {
    editor: Editor,
    layer_tree: LayerTree,
    properties: PropertiesPanel,
    canvas_view: CanvasView,
}

impl MapMakerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Until the New Map dialog exists, every session starts with a blank 4096x3072 Map.
        let map = Map::new(Canvas {
            width: 4096,
            height: 3072,
            scale: Scale {
                distance_per_unit: 0.25,
                unit: DistanceUnit::Miles,
            },
        });
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
        Self {
            editor: Editor::new(map),
            layer_tree: LayerTree::default(),
            properties: PropertiesPanel::default(),
            canvas_view: CanvasView::new(gpu),
        }
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // Text fields keep their own undo and keys.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::{Key, KeyboardShortcut, Modifiers};
        let shortcut = |modifiers, key| KeyboardShortcut::new(modifiers, key);
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
        let editor = &mut self.editor;
        ui.horizontal(|ui| {
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

impl eframe::App for MapMakerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.shortcuts(ui.ctx());
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
