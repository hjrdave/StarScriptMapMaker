//! The properties panel: shows and edits what is selected, through Commands.

mod label_stamp;

use eframe::egui;
use ssmap_core::{
    Color, Command, DistanceUnit, Id, MAX_RIPPLES, Map, Node, ObjectKind, Roughness, ShoreKind,
    ShoreStyle,
};

use crate::editor::Editor;
use crate::layer_tree::{landmass_name, landmasses, node_name, selected_landmass};
use crate::prop_widgets::{self, Edit, LiveEdit, Provenance, ScrubSpec, SectionHeader};
use crate::theme;
use crate::tool_section::ToolSection;

#[derive(Default)]
pub struct PropertiesPanel {
    /// The name being typed for a Layer, committed when the field loses focus.
    name_edit: Option<(Id, String)>,
    /// The text being typed for a Label, committed when the field loses focus.
    text_edit: Option<(Id, String)>,
    /// Turns the scrubs, sliders and swatches into one Undo step each.
    live: LiveEdit,
    /// "Manage Label styles…" was clicked; the window is opened by the owner of the tab.
    manage_label_styles: bool,
    /// The active tool's settings, above the selection.
    tool_section: ToolSection,
}

/// A group of settings that may be inherited from a default: the value shown, where it
/// comes from, how an edit is written and how it is reset.
struct Inherit<'a, T> {
    current: &'a T,
    provenance: Provenance,
    label: &'a str,
    set: &'a dyn Fn(T) -> Command,
    /// Goes back to the default. `None` where there is nothing to go back to.
    reset: Option<Command>,
}

impl PropertiesPanel {
    /// Whether "Manage Label styles…" was clicked since the last call.
    pub fn take_manage_label_styles(&mut self) -> bool {
        std::mem::take(&mut self.manage_label_styles)
    }

    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        // A change ends when the pointer is released, or when Escape cancelled it.
        self.live.apply(editor, Edit::default(), "", || {
            unreachable!("nothing changed")
        });
        // The Map section starts collapsed; the others start open.
        prop_widgets::default_closed(ui.ctx(), "map");
        crate::shell::panel_header(ui, "Properties", |_| {});
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.tool_section.show(ui, editor);
                ui.add_space(theme::SPACING[1]);
                self.selection(ui, editor);
            });
    }

    fn selection(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        match editor.selection() {
            [] => {
                if let Some(layer) = editor.active_layer().filter(|&id| editor.map.contains(id))
                    && let Some(Node::Layer(data)) = editor.map.node(layer).cloned()
                {
                    breadcrumb(ui, &editor.map, layer);
                    self.layer_properties(ui, editor, layer, &data);
                }
                self.map_properties(ui, editor);
            }
            [id] => {
                let id = *id;
                breadcrumb(ui, &editor.map, id);
                match editor.map.node(id).cloned() {
                    Some(Node::Layer(layer)) => self.layer_properties(ui, editor, id, &layer),
                    Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Label(_)) => {
                        let ObjectKind::Label(label) = object.kind else {
                            unreachable!("matched above")
                        };
                        self.label_properties(ui, editor, id, object.locked, &label);
                    }
                    Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Stamp(_)) => {
                        let ObjectKind::Stamp(stamp) = object.kind else {
                            unreachable!("matched above")
                        };
                        self.stamp_properties(ui, editor, id, object.locked, &stamp);
                    }
                    Some(Node::Object(object)) => {
                        self.object_properties(ui, editor, id, &object);
                    }
                    None => {}
                }
            }
            many => {
                heading(ui, &selection_summary(&editor.map, many));
                self.combine(ui, editor);
            }
        }
    }

    /// A Landmass or Lake: Locked and Fill, then Roughness and Shore style, and Combine for a
    /// Landmass.
    fn object_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        object: &ssmap_core::Object,
    ) {
        let title = node_name(&editor.map, id);
        let header = SectionHeader {
            kind: "object",
            title: &title,
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            let mut locked = object.locked;
            if prop_widgets::checkbox_row(ui, "Locked", &mut locked) {
                editor.run(Command::SetLocked { id, locked });
            }
            if let Some(fill) = object.kind.fill() {
                self.fill(ui, editor, id, fill);
            }
        });
        if object.kind.geometry().is_some() {
            self.object_roughness(ui, editor, id, object.kind.roughness());
        }
        if let Some(kind) = object.kind.shore_kind() {
            self.object_shore(ui, editor, id, kind);
        }
        if matches!(object.kind, ObjectKind::Landmass(_)) {
            self.combine(ui, editor);
        }
    }

    /// The fill swatch, whose popover changes the fill live. Everything picked until the
    /// pointer is released is one Undo step.
    fn fill(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, fill: Color) {
        let mut colour = Some(fill);
        let response = prop_widgets::swatch(ui, "Fill", &mut colour, false, Provenance::Plain);
        let fill = colour.unwrap_or(fill);
        self.live
            .apply(editor, response.edit, "Change Fill", || Command::SetFill {
                ids: vec![id],
                fill,
            });
    }

    /// Union, Subtract and Split Apart, with the same rules as the Edit actions.
    fn combine(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let header = SectionHeader {
            kind: "combine",
            title: "Combine",
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(editor.can_combine(), egui::Button::new("Union"))
                    .on_hover_text("Merge the selected Landmasses into the topmost one")
                    .clicked()
                {
                    editor.union_selection();
                }
                if ui
                    .add_enabled(editor.can_combine(), egui::Button::new("Subtract"))
                    .on_hover_text("Cut the topmost selected Landmass out of the others")
                    .clicked()
                {
                    editor.subtract_selection();
                }
                if ui
                    .add_enabled(editor.can_split(), egui::Button::new("Split Apart"))
                    .on_hover_text(
                        "Make each part of the selected Landmasses a Landmass of its own",
                    )
                    .clicked()
                {
                    editor.split_selection();
                }
            });
            caption(
                ui,
                "Subtract removes the top Landmass from the one below it.",
            );
        });
    }

    /// Applies what a field did. A reset goes back to the default; anything else is an edit of
    /// `value`, one Undo step per scrub.
    fn edit<T: Clone>(
        &mut self,
        editor: &mut Editor,
        edit: Edit,
        value: &T,
        group: &Inherit<'_, T>,
    ) {
        if edit.reset {
            if let Some(reset) = group.reset.clone() {
                editor.run(reset);
            }
            return;
        }
        self.live
            .apply(editor, edit, group.label, || (group.set)(value.clone()));
    }

    /// The Roughness section of a Landmass or Lake: its own settings, or the Map default.
    fn object_roughness(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        own: Option<&Roughness>,
    ) {
        let default = editor.map.defaults().roughness.clone();
        let current = own.unwrap_or(&default).clone();
        let provenance = if own.is_some() {
            Provenance::Overridden
        } else {
            Provenance::Inherited
        };
        let set = |roughness| Command::SetRoughness {
            ids: vec![id],
            roughness: Some(roughness),
        };
        let group = Inherit {
            current: &current,
            provenance,
            label: "Change Roughness",
            set: &set,
            reset: own.is_some().then(|| Command::SetRoughness {
                ids: vec![id],
                roughness: None,
            }),
        };
        let header = SectionHeader {
            kind: "roughness",
            title: "Roughness",
            badge: own.is_some().then_some("overridden"),
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            ui.push_id(("roughness", id), |ui| {
                self.roughness_fields(ui, editor, &group);
            });
        });
    }

    /// Amount, scale and seed, and a button that picks a new seed.
    fn roughness_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        group: &Inherit<'_, Roughness>,
    ) {
        let provenance = group.provenance;
        let mut next = group.current.clone();
        let spec = |label| ScrubSpec::new(label).provenance(provenance);
        let edit = prop_widgets::scrub_field(
            ui,
            &mut next.amount,
            &spec("Amount")
                .unit(" u")
                .range(0.0, 1000.0)
                .step(0.5)
                .decimals(1),
        );
        self.edit(editor, edit, &next, group);
        let edit = prop_widgets::scrub_field(
            ui,
            &mut next.scale,
            &spec("Scale")
                .unit(" u")
                .range(0.5, 10_000.0)
                .step(1.0)
                .decimals(1),
        );
        self.edit(editor, edit, &next, group);
        let mut seed = f64::from(next.seed);
        let edit =
            prop_widgets::scrub_field(ui, &mut seed, &spec("Seed").range(0.0, f64::from(u32::MAX)));
        next.seed = seed as u32;
        self.edit(editor, edit, &next, group);
        let reroll = ui
            .button("Re-roll")
            .on_hover_text("Pick another coastline with the same Amount and Scale");
        if reroll.clicked() {
            next.seed = Id::new().as_uuid().as_u128() as u32;
            self.edit(editor, Edit::instant(), &next, group);
        }
    }

    /// The Shore style section of a Landmass or Lake: its own settings, or the Map default.
    fn object_shore(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, kind: ShoreKind) {
        let own = match editor.map.node(id) {
            Some(Node::Object(object)) => object.kind.shore().cloned(),
            _ => None,
        };
        let default = editor.map.defaults().shore_for(kind).clone();
        let provenance = if own.is_some() {
            Provenance::Overridden
        } else {
            Provenance::Inherited
        };
        let current = own.clone().unwrap_or(default);
        let set = |shore| Command::SetShore {
            ids: vec![id],
            shore: Some(shore),
        };
        let group = Inherit {
            current: &current,
            provenance,
            label: "Change Shore Style",
            set: &set,
            reset: own.is_some().then(|| Command::SetShore {
                ids: vec![id],
                shore: None,
            }),
        };
        let header = SectionHeader {
            kind: "shore",
            title: "Shore style",
            badge: own.is_some().then_some("overridden"),
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            ui.push_id(("shore", id), |ui| {
                self.shore_fields(ui, editor, &group);
            });
        });
    }

    /// Outline, ripples and inner shade.
    fn shore_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        group: &Inherit<'_, ShoreStyle>,
    ) {
        let provenance = group.provenance;
        let mut style = group.current.clone();
        let spec = |label| ScrubSpec::new(label).provenance(provenance);
        let length = |label, max| spec(label).unit(" u").range(0.0, max).step(0.1).decimals(1);

        caption(ui, "Outline");
        let edit = prop_widgets::scrub_field(ui, &mut style.outline.width, &length("Width", 100.0));
        self.edit(editor, edit, &style, group);
        let mut colour = Some(style.outline.color);
        let edit = prop_widgets::swatch(ui, "Colour", &mut colour, false, provenance).edit;
        style.outline.color = colour.unwrap_or(style.outline.color);
        self.edit(editor, edit, &style, group);

        caption(ui, "Ripples");
        let mut count = f64::from(style.ripples.count);
        let edit = prop_widgets::scrub_field(
            ui,
            &mut count,
            &spec("Count").range(0.0, f64::from(MAX_RIPPLES)).step(0.1),
        );
        style.ripples.count = count as u32;
        self.edit(editor, edit, &style, group);
        let edit =
            prop_widgets::scrub_field(ui, &mut style.ripples.spacing, &length("Spacing", 500.0));
        self.edit(editor, edit, &style, group);
        let edit =
            prop_widgets::scrub_field(ui, &mut style.ripples.width, &length("Line width", 50.0));
        self.edit(editor, edit, &style, group);
        let mut colour = Some(style.ripples.color);
        let edit = prop_widgets::swatch(ui, "Line colour", &mut colour, false, provenance).edit;
        style.ripples.color = colour.unwrap_or(style.ripples.color);
        self.edit(editor, edit, &style, group);
        let edit = prop_widgets::scrub_field(
            ui,
            &mut style.ripples.fade,
            &spec("Fade").range(0.0, 1.0).step(0.01).decimals(2),
        );
        self.edit(editor, edit, &style, group);

        caption(ui, "Inner shade");
        let edit =
            prop_widgets::scrub_field(ui, &mut style.shading.width, &length("Shade width", 500.0));
        self.edit(editor, edit, &style, group);
        let mut colour = Some(style.shading.color);
        let edit = prop_widgets::swatch(ui, "Shade colour", &mut colour, false, provenance).edit;
        style.shading.color = colour.unwrap_or(style.shading.color);
        self.edit(editor, edit, &style, group);
    }

    /// The Map: Canvas, Scale and the Map-wide defaults. Collapsed until opened.
    fn map_properties(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let canvas = editor.map.canvas.clone();
        let header = SectionHeader {
            kind: "map",
            title: "Map",
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            row(ui, "Canvas", |ui| {
                ui.label(format!("{} × {}", canvas.width, canvas.height));
            });
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

            caption(ui, "Default Roughness");
            let current = editor.map.defaults().roughness.clone();
            let set = |roughness| Command::SetDefaultRoughness { roughness };
            let group = Inherit {
                current: &current,
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
                let current = editor.map.defaults().shore_for(kind).clone();
                let set = move |shore| Command::SetDefaultShore { kind, shore };
                let group = Inherit {
                    current: &current,
                    provenance: Provenance::Plain,
                    label,
                    set: &set,
                    reset: None,
                };
                ui.push_id(title, |ui| self.shore_fields(ui, editor, &group));
            }
        });
    }

    fn layer_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        layer: &ssmap_core::Layer,
    ) {
        let header = SectionHeader {
            kind: "layer",
            title: "Layer",
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            row(ui, "Name", |ui| {
                let text = match &mut self.name_edit {
                    Some((editing, text)) if *editing == id => text,
                    _ => {
                        self.name_edit = Some((id, layer.name.clone()));
                        &mut self.name_edit.as_mut().expect("just set").1
                    }
                };
                let field = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                if field.lost_focus() && *text != layer.name {
                    let name = text.clone();
                    if !editor.run(Command::RenameLayer { id, name }) {
                        self.name_edit = None;
                    }
                } else if !field.has_focus() && *text != layer.name {
                    // Changed elsewhere (undo, the tree): show the Map's value.
                    *text = layer.name.clone();
                }
            });

            let mut visible = layer.visible;
            if prop_widgets::checkbox_row(ui, "Visible", &mut visible) {
                editor.run(Command::SetVisible { id, visible });
            }
            let mut locked = layer.locked;
            if prop_widgets::checkbox_row(ui, "Locked", &mut locked) {
                editor.run(Command::SetLocked { id, locked });
            }

            // Dragging the slider is one gesture: live updates, one Undo step.
            let mut percent = f64::from(layer.opacity) * 100.0;
            let edit = prop_widgets::slider(
                ui,
                &mut percent,
                &ScrubSpec::new("Opacity").unit("%").range(0.0, 100.0),
            );
            self.live
                .apply(editor, edit, "Change Opacity", || Command::SetOpacity {
                    id,
                    opacity: (percent / 100.0).clamp(0.0, 1.0) as f32,
                });

            let mut depth_sort = layer.depth_sort;
            if prop_widgets::checkbox_row(ui, "Depth sort", &mut depth_sort) {
                editor.run(Command::SetDepthSort { id, depth_sort });
            }
            caption(ui, crate::layer_tree::DEPTH_SORT_HINT);

            row(ui, "Clip to", |ui| {
                let current = layer
                    .clip
                    .map_or("None".to_owned(), |t| landmass_name(&editor.map, t));
                egui::ComboBox::from_id_salt("clip")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(layer.clip.is_none(), "None").clicked() {
                            editor.run(Command::SetClip { id, target: None });
                        }
                        for target in landmasses(&editor.map) {
                            let name = landmass_name(&editor.map, target);
                            if ui
                                .selectable_label(layer.clip == Some(target), name)
                                .clicked()
                            {
                                editor.run(Command::SetClip {
                                    id,
                                    target: Some(target),
                                });
                            }
                        }
                    });
            });
            let selected = selected_landmass(&editor.map, editor.selection());
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Use selection"))
                .clicked()
            {
                editor.run(Command::SetClip {
                    id,
                    target: selected,
                });
            }
        });
    }
}

/// "Layer › Object" for a selected Object, or the Layer's own name.
fn breadcrumb(ui: &mut egui::Ui, map: &Map, id: Id) {
    let text = match (map.node(id), map.parent(id)) {
        (Some(Node::Object(_)), Some(layer)) => {
            format!("{} › {}", node_name(map, layer), node_name(map, id))
        }
        _ => node_name(map, id),
    };
    heading(ui, &text);
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let palette = theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(theme::SPACING[3]);
        ui.label(
            egui::RichText::new(text)
                .size(theme::BODY)
                .color(palette.text_secondary),
        );
    });
}

fn caption(ui: &mut egui::Ui, text: &str) {
    let palette = theme::palette(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .size(theme::CAPTION)
            .color(palette.text_tertiary),
    );
}

/// A label with something other than a scrub field next to it.
fn row(ui: &mut egui::Ui, label: &str, body: impl FnOnce(&mut egui::Ui)) {
    let palette = theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_sized(
            [64.0, 22.0],
            egui::Label::new(
                egui::RichText::new(label)
                    .size(theme::BODY)
                    .color(palette.text_secondary),
            ),
        );
        body(ui);
    });
}

/// "2 Landmasses + 1 Lake": what a multi-selection holds.
fn selection_summary(map: &Map, ids: &[Id]) -> String {
    const KINDS: [(&str, &str); 5] = [
        ("Layer", "Layers"),
        ("Landmass", "Landmasses"),
        ("Lake", "Lakes"),
        ("Stamp", "Stamps"),
        ("Label", "Labels"),
    ];
    let mut counts = [0usize; 5];
    for &id in ids {
        let kind = match map.node(id) {
            Some(Node::Layer(_)) => 0,
            Some(Node::Object(object)) => match object.kind {
                ObjectKind::Landmass(_) => 1,
                ObjectKind::Lake(_) => 2,
                ObjectKind::Stamp(_) => 3,
                ObjectKind::Label(_) => 4,
            },
            None => continue,
        };
        counts[kind] += 1;
    }
    KINDS
        .iter()
        .zip(counts)
        .filter(|&(_, n)| n > 0)
        .map(|(&(one, many), n)| format!("{n} {}", if n == 1 { one } else { many }))
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolId;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable};
    use ssmap_core::{Canvas, DistanceUnit, Label, MultiPolygon, Polygon, Scale};

    struct Fixture {
        editor: Editor,
        panel: PropertiesPanel,
    }

    /// A Map with one Layer holding one Landmass; nothing is selected.
    fn fixture() -> (Fixture, Id, Id) {
        let mut editor = Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let layer = editor.new_layer().expect("a Layer");
        let island = Id::new();
        assert!(editor.run(Command::CreateLandmass {
            id: island,
            parent: layer,
            index: None,
            geometry: square(0.0, 40.0),
            fill: None,
        }));
        editor.select(Vec::new());
        let panel = PropertiesPanel::default();
        (Fixture { editor, panel }, layer, island)
    }

    fn square(min: f64, size: f64) -> MultiPolygon {
        MultiPolygon(vec![Polygon {
            exterior: vec![
                [min, min],
                [min + size, min],
                [min + size, min + size],
                [min, min + size],
            ],
            holes: Vec::new(),
        }])
    }

    fn harness(fixture: Fixture) -> Harness<'static, Fixture> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(320.0, 900.0))
            .build_ui_state(
                |ui, fixture: &mut Fixture| {
                    theme::install(&ui.ctx().clone());
                    fixture.panel.show(ui, &mut fixture.editor);
                },
                fixture,
            );
        harness.run();
        harness
    }

    #[test]
    fn nothing_selected_shows_the_active_layer_and_a_collapsed_map_section() {
        let (fixture, _, _) = fixture();
        let harness = harness(fixture);
        // The active Layer's fields.
        for label in ["Visible", "Locked", "Depth sort", "Opacity", "Clip to"] {
            assert!(harness.query_by_label(label).is_some(), "{label}");
        }
        // The Map section is there, closed: its fields are not.
        assert!(harness.query_by_label("Map").is_some());
        assert!(harness.query_by_label("Canvas").is_none());
        assert!(harness.query_by_label("Amount").is_none());
    }

    #[test]
    fn an_overridden_roughness_resets_to_the_map_default() {
        let (mut fixture, _, island) = fixture();
        let default = fixture.editor.map.defaults().roughness.clone();
        let own = Roughness::new(default.amount + 5.0, default.scale, default.seed);
        assert!(fixture.editor.run(Command::SetRoughness {
            ids: vec![island],
            roughness: Some(own),
        }));
        fixture.editor.select(vec![island]);
        let mut harness = harness(fixture);

        harness.get_by_label("Reset Amount").click();
        harness.run();
        let editor = &harness.state().editor;
        let Some(Node::Object(object)) = editor.map.node(island) else {
            panic!("an Object")
        };
        assert_eq!(object.kind.roughness(), None);
        // Inherited again: no reset arrow.
        assert!(harness.query_by_label("Reset Amount").is_none());
    }

    #[test]
    fn re_rolling_overrides_the_roughness_with_another_seed() {
        let (mut fixture, _, island) = fixture();
        fixture.editor.select(vec![island]);
        let mut harness = harness(fixture);

        harness.get_by_label("Re-roll").click();
        harness.run();
        let editor = &harness.state().editor;
        let default = editor.map.defaults().roughness.clone();
        let Some(Node::Object(object)) = editor.map.node(island) else {
            panic!("an Object")
        };
        let own = object.kind.roughness().expect("an override");
        assert_eq!((own.amount, own.scale), (default.amount, default.scale));
        assert_ne!(own.seed, default.seed);
        assert_eq!(editor.undo_label(), Some("Change Roughness"));
    }

    #[test]
    fn combine_is_disabled_with_one_landmass() {
        let (mut fixture, _, island) = fixture();
        fixture.editor.select(vec![island]);
        let harness = harness(fixture);
        assert!(harness.get_by_label("Union").accesskit_node().is_disabled());
        assert!(
            harness
                .get_by_label("Subtract")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            harness
                .query_by_label("Subtract removes the top Landmass from the one below it.")
                .is_some()
        );
    }

    #[test]
    fn a_multi_selection_is_summarised_and_can_be_combined() {
        let (mut fixture, layer, island) = fixture();
        let other = Id::new();
        assert!(fixture.editor.run(Command::CreateLandmass {
            id: other,
            parent: layer,
            index: None,
            geometry: square(10.0, 50.0),
            fill: None,
        }));
        fixture.editor.select(vec![island, other]);
        let mut harness = harness(fixture);
        assert!(harness.query_by_label("2 Landmasses").is_some());
        assert!(!harness.get_by_label("Union").accesskit_node().is_disabled());

        harness.get_by_label("Union").click();
        harness.run();
        assert_eq!(harness.state().editor.undo_label(), Some("Union"));
    }

    #[test]
    fn an_overridden_label_size_resets_to_the_styles_size() {
        let (mut fixture, layer, _) = fixture();
        let style_size = Label::default()
            .resolve(fixture.editor.map.label_styles())
            .size;
        let label = Id::new();
        assert!(fixture.editor.run(Command::CreateLabel {
            id: label,
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Hello".to_owned(),
                size: Some(style_size + 10.0),
                ..Label::default()
            }),
        }));
        fixture.editor.select(vec![label]);
        let mut harness = harness(fixture);
        harness.get_by_label("Reset Size").click();
        harness.run();
        let editor = &harness.state().editor;
        let Some(Node::Object(object)) = editor.map.node(label) else {
            panic!("an Object")
        };
        let ObjectKind::Label(stored) = &object.kind else {
            panic!("a Label")
        };
        assert_eq!(stored.size, None);
        assert_eq!(stored.resolve(editor.map.label_styles()).size, style_size);
        // Inherited again: no reset arrow.
        assert!(harness.query_by_label("Reset Size").is_none());
    }

    #[test]
    fn manage_label_styles_asks_for_the_window() {
        let (mut fixture, layer, _) = fixture();
        let label = Id::new();
        assert!(fixture.editor.run(Command::CreateLabel {
            id: label,
            parent: layer,
            index: None,
            label: Box::new(Label {
                text: "Hello".to_owned(),
                ..Label::default()
            }),
        }));
        fixture.editor.select(vec![label]);
        let mut harness = harness(fixture);
        harness.get_by_label("Manage Label styles…").click();
        harness.run();
        assert!(harness.state_mut().panel.take_manage_label_styles());
    }

    #[test]
    fn shuffle_variant_in_properties_stays_within_the_variant_group() {
        use ssmap_core::asset::{AssetRef, PackLibrary, PackRoots};
        use ssmap_render::SpriteStore;
        let roots = PackRoots {
            built_in: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../crates/core/tests/packs"),
            user: None,
        };
        let (library, problems) = PackLibrary::load(&roots);
        assert!(problems.is_empty(), "{problems:?}");
        let (mut fixture, _, _) = fixture();
        fixture
            .editor
            .set_sprites(SpriteStore::new(std::sync::Arc::new(library)));
        fixture.editor.selected_asset = Some("test-pack/mountain-1".parse().unwrap());
        let stamp = fixture
            .editor
            .place_stamp([10.0, 10.0])
            .expect("a Stamp is placed");
        fixture.editor.select(vec![stamp]);
        // Placing picks any Asset of the Variant group.
        let before = fixture.editor.stamp(stamp).expect("a Stamp").asset.clone();
        let caption = fixture.editor.asset_caption(&before);
        assert!(caption == "Mountain · v1" || caption == "Mountain · v2");
        let mut harness = harness(fixture);
        assert!(harness.query_by_label(&caption).is_some());

        // The pick is random and may repeat the same Variant, so click until it changes.
        let group: Vec<AssetRef> = ["test-pack/mountain-1", "test-pack/mountain-2"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        let mut asset = before.clone();
        for _ in 0..64 {
            harness.get_by_label("Shuffle variant").click();
            harness.run();
            asset = harness
                .state()
                .editor
                .stamp(stamp)
                .expect("a Stamp")
                .asset
                .clone();
            assert!(group.contains(&asset));
            if asset != before {
                break;
            }
        }
        assert_ne!(asset, before);
        assert_eq!(harness.state().editor.undo_label(), Some("Shuffle Variant"));
    }

    /// The test pack installed in the Editor, optionally flagged `attribution_required`.
    fn install_test_pack(editor: &mut Editor, attribution_required: bool) {
        use ssmap_core::asset::{PackLibrary, PackSource, parse_manifest};
        use ssmap_render::SpriteStore;
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/core/tests/packs/test-pack/pack.toml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        if attribution_required {
            text = text.replacen("[pack]\n", "[pack]\nattribution_required = true\n", 1);
        }
        let pack = parse_manifest(&path, &text, PackSource::BuiltIn).unwrap();
        let mut library = PackLibrary::default();
        let mut problems = Vec::new();
        library.add(pack, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        editor.set_sprites(SpriteStore::new(std::sync::Arc::new(library)));
    }

    #[test]
    fn stamp_shows_the_picker_and_scatter_settings_and_select_shows_no_tool_section() {
        let (mut fixture, _, _) = fixture();
        install_test_pack(&mut fixture.editor, false);
        fixture.editor.active_tool = ToolId::Stamp;
        let mut harness = harness(fixture);
        for label in [
            "Stamp",
            "Size",
            "Place on",
            "Density",
            "Spacing",
            "Size jitter",
            "Rotation jitter",
            "Random flip",
            "Random variant",
        ] {
            assert!(harness.query_by_label(label).is_some(), "{label}");
        }
        assert!(harness.query_by_label("Tree (test-pack/tree)").is_some());

        harness.state_mut().editor.active_tool = ToolId::Select;
        harness.run();
        for label in ["Density", "Tree (test-pack/tree)"] {
            assert!(harness.query_by_label(label).is_none(), "{label}");
        }
    }

    #[test]
    fn picking_a_thumbnail_arms_its_asset() {
        let (mut fixture, _, _) = fixture();
        install_test_pack(&mut fixture.editor, false);
        fixture.editor.active_tool = ToolId::Stamp;
        let mut harness = harness(fixture);
        assert_eq!(harness.state().editor.selected_asset, None);
        harness.get_by_label("Tree (test-pack/tree)").click();
        harness.run();
        assert_eq!(
            harness.state().editor.selected_asset,
            Some("test-pack/tree".parse().unwrap())
        );
        assert_eq!(harness.state().editor.active_tool, ToolId::Stamp);
        // The picked Asset's pack · author · licence.
        assert!(
            harness
                .query_by_label_contains("Test pack · StarScript Map Maker contributors · CC0-1.0")
                .is_some()
        );
    }

    #[test]
    fn variants_show_their_count_and_attribution_shows_the_dot() {
        let (mut fixture, _, _) = fixture();
        install_test_pack(&mut fixture.editor, true);
        fixture.editor.active_tool = ToolId::Stamp;
        let mut harness = harness(fixture);
        assert!(
            harness
                .query_by_label("Tree (test-pack/tree), attribution required")
                .is_some()
        );
        assert!(
            harness
                .query_by_label("Mountain (test-pack/mountain-1), 2 variants, attribution required")
                .is_some()
        );
        harness
            .get_by_label("Tree (test-pack/tree), attribution required")
            .click();
        harness.run();
        assert!(
            harness
                .query_by_label_contains("Attribution required")
                .is_some()
        );
    }

    #[test]
    fn no_dot_without_attribution_and_an_empty_search_offers_a_way_out() {
        let (mut fixture, _, _) = fixture();
        install_test_pack(&mut fixture.editor, false);
        fixture.editor.active_tool = ToolId::Stamp;
        let mut harness = harness(fixture);
        assert!(
            harness
                .query_by_label_contains("attribution required")
                .is_none()
        );
        // The search field is the first text field, above the Layer's name.
        for step in 0..2 {
            let search = harness
                .get_all_by_role(egui::accesskit::Role::TextInput)
                .next()
                .expect("the search field");
            if step == 0 {
                search.click();
            } else {
                search.type_text("zzz");
            }
            harness.run();
        }
        harness.run();
        assert!(harness.query_by_label("No Assets match “zzz”").is_some());
        harness.get_by_label("Clear search").click();
        harness.run();
        assert!(harness.query_by_label("Tree (test-pack/tree)").is_some());
    }

    #[test]
    fn the_land_brush_section_sets_the_default_mode() {
        let (mut fixture, _, _) = fixture();
        fixture.editor.active_tool = ToolId::LandBrush;
        let mut harness = harness(fixture);
        assert!(harness.query_by_label("Size").is_some());
        harness.get_by_label("Erase").click();
        harness.run();
        assert!(harness.state().editor.erase_default);
        harness.get_by_label("Paint").click();
        harness.run();
        assert!(!harness.state().editor.erase_default);
    }

    #[test]
    fn tools_without_settings_have_no_section_and_the_eraser_has_only_size() {
        let (mut fixture, _, _) = fixture();
        fixture.editor.active_tool = ToolId::StampEraser;
        let mut harness = harness(fixture);
        assert!(harness.query_by_label("Stamp Eraser").is_some());
        assert!(harness.query_by_label("Size").is_some());
        assert!(harness.query_by_label("Density").is_none());
        for tool in [
            ToolId::Select,
            ToolId::Pan,
            ToolId::Lasso,
            ToolId::Label,
            ToolId::ExportRegion,
        ] {
            harness.state_mut().editor.active_tool = tool;
            harness.run();
            assert!(harness.query_by_label(tool.label()).is_none(), "{tool:?}");
        }
    }

    #[test]
    fn the_summary_counts_each_kind() {
        let (fixture, layer, island) = fixture();
        let map = &fixture.editor.map;
        assert_eq!(selection_summary(map, &[island]), "1 Landmass");
        assert_eq!(
            selection_summary(map, &[island, layer]),
            "1 Layer + 1 Landmass"
        );
        assert_eq!(selection_summary(map, &[]), "");
    }
}
