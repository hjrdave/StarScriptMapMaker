//! The properties panel: shows and edits what is selected, through Commands.

use eframe::egui;
use ssmap_core::{Color, Command, DistanceUnit, Id, Node, Roughness};

use crate::editor::Editor;
use crate::layer_tree::node_name;

#[derive(Default)]
pub struct PropertiesPanel {
    /// The name being typed for a Layer, committed when the field loses focus.
    name_edit: Option<(Id, String)>,
    /// Set while the fill picker is being changed, which makes one Undo step.
    fill_gesture: bool,
}

impl PropertiesPanel {
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        // A fill change ends when the pointer is released, or when Escape cancelled it.
        if self.fill_gesture && (!editor.gesture_open() || !ui.input(|i| i.pointer.any_down())) {
            self.fill_gesture = false;
            editor.end_gesture();
        }
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
                            if let Some(fill) = object.kind.fill() {
                                ui.label("Fill");
                                self.fill(ui, editor, id, fill);
                                ui.end_row();
                            }
                            if object.kind.geometry().is_some() {
                                object_roughness(ui, editor, id, object.kind.roughness());
                            }
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

    /// A colour button whose picker changes the fill live. Everything picked until the
    /// pointer is released is one Undo step.
    fn fill(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, fill: Color) {
        let mut rgba = egui::Color32::from_rgba_unmultiplied(fill.r, fill.g, fill.b, fill.a);
        let changed = egui::color_picker::color_edit_button_srgba(
            ui,
            &mut rgba,
            egui::color_picker::Alpha::OnlyBlend,
        )
        .changed();
        if !changed {
            return;
        }
        let [r, g, b, a] = rgba.to_srgba_unmultiplied();
        let command = Command::SetFill {
            ids: vec![id],
            fill: Color::rgba(r, g, b, a),
        };
        if !self.fill_gesture && !editor.gesture_open() {
            editor.begin_gesture("Change Fill");
            self.fill_gesture = true;
        }
        if self.fill_gesture {
            editor.gesture_replace(command);
        } else {
            // Another gesture is open; join it rather than rewinding its work.
            editor.gesture(command);
        }
    }

    fn map_properties(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let canvas = editor.map.canvas.clone();
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
            let current = editor.map.defaults().roughness.clone();
            roughness_rows(
                ui,
                editor,
                &current,
                "Change Default Roughness",
                |roughness| Command::SetDefaultRoughness { roughness },
            );
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

/// The Roughness rows of a Landmass or Lake: its own settings, or the Map-wide default.
fn object_roughness(ui: &mut egui::Ui, editor: &mut Editor, id: Id, own: Option<&Roughness>) {
    ui.label("Roughness");
    let mut use_default = own.is_none();
    if ui
        .checkbox(&mut use_default, "Use the Map default")
        .changed()
    {
        let roughness = (!use_default).then(|| editor.map.defaults().roughness.clone());
        editor.run(Command::SetRoughness {
            ids: vec![id],
            roughness,
        });
    }
    ui.end_row();
    match own {
        Some(own) => {
            let own = own.clone();
            roughness_rows(ui, editor, &own, "Change Roughness", |roughness| {
                Command::SetRoughness {
                    ids: vec![id],
                    roughness: Some(roughness),
                }
            });
        }
        None => {
            let Roughness {
                amount,
                scale,
                seed,
                ..
            } = editor.map.defaults().roughness;
            ui.label("");
            ui.weak(format!("amount {amount} u, scale {scale} u, seed {seed}"));
            ui.end_row();
        }
    }
}

/// Amount, scale and seed. Dragging a value is one gesture: live updates, one Undo step.
fn roughness_rows(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    current: &Roughness,
    label: &str,
    command: impl Fn(Roughness) -> Command,
) {
    let mut amount = current.amount;
    let mut scale = current.scale;
    let mut seed = current.seed;
    let row = |ui: &mut egui::Ui, name: &str, value: egui::DragValue<'_>| {
        ui.label(name);
        let response = ui.add(value);
        ui.end_row();
        response
    };
    let amount_value = egui::DragValue::new(&mut amount)
        .range(0.0..=1000.0)
        .speed(0.1)
        .suffix(" u");
    let scale_value = egui::DragValue::new(&mut scale)
        .range(0.5..=10_000.0)
        .speed(0.5)
        .suffix(" u");
    let responses = [
        row(ui, "Amount", amount_value),
        row(ui, "Scale", scale_value),
        row(ui, "Seed", egui::DragValue::new(&mut seed)),
    ];
    if responses.iter().any(egui::Response::drag_started) {
        editor.begin_gesture(label);
    }
    if responses.iter().any(egui::Response::changed) {
        editor.gesture(command(Roughness {
            amount,
            scale,
            seed,
            ..current.clone()
        }));
    }
    if responses.iter().any(egui::Response::drag_stopped) {
        editor.end_gesture();
    }
}
