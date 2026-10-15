//! The Label styles panel: create, rename, edit and delete the Map's named styles. Every
//! change is a Command, so it is undoable; editing a style restyles all of its Labels in one
//! Undo step.

use eframe::egui;
use ssmap_core::{Color, Command, Id, LabelStyle};
use ssmap_fonts::LABEL_FAMILIES;

use crate::editor::Editor;

#[derive(Default)]
pub struct LabelStylesPanel {
    pub open: bool,
    /// The style listed as being edited.
    selected: Option<Id>,
    /// The rename box: the style it belongs to and the text typed so far.
    rename: Option<(Id, String)>,
    colour_gesture: bool,
}

/// A name for a copy that no style has yet: "Name copy", then "Name copy 2", and so on.
pub fn free_name(taken: &[&str], base: &str) -> String {
    let first = format!("{base} copy");
    if !taken.contains(&first.as_str()) {
        return first;
    }
    (2..)
        .map(|n| format!("{first} {n}"))
        .find(|name| !taken.contains(&name.as_str()))
        .expect("an unbounded range")
}

/// The notice after deleting a style that `count` Labels used.
pub fn deleted_notice(name: &str, count: usize, default_name: &str) -> String {
    let labels = if count == 1 {
        "1 Label".to_owned()
    } else {
        format!("{count} Labels")
    };
    format!("Deleted “{name}”: {labels} moved to “{default_name}”, looking the same")
}

impl LabelStylesPanel {
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut open = self.open;
        egui::Window::new("Label styles")
            .open(&mut open)
            .default_width(300.0)
            .vscroll(true)
            .show(ctx, |ui| self.contents(ui, editor));
        self.open = open;
    }

    fn contents(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        if self.colour_gesture && ui.input(|i| !i.pointer.any_down()) {
            self.colour_gesture = false;
            editor.end_gesture();
        }
        let styles: Vec<LabelStyle> = editor.map.label_styles().iter().cloned().collect();
        if self
            .selected
            .is_none_or(|id| !styles.iter().any(|style| style.id == id))
        {
            self.selected = Some(LabelStyle::DEFAULT_ID);
        }
        let selected = self.selected.expect("set above");
        for style in &styles {
            let used = editor.map.labels_using(style.id).len();
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(selected == style.id, &style.name)
                    .clicked()
                {
                    self.selected = Some(style.id);
                    self.rename = None;
                }
                ui.weak(format!("{used} in use"));
            });
        }
        let current = styles
            .iter()
            .find(|style| style.id == selected)
            .expect("selected exists")
            .clone();
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .button("New")
                .on_hover_text("Copy the selected style")
                .clicked()
            {
                let names: Vec<&str> = styles.iter().map(|s| s.name.as_str()).collect();
                let copy = LabelStyle {
                    id: Id::new(),
                    name: free_name(&names, &current.name),
                    ..current.clone()
                };
                let id = copy.id;
                if editor.run(Command::CreateLabelStyle { style: copy }) {
                    self.selected = Some(id);
                    self.rename = None;
                }
            }
            let deletable = current.id != LabelStyle::DEFAULT_ID;
            if ui
                .add_enabled(deletable, egui::Button::new("Delete"))
                .on_disabled_hover_text("The default style can't be deleted")
                .clicked()
            {
                let count = editor.map.labels_using(current.id).len();
                if editor.run(Command::DeleteLabelStyle { id: current.id }) {
                    let default = editor.map.label_styles().default_style().name.clone();
                    editor.info(deleted_notice(&current.name, count, &default));
                }
            }
        });

        if self.rename.as_ref().is_none_or(|(id, _)| *id != current.id) {
            self.rename = Some((current.id, current.name.clone()));
        }
        let name = &mut self.rename.as_mut().expect("set above").1;
        let name_field = ui
            .horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(name)
            })
            .inner;
        if name_field.lost_focus() && *name != current.name {
            if !editor.run(Command::RenameLabelStyle {
                id: current.id,
                name: name.clone(),
            }) {
                *name = current.name.clone();
            }
        } else if !name_field.has_focus() && *name != current.name {
            *name = current.name.clone();
        }

        let mut next = current.clone();
        let mut dragged: Option<egui::Response> = None;
        let mut clicked = false;
        let mut coloured = false;
        egui::Grid::new("label style")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Font");
                egui::ComboBox::from_id_salt("style font")
                    .selected_text(&next.font.family)
                    .show_ui(ui, |ui| {
                        for family in LABEL_FAMILIES {
                            clicked |= ui
                                .selectable_value(
                                    &mut next.font.family,
                                    (*family).to_owned(),
                                    *family,
                                )
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
            });

        let changed = next != current;
        let set = Command::SetLabelStyle { style: next };
        if let Some(response) = dragged {
            // Dragging a value is one gesture: live updates, one Undo step.
            if response.drag_started() {
                editor.begin_gesture("Edit Label Style");
            }
            if changed {
                editor.gesture(set);
            }
            if response.drag_stopped() {
                editor.end_gesture();
            }
        } else if coloured && changed {
            if !self.colour_gesture {
                editor.begin_gesture("Change Label Style Colour");
                self.colour_gesture = true;
            }
            editor.gesture(set);
        } else if clicked && changed {
            editor.run(set);
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_get_a_free_name() {
        assert_eq!(free_name(&["Town"], "Town"), "Town copy");
        assert_eq!(free_name(&["Town", "Town copy"], "Town"), "Town copy 2");
        assert_eq!(
            free_name(&["Town", "Town copy", "Town copy 2"], "Town"),
            "Town copy 3"
        );
    }

    #[test]
    fn the_delete_notice_names_the_count() {
        assert!(deleted_notice("Town", 1, "Default").contains("1 Label moved"));
        assert!(deleted_notice("Town", 1000, "Default").contains("1000 Labels moved"));
    }
}
