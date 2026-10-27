//! The frame every modal dialog shares: radius 12, e3 elevation, the scrim behind, the title
//! style, a button row with the primary action on the right in gold, and Esc to cancel.

use eframe::egui::{self, Align, Frame, Layout, Margin, RichText, Stroke};

use crate::theme::{self, RADIUS_DIALOG};

/// What a frame of a dialog came to.
pub struct Shown<R> {
    /// What the contents returned.
    #[allow(
        dead_code,
        reason = "no caller reads it yet; kept with the contents' return value"
    )]
    pub inner: R,
    /// Esc (or a click on the scrim) asked to cancel.
    pub cancelled: bool,
}

/// Shows a modal dialog `title`, `width` points wide, with `contents` below the title.
pub fn show<R>(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    width: f32,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Shown<R> {
    let p = theme::palette(ctx);
    let frame = Frame {
        fill: p.bg_raised,
        stroke: Stroke::new(1.0, p.border_strong),
        corner_radius: egui::CornerRadius::same(RADIUS_DIALOG),
        shadow: theme::shadow_e3(),
        inner_margin: Margin::same(20),
        ..Frame::default()
    };
    let modal = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(theme::SCRIM)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.label(
                RichText::new(title)
                    .text_style(theme::title_style())
                    .color(p.text_primary),
            );
            ui.add_space(8.0);
            contents(ui)
        });
    Shown {
        cancelled: modal.should_close(),
        inner: modal.inner,
    }
}

/// Whether Enter was pressed with no text field or other widget holding the keyboard, which
/// is when it is safe to take it as "confirm".
pub fn enter_confirms(ctx: &egui::Context) -> bool {
    !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Enter))
}

/// The button row: `contents` is laid out right to left, so add the primary button first and
/// the dismissing ones after it.
pub fn buttons<R>(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(12.0);
    ui.with_layout(Layout::right_to_left(Align::Center), contents)
        .inner
}

/// The dialog's main action: gold, and on the right.
pub fn primary_button(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    let p = theme::palette(ui.ctx());
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(p.bg_window).strong()).fill(p.gold),
    )
}

/// The primary button with a shortcut hint beside its label ("Save ⌘S").
pub fn primary_button_with_shortcut(
    ui: &mut egui::Ui,
    text: &str,
    shortcut: &egui::KeyboardShortcut,
) -> egui::Response {
    let p = theme::palette(ui.ctx());
    let hint = ui.ctx().format_shortcut(shortcut);
    let response = ui.add(
        egui::Button::new(RichText::new(text).color(p.bg_window).strong())
            .fill(p.gold)
            .shortcut_text(RichText::new(hint).color(p.bg_window)),
    );
    // Screen readers (and tests) get the action's name, not the hint.
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    response
}
