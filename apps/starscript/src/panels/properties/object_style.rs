//! The Roughness and Shore style sections, for Landmasses and Lakes and for the Map defaults.

use eframe::egui;
use ssmap_core::{Command, Id, MAX_RIPPLES, Node, Roughness, ShoreStyle};

use super::{Inherit, PropertiesPanel, caption};
use crate::editor::Editor;
use crate::prop_widgets::{self, Edit, Provenance, ScrubSpec, SectionHeader};

impl PropertiesPanel {
    /// Applies what a field did. A reset goes back to the default; anything else is an edit of
    /// `value`, one Undo step per scrub.
    fn edit<T: Clone>(
        &mut self,
        editor: &mut Editor,
        edit: Edit,
        group: &Inherit<'_, T>,
        apply: impl Fn(&mut T),
    ) {
        if edit.reset {
            if let Some(reset) = group.reset.clone() {
                let _ = editor.run(reset);
            }
            return;
        }
        self.live
            .apply_all(editor, edit, group.label, || group.commands(apply));
    }

    /// The Roughness section of a Landmass or Lake: its own settings, or the Map default.
    pub(super) fn object_roughness(&mut self, ui: &mut egui::Ui, editor: &mut Editor, ids: &[Id]) {
        let ids: Vec<Id> = ids
            .iter()
            .copied()
            .filter(|&id| {
                matches!(editor.map.node(id), Some(Node::Object(o)) if o.kind.geometry().is_some())
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let default = editor.map.defaults().roughness.clone();
        let mut any_own = false;
        let members: Vec<Roughness> = ids
            .iter()
            .map(|&id| match editor.map.node(id) {
                Some(Node::Object(o)) => o.kind.roughness().map_or_else(
                    || default.clone(),
                    |own| {
                        any_own = true;
                        own.clone()
                    },
                ),
                _ => default.clone(),
            })
            .collect();
        let provenance = if any_own {
            Provenance::Overridden
        } else {
            Provenance::Inherited
        };
        let set = |i: usize, roughness| Command::SetRoughness {
            ids: vec![ids[i]],
            roughness: Some(roughness),
        };
        let group = Inherit {
            members: &members,
            provenance,
            label: "Change Roughness",
            set: &set,
            reset: any_own.then(|| Command::SetRoughness {
                ids: ids.clone(),
                roughness: None,
            }),
        };
        let header = SectionHeader {
            kind: "roughness",
            title: "Roughness",
            badge: any_own.then_some("overridden"),
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            ui.push_id(("roughness", &ids), |ui| {
                self.roughness_fields(ui, editor, &group);
            });
        });
    }

    /// Amount, scale and seed, and a button that picks a new seed.
    pub(super) fn roughness_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        group: &Inherit<'_, Roughness>,
    ) {
        let provenance = group.provenance;
        let mut next = group.current().clone();
        let spec = |label| ScrubSpec::new(label).provenance(provenance);
        let edit = prop_widgets::slider(
            ui,
            &mut next.amount,
            &spec("Amount")
                .unit(" u")
                .range(0.0, 1000.0)
                .slider_max(50.0)
                .default_value(0.0)
                .step(0.5)
                .decimals(1)
                .mixed(group.mixed(|r| r.amount)),
        );
        let amount = next.amount;
        self.edit(editor, edit, group, |r| r.amount = amount);
        let edit = prop_widgets::slider(
            ui,
            &mut next.scale,
            &spec("Scale")
                .unit(" u")
                .range(0.5, 10_000.0)
                .slider_max(200.0)
                .step(1.0)
                .decimals(1)
                .mixed(group.mixed(|r| r.scale)),
        );
        let scale = next.scale;
        self.edit(editor, edit, group, |r| r.scale = scale);
        let mut seed = f64::from(next.seed);
        let edit =
            prop_widgets::scrub_field(ui, &mut seed, &spec("Seed").range(0.0, f64::from(u32::MAX)));
        let seed = seed as u32;
        self.edit(editor, edit, group, |r| r.seed = seed);
        let reroll = ui
            .button("Re-roll")
            .on_hover_text("Pick another coastline with the same Amount and Scale");
        if reroll.clicked() {
            let seed = Id::new().as_uuid().as_u128() as u32;
            self.edit(editor, Edit::instant(), group, |r| r.seed = seed);
        }
    }

    /// The Shore style section of a Landmass or Lake: its own settings, or the Map default.
    pub(super) fn object_shore(&mut self, ui: &mut egui::Ui, editor: &mut Editor, ids: &[Id]) {
        let mut shored = Vec::new();
        let mut members = Vec::new();
        let mut any_own = false;
        for &id in ids {
            let Some(Node::Object(object)) = editor.map.node(id) else {
                continue;
            };
            let Some(kind) = object.kind.shore_kind() else {
                continue;
            };
            shored.push(id);
            members.push(match object.kind.shore() {
                Some(own) => {
                    any_own = true;
                    own.clone()
                }
                None => editor.map.defaults().shore_for(kind).clone(),
            });
        }
        if shored.is_empty() {
            return;
        }
        let provenance = if any_own {
            Provenance::Overridden
        } else {
            Provenance::Inherited
        };
        let set = |i: usize, shore| Command::SetShore {
            ids: vec![shored[i]],
            shore: Some(shore),
        };
        let group = Inherit {
            members: &members,
            provenance,
            label: "Change Shore Style",
            set: &set,
            reset: any_own.then(|| Command::SetShore {
                ids: shored.clone(),
                shore: None,
            }),
        };
        let header = SectionHeader {
            kind: "shore",
            title: "Shore style",
            badge: any_own.then_some("overridden"),
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            ui.push_id(("shore", &shored), |ui| {
                self.shore_fields(ui, editor, &group);
            });
        });
    }

    /// Outline, ripples and inner shade.
    pub(super) fn shore_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        group: &Inherit<'_, ShoreStyle>,
    ) {
        let provenance = group.provenance;
        let mut style = group.current().clone();
        let spec = |label| ScrubSpec::new(label).provenance(provenance);
        let length = |label, max: f64| {
            spec(label)
                .unit(" u")
                .range(0.0, max)
                .slider_max(max / 5.0)
                .step(0.1)
                .decimals(1)
        };

        caption(ui, "Outline");
        let edit = prop_widgets::slider(
            ui,
            &mut style.outline.width,
            &length("Width", 100.0).mixed(group.mixed(|s| s.outline.width)),
        );
        let width = style.outline.width;
        self.edit(editor, edit, group, |s| s.outline.width = width);
        let mut colour = Some(style.outline.color);
        let edit = prop_widgets::swatch(ui, "Colour", &mut colour, false, provenance).edit;
        let colour = colour.unwrap_or(style.outline.color);
        self.edit(editor, edit, group, |s| s.outline.color = colour);

        caption(ui, "Ripples");
        let mut count = f64::from(style.ripples.count);
        let edit = prop_widgets::number_field(
            ui,
            &mut count,
            &spec("Count")
                .range(0.0, f64::from(MAX_RIPPLES))
                .step(1.0)
                .mixed(group.mixed(|s| f64::from(s.ripples.count))),
        );
        let count = count as u32;
        self.edit(editor, edit, group, |s| s.ripples.count = count);
        let edit = prop_widgets::slider(
            ui,
            &mut style.ripples.spacing,
            &length("Spacing", 500.0).mixed(group.mixed(|s| s.ripples.spacing)),
        );
        let spacing = style.ripples.spacing;
        self.edit(editor, edit, group, |s| s.ripples.spacing = spacing);
        let edit = prop_widgets::slider(
            ui,
            &mut style.ripples.width,
            &length("Line width", 50.0).mixed(group.mixed(|s| s.ripples.width)),
        );
        let width = style.ripples.width;
        self.edit(editor, edit, group, |s| s.ripples.width = width);
        let mut colour = Some(style.ripples.color);
        let edit = prop_widgets::swatch(ui, "Line colour", &mut colour, false, provenance).edit;
        let colour = colour.unwrap_or(style.ripples.color);
        self.edit(editor, edit, group, |s| s.ripples.color = colour);
        let edit = prop_widgets::slider(
            ui,
            &mut style.ripples.fade,
            &spec("Fade")
                .range(0.0, 1.0)
                .step(0.01)
                .decimals(2)
                .mixed(group.mixed(|s| s.ripples.fade)),
        );
        let fade = style.ripples.fade;
        self.edit(editor, edit, group, |s| s.ripples.fade = fade);

        caption(ui, "Inner shade");
        let edit = prop_widgets::slider(
            ui,
            &mut style.shading.width,
            &length("Shade width", 500.0).mixed(group.mixed(|s| s.shading.width)),
        );
        let width = style.shading.width;
        self.edit(editor, edit, group, |s| s.shading.width = width);
        let mut colour = Some(style.shading.color);
        let edit = prop_widgets::swatch(ui, "Shade colour", &mut colour, false, provenance).edit;
        let colour = colour.unwrap_or(style.shading.color);
        self.edit(editor, edit, group, |s| s.shading.color = colour);
    }
}
