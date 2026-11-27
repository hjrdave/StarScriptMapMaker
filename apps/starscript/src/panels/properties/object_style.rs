//! The Fill style and Roughness sections, for Landmasses and Lakes and for the Map defaults.
//! The Shore style is in `shore`.

use eframe::egui;
use ssmap_core::{
    AssetRef, Command, FillStyle, FillTexture, Grain, Id, MAX_GRAIN_SIZE, MAX_MOTTLE_SCALE,
    MAX_TEXTURE_SCALE, MIN_GRAIN_SIZE, MIN_MOTTLE_SCALE, MIN_TEXTURE_SCALE, Mottle, Node,
    ObjectKind, Roughness,
};

use super::{Inherit, PropertiesPanel};
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

/// How a group of settings is framed: as a section of its own, or on a tab of the Landmass
/// and Lake panels.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Frame {
    /// A section with its heading and the "overridden" badge (a multi-selection).
    Section,
    /// On a tab: Roughness under a divider heading, Shore style with no heading of its own,
    /// since the tab already names it.
    Tab,
}

/// The finishes of a Landmass's fill, in the order of the switch.
const FINISHES: [&str; 3] = ["Flat", "Grain", "Mottled"];

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

    /// How a Landmass's fill is drawn: a Flat | Grain | Mottled switch, the settings of the
    /// finish picked, then the texture. Picking a finish turns the other off. A style with both
    /// (made before the switch) shows Mottled and the settings of both. Each scrub of a value
    /// is one Undo step. A Landmass with an override of its own also gets a button that returns
    /// it to the Map default.
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
        let mut finish = match (&style.grain, &style.mottle) {
            (_, Some(_)) => 2,
            (Some(_), None) => 1,
            (None, None) => 0,
        };
        if prop_widgets::segmented(ui, &FINISHES, &mut finish) {
            // The finish picked keeps its settings if it already had them.
            let next = FillStyle {
                grain: (finish == 1).then(|| style.grain.unwrap_or(Grain::DEFAULT)),
                mottle: (finish == 2).then(|| style.mottle.unwrap_or(Mottle::DEFAULT)),
                ..style.clone()
            };
            self.live
                .apply(editor, Edit::instant(), "Change Fill Style", || {
                    target.set(next)
                });
        }
        if let Some(grain) = style.grain {
            self.grain_fields(ui, editor, target, style.clone(), grain);
        }
        if let Some(mottle) = style.mottle {
            self.mottle_fields(ui, editor, target, style.clone(), mottle);
        }
        self.texture_fields(ui, editor, target, style);
    }

    /// The texture picker (None, or a texture Asset of an installed pack) and, with one
    /// picked, its Amount and Scale.
    fn texture_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        target: FillTarget,
        style: FillStyle,
    ) {
        let textures: Vec<(AssetRef, String)> = editor
            .sprites()
            .map(|sprites| {
                let library = sprites.library();
                library
                    .packs()
                    .iter()
                    .flat_map(|pack| {
                        pack.textures.iter().filter_map(|texture| {
                            let reference = AssetRef::new(&pack.id, &texture.id).ok()?;
                            Some((reference, texture.name.clone()))
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let current = style.texture.clone();
        if textures.is_empty() && current.is_none() {
            return;
        }
        let selected = current.as_ref().map_or_else(
            || "None".to_owned(),
            |t| {
                textures
                    .iter()
                    .find(|(reference, _)| *reference == t.asset)
                    .map_or_else(|| t.asset.to_string(), |(_, name)| name.clone())
            },
        );
        let mut picked: Option<Option<AssetRef>> = None;
        ui.horizontal(|ui| {
            ui.label("Texture");
            egui::ComboBox::from_id_salt("fill texture")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "None").clicked() {
                        picked = Some(None);
                    }
                    for (reference, name) in &textures {
                        let on = current.as_ref().is_some_and(|t| t.asset == *reference);
                        if ui.selectable_label(on, name).clicked() {
                            picked = Some(Some(reference.clone()));
                        }
                    }
                });
        });
        if let Some(pick) = picked {
            // Changing the texture keeps the Amount and Scale already set.
            let texture = pick.map(|asset| match &current {
                Some(t) => FillTexture { asset, ..t.clone() },
                None => FillTexture::new(asset),
            });
            let next = FillStyle {
                texture,
                ..style.clone()
            };
            self.live
                .apply(editor, Edit::instant(), "Change Fill Style", || {
                    target.set(next)
                });
        }
        let Some(texture) = current else {
            return;
        };
        // The amount is shown as a percentage and stored as a fraction.
        let (mut amount, mut scale) = (texture.amount * 100.0, texture.scale);
        let (amount_edit, scale_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::scrub_field(
                    ui,
                    &mut amount,
                    &ScrubSpec::new("Texture amount")
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
                    &mut scale,
                    &ScrubSpec::new("Texture scale")
                        .prefix("Scale")
                        .unit(" u")
                        .range(MIN_TEXTURE_SCALE, MAX_TEXTURE_SCALE)
                        .logarithmic()
                        .step(5.0)
                        .decimals(0),
                )
            },
        );
        let textured = |amount: f64, scale: f64| {
            target.set(FillStyle {
                texture: Some(FillTexture {
                    amount: amount / 100.0,
                    scale,
                    ..texture.clone()
                }),
                ..style.clone()
            })
        };
        self.live.apply(editor, amount_edit, "Change Texture", || {
            textured(amount, scale)
        });
        self.live.apply(editor, scale_edit, "Change Texture", || {
            textured(amount, scale)
        });
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
                ..style.clone()
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
        let with = |mottle: Mottle| {
            target.set(FillStyle {
                mottle: Some(mottle),
                ..style.clone()
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
    pub(super) fn object_roughness(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ids: &[Id],
        frame: Frame,
    ) {
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
            defaults: None,
        };
        let header = SectionHeader {
            kind: "roughness",
            title: "Roughness",
            badge: any_own.then_some("overridden"),
            action: None,
        };
        let body = |ui: &mut egui::Ui| {
            ui.push_id(("roughness", &ids), |ui| {
                self.roughness_fields(ui, editor, &group);
            });
        };
        match frame {
            Frame::Section => {
                prop_widgets::section(ui, header, body);
            }
            Frame::Tab => prop_widgets::divider_section(ui, header.title, |_| {}, body),
        }
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
}
