//! The floating tool dock at the bottom centre of the Canvas. Editor chrome only; it never
//! reaches an export.

use eframe::egui;

use crate::icons;
use crate::theme::{self, Palette};
use crate::tool_section;
use crate::tools::ToolId;

/// Seconds the pointer rests on a button before its tooltip shows.
const TOOLTIP_DELAY: f64 = 0.5;
/// Gap between a button and its tooltip, in points.
const TOOLTIP_GAP: f32 = 6.0;

/// The tool groups, split by dividers.
const GROUPS: [&[ToolId]; 4] = [
    &[ToolId::Select, ToolId::Pan],
    &[ToolId::LandBrush, ToolId::Lasso, ToolId::Label],
    &[ToolId::Stamp, ToolId::Scatter, ToolId::StampEraser],
    &[ToolId::ExportRegion],
];

/// The accessible name of a tool's button, which is also its tooltip text.
pub fn button_label(tool: ToolId) -> String {
    format!("{} ({})", tool.label(), tool.shortcut_text())
}

/// Draws the dock centred in `canvas` and sets `active` to the tool the user clicks.
pub fn show(ctx: &egui::Context, canvas: egui::Rect, active: &mut ToolId) {
    let palette = theme::palette(ctx);
    let anchor = egui::pos2(
        canvas.center().x,
        canvas.bottom() - theme::metrics::TOOL_DOCK_BOTTOM_OFFSET,
    );
    egui::Area::new(egui::Id::new("tool_dock"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(anchor)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(palette.bg_panel)
                .stroke(egui::Stroke::new(1.0, palette.border_subtle))
                .corner_radius(theme::RADIUS_DOCK)
                .shadow(theme::shadow_e2())
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (index, group) in GROUPS.iter().enumerate() {
                            if index > 0 {
                                divider(ui, palette);
                            }
                            for &tool in *group {
                                if tool_button(ui, palette, tool, *active == tool).clicked() {
                                    *active = tool;
                                }
                            }
                        }
                    });
                });
        });
}

fn divider(ui: &mut egui::Ui, palette: &Palette) {
    let height = theme::metrics::TOOL_BUTTON[1];
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(1.0 + 2.0 * theme::SPACING[0], height),
        egui::Sense::hover(),
    );
    let x = rect.center().x.round();
    ui.painter().line_segment(
        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
        egui::Stroke::new(1.0, palette.border_subtle),
    );
}

fn tool_button(ui: &mut egui::Ui, palette: &Palette, tool: ToolId, active: bool) -> egui::Response {
    let label = button_label(tool);
    let (rect, response) = ui.allocate_exact_size(
        egui::Vec2::from(theme::metrics::TOOL_BUTTON),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Button,
            ui.is_enabled(),
            active,
            label.clone(),
        )
    });
    if ui.is_rect_visible(rect) {
        let radius = theme::RADIUS_BUTTON;
        let painter = ui.painter();
        let tint = if !ui.is_enabled() {
            palette.text_tertiary.gamma_multiply(0.5)
        } else if active {
            painter.rect_filled(rect, radius, palette.gold_subtle);
            painter.rect_stroke(
                rect.shrink(0.5),
                radius,
                egui::Stroke::new(1.0, palette.gold_ring),
                egui::StrokeKind::Inside,
            );
            palette.gold_text
        } else if response.hovered() {
            painter.rect_filled(rect, radius, palette.hover_fill);
            palette.text_primary
        } else {
            palette.text_secondary
        };
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                radius,
                egui::Stroke::new(2.0, palette.select),
                egui::StrokeKind::Inside,
            );
        }
        icons::paint(ui, rect, tool_section::icon(tool), icons::GRID, tint);
    }
    tooltip(ui, palette, &response, &label);
    response
}

/// A tooltip above the button, once the pointer has rested on it for [`TOOLTIP_DELAY`].
fn tooltip(ui: &egui::Ui, palette: &Palette, response: &egui::Response, text: &str) {
    let ctx = ui.ctx();
    let started = egui::Id::new(("tool_dock_hover", response.id));
    if !response.hovered() {
        ctx.data_mut(|data| data.remove::<f64>(started));
        return;
    }
    let now = ctx.input(|input| input.time);
    let since = ctx.data_mut(|data| *data.get_temp_mut_or_insert_with(started, || now));
    let remaining = since + TOOLTIP_DELAY - now;
    if remaining > 0.0 {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining));
        return;
    }
    egui::Area::new(egui::Id::new("tool_dock_tooltip"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(response.rect.center_top() - egui::vec2(0.0, TOOLTIP_GAP))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(palette.bg_raised)
                .stroke(egui::Stroke::new(1.0, palette.border_strong))
                .corner_radius(theme::RADIUS_BUTTON)
                .shadow(theme::shadow_e1())
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .size(theme::CAPTION)
                            .color(palette.text_primary),
                    );
                });
        });
}
