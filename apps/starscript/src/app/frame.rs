//! One frame of a window: the panels around the Canvas, in the order they are laid out.

use super::*;
use crate::{tool_dock, window_chrome};

impl MapMakerApp {
    /// One frame of each window a tab was popped out into, each in a viewport of its own.
    pub(super) fn show_popped(&mut self, ctx: &egui::Context) {
        let mut windows = std::mem::take(&mut self.popped);
        windows.retain(|window| !window.tabs.is_empty());
        for window in &mut windows {
            std::mem::swap(&mut self.win, window);
            let builder = window_chrome(
                egui::ViewportBuilder::default()
                    .with_title(self.win.title.clone())
                    .with_inner_size([1100.0, 760.0]),
            );
            let id = self.win.viewport;
            ctx.show_viewport_immediate(id, builder, |ui, class| {
                if class == egui::ViewportClass::EmbeddedWindow {
                    // Without OS windows (UI tests) it floats over the main window.
                    egui::Window::new(self.win.title.clone())
                        .id(egui::Id::new(id))
                        .default_size([1100.0, 760.0])
                        .show(ui.ctx(), |ui| self.show(ui));
                } else {
                    self.show(ui);
                }
            });
            std::mem::swap(&mut self.win, window);
        }
        // A window whose last tab left or closed is gone with its viewport.
        windows.retain(|window| !window.tabs.is_empty());
        windows.append(&mut self.popped);
        self.popped = windows;
        for tab in std::mem::take(&mut self.app.returning) {
            self.win.tabs.push(tab);
            self.win.active = self.win.tabs.len() - 1;
            self.win.home_active = false;
        }
        self.settle_tab_drops();
        self.settle_payload_drops();
    }

    /// One frame of the editor.
    pub(super) fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.begin_frame(&ctx);
        let dialog_open = self.handle_input(ui, &ctx);
        let strip = egui::Frame::NONE.fill(theme::palette(&ctx).bg_chrome);
        egui::Panel::top("tabs")
            .exact_size(theme::metrics::TITLE_BAR_WITH_TABS)
            .frame(strip)
            .show(ui, |ui| self.tab_strip(ui, !dialog_open));
        if !self.native_menu_shown() {
            egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        }
        if self.win.tabs.is_empty() && !self.win.main {
            // The last tab just left; the window goes after this frame.
            return;
        }
        self.win.canvas_zone = None;
        self.win.layers_zone = None;
        if self.home_active() || self.start_page_showing() {
            self.home_screen(ui, !self.home_active());
            self.journal(&ctx);
            return;
        }
        self.win.home.left();
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        if self.win.main {
            self.app.credits.show(&ctx, Some(&mut tab.editor));
            self.app.updates.show(&ctx);
        }
        tab.label_styles.show(&ctx, &mut tab.editor);
        egui::Panel::bottom("status")
            .exact_size(theme::metrics::STATUS_BAR)
            .show(ui, |ui| self.status_bar(ui));
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        let layout = self.win.shell.current_layout();
        // The window grew: an overlay for a panel that now shows in full has nothing to open.
        if layout.layers != shell::PanelMode::Rail {
            self.win.shell.set_layers_overlay(false);
        }
        if layout.properties != shell::PanelMode::Rail {
            self.win.shell.set_properties_overlay(false);
        }
        let mut layers_divider = None;
        let mut properties_divider = None;
        let mut layers_rail = None;
        let mut properties_rail = None;
        match layout.layers {
            shell::PanelMode::Hidden => {}
            shell::PanelMode::Rail => {
                let panel = egui::Panel::left("layers")
                    .resizable(false)
                    .exact_size(layout.layers.width())
                    .show(ui, |ui| {
                        let path = shell::selection_path(tab.editor.map(), tab.editor.selection());
                        let current = self.win.shell.sidebar_section();
                        if let Some(click) = shell::layers_rail(ui, &path, current) {
                            self.win.shell.click_layers_rail(click);
                        }
                    });
                layers_rail = Some(panel.response.rect);
            }
            shell::PanelMode::Full(width) => {
                let panel = egui::Panel::left("layers")
                    .resizable(false)
                    .exact_size(width)
                    .show(ui, |ui| {
                        layers_panel(ui, &mut self.win.shell, tab, layout.layers_footer, false)
                    });
                layers_divider = Some(panel.response.rect);
            }
        }
        match layout.properties {
            shell::PanelMode::Hidden => {}
            shell::PanelMode::Rail => {
                let panel = egui::Panel::right("properties")
                    .resizable(false)
                    .exact_size(layout.properties.width())
                    .show(ui, |ui| {
                        if shell::properties_rail(ui) {
                            self.win.shell.click_properties_rail();
                        }
                    });
                properties_rail = Some(panel.response.rect);
            }
            shell::PanelMode::Full(width) => {
                let panel = egui::Panel::right("properties")
                    .resizable(false)
                    .exact_size(width)
                    .show(ui, |ui| tab.properties.show(ui, &mut tab.editor));
                if panel.inner {
                    self.win.shell.toggle_properties();
                }
                if tab.properties.take_manage_label_styles() {
                    tab.label_styles.open = true;
                }
                properties_divider = Some(panel.response.rect);
            }
        }
        let canvas = egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| tab.canvas_view.show(ui, &mut tab.editor));
        self.win.canvas_zone = Some(canvas.response.rect);
        self.win.layers_zone = layers_divider;
        if tab.properties.take_show_assets() {
            self.win.shell.show_section(shell::SidebarSection::Assets);
        }
        if tab.asset_browser.take_open_packs() {
            self.win.home.show_packs();
            self.win.home_active = self.win.main;
        }
        tool_dock::show(&ctx, canvas.response.rect, &mut tab.editor.active_tool);
        tab.editor.show_toasts(&ctx, canvas.response.rect);
        // After the Canvas, so the strips on its edges win the pointer over it.
        for (side, rect) in [
            (shell::Side::Left, layers_divider),
            (shell::Side::Right, properties_divider),
        ] {
            if let Some(rect) = rect {
                shell::divider(ui, &mut self.win.shell, side, rect);
            }
        }
        self.show_overlays(&ctx, canvas.response.rect, [layers_rail, properties_rail]);
        self.track_drag(&ctx);
        self.journal(&ctx);
    }

    /// The start of a frame: the in-app overlay, egui's zoom held at 1, and what Copy or Cut
    /// took last frame, which goes to the system clipboard as text.
    fn begin_frame(&mut self, ctx: &egui::Context) {
        if self.win.main {
            self.app.overlay.begin_frame(ctx);
            self.app.overlay.show(ctx);
            // ⌘+ and ⌘− zoom the Canvas only: keep egui's whole-UI zoom off and the scale at 1.
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
            if ctx.zoom_factor() != 1.0 {
                ctx.set_zoom_factor(1.0);
            }
        }
        // What Copy or Cut took last frame goes to the system clipboard as text.
        if let Some(text) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|tab| tab.editor.take_clipboard_text())
        {
            self.app.clipboard = clipboard::Clipboard::from_text(&text);
            ctx.copy_text(text);
        }
    }

    /// Handles close requests, drops, the Escape that closes a panel overlay and the shortcuts,
    /// then shows the dialogs and sets the window title. Returns whether a dialog is open.
    fn handle_input(&mut self, ui: &egui::Ui, ctx: &egui::Context) -> bool {
        // Dialogs take the keyboard while they are open.
        self.handle_close(ctx);
        let dialog_open = self.dialog_open();
        if std::mem::take(&mut self.win.drop_focus) {
            // The ⇥ that toggled the panels also moved keyboard focus into one of them.
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        let window_width = if self.win.main {
            ctx.content_rect().width()
        } else {
            ui.max_rect().width()
        };
        self.win.shell.set_window_width(window_width);
        let panels_were_hidden = self.win.shell.panels_hidden();
        if !dialog_open && self.win.shell.overlay_open() {
            // Esc closes an open panel overlay before anything else sees it.
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.win.shell.close_overlays();
            }
        }
        if !dialog_open {
            let _input = ssmap_render::profile::stage(ssmap_render::profile::Stage::Input);
            self.open_dropped_files(ctx);
            self.shortcuts(ctx);
        }
        self.win.drop_focus = self.win.shell.panels_hidden() != panels_were_hidden;
        self.dialogs(ctx);
        self.update_title(ctx);
        dialog_open
    }

    /// A rail's panel opened over the Canvas, which a press anywhere else closes. `rails` are the
    /// Layers and Properties rails, left out of that because they toggle it.
    fn show_overlays(
        &mut self,
        ctx: &egui::Context,
        canvas_rect: egui::Rect,
        rails: [Option<egui::Rect>; 2],
    ) {
        let [layers_rail, properties_rail] = rails;
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        // A rail's panel opened over the Canvas: Layers from the left, Properties as a
        // floating inspector pinned to the top right.
        let palette = theme::palette(ctx);
        let overlay_frame = egui::Frame::new()
            .fill(palette.bg_panel)
            .stroke(egui::Stroke::new(1.0, palette.border_strong))
            .shadow(egui::Shadow {
                offset: [0, 4],
                blur: 16,
                spread: 0,
                color: theme::SCRIM,
            });
        let mut overlay_rect = None;
        if self.win.shell.layers_overlay() {
            let width = self.win.shell.width(shell::Side::Left);
            let area = egui::Area::new(egui::Id::new("layers overlay"))
                .order(egui::Order::Foreground)
                .fixed_pos(canvas_rect.min)
                .show(ctx, |ui| {
                    overlay_frame.show(ui, |ui| {
                        ui.set_width(width);
                        ui.set_height(canvas_rect.height());
                        layers_panel(ui, &mut self.win.shell, tab, true, true);
                    });
                });
            overlay_rect = Some(area.response.rect);
            self.win.layers_zone = overlay_rect;
        } else if self.win.shell.properties_overlay() {
            const MARGIN: f32 = 8.0;
            let width = self.win.shell.width(shell::Side::Right);
            let height = (canvas_rect.height() - 2.0 * MARGIN).max(0.0);
            let area = egui::Area::new(egui::Id::new("properties inspector"))
                .order(egui::Order::Foreground)
                .fixed_pos(canvas_rect.right_top() + egui::vec2(-width - MARGIN, MARGIN))
                .show(ctx, |ui| {
                    overlay_frame.show(ui, |ui| {
                        ui.set_width(width);
                        ui.set_height(height);
                        if tab.properties.show(ui, &mut tab.editor) {
                            // Collapsing the floating inspector just closes it.
                            self.win.shell.close_overlays();
                        }
                    });
                });
            if tab.properties.take_manage_label_styles() {
                tab.label_styles.open = true;
            }
            overlay_rect = Some(area.response.rect);
        }
        if let Some(overlay_rect) = overlay_rect {
            // A press anywhere else closes it. The rails are left out, as they toggle it.
            let outside = ctx.input(|input| {
                input.pointer.any_pressed()
                    && input.pointer.interact_pos().is_some_and(|pos| {
                        !overlay_rect.contains(pos)
                            && !layers_rail.is_some_and(|rail| rail.contains(pos))
                            && !properties_rail.is_some_and(|rail| rail.contains(pos))
                    })
            });
            if outside {
                self.win.shell.close_overlays();
            }
        }
    }
}
