//! The properties panel: shows and edits what is selected, through Commands.

use eframe::egui;
use ssmap_core::{
    Align, Color, Command, DistanceUnit, Id, Label, Landmass, MAX_RIPPLES, Map, Node, ObjectKind,
    Roughness, ShoreKind, ShoreStyle, Stamp,
};
use ssmap_fonts::LABEL_FAMILIES;

use crate::editor::Editor;
use crate::layer_tree::{landmass_name, landmasses, node_name, selected_landmass};
use crate::prop_widgets::{self, Edit, LiveEdit, Provenance, ScrubSpec, SectionHeader};
use crate::theme;

#[derive(Default)]
pub struct PropertiesPanel {
    /// The name being typed for a Layer, committed when the field loses focus.
    name_edit: Option<(Id, String)>,
    /// The text being typed for a Label, committed when the field loses focus.
    text_edit: Option<(Id, String)>,
    /// Set while a Label colour picker's edits are being gathered into one gesture.
    colour_gesture: bool,
    /// Turns the scrubs, sliders and swatches into one Undo step each.
    live: LiveEdit,
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

    fn label_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        locked: bool,
        label: &Label,
    ) {
        // Colour pickers change on every frame of a drag: gather them into one Undo step,
        // closed when the pointer lets go.
        if self.colour_gesture && ui.input(|i| !i.pointer.any_down()) {
            self.colour_gesture = false;
            editor.end_gesture();
        }
        ui.label("Text");
        let text = match &mut self.text_edit {
            Some((editing, text)) if *editing == id => text,
            _ => {
                self.text_edit = Some((id, label.text.clone()));
                &mut self.text_edit.as_mut().expect("just set").1
            }
        };
        let field = ui.add(
            egui::TextEdit::multiline(text)
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if field.lost_focus() && *text != label.text {
            let next = Label {
                text: text.clone(),
                ..label.clone()
            };
            if !editor.run(Command::SetLabel {
                id,
                label: Box::new(next),
            }) {
                self.text_edit = None;
            }
        } else if !field.has_focus() && *text != label.text {
            // Changed elsewhere (undo, the text box on the Canvas): show the Map's value.
            *text = label.text.clone();
        }

        let styles = editor.map.label_styles().clone();
        let mut restyled = None;
        egui::Grid::new("label style")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Style");
                ui.horizontal(|ui| {
                    let current = styles
                        .get(label.style)
                        .unwrap_or_else(|| styles.default_style());
                    egui::ComboBox::from_id_salt("style")
                        .selected_text(&current.name)
                        .show_ui(ui, |ui| {
                            for style in styles.iter() {
                                if ui
                                    .selectable_label(style.id == current.id, &style.name)
                                    .clicked()
                                {
                                    restyled = Some(Label {
                                        style: style.id,
                                        ..label.clone()
                                    });
                                }
                            }
                        });
                    let overridden = label.font.is_some()
                        || label.size.is_some()
                        || label.letter_spacing.is_some()
                        || label.line_height.is_some()
                        || label.all_caps.is_some()
                        || label.fill.is_some()
                        || label.opacity.is_some()
                        || label.halo.is_some();
                    if ui
                        .add_enabled(overridden, egui::Button::new("Reset overrides"))
                        .on_hover_text("Follow the style again")
                        .clicked()
                    {
                        restyled = Some(Label {
                            font: None,
                            size: None,
                            letter_spacing: None,
                            line_height: None,
                            all_caps: None,
                            fill: None,
                            opacity: None,
                            halo: None,
                            ..label.clone()
                        });
                    }
                });
                ui.end_row();
            });
        if let Some(next) = restyled {
            editor.run(Command::SetLabel {
                id,
                label: Box::new(next),
            });
            return;
        }

        // Edits are made on the resolved settings; only what changed becomes an override.
        let mut next = label.resolve(&styles);
        // What changed this frame: a drag (one gesture), a click (one step) or a colour.
        let mut dragged: Option<egui::Response> = None;
        let mut clicked = false;
        let mut coloured = false;
        egui::Grid::new("label").num_columns(2).show(ui, |ui| {
            ui.label("Font");
            egui::ComboBox::from_id_salt("font")
                .selected_text(&next.font.family)
                .show_ui(ui, |ui| {
                    for family in LABEL_FAMILIES {
                        clicked |= ui
                            .selectable_value(&mut next.font.family, (*family).to_owned(), *family)
                            .clicked();
                    }
                });
            ui.end_row();

            ui.label("Weight");
            ui.horizontal(|ui| {
                drag(
                    ui.add(egui::DragValue::new(&mut next.font.weight).range(100..=900)),
                    &mut dragged,
                );
                clicked |= ui.checkbox(&mut next.font.italic, "Italic").changed();
            });
            ui.end_row();

            ui.label("Size");
            drag(
                ui.add(
                    egui::DragValue::new(&mut next.size)
                        .range(0.1..=f64::MAX)
                        .speed(0.5),
                ),
                &mut dragged,
            );
            ui.end_row();

            ui.label("Letter spacing");
            drag(
                ui.add(
                    egui::DragValue::new(&mut next.letter_spacing)
                        .speed(0.005)
                        .suffix(" em"),
                ),
                &mut dragged,
            );
            ui.end_row();

            ui.label("Line height");
            drag(
                ui.add(
                    egui::DragValue::new(&mut next.line_height)
                        .range(0.1..=10.0)
                        .speed(0.01)
                        .suffix("×"),
                ),
                &mut dragged,
            );
            ui.end_row();

            ui.label("Align");
            ui.horizontal(|ui| {
                for (align, name) in [
                    (Align::Left, "Left"),
                    (Align::Centre, "Centre"),
                    (Align::Right, "Right"),
                ] {
                    clicked |= ui.selectable_value(&mut next.align, align, name).clicked();
                }
            });
            ui.end_row();

            ui.label("Rotation");
            drag(
                ui.add(
                    egui::DragValue::new(&mut next.rotation)
                        .speed(0.5)
                        .suffix("°"),
                ),
                &mut dragged,
            );
            ui.end_row();

            ui.label("All caps");
            clicked |= ui.checkbox(&mut next.all_caps, "").changed();
            ui.end_row();

            ui.label("Fill");
            coloured |= colour_button(ui, &mut next.fill);
            ui.end_row();

            ui.label("Opacity");
            let mut percent = next.opacity * 100.0;
            let slider = ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%"));
            next.opacity = (percent / 100.0).clamp(0.0, 1.0);
            drag(slider, &mut dragged);
            ui.end_row();

            ui.label("Halo");
            ui.horizontal(|ui| {
                drag(
                    ui.add(
                        egui::DragValue::new(&mut next.halo.width)
                            .range(0.0..=f64::MAX)
                            .speed(0.1),
                    ),
                    &mut dragged,
                );
                coloured |= colour_button(ui, &mut next.halo.color);
            });
            ui.end_row();

            ui.label("Locked");
            let mut lock = locked;
            if ui.checkbox(&mut lock, "").changed() {
                editor.run(Command::SetLocked { id, locked: lock });
            }
            ui.end_row();
        });

        let edited = label.with_resolved(&styles, &next);
        let changed = edited != *label;
        let set = Command::SetLabel {
            id,
            label: Box::new(edited),
        };
        if let Some(response) = dragged {
            // Dragging a value is one gesture: live updates, one Undo step.
            if response.drag_started() {
                editor.begin_gesture("Edit Label");
            }
            if changed {
                editor.gesture(set);
            }
            if response.drag_stopped() {
                editor.end_gesture();
            }
        } else if coloured && changed {
            if !self.colour_gesture {
                editor.begin_gesture("Change Colour");
                self.colour_gesture = true;
            }
            editor.gesture(set);
        } else if clicked && changed {
            editor.run(set);
        }
    }
}

impl PropertiesPanel {
    /// Every override of a Stamp. Dragging a value is one gesture, a click one step, and a
    /// colour pick everything until the pointer lets go.
    fn stamp_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        locked: bool,
        stamp: &Stamp,
    ) {
        if self.colour_gesture && ui.input(|i| !i.pointer.any_down()) {
            self.colour_gesture = false;
            editor.end_gesture();
        }
        let mut next = stamp.clone();
        let mut dragged: Option<egui::Response> = None;
        let mut clicked = false;
        let mut coloured = false;
        egui::Grid::new("stamp").num_columns(2).show(ui, |ui| {
            ui.label("Asset");
            ui.label(next.asset.to_string());
            ui.end_row();

            ui.label("Position");
            ui.horizontal(|ui| {
                drag(
                    ui.add(
                        egui::DragValue::new(&mut next.position[0])
                            .speed(0.5)
                            .prefix("x "),
                    ),
                    &mut dragged,
                );
                drag(
                    ui.add(
                        egui::DragValue::new(&mut next.position[1])
                            .speed(0.5)
                            .prefix("y "),
                    ),
                    &mut dragged,
                );
            });
            ui.end_row();

            ui.label("Scale");
            drag(
                ui.add(
                    egui::DragValue::new(&mut next.scale)
                        .range(0.01..=100.0)
                        .speed(0.01)
                        .suffix("×"),
                ),
                &mut dragged,
            );
            ui.end_row();

            ui.label("Rotation");
            drag(
                ui.add(
                    egui::DragValue::new(&mut next.rotation)
                        .speed(0.5)
                        .suffix("°"),
                ),
                &mut dragged,
            );
            ui.end_row();

            ui.label("Flip");
            clicked |= ui.checkbox(&mut next.flip, "Horizontal").changed();
            ui.end_row();

            ui.label("Tint");
            ui.horizontal(|ui| {
                let mut tinted = next.tint.is_some();
                if ui.checkbox(&mut tinted, "").changed() {
                    next.tint = tinted.then(|| Color::rgb(0xff, 0xff, 0xff));
                    clicked = true;
                }
                if let Some(tint) = &mut next.tint {
                    coloured |= colour_button(ui, tint);
                }
            });
            ui.end_row();

            ui.label("Fill");
            ui.horizontal(|ui| {
                let mut own = next.fill.is_some();
                if ui.checkbox(&mut own, "Own colour").changed() {
                    next.fill = own.then_some(Landmass::DEFAULT_FILL);
                    clicked = true;
                }
                if let Some(fill) = &mut next.fill {
                    coloured |= colour_button(ui, fill);
                }
            });
            ui.end_row();

            ui.label("Opacity");
            let mut percent = next.opacity * 100.0;
            let slider = ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).suffix("%"));
            next.opacity = (percent / 100.0).clamp(0.0, 1.0);
            drag(slider, &mut dragged);
            ui.end_row();

            ui.label("Locked");
            let mut lock = locked;
            if ui.checkbox(&mut lock, "").changed() {
                editor.run(Command::SetLocked { id, locked: lock });
            }
            ui.end_row();
        });

        let set = Command::SetStamp {
            id,
            stamp: Box::new(next.clone()),
        };
        if let Some(response) = dragged {
            if response.drag_started() {
                editor.begin_gesture("Edit Stamp");
            }
            if next != *stamp {
                editor.gesture(set);
            }
            if response.drag_stopped() {
                editor.end_gesture();
            }
        } else if coloured && next != *stamp {
            if !self.colour_gesture {
                editor.begin_gesture("Change Colour");
                self.colour_gesture = true;
            }
            editor.gesture(set);
        } else if clicked && next != *stamp {
            editor.run(set);
        }
    }
}

/// Notes a value widget that changed, or began or ended a drag, this frame.
fn drag(response: egui::Response, dragged: &mut Option<egui::Response>) {
    if response.changed() || response.drag_started() || response.drag_stopped() {
        *dragged = Some(response);
    }
}

/// A colour swatch that opens a picker. Returns whether the colour changed.
fn colour_button(ui: &mut egui::Ui, color: &mut Color) -> bool {
    let mut picked = egui::Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a);
    let changed = egui::color_picker::color_edit_button_srgba(
        ui,
        &mut picked,
        egui::color_picker::Alpha::OnlyBlend,
    )
    .changed();
    if changed {
        let [r, g, b, a] = picked.to_srgba_unmultiplied();
        *color = Color::rgba(r, g, b, a);
    }
    changed
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
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable};
    use ssmap_core::{Canvas, DistanceUnit, MultiPolygon, Polygon, Scale};

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
