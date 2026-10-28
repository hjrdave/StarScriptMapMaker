//! The Layer area pinned under the list: opacity, depth sort and the clip target of the
//! selected Layer, or of the active Layer when none is selected.

use eframe::egui;
use ssmap_core::{Command, Id, Node};

use super::{DEPTH_SORT_HINT, LayerTree, landmass_name, landmasses, selected_landmass};
use crate::editor::Editor;
use crate::prop_widgets::{self, ScrubSpec, SectionHeader};
use crate::theme;

/// The Layer the area edits: the one Layer selected, else the active Layer.
fn shown_layer(editor: &Editor) -> Option<Id> {
    let layer = match editor.selection() {
        [id] if editor.map().node(*id).is_some_and(Node::is_layer) => Some(*id),
        _ => editor.active_layer(),
    };
    layer.filter(|&id| editor.map().node(id).is_some_and(Node::is_layer))
}

impl LayerTree {
    /// Shows the area and returns its height, or 0 when there is no Layer to show.
    pub(super) fn layer_area(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> f32 {
        let Some(id) = shown_layer(editor) else {
            return 0.0;
        };
        let Some(Node::Layer(layer)) = editor.map().node(id).cloned() else {
            return 0.0;
        };
        let top = ui.cursor().top();
        let palette = theme::palette(ui.ctx());
        ui.painter().hline(
            ui.max_rect().x_range(),
            top,
            egui::Stroke::new(1.0, palette.border_subtle),
        );
        ui.spacing_mut().item_spacing.y = theme::SPACING[1];
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
            ui.label(
                egui::RichText::new(DEPTH_SORT_HINT)
                    .size(theme::CAPTION)
                    .color(palette.text_tertiary),
            );

            clip_row(ui, editor, id, layer.clip);
        });
        ui.add_space(theme::SPACING[1]);
        ui.cursor().top() - top
    }
}

/// "Clip to" and its Landmass picker, with "Use selection" under it.
fn clip_row(ui: &mut egui::Ui, editor: &mut Editor, id: Id, clip: Option<Id>) {
    let palette = theme::palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_sized(
            [64.0, 22.0],
            egui::Label::new(
                egui::RichText::new("Clip to")
                    .size(theme::BODY)
                    .color(palette.text_secondary),
            ),
        );
        let current = clip.map_or("None".to_owned(), |t| landmass_name(editor.map(), t));
        egui::ComboBox::from_id_salt("clip")
            .selected_text(current)
            .show_ui(ui, |ui| {
                if ui.selectable_label(clip.is_none(), "None").clicked() {
                    let _ = editor.run(Command::SetClip { id, target: None });
                }
                for target in landmasses(editor.map()) {
                    let name = landmass_name(editor.map(), target);
                    if ui.selectable_label(clip == Some(target), name).clicked() {
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
}
