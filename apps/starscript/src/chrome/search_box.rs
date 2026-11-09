//! The search box shared by the Choose an Asset dialog, the Asset packs page and the Home tab:
//! 40 px tall with a search icon, a clear button once there is text, a `/` hint while empty
//! (pressing `/` focuses the box) and a gold border and ring on focus.

use eframe::egui;

use crate::icons::{self, Icon};
use crate::theme;

/// Height of the box in points.
const HEIGHT: f32 = 40.0;
/// Border width in points.
const BORDER: f32 = 1.5;
/// Text size in points.
const TEXT_SIZE: f32 = 14.0;
/// Width reserved at each end for the icon and the clear button.
const GUTTER: f32 = 36.0;

/// A single-line search field. Build it with [`SearchBox::new`] and add it with
/// [`SearchBox::show`].
pub(crate) struct SearchBox<'a> {
    text: &'a mut String,
    hint: &'a str,
    width: f32,
    id: egui::Id,
}

impl<'a> SearchBox<'a> {
    /// A box editing `text`, showing `hint` while empty, `width` points wide
    /// (`f32::INFINITY` fills the row). `id_salt` tells boxes on one screen apart.
    pub(crate) fn new(
        text: &'a mut String,
        hint: &'a str,
        width: f32,
        id_salt: &'static str,
    ) -> Self {
        Self {
            text,
            hint,
            width,
            id: egui::Id::new(("search box", id_salt)),
        }
    }

    /// Draws the box. The response is the text field's, with `rect` set to the whole box;
    /// it counts as changed when the clear button emptied the text.
    pub(crate) fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let p = theme::palette(ui.ctx());
        let width = self.width.min(ui.available_width()).max(GUTTER * 2.0);
        let (outer, _) = ui.allocate_exact_size(egui::vec2(width, HEIGHT), egui::Sense::hover());
        let clear_rect = egui::Rect::from_center_size(
            egui::pos2(outer.right() - GUTTER / 2.0 - 2.0, outer.center().y),
            egui::Vec2::splat(24.0),
        );
        let has_text = !self.text.is_empty();

        // `/` focuses the box when nothing else is being typed into. The typed character is
        // dropped so it does not land in the field.
        if ui.is_enabled() && ui.memory(|m| m.focused().is_none()) {
            let slash = ui.input_mut(|i| {
                let pressed = i.consume_key(egui::Modifiers::NONE, egui::Key::Slash);
                if pressed {
                    i.events
                        .retain(|e| !matches!(e, egui::Event::Text(t) if t == "/"));
                }
                pressed
            });
            if slash {
                ui.memory_mut(|m| m.request_focus(self.id));
            }
        }

        let focused = ui.memory(|m| m.has_focus(self.id));
        let radius = egui::CornerRadius::from(theme::RADIUS_BUTTON);
        let painter = ui.painter();
        painter.rect_filled(outer, radius, p.bg_field);
        if focused {
            painter.rect_stroke(
                outer,
                radius,
                egui::Stroke::new(3.0, p.gold_ring.gamma_multiply(0.45)),
                egui::StrokeKind::Outside,
            );
        }
        let border = if focused { p.gold } else { p.border_strong };
        painter.rect_stroke(
            outer,
            radius,
            egui::Stroke::new(BORDER, border),
            egui::StrokeKind::Inside,
        );
        icons::paint(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(outer.left() + GUTTER / 2.0 + 2.0, outer.center().y),
                egui::Vec2::splat(icons::GRID),
            ),
            Icon::Search,
            icons::GRID,
            if focused {
                p.gold_text
            } else {
                p.text_tertiary
            },
        );

        let field_rect = egui::Rect::from_min_max(
            egui::pos2(outer.left() + GUTTER + 2.0, outer.top()),
            egui::pos2(outer.right() - GUTTER - 2.0, outer.bottom()),
        );
        let mut field_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(field_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let mut response = field_ui.add_sized(
            field_rect.size(),
            egui::TextEdit::singleline(&mut *self.text)
                .id(self.id)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .font(egui::FontId::new(TEXT_SIZE, theme::ui_family()))
                .text_color(p.text_primary)
                .hint_text(
                    egui::RichText::new(self.hint)
                        .size(TEXT_SIZE)
                        .color(p.text_tertiary),
                )
                .vertical_align(egui::Align::Center)
                .desired_width(field_rect.width()),
        );

        if has_text {
            let clear = ui.interact(clear_rect, self.id.with("clear"), egui::Sense::click());
            if clear.hovered() {
                ui.painter()
                    .rect_filled(clear_rect, theme::RADIUS_CHIP, p.bg_raised);
            }
            icons::paint(
                ui,
                clear_rect,
                Icon::Close,
                icons::GRID,
                if clear.hovered() {
                    p.text_primary
                } else {
                    p.text_secondary
                },
            );
            clear.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Clear text")
            });
            if clear.clicked() {
                self.text.clear();
                response.mark_changed();
                ui.memory_mut(|m| m.request_focus(self.id));
            }
        } else {
            paint_slash_hint(ui, clear_rect, p.text_tertiary, p.border_strong);
        }
        response.rect = outer;
        response
    }
}

/// The `/` keycap shown at the right of an empty box.
fn paint_slash_hint(ui: &egui::Ui, rect: egui::Rect, text: egui::Color32, line: egui::Color32) {
    let keycap = egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0));
    ui.painter().rect_stroke(
        keycap,
        theme::RADIUS_CHIP,
        egui::Stroke::new(1.0, line),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        keycap.center(),
        egui::Align2::CENTER_CENTER,
        "/",
        egui::FontId::new(theme::MONO, theme::mono_family()),
        text,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ctx: &egui::Context, text: &mut String, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            SearchBox::new(text, "Search", 200.0, "test").show(ui);
        });
        output.textures_delta.clear();
    }

    fn slash() -> Vec<egui::Event> {
        vec![
            egui::Event::Key {
                key: egui::Key::Slash,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Text("/".to_owned()),
        ]
    }

    #[test]
    fn slash_focuses_the_box_without_typing_it() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        run(&ctx, &mut text, Vec::new());
        assert!(ctx.memory(|m| m.focused()).is_none());
        run(&ctx, &mut text, slash());
        run(&ctx, &mut text, Vec::new());
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new(("search box", "test")))));
        assert!(text.is_empty());
    }

    #[test]
    fn slash_types_normally_while_the_box_has_focus() {
        let ctx = egui::Context::default();
        let mut text = String::new();
        run(&ctx, &mut text, Vec::new());
        run(&ctx, &mut text, slash());
        run(&ctx, &mut text, Vec::new());
        run(&ctx, &mut text, vec![egui::Event::Text("/".to_owned())]);
        assert_eq!(text, "/");
    }
}
