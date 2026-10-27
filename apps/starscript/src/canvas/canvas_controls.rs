//! The controls drawn over the Canvas: the zoom control at the bottom right and the node-edit
//! banner at the top centre. Editor visuals only; they never reach an export.

use eframe::egui;

use crate::theme::{self, Palette};
use crate::zoom::{PRESETS, Zoom};

/// Gap between the zoom control and the Canvas's right edge, in points.
const EDGE_GAP: f32 = 16.0;
/// Gap between the node-edit banner and the Canvas's top edge, in points.
const BANNER_GAP: f32 = 12.0;

/// What the user chose in the node-edit banner.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BannerAction {
    Cancel,
    Done,
}

/// Where the zoom control's right-centre sits: on the tool dock's vertical centre line.
fn zoom_control_anchor(canvas: egui::Rect) -> egui::Pos2 {
    egui::pos2(
        canvas.right() - EDGE_GAP,
        canvas.bottom() - theme::metrics::FLOATING_BAR_CENTER_OFFSET,
    )
}

/// Width of the `−` and `+` sections, in points.
const SIDE_WIDTH: f32 = 40.0;
/// Width of the percentage section. Fixed so the pill keeps its size from `5%` to `800%`.
const MIDDLE_WIDTH: f32 = 68.0;

/// One click target of the zoom pill: plain text on the pill, a rounded fill on hover or press.
fn section(
    ui: &mut egui::Ui,
    palette: &Palette,
    text: &str,
    width: f32,
    tooltip: &str,
) -> egui::Response {
    let size = egui::vec2(width, theme::metrics::TOOL_DOCK_HEIGHT - 2.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        Some(palette.border_subtle)
    } else if response.hovered() || response.has_focus() {
        Some(palette.hover_fill)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter()
            .rect_filled(rect.shrink(3.0), theme::RADIUS_BUTTON, fill);
    }
    let font = if text.ends_with('%') {
        egui::FontId::proportional(theme::HEADING)
    } else {
        egui::FontId::proportional(theme::TITLE + 3.0)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        font,
        palette.text_primary,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    response.on_hover_text(tooltip)
}

/// A thin vertical line between two sections.
fn divider(ui: &mut egui::Ui, palette: &Palette) {
    let height = theme::metrics::TOOL_DOCK_HEIGHT - 2.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, palette.tab_divider);
}

/// The `−` | `84%` | `+` control over the bottom-right of `canvas`. `percent` is the current
/// zoom as a share of 1:1 in points; `field` keeps the menu's percentage text between frames.
pub(crate) fn zoom_control(
    ctx: &egui::Context,
    canvas: egui::Rect,
    percent: f64,
    can_zoom_to_selection: bool,
    field: &mut String,
) -> Option<Zoom> {
    let palette = theme::palette(ctx);
    let mut chosen = None;
    egui::Area::new(egui::Id::new("zoom_control"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_CENTER)
        .fixed_pos(zoom_control_anchor(canvas))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(palette.bg_panel)
                .stroke(egui::Stroke::new(1.0, palette.border_subtle))
                .corner_radius(theme::RADIUS_DOCK)
                .shadow(theme::shadow_e2())
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    ui.horizontal(|ui| {
                        let out = section(ui, palette, "−", SIDE_WIDTH, "Zoom out (⌘−)");
                        if out.clicked() {
                            chosen = Some(Zoom::Out);
                        }
                        divider(ui, palette);
                        let label = format!("{percent:.0}%");
                        let middle = section(ui, palette, &label, MIDDLE_WIDTH, "Zoom options");
                        egui::Popup::menu(&middle)
                            .id(egui::Id::new("zoom_control_menu"))
                            .show(|ui| {
                                if let Some(zoom) =
                                    zoom_menu(ui, percent, can_zoom_to_selection, field)
                                {
                                    chosen = Some(zoom);
                                    ui.close();
                                }
                            });
                        divider(ui, palette);
                        let zoom_in = section(ui, palette, "+", SIDE_WIDTH, "Zoom in (⌘+)");
                        if zoom_in.clicked() {
                            chosen = Some(Zoom::In);
                        }
                    });
                });
        });
    chosen
}

/// The dropdown: an editable percentage, the two framing commands and the presets.
fn zoom_menu(
    ui: &mut egui::Ui,
    percent: f64,
    can_zoom_to_selection: bool,
    field: &mut String,
) -> Option<Zoom> {
    let mut chosen = None;
    let id = egui::Id::new("zoom_field");
    if !ui.memory(|m| m.has_focus(id)) {
        *field = format!("{percent:.0}%");
    }
    let response = ui.add(egui::TextEdit::singleline(field).id(id).desired_width(72.0));
    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        let typed = field.trim().trim_end_matches('%').trim();
        if let Ok(value) = typed.parse::<f64>()
            && value > 0.0
        {
            chosen = Some(Zoom::Percent(value / 100.0));
        }
    }
    ui.separator();
    let item = |ui: &mut egui::Ui, text: &str, shortcut: &str, enabled: bool| {
        ui.add_enabled(
            enabled,
            egui::Button::new(text)
                .shortcut_text(shortcut)
                .min_size(egui::vec2(160.0, 0.0)),
        )
        .clicked()
    };
    if item(ui, "Zoom to Fit", "⇧1", true) {
        chosen = Some(Zoom::Fit);
    }
    if item(ui, "Zoom to Selection", "⇧2", can_zoom_to_selection) {
        chosen = Some(Zoom::Selection);
    }
    ui.separator();
    for preset in PRESETS {
        let text = format!("{:.0}%", preset * 100.0);
        let shortcut = if preset == 1.0 { "⇧0" } else { "" };
        if item(ui, &text, shortcut, true) {
            chosen = Some(Zoom::Percent(preset));
        }
    }
    chosen
}

/// The blue banner at the top centre of `canvas` while nodes are edited.
pub(crate) fn node_banner(
    ctx: &egui::Context,
    canvas: egui::Rect,
    selected: usize,
    total: usize,
) -> Option<BannerAction> {
    let palette = theme::palette(ctx);
    let mut action = None;
    egui::Area::new(egui::Id::new("node_edit_banner"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(canvas.center_top() + egui::vec2(0.0, BANNER_GAP))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(palette.select)
                .corner_radius(theme::RADIUS_BUTTON)
                .shadow(theme::shadow_e1())
                .inner_margin(egui::Margin::symmetric(10, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "Editing points · {selected} of {total} selected"
                            ))
                            .size(theme::BODY)
                            .color(egui::Color32::WHITE),
                        );
                        ui.add_space(theme::SPACING[3]);
                        let cancel = egui::Button::new(
                            egui::RichText::new("Cancel").color(egui::Color32::WHITE),
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(1.0, egui::Color32::WHITE));
                        if ui.add(cancel).clicked() {
                            action = Some(BannerAction::Cancel);
                        }
                        let done =
                            egui::Button::new(egui::RichText::new("Done").color(palette.select))
                                .fill(egui::Color32::WHITE);
                        if ui.add(done).clicked() {
                            action = Some(BannerAction::Done);
                        }
                    });
                });
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_pill_width_is_fixed() {
        // Three sections and two 1-pt dividers, whatever the percentage says.
        assert_eq!(SIDE_WIDTH * 2.0 + MIDDLE_WIDTH + 2.0, 150.0);
    }

    #[test]
    fn zoom_control_centre_matches_tool_dock_centre() {
        let canvas = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 600.0));
        let dock_centre = canvas.bottom()
            - theme::metrics::TOOL_DOCK_BOTTOM_OFFSET
            - theme::metrics::TOOL_DOCK_HEIGHT / 2.0;
        assert_eq!(zoom_control_anchor(canvas).y, dock_centre);
    }
}
