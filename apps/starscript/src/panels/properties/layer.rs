//! The Layer section: opacity, depth sort and the clip target.

use eframe::egui;
use ssmap_core::{Command, Id};

use super::{PropertiesPanel, caption, row};
use crate::editor::Editor;
use crate::layer_tree::{landmass_name, landmasses, selected_landmass};
use crate::prop_widgets::{self, ScrubSpec, SectionHeader};

impl PropertiesPanel {
    pub(super) fn layer_properties(
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
                let _ = editor.run(Command::SetDepthSort { id, depth_sort });
            }
            caption(ui, crate::layer_tree::DEPTH_SORT_HINT);

            row(ui, "Clip to", |ui| {
                let current = layer
                    .clip
                    .map_or("None".to_owned(), |t| landmass_name(editor.map(), t));
                egui::ComboBox::from_id_salt("clip")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(layer.clip.is_none(), "None").clicked() {
                            let _ = editor.run(Command::SetClip { id, target: None });
                        }
                        for target in landmasses(editor.map()) {
                            let name = landmass_name(editor.map(), target);
                            if ui
                                .selectable_label(layer.clip == Some(target), name)
                                .clicked()
                            {
                                let _ = editor.run(Command::SetClip {
                                    id,
                                    target: Some(target),
                                });
                            }
                        }
                    });
            });
            let selected = selected_landmass(editor.map(), editor.selection());
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Use selection"))
                .clicked()
            {
                let _ = editor.run(Command::SetClip {
                    id,
                    target: selected,
                });
            }
        });
    }
}
