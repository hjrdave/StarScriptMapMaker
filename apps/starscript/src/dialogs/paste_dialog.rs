//! The "what's missing, proceed?" popup shown before a Paste into a Map that lacks something
//! the copied layers need.

use eframe::egui;
use ssmap_core::PasteGaps;

use crate::dialog;

/// How the popup was answered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Paste,
    /// Cancel, Esc or a click on the scrim: nothing is pasted.
    Cancel,
}

/// Shows the popup for `gaps`; `None` until it is answered.
pub fn show(ctx: &egui::Context, gaps: &PasteGaps) -> Option<Choice> {
    let mut choice = None;
    let shown = dialog::show(
        ctx,
        "paste_missing",
        "Some things are missing",
        420.0,
        |ui| {
            ui.label("This Map doesn’t have everything the copied layers use:");
            ui.add_space(6.0);
            if !gaps.assets.is_empty() {
                let names: Vec<String> = gaps.assets.iter().map(ToString::to_string).collect();
                ui.label(format!(
                    "• Assets become placeholders until installed: {}",
                    names.join(", ")
                ));
            }
            if !gaps.fonts.is_empty() {
                ui.label(format!(
                    "• Fonts are swapped for {}: {}",
                    ssmap_fonts::FALLBACK_FAMILY,
                    gaps.fonts.join(", ")
                ));
            }
            if gaps.label_styles > 0 {
                ui.label(format!(
                    "• {} label{} will follow this Map’s default style",
                    gaps.label_styles,
                    if gaps.label_styles == 1 { "" } else { "s" }
                ));
            }
            dialog::buttons(ui, |ui| {
                if dialog::primary_button(ui, true, "Paste anyway").clicked() {
                    choice = Some(Choice::Paste);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        },
    );
    if shown.cancelled {
        choice = Some(Choice::Cancel);
    }
    choice
}
