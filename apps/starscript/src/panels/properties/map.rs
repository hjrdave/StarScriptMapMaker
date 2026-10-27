//! The Map section: Canvas, Scale, background and the Map-wide defaults.

use eframe::egui;
use ssmap_core::{Canvas, Color, Command, DistanceUnit, MapTemplate, ShoreKind};

use super::{Inherit, PropertiesPanel, caption, row};
use crate::editor::Editor;
use crate::prop_widgets::{self, Edit, Provenance, ScrubSpec, SectionHeader};

impl PropertiesPanel {
    /// Canvas width, height and the anchor they grow or shrink from. Each change is one
    /// `ResizeCanvas`; Objects are never touched.
    pub(super) fn canvas_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        canvas: &Canvas,
    ) {
        let spec = |label| ScrubSpec::new(label).range(1.0, 100_000.0).step(10.0);
        let mut width = f64::from(canvas.width);
        let mut height = f64::from(canvas.height);
        let width_edit = prop_widgets::number_field(ui, &mut width, &spec("Canvas width"));
        let height_edit = prop_widgets::number_field(ui, &mut height, &spec("Canvas height"));
        row(ui, "Anchor", |ui| {
            egui::Grid::new("canvas anchor")
                .spacing([2.0, 2.0])
                .show(ui, |ui| {
                    for y in 0..3u8 {
                        for x in 0..3u8 {
                            let name = anchor_name(x, y);
                            let selected = self.canvas_anchor == (x, y);
                            let button = ui
                                .add_sized(
                                    [18.0, 18.0],
                                    egui::Button::new(if selected { "●" } else { "" })
                                        .selected(selected),
                                )
                                .on_hover_text(&name);
                            button.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::RadioButton,
                                    true,
                                    &name,
                                )
                            });
                            if button.clicked() {
                                self.canvas_anchor = (x, y);
                            }
                        }
                        ui.end_row();
                    }
                });
        });
        if width_edit.changed || height_edit.changed {
            let (width, height) = (width.round() as u32, height.round() as u32);
            if width > 0 && height > 0 {
                let _ = editor.run(Command::ResizeCanvas {
                    origin: anchored_origin(canvas, width, height, self.canvas_anchor),
                    width,
                    height,
                });
            }
        }
    }

    /// The template the Map was made from, and its background: pick another template's look,
    /// or edit the water and texture fields. Either way the background changes on its own;
    /// the template shown stays the one the Map was made from.
    fn background_fields(&mut self, ui: &mut egui::Ui, editor: &mut Editor, canvas: &Canvas) {
        row(ui, "Template", |ui| {
            ui.label(canvas.template.as_deref().unwrap_or("None"));
        });
        caption(ui, "Background");
        let templates = self
            .templates
            .get_or_insert_with(|| MapTemplate::load_all().0);
        caption(ui, "Take the look of a template");
        ui.horizontal_wrapped(|ui| {
            for template in templates.iter() {
                let same = *template.background() == canvas.background;
                let name = format!("{} look", template.name());
                if ui.selectable_label(same, name).clicked() {
                    let _ = editor.run(Command::SetBackground {
                        background: template.background().clone(),
                    });
                }
            }
        });

        let mut next = canvas.background.clone();
        let mut edit = Edit::default();
        let mut merge = |next_edit: Edit| {
            edit.began |= next_edit.began;
            edit.changed |= next_edit.changed;
            edit.finished |= next_edit.finished;
        };
        let colour_field = |ui: &mut egui::Ui, label: &str, colour: &mut Color| {
            let mut value = Some(*colour);
            let edit = prop_widgets::swatch(ui, label, &mut value, false, Provenance::Plain).edit;
            *colour = value.unwrap_or(*colour);
            edit
        };
        merge(colour_field(ui, "Water colour", &mut next.water));
        let mut strength = f64::from(next.texture.strength);
        merge(prop_widgets::slider(
            ui,
            &mut strength,
            &ScrubSpec::new("Texture strength")
                .range(0.0, 1.0)
                .step(0.05)
                .decimals(2),
        ));
        next.texture.strength = strength as f32;
        let mut scale = f64::from(next.texture.scale);
        merge(prop_widgets::slider(
            ui,
            &mut scale,
            &ScrubSpec::new("Texture scale")
                .range(0.1, 20.0)
                .step(0.1)
                .decimals(1),
        ));
        next.texture.scale = scale as f32;
        merge(colour_field(ui, "Texture light", &mut next.texture.light));
        merge(colour_field(ui, "Texture dark", &mut next.texture.dark));
        self.live.apply(editor, edit, "Change Background", || {
            Command::SetBackground {
                background: next.clone(),
            }
        });
    }

    /// The Map: Canvas, Scale and the Map-wide defaults. Collapsed until opened.
    pub(super) fn map_properties(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let canvas = editor.map().canvas().clone();
        let header = SectionHeader {
            kind: "map",
            title: "Map",
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            self.canvas_fields(ui, editor, &canvas);
            let unit = match canvas.scale.unit {
                DistanceUnit::Miles => "mi",
                DistanceUnit::Kilometres => "km",
            };
            row(ui, "Scale", |ui| {
                ui.label(format!(
                    "{} {unit} per unit",
                    canvas.scale.distance_per_unit
                ));
            });

            self.background_fields(ui, editor, &canvas);

            caption(ui, "Default Roughness");
            let current = [editor.map().defaults().roughness.clone()];
            let set = |_, roughness| Command::SetDefaultRoughness { roughness };
            let group = Inherit {
                members: &current,
                provenance: Provenance::Plain,
                label: "Change Default Roughness",
                set: &set,
                reset: None,
            };
            ui.push_id("default roughness", |ui| {
                self.roughness_fields(ui, editor, &group);
            });

            for (kind, title, label) in [
                (
                    ShoreKind::Landmass,
                    "Default Landmass shore",
                    "Change Default Shore Style",
                ),
                (
                    ShoreKind::Lake,
                    "Default Lake shore",
                    "Change Default Lake Shore Style",
                ),
            ] {
                caption(ui, title);
                let current = [editor.map().defaults().shore_for(kind).clone()];
                let set = move |_, shore| Command::SetDefaultShore { kind, shore };
                let group = Inherit {
                    members: &current,
                    provenance: Provenance::Plain,
                    label,
                    set: &set,
                    reset: None,
                };
                ui.push_id(title, |ui| self.shore_fields(ui, editor, &group));
            }
        });
    }
}

/// Where the Canvas's top-left corner goes so that the `anchor` point of the grid stays fixed
/// in the workspace while the Canvas becomes `width` by `height`.
fn anchored_origin(canvas: &Canvas, width: u32, height: u32, anchor: (u8, u8)) -> [f64; 2] {
    let shift = |old: u32, new: u32, a: u8| (f64::from(old) - f64::from(new)) * f64::from(a) / 2.0;
    [
        canvas.origin[0] + shift(canvas.width, width, anchor.0),
        canvas.origin[1] + shift(canvas.height, height, anchor.1),
    ]
}

pub(super) fn anchor_name(x: u8, y: u8) -> String {
    let v = ["top", "middle", "bottom"][usize::from(y)];
    let h = ["left", "center", "right"][usize::from(x)];
    match (x, y) {
        (1, 1) => "Anchor center".to_owned(),
        (1, _) => format!("Anchor {v}"),
        (_, 1) => format!("Anchor {h}"),
        _ => format!("Anchor {v} {h}"),
    }
}
