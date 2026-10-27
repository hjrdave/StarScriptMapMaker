//! The Label and Stamp sections of the properties panel.

use eframe::egui;
use ssmap_core::{Align, Command, Id, Label, LabelStyles, Landmass, ResolvedLabel, Stamp};
use ssmap_fonts::LABEL_FAMILIES;

use super::{PropertiesPanel, row};
use crate::editor::Editor;
use crate::prop_widgets::{self, Edit, Provenance, ScrubSpec, SectionHeader};

/// What a Label field needs to write its edit back.
struct LabelEdit<'a> {
    id: Id,
    label: &'a Label,
    styles: &'a LabelStyles,
}

fn provenance(overridden: bool) -> Provenance {
    if overridden {
        Provenance::Overridden
    } else {
        Provenance::Inherited
    }
}

impl PropertiesPanel {
    pub(super) fn label_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        locked: bool,
        label: &Label,
    ) {
        let overrides = [
            label.font.is_some(),
            label.size.is_some(),
            label.letter_spacing.is_some(),
            label.line_height.is_some(),
            label.arch.is_some(),
            label.all_caps.is_some(),
            label.fill.is_some(),
            label.opacity.is_some(),
            label.halo.is_some(),
        ]
        .into_iter()
        .filter(|&overridden| overridden)
        .count();
        let badge = format!("{overrides} overridden");
        let header = SectionHeader {
            kind: "label",
            title: "Label",
            badge: (overrides > 0).then_some(badge.as_str()),
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            self.label_fields(ui, editor, id, locked, label, overrides);
        });
    }

    fn label_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        locked: bool,
        label: &Label,
        overrides: usize,
    ) {
        self.label_text_field(ui, editor, id, label);
        let styles = editor.map().label_styles().clone();
        if self.label_style_row(ui, editor, id, label, overrides, &styles) {
            return;
        }

        // Edits are made on the resolved settings; only what changed becomes an override.
        let mut next = label.resolve(&styles);
        let ctx = LabelEdit {
            id,
            label,
            styles: &styles,
        };

        self.label_font_fields(ui, editor, &ctx, &mut next);
        self.label_metric_fields(ui, editor, &ctx, &mut next);
        self.label_look_fields(ui, editor, &ctx, &mut next);

        let mut lock = locked;
        if prop_widgets::checkbox_row(ui, "Locked", &mut lock) {
            let _ = editor.run(Command::SetLocked { id, locked: lock });
        }
    }

    /// The Label's text box. Edits to it are one Undo step when it loses focus.
    fn label_text_field(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, label: &Label) {
        ui.label("Text");
        if self
            .text_edit
            .as_ref()
            .is_none_or(|(editing, _)| *editing != id)
        {
            self.text_edit = Some((id, label.text.clone()));
        }
        let text = &mut self
            .text_edit
            .get_or_insert_with(|| (id, label.text.clone()))
            .1;
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
            if editor
                .run(Command::SetLabel {
                    id,
                    label: Box::new(next),
                })
                .is_err()
            {
                self.text_edit = None;
            }
        } else if !field.has_focus() && *text != label.text {
            // Changed elsewhere (undo, the text box on the Canvas): show the Map's value.
            *text = label.text.clone();
        }
    }

    /// The Label style row: the style picker, resetting overrides and the Manage button. Returns
    /// whether it restyled the Label, in which case the fields below are stale this frame.
    fn label_style_row(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        label: &Label,
        overrides: usize,
        styles: &LabelStyles,
    ) -> bool {
        let mut restyled = None;
        prop_widgets::field_row(ui, "Label style", Provenance::Plain, |ui| {
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
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(overrides > 0, egui::Button::new("Reset overrides"))
                .on_hover_text("Follow the style again")
                .clicked()
            {
                restyled = Some(Label {
                    font: None,
                    size: None,
                    letter_spacing: None,
                    line_height: None,
                    arch: None,
                    all_caps: None,
                    fill: None,
                    opacity: None,
                    halo: None,
                    ..label.clone()
                });
            }
            if ui.button("Manage Label styles…").clicked() {
                self.manage_label_styles = true;
            }
        });
        if let Some(next) = restyled {
            let _ = editor.run(Command::SetLabel {
                id,
                label: Box::new(next),
            });
            return true;
        }
        false
    }

    /// The font family, weight and italic.
    fn label_font_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let label = ctx.label;
        let font = provenance(label.font.is_some());
        let mut edit = Edit::default();
        let reset = prop_widgets::field_row(ui, "Font", font, |ui| {
            egui::ComboBox::from_id_salt("font")
                .selected_text(&*next.font.family)
                .show_ui(ui, |ui| {
                    for family in LABEL_FAMILIES {
                        if ui
                            .selectable_value(&mut next.font.family, (*family).to_owned(), *family)
                            .clicked()
                        {
                            edit = Edit::instant();
                        }
                    }
                });
        });
        edit.reset = reset;
        self.label_apply(editor, ctx, edit, &*next, |l| l.font = None);

        let mut weight = f64::from(next.font.weight);
        let edit = prop_widgets::number_field(
            ui,
            &mut weight,
            &ScrubSpec::new("Weight")
                .range(100.0, 900.0)
                .step(100.0)
                .provenance(font),
        );
        if edit.changed {
            next.font.weight = weight as u16;
        }
        self.label_apply_as("Change Label Weight", editor, ctx, edit, &*next, |l| {
            l.font = None
        });
        if prop_widgets::checkbox_row(ui, "Italic", &mut next.font.italic) {
            self.label_apply(editor, ctx, Edit::instant(), &*next, |l| l.font = None);
        }
    }

    /// The size, letter spacing, line height and arch.
    fn label_metric_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let label = ctx.label;
        let edit = prop_widgets::slider(
            ui,
            &mut next.size,
            &ScrubSpec::new("Size")
                .unit(" u")
                .range(0.1, 100_000.0)
                .slider_max(500.0)
                .logarithmic()
                .step(0.5)
                .decimals(1)
                .provenance(provenance(label.size.is_some())),
        );
        self.label_apply_as("Change Label Size", editor, ctx, edit, &*next, |l| {
            l.size = None
        });

        let edit = prop_widgets::slider(
            ui,
            &mut next.letter_spacing,
            &ScrubSpec::new("Letter spacing")
                .unit(" em")
                .range(-1.0, 5.0)
                .slider_max(1.0)
                .default_value(0.0)
                .step(0.005)
                .decimals(3)
                .provenance(provenance(label.letter_spacing.is_some())),
        );
        self.label_apply_as(
            "Change Label Letter Spacing",
            editor,
            ctx,
            edit,
            &*next,
            |l| l.letter_spacing = None,
        );

        let edit = prop_widgets::slider(
            ui,
            &mut next.line_height,
            &ScrubSpec::new("Line height")
                .unit("×")
                .range(0.1, 10.0)
                .slider_max(3.0)
                .step(0.01)
                .decimals(2)
                .provenance(provenance(label.line_height.is_some())),
        );
        self.label_apply_as("Change Label Line Height", editor, ctx, edit, &*next, |l| {
            l.line_height = None
        });

        let mut percent = next.arch * 100.0;
        let edit = prop_widgets::slider(
            ui,
            &mut percent,
            &ScrubSpec::new("Arch")
                .unit("%")
                .range(-100.0, 100.0)
                .default_value(0.0)
                .decimals(0)
                .provenance(provenance(label.arch.is_some())),
        );
        if edit.changed {
            next.arch = (percent / 100.0).clamp(-1.0, 1.0);
        }
        self.label_apply_as("Change Label Arch", editor, ctx, edit, &*next, |l| {
            l.arch = None
        });
    }

    /// The case, alignment, rotation, colours, opacity and halo.
    fn label_look_fields(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let label = ctx.label;
        let mut edit = Edit::default();
        let reset =
            prop_widgets::field_row(ui, "All caps", provenance(label.all_caps.is_some()), |ui| {
                if prop_widgets::checkbox_row(ui, "On", &mut next.all_caps) {
                    edit = Edit::instant();
                }
            });
        edit.reset = reset;
        self.label_apply(editor, ctx, edit, &*next, |l| l.all_caps = None);

        let mut align = match next.align {
            Align::Left => 0,
            Align::Centre => 1,
            Align::Right => 2,
        };
        if prop_widgets::segmented(ui, &["Left", "Centre", "Right"], &mut align) {
            next.align = [Align::Left, Align::Centre, Align::Right][align];
            self.label_apply(editor, ctx, Edit::instant(), &*next, |_| {});
        }

        let edit = prop_widgets::scrub_field(
            ui,
            &mut next.rotation,
            &ScrubSpec::new("Rotation").unit("°").step(0.5).decimals(1),
        );
        self.label_apply(editor, ctx, edit, &*next, |_| {});

        let mut colour = Some(next.fill);
        let response = prop_widgets::swatch(
            ui,
            "Fill",
            &mut colour,
            false,
            provenance(label.fill.is_some()),
        );
        if response.edit.changed {
            next.fill = colour.unwrap_or(next.fill);
        }
        self.label_apply(editor, ctx, response.edit, &*next, |l| l.fill = None);

        let mut percent = f64::from(next.opacity) * 100.0;
        let edit = prop_widgets::slider(
            ui,
            &mut percent,
            &ScrubSpec::new("Opacity")
                .unit("%")
                .range(0.0, 100.0)
                .provenance(provenance(label.opacity.is_some())),
        );
        if edit.changed {
            next.opacity = (percent / 100.0).clamp(0.0, 1.0) as f32;
        }
        self.label_apply_as("Change Label Opacity", editor, ctx, edit, &*next, |l| {
            l.opacity = None
        });

        let halo = provenance(label.halo.is_some());
        let edit = prop_widgets::slider(
            ui,
            &mut next.halo.width,
            &ScrubSpec::new("Halo")
                .unit(" u")
                .range(0.0, 100_000.0)
                .slider_max(100.0)
                .default_value(0.0)
                .step(0.1)
                .decimals(1)
                .provenance(halo),
        );
        self.label_apply_as("Change Label Halo", editor, ctx, edit, &*next, |l| {
            l.halo = None
        });
        let mut colour = Some(next.halo.color);
        let response = prop_widgets::swatch(ui, "Halo colour", &mut colour, false, halo);
        if response.edit.changed {
            next.halo.color = colour.unwrap_or(next.halo.color);
        }
        self.label_apply(editor, ctx, response.edit, &*next, |l| l.halo = None);
    }

    /// Applies what a Label field did: a reset clears that one override with `clear`, anything
    /// else writes `next`, one Undo step per scrub.
    fn label_apply(
        &mut self,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        edit: Edit,
        next: &ResolvedLabel,
        clear: impl FnOnce(&mut Label),
    ) {
        self.label_apply_as("Edit Label", editor, ctx, edit, next, clear);
    }

    /// [`Self::label_apply`] with the name its Undo step gets.
    fn label_apply_as(
        &mut self,
        name: &str,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        edit: Edit,
        next: &ResolvedLabel,
        clear: impl FnOnce(&mut Label),
    ) {
        if edit.reset {
            let mut cleared = ctx.label.clone();
            clear(&mut cleared);
            let _ = editor.run(Command::SetLabel {
                id: ctx.id,
                label: Box::new(cleared),
            });
            return;
        }
        self.live.apply(editor, edit, name, || Command::SetLabel {
            id: ctx.id,
            label: Box::new(ctx.label.with_resolved(ctx.styles, next)),
        });
    }

    /// Every field of a Stamp. A scrub is one gesture, a click one step.
    pub(super) fn stamp_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
        locked: bool,
        stamp: &Stamp,
    ) {
        let caption = editor.asset_caption(&stamp.asset);
        let header = SectionHeader {
            kind: "stamp",
            title: "Stamp",
            badge: None,
            action: None,
        };
        prop_widgets::section(ui, header, |ui| {
            row(ui, "Asset", |ui| {
                ui.label(&caption);
            });
            let mut next = stamp.clone();

            let spec = |label| ScrubSpec::new(label).step(0.5).decimals(1);
            let edit = prop_widgets::scrub_field(ui, &mut next.position[0], &spec("Position X"));
            self.stamp_apply(editor, id, edit, &next);
            let edit = prop_widgets::scrub_field(ui, &mut next.position[1], &spec("Position Y"));
            self.stamp_apply(editor, id, edit, &next);

            let mut percent = next.scale * 100.0;
            let edit = prop_widgets::scrub_field(
                ui,
                &mut percent,
                &ScrubSpec::new("Scale").unit("%").range(1.0, 10_000.0),
            );
            if edit.changed {
                next.scale = percent / 100.0;
            }
            self.stamp_apply(editor, id, edit, &next);

            let edit = prop_widgets::scrub_field(
                ui,
                &mut next.rotation,
                &ScrubSpec::new("Rotation").unit("°").step(0.5).decimals(1),
            );
            self.stamp_apply(editor, id, edit, &next);

            prop_widgets::field_row(ui, "Flip", Provenance::Plain, |ui| {
                if prop_widgets::checkbox_row(ui, "Horizontal", &mut next.flip) {
                    self.stamp_apply(editor, id, Edit::instant(), &next);
                }
            });

            let mut tint = next.tint;
            let response = prop_widgets::swatch(ui, "Tint", &mut tint, true, Provenance::Plain);
            if response.edit.changed {
                next.tint = tint;
            }
            self.stamp_apply(editor, id, response.edit, &next);

            let mut own = next.fill.is_some();
            prop_widgets::field_row(ui, "Fill", Provenance::Plain, |ui| {
                if prop_widgets::checkbox_row(ui, "Own colour", &mut own) {
                    next.fill = own.then_some(Landmass::DEFAULT_FILL);
                    self.stamp_apply(editor, id, Edit::instant(), &next);
                }
            });
            if let Some(fill) = next.fill {
                let mut colour = Some(fill);
                let response =
                    prop_widgets::swatch(ui, "Fill colour", &mut colour, false, Provenance::Plain);
                if response.edit.changed {
                    next.fill = colour;
                }
                self.stamp_apply(editor, id, response.edit, &next);
            }

            let mut percent = f64::from(next.opacity) * 100.0;
            let edit = prop_widgets::slider(
                ui,
                &mut percent,
                &ScrubSpec::new("Opacity").unit("%").range(0.0, 100.0),
            );
            if edit.changed {
                next.opacity = (percent / 100.0).clamp(0.0, 1.0) as f32;
            }
            self.stamp_apply(editor, id, edit, &next);

            let mut lock = locked;
            if prop_widgets::checkbox_row(ui, "Locked", &mut lock) {
                let _ = editor.run(Command::SetLocked { id, locked: lock });
            }

            if ui
                .add_enabled(editor.can_shuffle(), egui::Button::new("Shuffle variant"))
                .on_hover_text("Re-roll this Stamp within its Variant group")
                .clicked()
            {
                editor.shuffle_variants();
            }
        });
    }

    fn stamp_apply(&mut self, editor: &mut Editor, id: Id, edit: Edit, next: &Stamp) {
        self.live
            .apply(editor, edit, "Edit Stamp", || Command::SetStamp {
                id,
                stamp: Box::new(next.clone()),
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::{Canvas, DistanceUnit, Map, Scale};

    fn editor_with_layer() -> (Editor, Id) {
        let mut editor = Editor::new(Map::new(Canvas::new(
            100,
            100,
            Scale::new(1.0, DistanceUnit::Miles),
        )));
        let layer = editor.new_layer().expect("a Layer");
        (editor, layer)
    }

    fn add_label(editor: &mut Editor, layer: Id, mut label: Label) -> Id {
        label.text = "Hello".into();
        let id = Id::new();
        assert!(
            editor
                .run(Command::CreateLabel {
                    id,
                    parent: layer,
                    index: None,
                    label: Box::new(label),
                })
                .is_ok()
        );
        id
    }

    #[test]
    fn provenance_is_overridden_only_where_the_label_sets_the_field() {
        assert_eq!(provenance(true), Provenance::Overridden);
        assert_eq!(provenance(false), Provenance::Inherited);
    }

    #[test]
    fn a_label_edit_writes_the_resolved_value_as_an_override() {
        let (mut editor, layer) = editor_with_layer();
        let id = add_label(&mut editor, layer, Label::default());
        let styles = editor.map().label_styles().clone();
        let label = editor.label(id).expect("a Label").clone();
        let mut next = label.resolve(&styles);
        next.size += 10.0;
        let ctx = LabelEdit {
            id,
            label: &label,
            styles: &styles,
        };
        let mut panel = PropertiesPanel::default();
        panel.label_apply(&mut editor, &ctx, Edit::instant(), &next, |l| l.size = None);
        assert_eq!(editor.label(id).expect("a Label").size, Some(next.size));
        assert_eq!(editor.undo_label(), Some("Edit Label"));
    }

    #[test]
    fn a_label_reset_clears_the_override_in_one_undo_step() {
        let (mut editor, layer) = editor_with_layer();
        let label = Label {
            size: Some(40.0),
            ..Label::default()
        };
        let id = add_label(&mut editor, layer, label);
        let label = editor.label(id).expect("a Label").clone();
        let styles = editor.map().label_styles().clone();
        let next = label.resolve(&styles);
        let ctx = LabelEdit {
            id,
            label: &label,
            styles: &styles,
        };
        let edit = Edit {
            reset: true,
            ..Edit::default()
        };
        let mut panel = PropertiesPanel::default();
        panel.label_apply(&mut editor, &ctx, edit, &next, |l| l.size = None);
        assert_eq!(editor.label(id).expect("a Label").size, None);
        assert_eq!(editor.undo_label(), Some("Edit Label"));
        editor.undo();
        assert_eq!(editor.label(id).expect("a Label").size, Some(40.0));
    }

    #[test]
    fn a_stamp_edit_is_one_named_undo_step() {
        let (mut editor, layer) = editor_with_layer();
        let id = Id::new();
        let stamp = Stamp::new("pack/asset".parse().expect("an Asset ref"), [10.0, 10.0]);
        assert!(
            editor
                .run(Command::CreateStamp {
                    id,
                    parent: layer,
                    index: None,
                    stamp: Box::new(stamp.clone()),
                })
                .is_ok()
        );
        let next = Stamp {
            scale: 2.0,
            ..stamp
        };
        let mut panel = PropertiesPanel::default();
        panel.stamp_apply(&mut editor, id, Edit::instant(), &next);
        assert_eq!(editor.stamp(id).expect("a Stamp").scale, 2.0);
        assert_eq!(editor.undo_label(), Some("Edit Stamp"));
    }
}
