//! The properties panel: shows and edits what is selected, through Commands.

use eframe::egui;
use ssmap_core::{Command, DistanceUnit, Id, Node};

use crate::editor::Editor;
use crate::layer_tree::node_name;

#[derive(Default)]
pub struct PropertiesPanel {
    /// The name being typed for a Layer, committed when the field loses focus.
    name_edit: Option<(Id, String)>,
}

impl PropertiesPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        ui.heading("Properties");
        ui.separator();
        match editor.selection() {
            [] => self.map_properties(ui, editor),
            [id] => {
                let id = *id;
                match editor.map.node(id).cloned() {
                    Some(Node::Layer(layer)) => self.layer_properties(ui, editor, id, &layer),
                    Some(Node::Object(object)) => {
                        egui::Grid::new("object").num_columns(2).show(ui, |ui| {
                            ui.label("Kind");
                            ui.label(node_name(&editor.map, id));
                            ui.end_row();
                            ui.label("Locked");
                            let mut locked = object.locked;
                            if ui.checkbox(&mut locked, "").changed() {
                                editor.run(Command::SetLocked { id, locked });
                            }
                            ui.end_row();
                        });
                    }
                    None => {}
                }
            }
            many => {
                ui.label(format!("{} items selected", many.len()));
            }
        }
    }

    fn map_properties(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        let canvas = &editor.map.canvas;
        egui::Grid::new("map").num_columns(2).show(ui, |ui| {
            ui.label("Canvas");
            ui.label(format!("{} × {}", canvas.width, canvas.height));
            ui.end_row();
            ui.label("Scale");
            let unit = match canvas.scale.unit {
                DistanceUnit::Miles => "mi",
                DistanceUnit::Kilometres => "km",
            };
            ui.label(format!(
                "{} {unit} per unit",
                canvas.scale.distance_per_unit
            ));
            ui.end_row();
        });
    }

    fn layer_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        layer: &ssmap_core::Layer,
    ) {
        egui::Grid::new("layer").num_columns(2).show(ui, |ui| {
            ui.label("Name");
            let text = match &mut self.name_edit {
                Some((editing, text)) if *editing == id => text,
                _ => {
                    self.name_edit = Some((id, layer.name.clone()));
                    &mut self.name_edit.as_mut().expect("just set").1
                }
            };
            let field = ui.text_edit_singleline(text);
            if field.lost_focus() && *text != layer.name {
                let name = text.clone();
                if !editor.run(Command::RenameLayer { id, name }) {
                    self.name_edit = None;
                }
            } else if !field.has_focus() && *text != layer.name {
                // Changed elsewhere (undo, the tree): show the Map's value.
                *text = layer.name.clone();
            }
            ui.end_row();

            ui.label("Visible");
            let mut visible = layer.visible;
            if ui.checkbox(&mut visible, "").changed() {
                editor.run(Command::SetVisible { id, visible });
            }
            ui.end_row();

            ui.label("Locked");
            let mut locked = layer.locked;
            if ui.checkbox(&mut locked, "").changed() {
                editor.run(Command::SetLocked { id, locked });
            }
            ui.end_row();

            ui.label("Opacity");
            let mut percent = layer.opacity * 100.0;
            let slider = ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%"));
            // Dragging the slider is one gesture: live updates, one Undo step.
            if slider.drag_started() {
                editor.begin_gesture("Change Opacity");
            }
            if slider.changed() {
                editor.gesture(Command::SetOpacity {
                    id,
                    opacity: (percent / 100.0).clamp(0.0, 1.0),
                });
            }
            if slider.drag_stopped() {
                editor.end_gesture();
            }
            ui.end_row();

            if let Some(target) = layer.clip {
                ui.label("Clipped to");
                ui.label(node_name(&editor.map, target));
                ui.end_row();
            }
        });
    }
}
