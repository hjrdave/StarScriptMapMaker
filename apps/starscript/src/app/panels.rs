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
        let placement = self.placement();
        // Embedded windows share the screen's coordinates; real ones are placed on it.
        self.win.screen_origin = if ui.ctx().embed_viewports() {
            egui::Pos2::ZERO
        } else {
            ui.input(|i| i.viewport().inner_rect)
                .map_or(egui::Pos2::ZERO, |rect| rect.min)
        };
        self.win.content_size = ui
            .input(|i| i.viewport().inner_rect)
            .map_or_else(|| ui.ctx().content_rect().size(), |rect| rect.size());
        let origin = self.win.screen_origin;
        let viewport = self.win.viewport;
        let actions = ui
            .add_enabled_ui(enabled, |ui| {
                tab_strip::show(
                    ui,
                    &infos,
                    current,
                    &mut self.win.strip,
                    CHROME_INSET,
                    CUSTOM_CHROME,
                    placement,
                )
            })
            .inner;
        for action in actions {
            match action {
                tab_strip::Action::MoveToNewWindow(index) => self.move_tab_to_new_window(index),
                tab_strip::Action::MoveToMainWindow(index) => self.move_tab_to_main_window(index),
                tab_strip::Action::Dropped { index, at } => {
                    let at = at + origin.to_vec2();
                    self.app.tab_drops.push(TabDrop {
                        from: viewport,
                        index,
                        at,
                    });
                    self.app.egui_ctx.request_repaint();
                }
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
                canvas: (tab.editor.map.canvas.width, tab.editor.map.canvas.height),
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
        };
        if self.win.main {
            self.app.credits.show(&ctx, None);
            self.app.updates.show(&ctx);
        }
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
                home::Action::Credits => self.app.credits.open = true,
                home::Action::AddPack => self.win.pack_dialog.folder(&ctx, "Add Asset pack", None),
                home::Action::AddPackFolder(folder) => self.add_pack(&ctx, folder),
                home::Action::DeletePack(id) => self.win.pack_delete_prompt = Some(id),
                home::Action::DismissPackStatus => self.win.pack_install.dismiss(),
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
                    &tab.editor.map.canvas,
                    point,
                    real_world,
                )));
            }
            let summary = shell::selection_summary(
                &tab.editor.map,
                tab.editor.selection(),
                tab.editor.active_layer(),
            );
            if !summary.is_empty() {
                ui.separator();
                ui.label(text(summary));
            }
        });
    }
}

/// The Layers panel: its header, the tree and the bottom bar with the collapse control. As an
/// overlay, collapsing closes it.
pub(super) fn layers_panel(
    ui: &mut egui::Ui,
    shell: &mut shell::Shell,
    tab: &mut MapTab,
    footer: bool,
    overlay: bool,
) {
    if shell.sidebar_section() == shell::SidebarSection::History {
        shell::panel_header(ui, "History", |_| {});
        let (steps, applied) = tab.editor.history_steps();
        if let Some(target) = shell::history_list(ui, &steps, applied) {
            tab.editor.jump_to_history(target);
        }
        return;
    }
    shell::panel_header(ui, "Layers", |ui| {
        if icons::icon_button(ui, icons::Icon::Add, "New Layer (⇧⌘N)").clicked() {
            tab.editor.new_layer();
        }
    });
    let collapse = tab.layer_tree.show(ui, &mut tab.editor, footer);
    if std::mem::take(&mut tab.layer_tree.paste_requested)
        && let Some(at) = tab.canvas_view.paste_point(None)
    {
        tab.editor.paste(at);
    }
    if collapse {
        if overlay {
            shell.close_overlays();
        } else {
            shell.set_layers_collapsed(true);
        }
    }
}
