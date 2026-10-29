//! The Map section: the Canvas, its background and the Map-wide defaults.

use eframe::egui;
use ssmap_core::{Canvas, Color, Command, DistanceUnit, Id, MapTemplate, ShoreKind};

use super::{Inherit, PropertiesPanel, caption, row};
use crate::editor::Editor;
use crate::prop_widgets::{self, Edit, Handle, Provenance, ScrubSpec, SectionHeader};

impl PropertiesPanel {
    /// Width, height and the anchor they grow or shrink from, of the board `id` whose box is
    /// `canvas`: the Canvas, or an Art board. Each change is one Command; Objects are never
    /// touched.
    pub(super) fn canvas_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        canvas: &Canvas,
    ) {
        let primary = id == editor.map().primary_board();
        let (width_label, height_label) = if primary {
            ("Canvas width", "Canvas height")
        } else {
            ("Board width", "Board height")
        };
        // A drag is one gesture. Each frame's size is worked out from the Canvas as it was
        // when the drag began, so the anchor holds still while the gesture is replaced.
        let base = match &self.resize_base {
            Some((board, base)) if *board == id => base.clone(),
            _ => canvas.clone(),
        };
        let spec = |label, handle| {
            ScrubSpec::new(label)
                .handle(Handle::Text(handle))
                .range(1.0, 100_000.0)
                .step(10.0)
        };
        let mut width = f64::from(canvas.width);
        let mut height = f64::from(canvas.height);
        let (width_edit, height_edit) = prop_widgets::pair(
            ui,
            |ui| prop_widgets::scrub_field(ui, &mut width, &spec(width_label, "W")),
            |ui| prop_widgets::scrub_field(ui, &mut height, &spec(height_label, "H")),
        );
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
        let edit = merge_edits(width_edit, height_edit);
        if edit.began {
            self.resize_base = Some((id, canvas.clone()));
        }
        let (width, height) = (width.round() as u32, height.round() as u32);
        if width > 0 && height > 0 {
            let origin = anchored_origin(&base, width, height, self.canvas_anchor);
            let command = resize(editor, id, origin, width, height);
            let name = if primary {
                "Resize Canvas"
            } else {
                "Resize Board"
            };
            self.live.apply(editor, edit, name, || command);
        }
        if edit.finished {
            self.resize_base = None;
        }
    }

    /// The template the Map was made from, and its background: pick another template's look,
    /// or edit the water and texture fields. Either way the background changes on its own;
    /// the template shown stays the one the Map was made from.
    pub(super) fn background_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        board: Id,
        canvas: &Canvas,
    ) {
        let primary = board == editor.map().primary_board();
        let set = move |background: ssmap_core::Background| {
            if primary {
                Command::SetBackground { background }
            } else {
                Command::SetBoardBackground {
                    id: board,
                    background,
                }
            }
        };
        if primary {
            row(ui, "Template", |ui| {
                ui.label(canvas.template.as_deref().unwrap_or("None"));
            });
        }
        let templates = self
            .templates
            .get_or_insert_with(|| MapTemplate::load_all().0);
        caption(ui, "Take the look of a template");
        ui.horizontal_wrapped(|ui| {
            for template in templates.iter() {
                let same = *template.background() == canvas.background;
                let name = format!("{} look", template.name());
                if ui.selectable_label(same, name).clicked() {
                    let _ = editor.run(set(template.background().clone()));
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
        merge(prop_widgets::scrub_field(
            ui,
            &mut strength,
            &ScrubSpec::new("Texture strength")
                .range(0.0, 1.0)
                .step(0.05)
                .decimals(2),
        ));
        next.texture.strength = strength as f32;
        let mut scale = f64::from(next.texture.scale);
        merge(prop_widgets::scrub_field(
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
        self.live
            .apply(editor, edit, "Change Background", || set(next.clone()));
    }

    /// The Map section of the sidebar: Canvas, Scale, background and the Map-wide defaults.
    pub(crate) fn map_panel(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        // A change ends when the pointer is released, or when Escape cancelled it.
        self.live.apply(editor, Edit::default(), "", || {
            unreachable!("nothing changed")
        });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| self.map_fields(ui, editor));
    }

    /// Three sections: the Canvas (size, anchor and scale), its Background, and the Map-wide
    /// defaults that Landmasses and Lakes inherit.
    fn map_fields(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let canvas = editor.map().canvas().clone();
        let primary = editor.map().primary_board();
        let header = |kind, title| SectionHeader {
            kind,
            title,
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header("map-canvas", "Canvas"), |ui| {
            self.canvas_fields(ui, editor, primary, &canvas);
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
        });
        prop_widgets::section(ui, header("map-background", "Background"), |ui| {
            self.background_fields(ui, editor, primary, &canvas);
        });
        prop_widgets::section(ui, header("map-defaults", "Defaults"), |ui| {
            self.map_defaults(ui, editor);
        });
    }

    /// The Roughness and shore styles that Landmasses and Lakes take unless they set their own.
    fn map_defaults(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        caption(ui, "Roughness");
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
                "Landmass shore",
                "Change Default Shore Style",
            ),
            (
                ShoreKind::Lake,
                "Lake shore",
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
    }
}

/// Two edits made in one frame as one: began or changed or finished if either did.
pub(super) fn merge_edits(a: Edit, b: Edit) -> Edit {
    Edit {
        began: a.began || b.began,
        changed: a.changed || b.changed,
        finished: a.finished || b.finished,
        reset: false,
    }
}

/// The Command that sets a board's origin and size: for the Canvas the one older Maps and
/// journals already use, for an Art board the board's own.
pub(super) fn resize(
    editor: &Editor,
    id: Id,
    origin: [f64; 2],
    width: u32,
    height: u32,
) -> Command {
    if id == editor.map().primary_board() {
        Command::ResizeCanvas {
            origin,
            width,
            height,
        }
    } else {
        Command::ResizeBoard {
            id,
            origin,
            width,
            height,
        }
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
