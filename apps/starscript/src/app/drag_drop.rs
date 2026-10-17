//! Dragging tabs between windows, and Assets and Layers onto them.

use super::*;

/// A tab let go after a drag: where, on the screen, decides which window's strip it joins.
pub(super) struct TabDrop {
    pub(super) from: egui::ViewportId,
    pub(super) index: usize,
    pub(super) at: egui::Pos2,
}

/// What is dragged out of a Layers panel or an Asset browser.
enum Dragged {
    Asset(ssmap_core::asset::AssetRef),
    Layer(ssmap_core::Id),
}

/// An Asset or Layer let go after a drag, with where on the screen.
pub(super) struct PayloadDrop {
    item: Dragged,
    /// The window and tab index of the Map a Layer was dragged out of.
    source: Option<(egui::ViewportId, usize)>,
    at: egui::Pos2,
}

impl MapMakerApp {
    /// Settles the tabs dropped this frame: onto the strip of the window under the pointer, at
    /// the drop position, or out into a window of their own when no window is under it. Dropped over a
    /// window but off its strip they stay put. Runs
    /// once every window has had its frame, so all of them can be reached.
    pub(super) fn settle_tab_drops(&mut self) {
        for drop in std::mem::take(&mut self.app.tab_drops) {
            let mut windows: Vec<&mut WindowState> = std::iter::once(&mut self.win)
                .chain(self.popped.iter_mut())
                .collect();
            let Some(source) = windows.iter().position(|w| w.viewport == drop.from) else {
                continue;
            };
            if !windows[source].can_release(drop.index) {
                continue;
            }
            let target = windows
                .iter()
                .position(|w| w.strip_on_screen().contains(drop.at));
            let mut popped_out = None;
            match target {
                Some(target) if target == source => {
                    let window = &mut *windows[source];
                    let local = drop.at.x - window.screen_origin.x;
                    let mut to = window.strip.geometry.drop_index(local, window.tabs.len());
                    if to > drop.index {
                        to -= 1;
                    }
                    if to != drop.index {
                        let tab = window.take_tab(drop.index);
                        window.tabs.insert(to, tab);
                        window.active = to;
                        window.home_active = false;
                    }
                }
                Some(target) => {
                    let tab = windows[source].take_tab(drop.index);
                    let window = &mut *windows[target];
                    let local = drop.at.x - window.screen_origin.x;
                    let to = window.strip.geometry.drop_index(local, window.tabs.len());
                    window.tabs.insert(to, tab);
                    window.active = to;
                    window.home_active = false;
                }
                // Over a window but off its strip: the tab stays where it is.
                None if windows
                    .iter()
                    .any(|w| w.content_on_screen().contains(drop.at)) => {}
                // The only tab of a popped-out window is already in a window of its own.
                None if !windows[source].main && windows[source].tabs.len() == 1 => {}
                None => {
                    let shell = windows[source].shell.clone();
                    let tab = windows[source].take_tab(drop.index);
                    popped_out = Some((tab, shell));
                }
            }
            if let Some((tab, shell)) = popped_out {
                self.popped_count += 1;
                let mut win = self.new_window_state(egui::ViewportId::from_hash_of((
                    "popped window",
                    self.popped_count,
                )));
                win.shell = shell;
                win.tabs.push(tab);
                self.popped.push(win);
            }
        }
        self.popped.retain(|window| !window.tabs.is_empty());
    }

    /// While an Asset or Layer is dragged: shows the copy cursor and a highlight on the Canvas
    /// and Layers panel of this window when the pointer is over them (a Layer from its own Map
    /// is a move, not a copy), and queues the drop when it is let go.
    pub(super) fn track_drag(&mut self, ctx: &egui::Context) {
        use egui::DragAndDrop;
        let item = if let Some(asset) = DragAndDrop::payload::<ssmap_core::asset::AssetRef>(ctx) {
            Dragged::Asset((*asset).clone())
        } else if let Some(id) = DragAndDrop::payload::<ssmap_core::Id>(ctx) {
            Dragged::Layer(*id)
        } else {
            self.app.drag_hover = None;
            self.app.drag_copy = false;
            self.app.drag_source = None;
            return;
        };
        let origin = self.win.screen_origin.to_vec2();
        let pointer = ctx.input(|i| i.pointer.latest_pos());
        // The window whose Layers panel is being dragged from is where the drag started.
        let here = (self.win.viewport, self.win.active);
        if std::mem::take(&mut self.win.tabs[self.win.active].layer_tree.dragging) {
            self.app.drag_source = Some(here);
        }
        if ctx.input(|i| i.pointer.any_released())
            && let Some(at) = pointer
        {
            self.app.payload_drops.push(PayloadDrop {
                item,
                source: self.app.drag_source.take(),
                at: at + origin,
            });
            DragAndDrop::clear_payload(ctx);
            self.app.drag_hover = None;
            self.app.egui_ctx.request_repaint();
            return;
        }
        if ctx.input(|i| i.pointer.primary_down())
            && let Some(at) = pointer
        {
            self.app.drag_hover = Some(at + origin);
        }
        if self.app.drag_copy {
            ctx.set_cursor_icon(egui::CursorIcon::Copy);
        }
        let Some(hover) = self.app.drag_hover else {
            return;
        };
        let mut zones = Vec::new();
        match item {
            Dragged::Asset(_) => zones.extend(self.win.canvas_on_screen()),
            Dragged::Layer(_) if self.app.drag_source != Some(here) => {
                zones.extend(self.win.canvas_on_screen());
                zones.extend(self.win.layers_on_screen());
            }
            Dragged::Layer(_) => {}
        }
        if let Some(zone) = zones.into_iter().find(|zone| zone.contains(hover)) {
            let palette = theme::palette(ctx);
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop highlight"),
            ));
            let zone = zone.translate(-origin);
            painter.rect_filled(zone, 0.0, palette.gold_subtle);
            painter.rect_stroke(
                zone.shrink(1.0),
                0.0,
                egui::Stroke::new(2.0, palette.gold_ring),
                egui::StrokeKind::Inside,
            );
            self.app.drag_over_target = true;
            ctx.request_repaint();
        }
    }

    /// Settles the Assets and Layers dropped this frame, once every window has had its frame:
    /// an Asset on a Canvas is placed there; a Layer on a Canvas or a Layers panel is copied
    /// into that window's Map, which is one Undo step there, and the Map it came from is
    /// unchanged. The topmost window under the pointer takes it.
    pub(super) fn settle_payload_drops(&mut self) {
        for drop in std::mem::take(&mut self.app.payload_drops) {
            let mut windows: Vec<&mut WindowState> = std::iter::once(&mut self.win)
                .chain(self.popped.iter_mut())
                .collect();
            let Some(target) = windows.iter().rposition(|w| {
                w.canvas_on_screen()
                    .is_some_and(|zone| zone.contains(drop.at))
                    || matches!(drop.item, Dragged::Layer(_))
                        && w.layers_on_screen()
                            .is_some_and(|zone| zone.contains(drop.at))
            }) else {
                continue;
            };
            let clipboard = match (&drop.item, drop.source) {
                (Dragged::Layer(id), Some((viewport, index))) => {
                    let Some(source) = windows.iter().position(|w| w.viewport == viewport) else {
                        continue;
                    };
                    // Dropped back on its own Map: that is a move, which the Layers panel does.
                    if target == source && windows[source].active == index {
                        continue;
                    }
                    let Some(clipboard) = windows[source]
                        .tabs
                        .get_mut(index)
                        .and_then(|tab| tab.editor.drag_clipboard(*id))
                    else {
                        continue;
                    };
                    Some(clipboard)
                }
                (Dragged::Layer(_), None) => continue,
                (Dragged::Asset(_), _) => None,
            };
            let window = &mut *windows[target];
            let origin = window.screen_origin;
            let on_canvas = window
                .canvas_on_screen()
                .is_some_and(|zone| zone.contains(drop.at));
            let tab = &mut window.tabs[window.active];
            let point = tab
                .canvas_view
                .canvas_point(drop.at - origin.to_vec2())
                .filter(|_| on_canvas)
                .or_else(|| tab.canvas_view.paste_point(None));
            let Some(point) = point else {
                continue;
            };
            match (drop.item, clipboard) {
                (Dragged::Asset(asset), _) if on_canvas => {
                    tab.editor.drop_asset(asset, point);
                }
                (Dragged::Layer(_), Some(clipboard)) => {
                    tab.editor.paste_clipboard(clipboard, point);
                }
                _ => {}
            }
        }
    }

    /// Test hook: lets go of the Asset `asset` with the pointer at `at` on the screen, as the
    /// end of a drag from an Asset browser does.
    #[doc(hidden)]
    pub fn drop_asset(&mut self, asset: ssmap_core::asset::AssetRef, at: egui::Pos2) {
        self.app.payload_drops.push(PayloadDrop {
            item: Dragged::Asset(asset),
            source: None,
            at,
        });
        self.app.egui_ctx.request_repaint();
    }

    /// Test hook: lets go of the Layer or Object `id` of the active Map of window `window` (0 is
    /// the main one) with the pointer at `at` on the screen, as the end of a drag from that
    /// window's Layers panel does.
    #[doc(hidden)]
    pub fn drop_layer(&mut self, window: usize, id: ssmap_core::Id, at: egui::Pos2) {
        let state = if window == 0 {
            &self.win
        } else {
            &self.popped[window - 1]
        };
        self.app.payload_drops.push(PayloadDrop {
            item: Dragged::Layer(id),
            source: Some((state.viewport, state.active)),
            at,
        });
        self.app.egui_ctx.request_repaint();
    }

    /// Test hook: where window `window`'s Canvas and Layers panel are on the screen (0 is the
    /// main window).
    #[doc(hidden)]
    pub fn drop_zones(&self, window: usize) -> (Option<egui::Rect>, Option<egui::Rect>) {
        let state = if window == 0 {
            &self.win
        } else {
            &self.popped[window - 1]
        };
        (state.canvas_on_screen(), state.layers_on_screen())
    }
}
