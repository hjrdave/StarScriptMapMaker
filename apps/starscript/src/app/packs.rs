//! Asset packs: loading, adding and deleting them, and the art the open Maps draw from.

use super::*;
use crate::{dialog, pack_page};

/// What the user is told about a pack that could not be loaded.
pub(super) fn problem_notice(problem: &ssmap_core::asset::LoadProblem) -> String {
    format!("Asset pack problem: {problem}")
}

impl MapMakerApp {
    /// Tells the user about the packs that could not be loaded.
    fn notify_pack_problems(&mut self, problems: Vec<ssmap_core::asset::LoadProblem>) {
        for problem in &problems {
            self.notify_error(problem_notice(problem));
        }
    }

    /// Loads the packs under `roots` afresh, for after one is installed or deleted. Every
    /// open Map, the Asset picker and the Canvas see the change on the next frame, without a
    /// restart. Returns what could not be loaded.
    pub fn reload_packs(&mut self, roots: &PackRoots) -> Vec<ssmap_core::asset::LoadProblem> {
        let (library, problems) = PackLibrary::load(roots);
        self.app.credits = Credits::new(&library);
        self.app.sprites.set_library(Arc::new(library));
        problems
    }

    /// Starts adding the pack folder `folder` in the background. The Asset packs page shows
    /// the progress and the result; the picker and open Maps see the pack once it is in.
    pub fn add_pack(&mut self, ctx: &egui::Context, folder: PathBuf) {
        self.win.pack_install.start(
            ctx,
            folder,
            self.app.sprites.library(),
            self.app.pack_roots.clone(),
        );
    }

    /// Takes in a finished install: reloads the packs so every view sees the new one.
    pub(super) fn finish_pack_install(&mut self) {
        if self.win.pack_install.poll().is_some() {
            let roots = self.app.pack_roots.clone();
            let problems = self.reload_packs(&roots);
            self.notify_pack_problems(problems);
        }
    }

    /// Sets where packs are loaded from and added to, for tests.
    pub fn set_pack_roots(&mut self, roots: PackRoots) {
        self.app.pack_roots = roots;
    }

    /// The message about the last Add pack, for tests.
    pub fn pack_install_status(&self) -> Option<&pack_install::Status> {
        self.win.pack_install.status()
    }

    /// Whether the Add pack folder dialog is showing, for tests.
    pub fn pack_dialog_open(&self) -> bool {
        self.win.pack_dialog.is_open()
    }

    /// The store the open Maps draw from, for tests.
    pub fn sprites(&self) -> &SpriteStore {
        &self.app.sprites
    }

    /// The Editor of the Map showing, for tests; `None` on Home.
    pub fn active_editor_mut(&mut self) -> Option<&mut Editor> {
        if !self.map_showing() {
            return None;
        }
        self.win
            .tabs
            .get_mut(self.win.active)
            .map(|tab| &mut tab.editor)
    }

    /// How many Stamps across the open Maps place an Asset of pack `id`.
    pub fn pack_stamp_count(&self, id: &str) -> usize {
        let library = self.app.sprites.library();
        self.win
            .tabs
            .iter()
            .map(|tab| library.pack_usage(tab.editor.map(), id))
            .sum()
    }

    /// Confirms moving an Asset pack to the Trash. Cancel is the default: Enter and Esc cancel.
    pub(super) fn pack_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.win.pack_delete_prompt.clone() else {
            return;
        };
        let name = self
            .app
            .sprites
            .library()
            .pack(&id)
            .map_or_else(|| id.clone(), |pack| pack.name.clone());
        let stamps = self.pack_stamp_count(&id);
        let mut delete = false;
        let mut cancel = false;
        let shown = dialog::show(
            ctx,
            "delete_pack",
            &format!("Delete “{name}”?"),
            380.0,
            |ui| {
                ui.label(format!(
                    "The pack is moved to the {}, so you can put it back from there.",
                    file_actions::TRASH_NAME
                ));
                ui.add_space(6.0);
                ui.label(pack_page::stamp_usage_label(stamps));
                dialog::buttons(ui, |ui| {
                    let danger = theme::palette(ui.ctx()).danger;
                    let button =
                        egui::Button::new(egui::RichText::new("Delete").strong()).fill(danger);
                    if ui.add(button).clicked() {
                        delete = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            },
        );
        if shown.cancelled || dialog::enter_confirms(ctx) {
            cancel = true;
        }
        if delete {
            self.win.pack_delete_prompt = None;
            let roots = self.app.pack_roots.clone();
            // The library refuses built-in packs and anything outside the user packs folder.
            let mut library = PackLibrary::clone(&self.app.sprites.library());
            match library.delete_pack(&id, &roots, self.win.pack_trash) {
                Ok(_) => {
                    let problems = self.reload_packs(&roots);
                    self.notify_pack_problems(problems);
                }
                Err(err) => self.notify_error(format!("Couldn’t delete “{name}”: {err}")),
            }
        } else if cancel {
            self.win.pack_delete_prompt = None;
        }
    }

    /// Sets what moves a pack to the Trash, for tests.
    pub fn set_pack_trash(&mut self, trash: fn(&Path) -> Result<(), TrashError>) {
        self.win.pack_trash = trash;
    }

    /// Whether the delete-pack confirmation is showing, for tests.
    pub fn pack_delete_prompt_open(&self) -> bool {
        self.win.pack_delete_prompt.is_some()
    }
}
