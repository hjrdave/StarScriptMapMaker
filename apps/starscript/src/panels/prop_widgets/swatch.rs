//! The colour swatch and its picker.

use super::picker::{paint_chip, percent_text, picker};
use super::*;

/// What the swatch's value became.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwatchChange {
    Colour(Color),
    /// "None" was chosen, for an optional colour.
    None,
}

/// The result of [`swatch`].
pub struct SwatchResponse {
    pub edit: Edit,
    pub change: Option<SwatchChange>,
}

/// A colour field (swatch and hex in one box) that opens a popover with the picker. With
/// `optional`, the popover also offers "None" and `colour` may be `None`. A picker drag is one
/// gesture, finished when the pointer is released.
pub fn swatch(
    ui: &mut egui::Ui,
    label: &str,
    colour: &mut Option<Color>,
    optional: bool,
    provenance: Provenance,
) -> SwatchResponse {
    swatch_row(
        ui,
        label,
        label,
        Column::Label,
        colour,
        optional,
        provenance,
    )
}

/// [`swatch`] whose label column shows `shown` ("Colour") while `label` ("Water colour") stays
/// its accessible name, for a row a group caption already gives the context for.
pub fn swatch_shown_as(
    ui: &mut egui::Ui,
    label: &str,
    shown: &str,
    colour: &mut Option<Color>,
    optional: bool,
    provenance: Provenance,
) -> SwatchResponse {
    swatch_row(
        ui,
        label,
        shown,
        Column::Label,
        colour,
        optional,
        provenance,
    )
}

/// [`swatch`] with no label column, for a field whose name sits above it (or in a half-width
/// column): `label` is only the accessible name. The hex text truncates before it can push the
/// reset arrow out of a narrow column.
pub fn swatch_field(
    ui: &mut egui::Ui,
    label: &str,
    colour: &mut Option<Color>,
    optional: bool,
    provenance: Provenance,
) -> SwatchResponse {
    swatch_row(ui, label, label, Column::None, colour, optional, provenance)
}

/// What the swatch row puts before the chip.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Column {
    /// The label in the label column.
    Label,
    /// An empty label column, to line up under another row.
    Blank,
    /// Nothing: the chip starts at the left edge.
    None,
}

/// [`swatch`] under the row that chose it (Colour: Tint): the label column stays empty and
/// `label` is only the accessible name.
pub fn swatch_unlabelled(
    ui: &mut egui::Ui,
    label: &str,
    colour: &mut Option<Color>,
    optional: bool,
) -> SwatchResponse {
    swatch_row(
        ui,
        label,
        label,
        Column::Blank,
        colour,
        optional,
        Provenance::Plain,
    )
}

fn swatch_row(
    ui: &mut egui::Ui,
    label: &str,
    shown: &str,
    column: Column,
    colour: &mut Option<Color>,
    optional: bool,
    provenance: Provenance,
) -> SwatchResponse {
    let id = ui.make_persistent_id(("swatch", label));
    let mut edit = Edit::default();
    let enabled = ui.is_enabled();
    let response = ui
        .horizontal(|ui| {
            if column != Column::None {
                let (label_rect, _) = ui.allocate_exact_size(
                    Vec2::new(LABEL_COLUMN, metrics::FIELD[0]),
                    Sense::hover(),
                );
                if column == Column::Label {
                    paint_label(ui, label_rect, shown, provenance);
                }
            }
            let room = (ui.available_width() - gap_before_trailing(ui, provenance)).max(40.0);
            let (rect, response) = ui.allocate_exact_size(
                Vec2::new(room, metrics::FIELD[0]),
                if enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            response.widget_info(|| {
                let mut info =
                    egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, enabled, label);
                info.current_text_value = Some(colour.map_or("None".to_owned(), spoken));
                info
            });
            paint_closed(ui, rect, *colour, enabled, response.hovered());
            if trailing(ui, label, provenance) {
                edit.reset = true;
            }
            response
        })
        .inner;

    let mut pending: Option<SwatchChange> = None;
    if enabled {
        let before_id = id.with("before");
        // Esc closes the popover (egui's popup does that itself).
        let popup = egui::Popup::from_toggle_button_response(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                // What the value was when the popover opened, for the Before swatch.
                let stored: Option<Option<Color>> = ui.data(|d| d.get_temp(before_id));
                let before = stored.unwrap_or_else(|| {
                    ui.data_mut(|d| d.insert_temp(before_id, *colour));
                    *colour
                });
                pending = picker(ui, id, *colour, before, optional);
            });
        if popup.is_none() {
            ui.data_mut(|d| d.remove::<Option<Color>>(before_id));
        }
    }

    let editing_id = id.with("editing");
    let editing = ui.data(|d| d.get_temp::<bool>(editing_id).unwrap_or(false));
    if let Some(next) = pending {
        edit.began = !editing;
        edit.changed = true;
        ui.data_mut(|d| d.insert_temp(editing_id, true));
        *colour = match next {
            SwatchChange::Colour(c) => Some(c),
            SwatchChange::None => None,
        };
    }
    // A picker edit ends when the pointer is released.
    if (editing || pending.is_some()) && !ui.input(|i| i.pointer.any_down()) {
        edit.finished = true;
        ui.data_mut(|d| d.insert_temp(editing_id, false));
    }
    SwatchResponse {
        edit,
        change: pending,
    }
}

/// What the closed field's accessible value says: the hex, then the opacity when translucent.
fn spoken(colour: Color) -> String {
    let hex = hex_text(colour);
    if colour.a < 255 {
        format!("{hex} {}", percent_text(colour.a))
    } else {
        hex
    }
}

/// The closed field: the height, border and corners of the other fields, with the swatch and hex
/// inside and a translucent colour's opacity muted at the right edge.
fn paint_closed(ui: &egui::Ui, rect: egui::Rect, colour: Option<Color>, enabled: bool, hot: bool) {
    let palette = theme::palette(ui.ctx());
    let outline = if hot && enabled {
        palette.border_strong
    } else {
        palette.border_subtle
    };
    ui.painter().rect(
        rect,
        theme::RADIUS_FIELD,
        palette.bg_field,
        Stroke::new(1.0, outline),
        StrokeKind::Inside,
    );
    let side = rect.height() - 8.0;
    let chip = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 4.0, rect.center().y - side / 2.0),
        Vec2::splat(side),
    );
    paint_chip(ui, chip, colour, palette);
    let mut right = rect.right() - 6.0;
    if let Some(c) = colour.filter(|c| c.a < 255) {
        let percent = ui.painter().text(
            egui::pos2(right, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            percent_text(c.a),
            egui::FontId::proportional(theme::CAPTION),
            palette.text_tertiary,
        );
        right = percent.left() - 6.0;
    }
    let (shown, tint) = match colour {
        Some(c) if enabled => (hex_text(c), palette.text_primary),
        Some(c) => (hex_text(c), palette.text_tertiary),
        None => ("None".to_owned(), palette.text_tertiary),
    };
    let mut job = egui::text::LayoutJob::simple_singleline(
        shown,
        egui::FontId::new(theme::MONO, theme::mono_family()),
        tint,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - chip.right() - 6.0).max(0.0));
    let galley = ui.painter().layout_job(job);
    let at = egui::pos2(chip.right() + 6.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, tint);
}
