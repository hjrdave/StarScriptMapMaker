//! The Map section: the Canvas, its background and the Map-wide defaults.

use eframe::egui;
use ssmap_core::{Canvas, Color, Command, DistanceUnit, Id, MapTemplate, ShoreKind};

use crate::icons::Icon;

use super::{Inherit, PropertiesPanel, caption, row};
use crate::editor::Editor;
use crate::prop_widgets::{self, Edit, Handle, Provenance, ScrubSpec};

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
        scale: Option<&str>,
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
        let chain_id = egui::Id::new("canvas aspect chain");
        let mut linked = ui.ctx().data(|d| d.get_temp::<bool>(chain_id)) == Some(true);
        let field_height = crate::theme::metrics::FIELD[0];
        let (width_edit, height_edit) = ui
            .horizontal(|ui| {
                const CHAIN: f32 = 24.0;
                ui.spacing_mut().item_spacing.x = 2.0;
                let each = ((ui.available_width() - CHAIN - 4.0) / 2.0).max(40.0);
                let size = egui::vec2(each, field_height);
                let w = ui
                    .allocate_ui(size, |ui| {
                        prop_widgets::scrub_field(ui, &mut width, &spec(width_label, "W"))
                    })
                    .inner;
                if prop_widgets::chain_button(ui, &mut linked) {
                    ui.ctx().data_mut(|d| d.insert_temp(chain_id, linked));
                    // The ratio is the one at the moment the chain is turned on.
                    self.chain_ratio = linked.then(|| {
                        (
                            id,
                            f64::from(canvas.width) / f64::from(canvas.height.max(1)),
                        )
                    });
                }
                let h = ui
                    .allocate_ui(size, |ui| {
                        prop_widgets::scrub_field(ui, &mut height, &spec(height_label, "H"))
                    })
                    .inner;
                (w, h)
            })
            .inner;
        if linked && self.chain_ratio.is_none_or(|(board, _)| board != id) {
            self.chain_ratio = Some((
                id,
                f64::from(canvas.width) / f64::from(canvas.height.max(1)),
            ));
        }
        if let (true, Some((_, ratio))) = (linked, self.chain_ratio) {
            let (w, h) = (width.round(), height.round());
            let (changed_w, changed_h) =
                (w != f64::from(canvas.width), h != f64::from(canvas.height));
            let limit = |v: f64| v.round().clamp(1.0, 100_000.0);
            if changed_w && !changed_h {
                height = limit(w / ratio);
            } else if changed_h && !changed_w {
                width = limit(h * ratio);
            }
        }
        ui.columns(2, |columns| {
            if let Some(scale) = scale {
                row(&mut columns[0], "Scale", |ui| {
                    ui.label(scale);
                });
            }
            self.anchor_grid(&mut columns[1]);
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

    /// The 3×3 grid that says which point of the board stays put when it is resized.
    fn anchor_grid(&mut self, ui: &mut egui::Ui) {
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
                if look_card(ui, template.name(), template.background(), same).clicked() {
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

    /// Three tabs: the Canvas (size, scale and anchor), the Style (background and Roughness),
    /// and the Shores that Landmasses and Lakes inherit. The tab is remembered for the session.
    fn map_fields(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let canvas = editor.map().canvas().clone();
        let primary = editor.map().primary_board();
        let tab_id = egui::Id::new("map panel tab");
        let mut tab = ui.ctx().data(|d| d.get_temp::<usize>(tab_id)).unwrap_or(0);
        if prop_widgets::tab_bar(ui, &["Canvas", "Style", "Shores"], &mut tab) {
            ui.ctx().data_mut(|d| d.insert_temp(tab_id, tab));
        }
        match tab {
            0 => prop_widgets::divider_section(
                ui,
                "Canvas",
                |_| {},
                |ui| {
                    let unit = match canvas.scale.unit {
                        DistanceUnit::Miles => "mi",
                        DistanceUnit::Kilometres => "km",
                    };
                    let scale = format!("{} {unit} per unit", canvas.scale.distance_per_unit);
                    self.canvas_fields(ui, editor, primary, &canvas, Some(&scale));
                },
            ),
            1 => {
                prop_widgets::divider_section(
                    ui,
                    "Background",
                    |_| {},
                    |ui| {
                        self.background_fields(ui, editor, primary, &canvas);
                    },
                );
                prop_widgets::divider_section(
                    ui,
                    "Roughness",
                    |_| {},
                    |ui| {
                        self.default_roughness(ui, editor);
                    },
                );
            }
            _ => self.shores_tab(ui, editor),
        }
    }

    /// The Roughness that Landmasses and Lakes take unless they set their own.
    fn default_roughness(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
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
    }

    /// One set of shore fields, for the Landmass shore or the Lake shore: an icon switch at the
    /// right of the header picks which. The two keep their own values.
    fn shores_tab(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let switch_id = egui::Id::new("map panel shore");
        let mut which = ui
            .ctx()
            .data(|d| d.get_temp::<usize>(switch_id))
            .unwrap_or(0);
        let (kind, title, label) = if which == 0 {
            (
                ShoreKind::Landmass,
                "Landmass shore",
                "Change Default Shore Style",
            )
        } else {
            (
                ShoreKind::Lake,
                "Lake shore",
                "Change Default Lake Shore Style",
            )
        };
        prop_widgets::divider_section(
            ui,
            title,
            |ui| {
                let options = [(Icon::LandBrush, "Landmass"), (Icon::LakeBrush, "Lake")];
                if prop_widgets::icon_switch(ui, &options, &mut which) {
                    ui.ctx().data_mut(|d| d.insert_temp(switch_id, which));
                }
            },
            |ui| {
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
            },
        );
    }
}

/// A swatch card for a template's look: its water colour with its name on it. Named
/// "<name> look" for assistive tools.
fn look_card(
    ui: &mut egui::Ui,
    name: &str,
    look: &ssmap_core::Background,
    same: bool,
) -> egui::Response {
    let palette = crate::theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(84.0, 48.0), egui::Sense::click());
    let label = format!("{name} look");
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), same, &label)
    });
    let water = look.water;
    let fill = egui::Color32::from_rgb(water.r, water.g, water.b);
    ui.painter()
        .rect_filled(rect, crate::theme::RADIUS_BUTTON, fill);
    let stroke = if same {
        egui::Stroke::new(2.0, palette.gold)
    } else if response.hovered() {
        egui::Stroke::new(1.0, palette.border_strong)
    } else {
        egui::Stroke::new(1.0, palette.border_subtle)
    };
    ui.painter().rect_stroke(
        rect,
        crate::theme::RADIUS_BUTTON,
        stroke,
        egui::StrokeKind::Inside,
    );
    let luma = 0.299 * f32::from(water.r) + 0.587 * f32::from(water.g) + 0.114 * f32::from(water.b);
    let text = if luma > 140.0 {
        egui::Color32::from_rgb(20, 22, 26)
    } else {
        egui::Color32::WHITE
    };
    ui.painter().text(
        rect.left_bottom() + egui::vec2(6.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        name,
        egui::FontId::proportional(crate::theme::CAPTION),
        text,
    );
    response.on_hover_text(label)
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
