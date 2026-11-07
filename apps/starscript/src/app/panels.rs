//! The panels and bars around the Canvas: tab strip, Home, status bar and Layers panel.

use super::*;
use crate::{CHROME_INSET, CUSTOM_CHROME, icons};

impl MapMakerApp {
    /// Shows the active Map's file name in the window title, with a dot while it has unsaved
    /// changes.
    pub(super) fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.win.tabs.get(self.win.active) {
            Some(tab) => {
                let dirty = if tab.has_unsaved_changes() {
                    " •"
                } else {
                    ""
                };
                format!("{}{dirty} — StarScript Map Maker", tab.name())
            }
            None => "StarScript Map Maker".to_owned(),
        };
        if title != self.win.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.win.title = title;
        }
    }

    /// The strip of tabs in the title-bar area: Home, one tab per open Map, and a +.
    pub(super) fn tab_strip(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let infos: Vec<_> = self
            .win
            .tabs
            .iter()
            .map(|tab| tab_strip::TabInfo {
                name: tab.name(),
                path: tab.document.path().map(|path| path.display().to_string()),
                unsaved: tab.has_unsaved_changes(),
            })
            .collect();
        let current = self.current_slot();
        // macOS hides the window buttons in full screen, so the strip keeps no gap for them.
        let full_screen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
        let inset = if full_screen { 0.0 } else { CHROME_INSET };
        let actions = ui
            .add_enabled_ui(enabled, |ui| {
                tab_strip::show(
                    ui,
                    &infos,
                    current,
                    &mut self.win.strip,
                    inset,
                    CUSTOM_CHROME,
                )
            })
            .inner;
        for action in actions {
            match action {
                tab_strip::Action::Dropped { index, at } => self.drop_tab(index, at),
                tab_strip::Action::Show(slot) => self.show_slot(slot),
                tab_strip::Action::Close(index) => self.close_tab(index),
                tab_strip::Action::NewMap => self.new_blank_tab(),
            }
        }
    }

    /// The Home tab: the Maps the app knows of, and what the user does with them. With
    /// `start_page` it is the lighter start page of a blank tab, where a Map opens in that tab.
    pub(super) fn home_screen(&mut self, ui: &mut egui::Ui, start_page: bool) {
        let ctx = ui.ctx().clone();
        let tabs: Vec<home::OpenTab> = self
            .win
            .tabs
            .iter()
            .map(|tab| home::OpenTab {
                path: tab.document.path().map(Path::to_path_buf),
                name: tab.document.name(),
                canvas: (
                    tab.editor.map().canvas().width,
                    tab.editor.map().canvas().height,
                ),
                blank: tab.is_blank(),
            })
            .collect();
        self.win.home.entered();
        self.win.home.poll(&ctx);
        let wanted: Vec<PathBuf> = self
            .app
            .recent
            .paths()
            .iter()
            .cloned()
            .chain(tabs.iter().filter_map(|tab| tab.path.clone()))
            .collect();
        self.win.home.request(wanted.iter().map(PathBuf::as_path));
        let known = home::known_entries(self.app.recent.paths(), &tabs, |path| {
            self.win.home.info(path)
        });
        let open = home::open_entries(&tabs, |path| self.win.home.info(path));
        let data = home::Data {
            known: &known,
            open: &open,
            packs: self.app.credits.page().packs.len(),
            sprites: Some(&self.app.sprites),
            pack_status: self.win.pack_install.status(),
            notices: &self.win.notices,
        };
        self.app.credits.show(&ctx, None);
        self.app.updates.show(&ctx);
        let actions = if start_page {
            self.win.home.start_page(ui, &data)
        } else {
            self.win.home.show(ui, &data, &mut self.app.updates)
        };
        if self.win.home.loading() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        for action in actions {
            match action {
                home::Action::NewMap => self.win.new_map_dialog.open = true,
                home::Action::OpenDialog => self.show_open_dialog(),
                home::Action::Open(path) => {
                    self.open_path(&path);
                }
                home::Action::Focus(index) => self.focus_tab(index),
                home::Action::Remove(path) => {
                    self.app.recent.remove(&path);
                    self.win.home.forget(&path);
                }
                home::Action::Reveal(path) => {
                    if let Err(err) = file_actions::reveal(&path) {
                        self.notify_error(format!("Couldn’t show the file: {err}"));
                    }
                }
                home::Action::Delete(path) => self.win.delete_prompt = Some(path),
                home::Action::ShowHome => self.show_slot(tab_strip::Slot::Home),
                home::Action::ShowPacks => {
                    self.show_slot(tab_strip::Slot::Home);
                    self.win.home.show_packs();
                }
                home::Action::Credits => self.app.credits.open = true,
                home::Action::AddPack => self.win.pack_dialog.folder(&ctx, "Add Asset pack", None),
                home::Action::AddPackFolder(folder) => self.add_pack(&ctx, folder),
                home::Action::DeletePack(id) => self.win.pack_delete_prompt = Some(id),
                home::Action::DismissPackStatus => self.win.pack_install.dismiss(),
                home::Action::DismissNotices => self.win.notices.clear(),
            }
        }
    }

    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.win.tabs.get(self.win.active) else {
            return;
        };
        let palette = theme::palette(ui.ctx());
        let text = |text: String| {
            egui::RichText::new(text)
                .size(theme::CAPTION)
                .color(palette.text_secondary)
        };
        ui.horizontal_centered(|ui| {
            ui.add_space(theme::SPACING[3]);
            let ctx = ui.ctx().clone();
            let ppp = f64::from(ctx.pixels_per_point());
            if let Some(zoom) = tab.canvas_view.zoom(ppp) {
                ui.label(text(format!("{:.0}%", zoom * 100.0)));
            }
            let point = ctx
                .pointer_hover_pos()
                .and_then(|pointer| tab.canvas_view.canvas_point(pointer));
            if let Some(point) = point {
                ui.separator();
                let real_world = self.win.shell.current_layout().real_world_coordinates;
                ui.label(text(shell::cursor_text(
                    tab.editor.map().canvas(),
                    point,
                    real_world,
                )));
            }
            let summary = shell::selection_summary(
                tab.editor.map(),
                tab.editor.selection(),
                tab.editor.active_layer(),
            );
            if !summary.is_empty() {
                ui.separator();
                ui.label(text(summary));
            }
            // The Object and Layer counts the Layers list used to show under it.
            if let Some(counts) = tab.layer_tree.counts_text() {
                ui.separator();
                ui.label(text(counts));
            }
        });
    }
}

/// The sidebar's panel: the section the rail's open icon names (Layers, Map, Art boards, or
/// History). It has no controls of its own for closing: the rail's icon does that.
pub(super) fn layers_panel(ui: &mut egui::Ui, shell: &mut shell::Shell, tab: &mut MapTab) {
    let section = shell.sidebar_section();
    sidebar_section(ui, section, tab);
}

/// The content of one sidebar section.
fn sidebar_section(ui: &mut egui::Ui, section: shell::SidebarSection, tab: &mut MapTab) {
    match section {
        shell::SidebarSection::Map => {
            let tint = crate::theme::palette(ui.ctx()).text_primary;
            shell::icon_header(ui, icons::Icon::Map, tint, "Map", |_| {});
            tab.properties.map_panel(ui, &mut tab.editor);
        }
        shell::SidebarSection::Boards => {
            let tint = crate::theme::palette(ui.ctx()).text_primary;
            shell::icon_header(ui, icons::Icon::ExportRegion, tint, "Art boards", |ui| {
                if icons::icon_button(ui, icons::Icon::Add, "New Art Board").clicked()
                    && let Some(board) = tab.editor.new_art_board()
                {
                    tab.board_list.jump_to(board);
                }
            });
            tab.properties
                .boards_panel(ui, &mut tab.editor, &mut tab.board_list);
        }
        shell::SidebarSection::History => {
            // Step Back and Step Forward are Undo and Redo, so they carry their shortcuts.
            let mac = cfg!(target_os = "macos");
            let hint = |command| {
                crate::menu::shortcut(command)
                    .map(|shortcut| format!(" ({})", shortcut.label(mac)))
                    .unwrap_or_default()
            };
            let back = format!("Step Back{}", hint(crate::menu::Command::Undo));
            let forward = format!("Step Forward{}", hint(crate::menu::Command::Redo));
            let (can_back, can_forward) = (tab.editor.can_undo(), tab.editor.can_redo());
            let tint = crate::theme::palette(ui.ctx()).text_primary;
            shell::icon_header(ui, icons::Icon::History, tint, "History", |ui| {
                // Right to left: Step Forward is added first so Step Back sits left of it.
                ui.add_enabled_ui(can_forward, |ui| {
                    if icons::icon_button_flipped(ui, icons::Icon::Undo, &forward, true).clicked() {
                        tab.editor.redo();
                    }
                });
                ui.add_enabled_ui(can_back, |ui| {
                    if icons::icon_button(ui, icons::Icon::Undo, &back).clicked() {
                        tab.editor.undo();
                    }
                });
            });
            let (steps, applied) = tab.editor.history_steps();
            if let Some(target) = shell::history_list(ui, &steps, applied) {
                tab.editor.jump_to_history(target);
            }
        }
        shell::SidebarSection::Layers => {
            let tint = crate::theme::palette(ui.ctx()).text_primary;
            shell::icon_header(ui, icons::Icon::LayerGroup, tint, "Layers", |ui| {
                if icons::icon_button(ui, icons::Icon::Add, "New Layer (⇧⌘N)").clicked() {
                    tab.editor.new_layer();
                }
            });
            tab.layer_tree.show(ui, &mut tab.editor);
            if std::mem::take(&mut tab.layer_tree.paste_requested)
                && let Some(at) = tab.canvas_view.paste_point(None)
            {
                tab.editor.paste(at);
            }
        }
    }
}
