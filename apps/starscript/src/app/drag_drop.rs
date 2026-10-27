//! Dragging an Asset out of an Asset browser onto the Canvas.

use super::*;

impl MapMakerApp {
    /// While an Asset is dragged: shows the copy cursor and a highlight on the Canvas when the
    /// pointer is over it, and places the Asset where it is let go.
    pub(super) fn track_drag(&mut self, ctx: &egui::Context) {
        use egui::DragAndDrop;
        let Some(asset) = DragAndDrop::payload::<ssmap_core::asset::AssetRef>(ctx) else {
            return;
        };
        let Some(at) = ctx.input(|i| i.pointer.latest_pos()) else {
            return;
        };
        let Some(zone) = self.win.canvas_zone.filter(|zone| zone.contains(at)) else {
            if ctx.input(|i| i.pointer.any_released()) {
                DragAndDrop::clear_payload(ctx);
            }
            return;
        };
        if ctx.input(|i| i.pointer.any_released()) {
            DragAndDrop::clear_payload(ctx);
            self.drop_asset((*asset).clone(), at);
            return;
        }
        ctx.set_cursor_icon(egui::CursorIcon::Copy);
        let palette = theme::palette(ctx);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop highlight"),
        ));
        painter.rect_filled(zone, 0.0, palette.gold_subtle);
        painter.rect_stroke(
            zone.shrink(1.0),
            0.0,
            egui::Stroke::new(2.0, palette.gold_ring),
            egui::StrokeKind::Inside,
        );
        ctx.request_repaint();
    }

    /// Places `asset` on the Canvas at the window point `at` as a Stamp; does nothing when `at`
    /// is not over the Canvas. Also a test hook, standing in for the end of a drag.
    #[doc(hidden)]
    pub fn drop_asset(&mut self, asset: ssmap_core::asset::AssetRef, at: egui::Pos2) {
        if !self.win.canvas_zone.is_some_and(|zone| zone.contains(at)) {
            return;
        }
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        if let Some(point) = tab.canvas_view.canvas_point(at) {
            tab.editor.drop_asset(asset, point);
        }
    }

    /// Test hook: where the Canvas is in the window, once it has been drawn.
    #[doc(hidden)]
    pub fn canvas_zone(&self) -> Option<egui::Rect> {
        self.win.canvas_zone
    }
}
