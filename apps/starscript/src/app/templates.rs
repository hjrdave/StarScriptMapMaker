//! Save as Template: the open Map's look written to the user templates folder.

use super::*;
use crate::dialog;

impl MapMakerApp {
    /// The Save as Template dialog, while it is open.
    pub(super) fn template_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut name) = self.win.template_prompt.take() else {
            return;
        };
        let (mut save, mut cancel) = (false, false);
        let shown = dialog::show(ctx, "save_template", "Save as Template", 380.0, |ui| {
            ui.label("New Maps made from it get this Map’s background, shore and lake styles, Label styles and Layers.");
            ui.add_space(8.0);
            let field = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .hint_text("Template name")
                    .desired_width(f32::INFINITY),
            );
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            let enabled = !name.trim().is_empty();
            dialog::buttons(ui, |ui| {
                if dialog::primary_button(ui, enabled, "Save").clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if dialog::enter_confirms(ctx) && !name.trim().is_empty() {
            save = true;
        }
        if shown.cancelled {
            cancel = true;
        }
        if save {
            self.save_template(&name);
        } else if !cancel {
            self.win.template_prompt = Some(name);
        }
    }

    /// Writes the open Map as a template called `name` and lists it in New Map.
    fn save_template(&mut self, name: &str) {
        let Some(tab) = self.win.tabs.get(self.win.active) else {
            return;
        };
        let editor = &tab.editor;
        let template = MapTemplate::from_map(name, editor.map(), editor.default_layers);
        let dialog = &mut self.win.new_map_dialog;
        let saved = match dialog.user_dir() {
            Some(dir) => template.save_to(dir),
            None => Err(TemplateError::NoUserFolder),
        };
        match saved {
            Ok(saved) => {
                dialog.reload();
                dialog.select_named(saved.name());
            }
            Err(err) => self.notify_error(format!("Couldn’t save the template: {err}")),
        }
    }
}
