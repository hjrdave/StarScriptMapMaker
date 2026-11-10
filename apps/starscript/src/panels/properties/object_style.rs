//! The Roughness and Shore style sections, for Landmasses and Lakes and for the Map defaults.

use eframe::egui;
use ssmap_core::{
    Color, Command, FillStyle, Grain, Id, MAX_GRAIN_SIZE, MAX_MOTTLE_SCALE, MAX_REACH, MAX_RIPPLES,
    MIN_GRAIN_SIZE, MIN_LINE_SPACING, MIN_MOTTLE_SCALE, Mottle, Node, ObjectKind, Roughness,
    ShoreStyle,
};

use super::{Inherit, PropertiesPanel, group_caption};
use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::prop_widgets::{self, Edit, Provenance, ScrubSpec, SectionHeader};

/// What the Fill style fields change: one Landmass's override, or the Map-wide default.
#[derive(Clone, Copy)]
pub(super) enum FillTarget {
    /// A Landmass's own Fill style.
    Landmass(Id),
    /// The Fill style of every Landmass without its own.
    MapDefault,
}

impl FillTarget {
    /// The Command that sets `style` on this target.
    fn set(self, style: FillStyle) -> Command {
        match self {
            FillTarget::Landmass(id) => Command::SetFillStyle {
                ids: vec![id],
                style: Some(style),
            },
            FillTarget::MapDefault => Command::SetDefaultFillStyle { style },
        }
    }
}

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

    /// How a Landmass's fill is drawn: Grain and Mottled switches, and the settings of each
    /// one that is on. Each scrub of a value is one Undo step. A Landmass with an override of
    /// its own also gets a button that returns it to the Map default.
    pub(super) fn fill_style(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        target: FillTarget,
        style: FillStyle,
    ) {
        if let FillTarget::Landmass(id) = target {
            let own = matches!(
                editor.map().node(id),
                Some(Node::Object(o)) if o.kind.fill_style().is_some()
            );
            if own && ui.button("Use Map default").clicked() {
                let clear = Command::SetFillStyle {
                    ids: vec![id],
                    style: None,
                };
                self.live
                    .apply(editor, Edit::instant(), "Change Fill Style", || clear);
            }
        }
        let set = |next: FillStyle| target.set(next);
        let mut grain_on = style.grain.is_some();
        if prop_widgets::checkbox_row(ui, "Grain", &mut grain_on) {
            let next = FillStyle {
                grain: grain_on.then_some(Grain::DEFAULT),
                ..style
            };
            self.live
                .apply(editor, Edit::instant(), "Change Fill Style", || set(next));
        }
        if let Some(grain) = style.grain {
            self.grain_fields(ui, editor, target, style, grain);
        }
        let mut mottle_on = style.mottle.is_some();
        if prop_widgets::checkbox_row(ui, "Mottled", &mut mottle_on) {
            let next = FillStyle {
                mottle: mottle_on.then_some(Mottle::DEFAULT),
                ..style
            };
            self.live
                .apply(editor, Edit::instant(), "Change Fill Style", || set(next));
        }
        if let Some(mottle) = style.mottle {
            self.mottle_fields(ui, editor, target, style, mottle);
        }
    }

    /// Amount and size of the grain of `style`.
    fn grain_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        target: FillTarget,
        style: FillStyle,
        grain: Grain,
    ) {
        // The amount is shown as a percentage and stored as a fraction.
        let (mut amount, mut size) = (grain.amount * 100.0, grain.size);
        let (amount_edit, size_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut amount,
                    &ScrubSpec::new("Grain amount")
                        .prefix("Amount")
                        .unit("%")
                        .range(0.0, 100.0)
                        .step(1.0)
                        .decimals(0),
                )
            },
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut size,
                    &ScrubSpec::new("Grain size")
                        .prefix("Size")
                        .unit(" u")
                        .range(MIN_GRAIN_SIZE, MAX_GRAIN_SIZE)
                        .step(0.25)
                        .decimals(2),
                )
            },
        );
        let grained = |amount: f64, size: f64| {
            target.set(FillStyle {
                grain: Some(Grain {
                    amount: amount / 100.0,
                    size,
                }),
                ..style
            })
        };
        self.live.apply(editor, amount_edit, "Change Grain", || {
            grained(amount, size)
        });
        self.live
            .apply(editor, size_edit, "Change Grain", || grained(amount, size));
    }

    /// Colour, scale, contrast and seed of the mottle of `style`.
    fn mottle_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        target: FillTarget,
        style: FillStyle,
        mottle: Mottle,
    ) {
        let with = move |mottle: Mottle| {
            target.set(FillStyle {
                mottle: Some(mottle),
                ..style
            })
        };
        // The patch colour; its popover changes it live, as the fill's does.
        let mut colour = Some(mottle.color);
        let response =
            prop_widgets::swatch(ui, "Mottle colour", &mut colour, false, Provenance::Plain);
        let color = colour.unwrap_or(mottle.color);
        self.live.apply(editor, response.edit, "Change Mottle", || {
            with(Mottle { color, ..mottle })
        });
        // Scale and contrast, the contrast as a percentage.
        let (mut scale, mut contrast) = (mottle.scale, mottle.contrast * 100.0);
        let (scale_edit, contrast_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut scale,
                    &ScrubSpec::new("Mottle scale")
                        .prefix("Scale")
                        .unit(" u")
                        .range(MIN_MOTTLE_SCALE, MAX_MOTTLE_SCALE)
                        .logarithmic()
                        .step(5.0)
                        .decimals(0),
                )
            },
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut contrast,
                    &ScrubSpec::new("Mottle contrast")
                        .prefix("Contrast")
                        .unit("%")
                        .range(0.0, 100.0)
                        .step(1.0)
                        .decimals(0),
                )
            },
        );
        self.live.apply(editor, scale_edit, "Change Mottle", || {
            with(Mottle { scale, ..mottle })
        });
        self.live.apply(editor, contrast_edit, "Change Mottle", || {
            with(Mottle {
                contrast: contrast / 100.0,
                ..mottle
            })
        });
        // The seed is the same kind of field, half the row wide, with the dice beside it.
        let mut seed = f64::from(mottle.seed);
        let (seed_edit, reroll) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut seed,
                    &ScrubSpec::new("Mottle seed")
                        .prefix("Seed")
                        .range(0.0, f64::from(u32::MAX)),
                )
            },
            |ui| {
                ui.horizontal(|ui| icons::icon_button(ui, Icon::ReRoll, "Re-roll mottle").clicked())
                    .inner
            },
        );
        let seed = seed as u32;
        self.live.apply(editor, seed_edit, "Change Mottle", || {
            with(Mottle { seed, ..mottle })
        });
        if reroll {
            let seed = Id::new().as_uuid().as_u128() as u32;
            self.live
                .apply(editor, Edit::instant(), "Change Mottle", || {
                    with(Mottle { seed, ..mottle })
                });
        }
    }

    /// The Roughness section of a Landmass or Lake: its own settings, or the Map default.
    /// Rivers have none, so a selection with one shows none.
    pub(super) fn object_roughness(&mut self, ui: &mut egui::Ui, editor: &mut Editor, ids: &[Id]) {
        let river = |id: Id| matches!(editor.map().node(id), Some(Node::Object(o)) if matches!(o.kind, ObjectKind::River(_)));
        if ids.iter().any(|&id| river(id)) {
            return;
        }
        let ids: Vec<Id> = ids
            .iter()
            .copied()
            .filter(|&id| {
                matches!(editor.map().node(id), Some(Node::Object(o)) if o.kind.geometry().is_some())
            })
            .collect();
        if ids.is_empty() {
            return;
        }
        let default = editor.map().defaults().roughness.clone();
        let mut any_own = false;
        let members: Vec<Roughness> = ids
            .iter()
            .map(|&id| match editor.map().node(id) {
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
        let (amount_edit, scale_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut next.amount,
                    &spec("Amount")
                        .prefix("Amount")
                        .unit(" u")
                        .range(0.0, 1000.0)
                        .step(0.5)
                        .decimals(1)
                        .mixed(group.mixed(|r| r.amount)),
                )
            },
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut next.scale,
                    &spec("Scale")
                        .prefix("Scale")
                        .unit(" u")
                        .range(0.5, 10_000.0)
                        .step(1.0)
                        .decimals(1)
                        .mixed(group.mixed(|r| r.scale)),
                )
            },
        );
        let (amount, scale) = (next.amount, next.scale);
        self.edit(editor, amount_edit, group, |r| r.amount = amount);
        self.edit(editor, scale_edit, group, |r| r.scale = scale);
        // The seed is the same kind of field, half the row wide, with the dice beside it.
        let mut seed = f64::from(next.seed);
        let (edit, reroll) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut seed,
                    &spec("Seed").prefix("Seed").range(0.0, f64::from(u32::MAX)),
                )
            },
            |ui| {
                ui.horizontal(|ui| icons::icon_button(ui, Icon::ReRoll, "Re-roll").clicked())
                    .inner
            },
        );
        let seed = seed as u32;
        self.edit(editor, edit, group, |r| r.seed = seed);
        if reroll {
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
            let Some(Node::Object(object)) = editor.map().node(id) else {
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
                None => editor.map().defaults().shore_for(kind).clone(),
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

    /// Outline, ripples and inner shade (variant C of the Shores tab): each group under a
    /// caption, every number a field with its name inside the box, two to a row (a lone one
    /// takes half), and every colour a row with its label.
    pub(super) fn shore_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        group: &Inherit<'_, ShoreStyle>,
    ) {
        let provenance = group.provenance;
        let mut style = group.current().clone();
        let spec = |label| ScrubSpec::new(label).provenance(provenance);
        let length = |label, max: f64| spec(label).unit(" u").range(0.0, max).step(0.1).decimals(1);
        let colour_row = |ui: &mut egui::Ui, label: &str, colour: Color| {
            let mut value = Some(colour);
            let edit =
                prop_widgets::swatch_shown_as(ui, label, "Colour", &mut value, false, provenance)
                    .edit;
            (edit, value.unwrap_or(colour))
        };

        group_caption(ui, "Outline");
        let (edit, ()) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut style.outline.width,
                    &length("Outline width", 100.0)
                        .prefix("Width")
                        .mixed(group.mixed(|s| s.outline.width)),
                )
            },
            |_| {},
        );
        let width = style.outline.width;
        self.edit(editor, edit, group, |s| s.outline.width = width);
        let (edit, colour) = colour_row(ui, "Outline colour", style.outline.color);
        self.edit(editor, edit, group, |s| s.outline.color = colour);

        group_caption(ui, "Ripples");
        let mut count = f64::from(style.ripples.count);
        let (count_edit, spacing_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut count,
                    &spec("Ripple count")
                        .prefix("Count")
                        .range(0.0, f64::from(MAX_RIPPLES))
                        .step(1.0)
                        .mixed(group.mixed(|s| f64::from(s.ripples.count))),
                )
            },
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut style.ripples.spacing,
                    &length("Ripple spacing", 500.0)
                        .prefix("Spacing")
                        .mixed(group.mixed(|s| s.ripples.spacing)),
                )
            },
        );
        let count = count as u32;
        let spacing = style.ripples.spacing;
        self.edit(editor, count_edit, group, |s| s.ripples.count = count);
        self.edit(editor, spacing_edit, group, |s| s.ripples.spacing = spacing);
        let (width_edit, fade_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut style.ripples.width,
                    &length("Ripple line width", 50.0)
                        .prefix("Width")
                        .mixed(group.mixed(|s| s.ripples.width)),
                )
            },
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut style.ripples.fade,
                    &spec("Ripple fade")
                        .prefix("Fade")
                        .range(0.0, 1.0)
                        .step(0.01)
                        .decimals(2)
                        .mixed(group.mixed(|s| s.ripples.fade)),
                )
            },
        );
        let (width, fade) = (style.ripples.width, style.ripples.fade);
        self.edit(editor, width_edit, group, |s| s.ripples.width = width);
        self.edit(editor, fade_edit, group, |s| s.ripples.fade = fade);
        let (edit, colour) = colour_row(ui, "Ripple line colour", style.ripples.color);
        self.edit(editor, edit, group, |s| s.ripples.color = colour);

        group_caption(ui, "Inner shade");
        let (edit, ()) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut style.shading.width,
                    &length("Shade width", 500.0)
                        .prefix("Width")
                        .mixed(group.mixed(|s| s.shading.width)),
                )
            },
            |_| {},
        );
        let width = style.shading.width;
        self.edit(editor, edit, group, |s| s.shading.width = width);
        let (edit, colour) = colour_row(ui, "Shade colour", style.shading.color);
        self.edit(editor, edit, group, |s| s.shading.color = colour);

        self.shore_water_fields(ui, editor, group, &mut style);
        group_caption(ui, "Preset");
        if ui.button("Soft glow").clicked() {
            self.edit(editor, Edit::instant(), group, |s| {
                *s = ShoreStyle::soft_glow();
            });
        }
    }

    /// The outer glow and water lines of a Shore style, the part of the Shores fields that
    /// lies on the water.
    fn shore_water_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        group: &Inherit<'_, ShoreStyle>,
        style: &mut ShoreStyle,
    ) {
        let provenance = group.provenance;
        let spec = |label| ScrubSpec::new(label).provenance(provenance);
        let length = |label, max: f64| spec(label).unit(" u").range(0.0, max).step(0.1).decimals(1);
        let colour_row = |ui: &mut egui::Ui, label: &str, colour: Color| {
            let mut value = Some(colour);
            let edit =
                prop_widgets::swatch_shown_as(ui, label, "Colour", &mut value, false, provenance)
                    .edit;
            (edit, value.unwrap_or(colour))
        };

        group_caption(ui, "Outer glow");
        let (edit, ()) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut style.glow.width,
                    &length("Glow width", MAX_REACH)
                        .prefix("Width")
                        .mixed(group.mixed(|s| s.glow.width)),
                )
            },
            |_| {},
        );
        let width = style.glow.width;
        self.edit(editor, edit, group, |s| s.glow.width = width);
        let (edit, colour) = colour_row(ui, "Glow colour", style.glow.color);
        self.edit(editor, edit, group, |s| s.glow.color = colour);

        group_caption(ui, "Water lines");
        let (length_edit, spacing_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut style.lines.length,
                    &length("Water line length", MAX_REACH)
                        .prefix("Length")
                        .mixed(group.mixed(|s| s.lines.length)),
                )
            },
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut style.lines.spacing,
                    &length("Water line spacing", MAX_REACH)
                        .prefix("Spacing")
                        .range(MIN_LINE_SPACING, MAX_REACH)
                        .mixed(group.mixed(|s| s.lines.spacing)),
                )
            },
        );
        let (line_length, spacing) = (style.lines.length, style.lines.spacing);
        self.edit(editor, length_edit, group, |s| s.lines.length = line_length);
        self.edit(editor, spacing_edit, group, |s| s.lines.spacing = spacing);
        let (edit, ()) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut style.lines.width,
                    &length("Water line width", 50.0)
                        .prefix("Width")
                        .mixed(group.mixed(|s| s.lines.width)),
                )
            },
            |_| {},
        );
        let width = style.lines.width;
        self.edit(editor, edit, group, |s| s.lines.width = width);
        self.hand_drawn_fields(ui, editor, group, style);
        let (edit, colour) = colour_row(ui, "Water line colour", style.lines.color);
        self.edit(editor, edit, group, |s| s.lines.color = colour);
    }

    /// Taper, variation and seed of the water lines: how hand drawn they look.
    fn hand_drawn_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        group: &Inherit<'_, ShoreStyle>,
        style: &mut ShoreStyle,
    ) {
        let provenance = group.provenance;
        let percent = |label| {
            ScrubSpec::new(label)
                .provenance(provenance)
                .unit("%")
                .range(0.0, 100.0)
                .step(1.0)
                .decimals(0)
        };
        let (mut taper, mut variation) = (style.lines.taper * 100.0, style.lines.variation * 100.0);
        let (taper_edit, variation_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut taper,
                    &percent("Water line taper")
                        .prefix("Taper")
                        .mixed(group.mixed(|s| s.lines.taper)),
                )
            },
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut variation,
                    &percent("Water line variation")
                        .prefix("Variation")
                        .mixed(group.mixed(|s| s.lines.variation)),
                )
            },
        );
        self.edit(editor, taper_edit, group, |s| s.lines.taper = taper / 100.0);
        self.edit(editor, variation_edit, group, |s| {
            s.lines.variation = variation / 100.0;
        });
        let mut seed = f64::from(style.lines.seed);
        let (seed_edit, ()) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut seed,
                    &ScrubSpec::new("Water line seed")
                        .provenance(provenance)
                        .prefix("Seed")
                        .range(0.0, f64::from(u32::MAX))
                        .mixed(group.mixed(|s| f64::from(s.lines.seed))),
                )
            },
            |_| {},
        );
        let seed = seed as u32;
        self.edit(editor, seed_edit, group, |s| s.lines.seed = seed);
    }
}
