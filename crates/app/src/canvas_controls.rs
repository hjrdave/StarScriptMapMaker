//! The controls drawn over the Canvas: the zoom control at the bottom right and the node-edit
//! banner at the top centre. Editor visuals only; they never reach an export.

use eframe::egui;

use crate::theme::{self, Palette};
use crate::zoom::{PRESETS, Zoom};

/// Gap between the zoom control and the Canvas's right and bottom edges, in points.
const EDGE_GAP: f32 = 16.0;
/// Gap between the node-edit banner and the Canvas's top edge, in points.
const BANNER_GAP: f32 = 12.0;

fn panel_frame(palette: &Palette) -> egui::Frame {
    egui::Frame::NONE
        .fill(palette.bg_panel)
        .stroke(egui::Stroke::new(1.0, palette.border_subtle))
        .corner_radius(theme::RADIUS_BUTTON)
        .shadow(theme::shadow_e1())
        .inner_margin(egui::Margin::same(2))
}

/// What the user chose in the node-edit banner.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BannerAction {
    Cancel,
    Done,
}

/// The `−` | `84%` | `+` control over the bottom-right of `canvas`. `percent` is the current
/// zoom as a share of 1:1 in points; `field` keeps the menu's percentage text between frames.
pub fn zoom_control(
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
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(canvas.right_bottom() - egui::vec2(EDGE_GAP, EDGE_GAP))
        .show(ctx, |ui| {
            panel_frame(palette).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if ui
                        .add(egui::Button::new("−").frame(false))
                        .on_hover_text("Zoom out (⌘−)")
                        .clicked()
                    {
                        chosen = Some(Zoom::Out);
                    }
                    let label = format!("{percent:.0}%");
                    ui.menu_button(label, |ui| {
                        if let Some(zoom) = zoom_menu(ui, percent, can_zoom_to_selection, field) {
                            chosen = Some(zoom);
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Zoom options");
                    if ui
                        .add(egui::Button::new("+").frame(false))
                        .on_hover_text("Zoom in (⌘+)")
                        .clicked()
                    {
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
pub fn node_banner(
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
                                "Editing nodes · {selected} of {total} selected"
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
