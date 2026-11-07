//! The Label section of the properties panel.

use eframe::egui;
use ssmap_core::{Align, Command, Id, Label, LabelStyles, ResolvedLabel};
use ssmap_fonts::LABEL_FAMILIES;

use super::PropertiesPanel;
use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::prop_widgets::{self, Edit, Handle, Provenance, ScrubSpec};

/// What a Label field needs to write its edit back.
struct LabelEdit<'a> {
    id: Id,
    label: &'a Label,
    styles: &'a LabelStyles,
}

/// A Label field is marked only when this Label overrides its style: a gold dot after the name
/// and a reset arrow. A value taken from the style carries no mark (no link glyph).
fn provenance(overridden: bool) -> Provenance {
    if overridden {
        Provenance::Overridden
    } else {
        Provenance::Plain
    }
}

impl PropertiesPanel {
    pub(super) fn label_properties(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Id,
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
        let mut restyled = None;
        let mut reset = false;
        prop_widgets::divider_section(
            ui,
            "Text",
            |_| {},
            |ui| self.label_text_field(ui, editor, id, label),
        );
        let styles = editor.map().label_styles().clone();
        // Edits are made on the resolved settings; only what changed becomes an override.
        let mut next = label.resolve(&styles);
        let ctx = LabelEdit {
            id,
            label,
            styles: &styles,
        };
        prop_widgets::divider_section(
            ui,
            "Typography",
            |ui| {
                if overrides > 0 {
                    reset = icons::icon_button(ui, Icon::Reset, "Reset overrides").clicked();
                }
            },
            |ui| {
                if self.label_style_row(ui, label, &styles, &mut restyled) {
                    return;
                }
                self.label_typography(ui, editor, &ctx, &mut next);
            },
        );
        if restyled.is_none() && !reset {
            prop_widgets::divider_section(
                ui,
                "Fill",
                |_| {},
                |ui| self.label_fill(ui, editor, &ctx, &mut next),
            );
            prop_widgets::divider_section(
                ui,
                "Halo",
                |_| {},
                |ui| self.label_halo(ui, editor, &ctx, &mut next),
            );
            prop_widgets::divider_section(
                ui,
                "Appearance",
                |_| {},
                |ui| self.label_appearance(ui, editor, &ctx, &mut next),
            );
        }
        if reset {
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
        if let Some(next) = restyled {
            let _ = editor.run(Command::SetLabel {
                id,
                label: Box::new(next),
            });
        }
    }

    /// The Label's text box. Edits to it are one Undo step when it loses focus.
    fn label_text_field(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Id, label: &Label) {
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

    /// The Label style row: the style picker and the way to Manage Label styles. Returns
    /// whether it restyled the Label, in which case the fields below are stale this frame.
    fn label_style_row(
        &mut self,
        ui: &mut egui::Ui,
        label: &Label,
        styles: &LabelStyles,
        restyled: &mut Option<Label>,
    ) -> bool {
        prop_widgets::named_above(ui, "Style", Provenance::Plain, |ui| {
            let current = styles
                .get(label.style)
                .unwrap_or_else(|| styles.default_style());
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("style")
                    .width((ui.available_width() - 28.0).max(40.0))
                    .selected_text(&current.name)
                    .show_ui(ui, |ui| {
                        for style in styles.iter() {
                            if ui
                                .selectable_label(style.id == current.id, &style.name)
                                .clicked()
                            {
                                *restyled = Some(Label {
                                    style: style.id,
                                    ..label.clone()
                                });
                            }
                        }
                    });
                if icons::icon_button(ui, Icon::More, "Manage Label styles…").clicked() {
                    self.manage_label_styles = true;
                }
            });
        });
        restyled.is_some()
    }

    /// The font family, weight and size, line height and letter spacing, arch and format, and align.
    fn label_typography(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        self.label_font_family(ui, editor, ctx, next);
        self.label_weight_size(ui, editor, ctx, next);
        self.label_spacing(ui, editor, ctx, next);
        self.label_arch_format(ui, editor, ctx, next);
        self.label_align(ui, editor, ctx, next);
    }

    /// The font family, full width.
    fn label_font_family(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let label = ctx.label;
        let font = provenance(label.font.is_some());
        let mut edit = Edit::default();
        let reset = prop_widgets::named_above(ui, "Font", font, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("font")
                    .width(ui.available_width() - 28.0)
                    .selected_text(&*next.font.family)
                    .show_ui(ui, |ui| {
                        for family in LABEL_FAMILIES {
                            if ui
                                .selectable_value(
                                    &mut next.font.family,
                                    (*family).to_owned(),
                                    *family,
                                )
                                .clicked()
                            {
                                edit = Edit::instant();
                            }
                        }
                    });
                font == Provenance::Overridden
                    && icons::icon_button(ui, Icon::Reset, "Reset Font").clicked()
            })
            .inner
        });
        edit.reset = reset;
        self.label_apply(editor, ctx, edit, &*next, |l| l.font = None);
    }

    /// Weight and size on one row, each name above its field and a handle inside it.
    fn label_weight_size(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let label = ctx.label;
        let font = provenance(label.font.is_some());
        let mut weight = f64::from(next.font.weight);
        let (weight_edit, size_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::named_above(ui, "Weight", font, |ui| {
                    prop_widgets::scrub_field(
                        ui,
                        &mut weight,
                        &ScrubSpec::new("Weight")
                            .handle(Handle::Text("W"))
                            .range(100.0, 900.0)
                            .step(100.0)
                            .provenance(font),
                    )
                })
            },
            |ui| {
                let size = provenance(label.size.is_some());
                prop_widgets::named_above(ui, "Size", size, |ui| {
                    prop_widgets::scrub_field(
                        ui,
                        &mut next.size,
                        &ScrubSpec::new("Size")
                            .handle(Handle::Text("S"))
                            .unit(" u")
                            .range(0.1, 100_000.0)
                            .step(0.5)
                            .decimals(1)
                            .provenance(size),
                    )
                })
            },
        );
        if weight_edit.changed {
            next.font.weight = weight as u16;
        }
        self.label_apply_as(
            "Change Label Weight",
            editor,
            ctx,
            weight_edit,
            &*next,
            |l| l.font = None,
        );
        self.label_apply_as("Change Label Size", editor, ctx, size_edit, &*next, |l| {
            l.size = None
        });
    }

    /// Line height and letter spacing on one row.
    fn label_spacing(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let label = ctx.label;
        let (height_edit, spacing_edit) = prop_widgets::pair(
            ui,
            |ui| {
                let line = provenance(label.line_height.is_some());
                prop_widgets::named_above(ui, "Line height", line, |ui| {
                    prop_widgets::scrub_field(
                        ui,
                        &mut next.line_height,
                        &ScrubSpec::new("Line height")
                            .handle(Handle::Text("↕"))
                            .unit("×")
                            .range(0.1, 10.0)
                            .step(0.01)
                            .decimals(2)
                            .provenance(line),
                    )
                })
            },
            |ui| {
                let spacing = provenance(label.letter_spacing.is_some());
                prop_widgets::named_above(ui, "Letter spacing", spacing, |ui| {
                    prop_widgets::scrub_field(
                        ui,
                        &mut next.letter_spacing,
                        &ScrubSpec::new("Letter spacing")
                            .handle(Handle::Text("↔"))
                            .unit(" em")
                            .range(-1.0, 5.0)
                            .step(0.005)
                            .decimals(3)
                            .provenance(spacing),
                    )
                })
            },
        );
        self.label_apply_as(
            "Change Label Line Height",
            editor,
            ctx,
            height_edit,
            &*next,
            |l| l.line_height = None,
        );
        self.label_apply_as(
            "Change Label Letter Spacing",
            editor,
            ctx,
            spacing_edit,
            &*next,
            |l| l.letter_spacing = None,
        );
    }

    /// Arch, and Format as two icon toggles.
    fn label_arch_format(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let label = ctx.label;
        let mut percent = next.arch * 100.0;
        let mut toggled = false;
        let arch_edit = prop_widgets::pair(
            ui,
            |ui| {
                let arch = provenance(label.arch.is_some());
                prop_widgets::named_above(ui, "Arch", arch, |ui| {
                    prop_widgets::scrub_field(
                        ui,
                        &mut percent,
                        &ScrubSpec::new("Arch")
                            .handle(Handle::Icon(Icon::Shape))
                            .unit("%")
                            .range(-100.0, 100.0)
                            .decimals(0)
                            .provenance(arch),
                    )
                })
            },
            |ui| {
                prop_widgets::named_above(ui, "Format", Provenance::Plain, |ui| {
                    let options = [
                        (Icon::Italic, "Italic", next.font.italic),
                        (Icon::AllCaps, "All caps", next.all_caps),
                    ];
                    match prop_widgets::mode_toggles(ui, &options) {
                        Some(0) => next.font.italic = !next.font.italic,
                        Some(_) => next.all_caps = !next.all_caps,
                        None => return,
                    }
                    toggled = true;
                });
            },
        )
        .0;
        if arch_edit.changed {
            next.arch = (percent / 100.0).clamp(-1.0, 1.0);
        }
        self.label_apply_as("Change Label Arch", editor, ctx, arch_edit, &*next, |l| {
            l.arch = None
        });
        if toggled {
            self.label_apply(editor, ctx, Edit::instant(), &*next, |_| {});
        }
    }

    /// Align: three icon buttons.
    fn label_align(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let aligns = [Align::Left, Align::Centre, Align::Right];
        let options = [
            (Icon::AlignLeft, "Align left", false),
            (Icon::AlignCentre, "Align centre", false),
            (Icon::AlignRight, "Align right", false),
        ];
        let selected = aligns.iter().position(|&a| a == next.align).unwrap_or(0);
        let aligned = prop_widgets::named_above(ui, "Align", Provenance::Plain, |ui| {
            prop_widgets::mode_buttons(ui, &options, selected).map(|index| aligns[index])
        });
        if let Some(align) = aligned {
            next.align = align;
            self.label_apply(editor, ctx, Edit::instant(), &*next, |_| {});
        }
    }

    /// The Label's colour.
    fn label_fill(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let mut colour = Some(next.fill);
        let fill = provenance(ctx.label.fill.is_some());
        let response = prop_widgets::named_above(ui, "Fill", fill, |ui| {
            prop_widgets::swatch_field(ui, "Fill", &mut colour, false, fill)
        });
        if response.edit.changed {
            next.fill = colour.unwrap_or(next.fill);
        }
        self.label_apply(editor, ctx, response.edit, &*next, |l| l.fill = None);
    }

    /// The halo's size and, once it has a size, its colour.
    fn label_halo(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let halo = provenance(ctx.label.halo.is_some());
        let has_halo = next.halo.width > 0.0;
        let mut colour = Some(next.halo.color);
        let (size_edit, colour_edit) = prop_widgets::pair(
            ui,
            |ui| {
                prop_widgets::named_above(ui, "Size", halo, |ui| {
                    prop_widgets::scrub_field(
                        ui,
                        &mut next.halo.width,
                        &ScrubSpec::new("Halo")
                            .handle(Handle::Text("S"))
                            .unit(" u")
                            .range(0.0, 100_000.0)
                            .step(0.1)
                            .decimals(1)
                            .provenance(halo),
                    )
                })
            },
            |ui| {
                // The halo's colour matters only once it has a width.
                has_halo.then(|| {
                    prop_widgets::named_above(ui, "Colour", halo, |ui| {
                        prop_widgets::swatch_field(ui, "Halo colour", &mut colour, false, halo).edit
                    })
                })
            },
        );
        self.label_apply_as("Change Label Halo", editor, ctx, size_edit, &*next, |l| {
            l.halo = None
        });
        if let Some(edit) = colour_edit {
            if edit.changed {
                next.halo.color = colour.unwrap_or(next.halo.color);
            }
            self.label_apply(editor, ctx, edit, &*next, |l| l.halo = None);
        }
    }

    /// Opacity and rotation.
    fn label_appearance(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ctx: &LabelEdit<'_>,
        next: &mut ResolvedLabel,
    ) {
        let mut percent = f64::from(next.opacity) * 100.0;
        let (opacity_edit, rotation_edit) = prop_widgets::pair(
            ui,
            |ui| {
                let opacity = provenance(ctx.label.opacity.is_some());
                prop_widgets::named_above(ui, "Opacity", opacity, |ui| {
                    prop_widgets::scrub_field(
                        ui,
                        &mut percent,
                        &ScrubSpec::new("Opacity")
                            .handle(Handle::Text("O"))
                            .unit("%")
                            .range(0.0, 100.0)
                            .provenance(opacity),
                    )
                })
            },
            |ui| {
                prop_widgets::named_above(ui, "Rotation", Provenance::Plain, |ui| {
                    prop_widgets::scrub_field(
                        ui,
                        &mut next.rotation,
                        &ScrubSpec::new("Rotation")
                            .handle(Handle::Icon(Icon::Rotate))
                            .unit("°")
                            .step(0.5)
                            .decimals(1),
                    )
                })
            },
        );
        if opacity_edit.changed {
            next.opacity = (percent / 100.0).clamp(0.0, 1.0) as f32;
        }
        self.label_apply_as(
            "Change Label Opacity",
            editor,
            ctx,
            opacity_edit,
            &*next,
            |l| l.opacity = None,
        );
        self.label_apply(editor, ctx, rotation_edit, &*next, |_| {});
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
        // A value from the style has no link glyph.
        assert_eq!(provenance(false), Provenance::Plain);
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
}
