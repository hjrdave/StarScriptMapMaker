//! The header of a selected Object: "Layer › Name" with the lock at the right end. The name
//! appears here once, so no section under it repeats it.

use eframe::egui;
use ssmap_core::{Command, Id, Node};

use super::PropertiesPanel;
use crate::editor::Editor;
use crate::icons::{self, Icon};
use crate::layer_tree::node_name;
use crate::prop_widgets::Edit;
use crate::theme;

impl PropertiesPanel {
    /// The breadcrumb of the selected Objects `ids` with the lock button, which locks or unlocks
    /// all of them as one Undo step. `title` names them (the Object's own name for one, "3
    /// Stamps" for several) and `noun` is what the lock button says it locks.
    pub(super) fn object_header(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        ids: &[Id],
        title: &str,
        noun: &str,
    ) {
        let palette = theme::palette(ui.ctx());
        let map = editor.map();
        let layer = ids
            .first()
            .and_then(|&id| map.parent(id))
            .filter(|&layer| ids.iter().all(|&id| map.parent(id) == Some(layer)));
        let locked = ids
            .iter()
            .all(|&id| matches!(map.node(id), Some(Node::Object(object)) if object.locked));
        let lock_label = format!("{} {noun}", if locked { "Unlock" } else { "Lock" });
        let mut toggled = false;
        ui.horizontal(|ui| {
            ui.add_space(theme::SPACING[3]);
            ui.spacing_mut().item_spacing.x = theme::SPACING[1];
            if let Some(layer) = layer {
                for (text, colour) in [
                    (node_name(map, layer), palette.text_tertiary),
                    ("›".to_owned(), palette.text_tertiary),
                ] {
                    ui.label(egui::RichText::new(text).size(theme::CAPTION).color(colour));
                }
            }
            ui.label(
                egui::RichText::new(title)
                    .size(theme::CAPTION)
                    .color(palette.text_secondary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let icon = if locked { Icon::LockFilled } else { Icon::Lock };
                toggled = icons::icon_button(ui, icon, &lock_label).clicked();
            });
        });
        if toggled {
            let commands = ids
                .iter()
                .map(|&id| Command::SetLocked {
                    id,
                    locked: !locked,
                })
                .collect();
            let label = if locked { "Unlock" } else { "Lock" };
            self.live
                .apply_all(editor, Edit::instant(), label, || commands);
        }
    }
}
