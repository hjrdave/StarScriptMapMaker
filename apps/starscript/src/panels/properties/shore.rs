//! The Shore style fields, for Landmasses and Lakes and for the Map defaults (issue 651): the
//! preset, then the Details in groups. Fields sit two to a row with short names in their boxes,
//! and each colour has its opacity beside it. A group that is off (its width, count, length or
//! density is 0) shrinks to one line with a box to turn it on. On a Landmass or Lake, only the
//! fields that override the Map default show it, with a reset arrow; the others show nothing.

use eframe::egui;
use ssmap_core::{
    Color, Command, MAX_REACH, MAX_RIPPLES, MAX_WAVE_DENSITY, MAX_WAVE_SIZE, MIN_LINE_SPACING,
    Node, ShoreKind, ShoreStyle,
};

use super::object_style::Frame;
use super::{Defaults, Inherit, PropertiesPanel};
use crate::editor::Editor;
use crate::prop_widgets::{self, Edit, ScrubSpec, SectionHeader};
use crate::theme;

/// A number in a Shore style: its accessible name, the short name painted in its box, how it
/// is read and written, and how it is shown.
struct Number {
    label: &'static str,
    short: &'static str,
    get: fn(&ShoreStyle) -> f64,
    set: fn(&mut ShoreStyle, f64),
    /// The shown value is the stored one times this (100 for a fraction shown as a percentage).
    scale: f64,
    unit: &'static str,
    /// The least stored value.
    min: f64,
    /// The most shown value.
    max: f64,
    step: f64,
    decimals: usize,
}

impl Number {
    /// A length in Canvas units, from 0 to `max`.
    fn length(
        label: &'static str,
        short: &'static str,
        get: fn(&ShoreStyle) -> f64,
        set: fn(&mut ShoreStyle, f64),
        max: f64,
    ) -> Self {
        Self {
            label,
            short,
            get,
            set,
            scale: 1.0,
            unit: " u",
            min: 0.0,
            max,
            step: 0.1,
            decimals: 1,
        }
    }

    /// A whole number from 0 to `max`: a count or a seed.
    fn whole(
        label: &'static str,
        short: &'static str,
        get: fn(&ShoreStyle) -> f64,
        set: fn(&mut ShoreStyle, f64),
        max: f64,
    ) -> Self {
        Self {
            unit: "",
            step: 1.0,
            decimals: 0,
            ..Self::length(label, short, get, set, max)
        }
    }

    /// A fraction from 0 to 1, shown as a percentage.
    fn percent(
        label: &'static str,
        short: &'static str,
        get: fn(&ShoreStyle) -> f64,
        set: fn(&mut ShoreStyle, f64),
    ) -> Self {
        Self {
            scale: 100.0,
            unit: "%",
            max: 100.0,
            step: 1.0,
            decimals: 0,
            ..Self::length(label, short, get, set, 1.0)
        }
    }

    /// The alpha of a colour (0 to 255), shown as an opacity percentage.
    fn opacity(
        label: &'static str,
        get: fn(&ShoreStyle) -> f64,
        set: fn(&mut ShoreStyle, f64),
    ) -> Self {
        Self {
            scale: 100.0 / 255.0,
            ..Self::percent(label, "Opacity", get, set)
        }
    }

    fn min(self, min: f64) -> Self {
        Self { min, ..self }
    }
}

/// An alpha from a value the opacity field gave.
fn alpha(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// A colour in a Shore style: its accessible name and how it is read and written.
struct Colour {
    label: &'static str,
    get: fn(&ShoreStyle) -> Color,
    set: fn(&mut ShoreStyle, Color),
}

/// One group of the Details: its rows of numbers, then its colour with the opacity. The first
/// number is the one that turns the group off at 0.
struct Group {
    name: &'static str,
    rows: Vec<(Number, Option<Number>)>,
    colour: Colour,
    opacity: Number,
    /// What the first number becomes when the group is turned on with nothing better to take
    /// it from.
    on: f64,
}

/// The groups of the Details, in order.
fn groups() -> [Group; 6] {
    [
        outline(),
        ripples(),
        inner_shade(),
        outer_glow(),
        water_lines(),
        wave_marks(),
    ]
}

fn outline() -> Group {
    Group {
        name: "Outline",
        rows: vec![(
            Number::length(
                "Outline width",
                "W",
                |s| s.outline.width,
                |s, v| s.outline.width = v,
                100.0,
            ),
            None,
        )],
        colour: Colour {
            label: "Outline colour",
            get: |s| s.outline.color,
            set: |s, c| s.outline.color = c,
        },
        opacity: Number::opacity(
            "Outline opacity",
            |s| f64::from(s.outline.color.a),
            |s, v| s.outline.color.a = alpha(v),
        ),
        on: 1.5,
    }
}

fn ripples() -> Group {
    Group {
        name: "Ripples",
        rows: vec![
            (
                Number::whole(
                    "Ripple count",
                    "Count",
                    |s| f64::from(s.ripples.count),
                    |s, v| s.ripples.count = v as u32,
                    f64::from(MAX_RIPPLES),
                ),
                Some(Number::length(
                    "Ripple spacing",
                    "Gap",
                    |s| s.ripples.spacing,
                    |s, v| s.ripples.spacing = v,
                    500.0,
                )),
            ),
            (
                Number::length(
                    "Ripple line width",
                    "W",
                    |s| s.ripples.width,
                    |s, v| s.ripples.width = v,
                    50.0,
                ),
                Some(Number {
                    unit: "",
                    step: 0.01,
                    decimals: 2,
                    ..Number::length(
                        "Ripple fade",
                        "Fade",
                        |s| s.ripples.fade,
                        |s, v| s.ripples.fade = v,
                        1.0,
                    )
                }),
            ),
        ],
        colour: Colour {
            label: "Ripple line colour",
            get: |s| s.ripples.color,
            set: |s, c| s.ripples.color = c,
        },
        opacity: Number::opacity(
            "Ripple line opacity",
            |s| f64::from(s.ripples.color.a),
            |s, v| s.ripples.color.a = alpha(v),
        ),
        on: 3.0,
    }
}

fn inner_shade() -> Group {
    Group {
        name: "Inner shade",
        rows: vec![(
            Number::length(
                "Shade width",
                "W",
                |s| s.shading.width,
                |s, v| s.shading.width = v,
                500.0,
            ),
            None,
        )],
        colour: Colour {
            label: "Shade colour",
            get: |s| s.shading.color,
            set: |s, c| s.shading.color = c,
        },
        opacity: Number::opacity(
            "Shade opacity",
            |s| f64::from(s.shading.color.a),
            |s, v| s.shading.color.a = alpha(v),
        ),
        on: 18.0,
    }
}

fn outer_glow() -> Group {
    Group {
        name: "Outer glow",
        rows: vec![(
            Number::length(
                "Glow width",
                "W",
                |s| s.glow.width,
                |s, v| s.glow.width = v,
                MAX_REACH,
            ),
            None,
        )],
        colour: Colour {
            label: "Glow colour",
            get: |s| s.glow.color,
            set: |s, c| s.glow.color = c,
        },
        opacity: Number::opacity(
            "Glow opacity",
            |s| f64::from(s.glow.color.a),
            |s, v| s.glow.color.a = alpha(v),
        ),
        on: 26.0,
    }
}

fn water_lines() -> Group {
    Group {
        name: "Water lines",
        rows: vec![
            (
                Number::length(
                    "Water line length",
                    "Length",
                    |s| s.lines.length,
                    |s, v| s.lines.length = v,
                    MAX_REACH,
                ),
                Some(
                    Number::length(
                        "Water line spacing",
                        "Gap",
                        |s| s.lines.spacing,
                        |s, v| s.lines.spacing = v,
                        MAX_REACH,
                    )
                    .min(MIN_LINE_SPACING),
                ),
            ),
            (
                Number::length(
                    "Water line width",
                    "W",
                    |s| s.lines.width,
                    |s, v| s.lines.width = v,
                    50.0,
                ),
                Some(Number::percent(
                    "Water line taper",
                    "Taper",
                    |s| s.lines.taper,
                    |s, v| s.lines.taper = v,
                )),
            ),
            (
                Number::percent(
                    "Water line variation",
                    "Vary",
                    |s| s.lines.variation,
                    |s, v| s.lines.variation = v,
                ),
                Some(Number::whole(
                    "Water line seed",
                    "Seed",
                    |s| f64::from(s.lines.seed),
                    |s, v| s.lines.seed = v as u32,
                    f64::from(u32::MAX),
                )),
            ),
        ],
        colour: Colour {
            label: "Water line colour",
            get: |s| s.lines.color,
            set: |s, c| s.lines.color = c,
        },
        opacity: Number::opacity(
            "Water line opacity",
            |s| f64::from(s.lines.color.a),
            |s, v| s.lines.color.a = alpha(v),
        ),
        on: 36.0,
    }
}

fn wave_marks() -> Group {
    Group {
        name: "Wave marks",
        rows: vec![
            (
                Number {
                    unit: "",
                    ..Number::length(
                        "Wave mark density",
                        "Density",
                        |s| s.waves.density,
                        |s, v| s.waves.density = v,
                        MAX_WAVE_DENSITY,
                    )
                },
                Some(
                    Number::length(
                        "Wave mark size",
                        "Size",
                        |s| s.waves.size,
                        |s, v| s.waves.size = v,
                        MAX_WAVE_SIZE,
                    )
                    .min(1.0),
                ),
            ),
            (
                Number::whole(
                    "Wave mark seed",
                    "Seed",
                    |s| f64::from(s.waves.seed),
                    |s, v| s.waves.seed = v as u32,
                    f64::from(u32::MAX),
                ),
                None,
            ),
        ],
        colour: Colour {
            label: "Wave mark colour",
            get: |s| s.waves.color,
            set: |s, c| s.waves.color = c,
        },
        opacity: Number::opacity(
            "Wave mark opacity",
            |s| f64::from(s.waves.color.a),
            |s, v| s.waves.color.a = alpha(v),
        ),
        on: 2.0,
    }
}

impl PropertiesPanel {
    /// The Shore style section of a Landmass or Lake: its own settings, or the Map default.
    pub(super) fn object_shore(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ids: &[ssmap_core::Id],
        frame: Frame,
    ) {
        let mut shored = Vec::new();
        let mut shore_kind = ShoreKind::Landmass;
        let mut members = Vec::new();
        let mut defaults = Vec::new();
        let mut any_own = false;
        for &id in ids {
            let Some(Node::Object(object)) = editor.map().node(id) else {
                continue;
            };
            let Some(kind) = object.kind.shore_kind() else {
                continue;
            };
            if shored.is_empty() {
                shore_kind = kind;
            }
            shored.push(id);
            let default = editor.map().defaults().shore_for(kind).clone();
            members.push(match object.kind.shore() {
                Some(own) => {
                    any_own = true;
                    own.clone()
                }
                None => default.clone(),
            });
            defaults.push(default);
        }
        if shored.is_empty() {
            return;
        }
        let set = |i: usize, shore| Command::SetShore {
            ids: vec![shored[i]],
            shore: Some(shore),
        };
        let clear = |i: usize| Command::SetShore {
            ids: vec![shored[i]],
            shore: None,
        };
        let group = Inherit {
            members: &members,
            provenance: prop_widgets::Provenance::Plain,
            label: "Change Shore Style",
            set: &set,
            reset: None,
            defaults: Some(Defaults {
                values: &defaults,
                clear: &clear,
            }),
        };
        let header = SectionHeader {
            kind: "shore",
            title: "Shore style",
            badge: any_own.then_some("overridden"),
            action: None,
        };
        let mut body = |ui: &mut egui::Ui| {
            ui.push_id(("shore", &shored), |ui| {
                self.shore_fields(ui, editor, shore_kind, &group);
            });
        };
        match frame {
            Frame::Section => {
                prop_widgets::section(ui, header, body);
            }
            Frame::Tab => body(ui),
        }
    }

    /// The preset, then the Details heading with "Reset to preset" while a Landmass or Lake was
    /// changed since its last pick, then the groups.
    pub(super) fn shore_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        kind: ShoreKind,
        group: &Inherit<'_, ShoreStyle>,
    ) {
        self.shore_preset_picker(ui, editor, kind, group);
        // Only a Landmass or Lake remembers the preset it was given.
        let changed = group.defaults.is_some()
            && group
                .members
                .iter()
                .any(|s| s.changed_from_preset(kind).is_some());
        let mut back = false;
        prop_widgets::divider_section(
            ui,
            "Details",
            |ui| {
                if changed {
                    let link = egui::RichText::new("Reset to preset").size(theme::CAPTION);
                    back = ui
                        .link(link)
                        .on_hover_text("Go back to the preset last picked")
                        .clicked();
                }
            },
            |ui| {
                for each in groups() {
                    self.shore_group(ui, editor, kind, group, &each);
                }
            },
        );
        if back {
            let commands = group
                .members
                .iter()
                .enumerate()
                .filter_map(|(i, s)| s.changed_from_preset(kind).map(|p| (group.set)(i, p)))
                .collect();
            self.live
                .apply_all(editor, Edit::instant(), group.label, || commands);
        }
    }

    /// One group: the box that turns it on or off and, while on, its fields.
    fn shore_group(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        kind: ShoreKind,
        group: &Inherit<'_, ShoreStyle>,
        each: &Group,
    ) {
        let switch = &each.rows[0].0;
        let was_on = (switch.get)(group.current()) > 0.0;
        let mut on = was_on;
        ui.add_space(theme::SPACING[2]);
        let palette = theme::palette(ui.ctx());
        let name = egui::RichText::new(each.name)
            .size(theme::CAPTION)
            .color(palette.text_secondary);
        if ui.checkbox(&mut on, name).changed() {
            let value = if on {
                on_value(kind, group, switch, each.on)
            } else {
                0.0
            };
            self.live
                .apply_all(editor, Edit::instant(), group.label, || {
                    group.commands(|s| (switch.set)(s, value))
                });
        }
        if !was_on {
            return;
        }
        ui.add_space(theme::SPACING[1]);
        for (left, right) in &each.rows {
            let (a, b) = prop_widgets::pair(
                ui,
                |ui| number_field(ui, group, left),
                |ui| right.as_ref().map(|n| number_field(ui, group, n)),
            );
            self.apply_number(editor, group, left, a);
            if let (Some(n), Some(b)) = (right, b) {
                self.apply_number(editor, group, n, b);
            }
        }
        let ((colour_edit, colour), opacity) = prop_widgets::pair(
            ui,
            |ui| {
                let shown = (each.colour.get)(group.current());
                let mut value = Some(shown);
                let provenance = group.provenance_of(each.colour.get);
                let edit = prop_widgets::swatch_beside_opacity(
                    ui,
                    each.colour.label,
                    &mut value,
                    provenance,
                )
                .edit;
                (edit, value.unwrap_or(shown))
            },
            |ui| number_field(ui, group, &each.opacity),
        );
        let field = &each.colour;
        if colour_edit.reset {
            self.reset_shore(editor, group, |s, d| (field.set)(s, (field.get)(d)));
        } else {
            self.live.apply_all(editor, colour_edit, group.label, || {
                group.commands(|s| (field.set)(s, colour))
            });
        }
        self.apply_number(editor, group, &each.opacity, opacity);
    }

    /// Writes what a number field did: its value on every member, or with its reset arrow,
    /// the default's value.
    fn apply_number(
        &mut self,
        editor: &mut Editor,
        group: &Inherit<'_, ShoreStyle>,
        number: &Number,
        (edit, shown): (Edit, f64),
    ) {
        if edit.reset {
            self.reset_shore(editor, group, |s, d| (number.set)(s, (number.get)(d)));
            return;
        }
        let value = shown / number.scale;
        self.live.apply_all(editor, edit, group.label, || {
            group.commands(|s| (number.set)(s, value))
        });
    }

    /// Gives one field of each member its default's value, as one Undo step. A member left
    /// drawing the same as its default follows the default again.
    fn reset_shore(
        &mut self,
        editor: &mut Editor,
        group: &Inherit<'_, ShoreStyle>,
        copy: impl Fn(&mut ShoreStyle, &ShoreStyle),
    ) {
        let Some(defaults) = &group.defaults else {
            return;
        };
        let commands = group
            .members
            .iter()
            .zip(defaults.values)
            .enumerate()
            .filter_map(|(i, (member, default))| {
                let mut next = member.clone();
                copy(&mut next, default);
                if next == *member {
                    None
                } else if next.same_look(default) {
                    Some((defaults.clear)(i))
                } else {
                    Some((group.set)(i, next))
                }
            })
            .collect();
        self.live
            .apply_all(editor, Edit::instant(), group.label, || commands);
    }

    /// A dropdown of the named Shore presets for `kind`. It shows the preset every selected
    /// style matches, or "Custom". Choosing one sets the whole Shore style (an override, on a
    /// Landmass or Lake, which remembers the name); the fields below stay editable.
    fn shore_preset_picker(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        kind: ShoreKind,
        group: &Inherit<'_, ShoreStyle>,
    ) {
        let mut names = group.members.iter().map(|s| s.preset_name(kind));
        let shown = names
            .next()
            .flatten()
            .filter(|first| names.all(|n| n == Some(*first)))
            .unwrap_or("Custom");
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.label("Preset");
            egui::ComboBox::from_id_salt("shore preset")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (name, style) in ShoreStyle::presets(kind) {
                        if ui.selectable_label(shown == name, name).clicked() {
                            picked = Some(if group.defaults.is_some() {
                                ShoreStyle {
                                    preset: Some(name.to_owned()),
                                    ..style
                                }
                            } else {
                                style
                            });
                        }
                    }
                });
        });
        if let Some(style) = picked {
            self.live
                .apply_all(editor, Edit::instant(), group.label, || {
                    group.commands(|s| *s = style.clone())
                });
        }
    }
}

/// A number's field in a half-width column, with its short name in the box. Returns what it
/// did and the value it shows.
fn number_field(
    ui: &mut egui::Ui,
    group: &Inherit<'_, ShoreStyle>,
    number: &Number,
) -> (Edit, f64) {
    let mut shown = (number.get)(group.current()) * number.scale;
    let spec = ScrubSpec::new(number.label)
        .prefix(number.short)
        .unit(number.unit)
        .range(number.min * number.scale, number.max)
        .step(number.step)
        .decimals(number.decimals)
        .provenance(group.provenance_of(number.get))
        .mixed(group.mixed(number.get));
    (prop_widgets::scrub_field(ui, &mut shown, &spec), shown)
}

/// What a group's first number becomes when it is turned on: the value in the preset last
/// picked, else the default's, else the first preset's that has it on, else `fallback`.
fn on_value(
    kind: ShoreKind,
    group: &Inherit<'_, ShoreStyle>,
    number: &Number,
    fallback: f64,
) -> f64 {
    let picked = group
        .current()
        .preset
        .as_deref()
        .and_then(|name| ShoreStyle::picked(kind, name));
    let default = group
        .defaults
        .as_ref()
        .and_then(|d| d.values.first())
        .cloned();
    let presets = ShoreStyle::presets(kind)
        .into_iter()
        .chain(ShoreStyle::presets(ShoreKind::Landmass))
        .map(|(_, style)| style);
    picked
        .into_iter()
        .chain(default)
        .chain(presets)
        .map(|style| (number.get)(&style))
        .find(|&value| value > 0.0)
        .unwrap_or(fallback)
}
