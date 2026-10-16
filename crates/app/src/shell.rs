//! The window shell around a Map tab: the Layers and Properties columns, their widths and
//! collapse state (the same for every tab, kept across launches), and the text of the status
//! bar.

use eframe::egui;
use serde::{Deserialize, Serialize};
use ssmap_core::{Canvas, DistanceUnit, Id, Map, Node, ObjectKind};

use crate::icons::{self, Icon};
use crate::layer_tree::node_name;
use crate::theme::{self, metrics};

const STORAGE_KEY: &str = "shell";
/// How wide the draggable strip on a column's inner edge is, in points.
const DIVIDER: f32 = 5.0;

/// Which side of the Canvas a column sits on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shell {
    layers_width: f32,
    properties_width: f32,
    /// The user collapsed Layers to its rail.
    layers_collapsed: bool,
    /// The user hid Properties.
    properties_hidden: bool,
    /// ⇥ hid both panels. Kept apart from the two flags above so that showing them again
    /// restores exactly what the user had collapsed.
    panels_hidden: bool,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            layers_width: metrics::LAYERS_PANEL_DEFAULT,
            properties_width: metrics::PROPERTIES_PANEL_DEFAULT,
            layers_collapsed: false,
            properties_hidden: false,
            panels_hidden: false,
        }
    }
}

impl Shell {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| storage.get_string(STORAGE_KEY))
            .and_then(|json| serde_json::from_str::<Self>(&json).ok())
            .unwrap_or_default()
            .clamped()
    }

    pub fn store(&self, storage: &mut dyn eframe::Storage) {
        let json = serde_json::to_string(self).expect("shell state serialises");
        storage.set_string(STORAGE_KEY, json);
    }

    fn clamped(mut self) -> Self {
        self.set_width(Side::Left, self.layers_width);
        self.set_width(Side::Right, self.properties_width);
        self
    }

    /// The width of the column on `side`, as last set.
    pub fn width(&self, side: Side) -> f32 {
        match side {
            Side::Left => self.layers_width,
            Side::Right => self.properties_width,
        }
    }

    /// Sets a column's width, held to its range.
    pub fn set_width(&mut self, side: Side, width: f32) {
        let (range, slot) = match side {
            Side::Left => (
                metrics::LAYERS_PANEL_MIN..=metrics::LAYERS_PANEL_MAX,
                &mut self.layers_width,
            ),
            Side::Right => (
                metrics::PROPERTIES_PANEL_MIN..=metrics::PROPERTIES_PANEL_MAX,
                &mut self.properties_width,
            ),
        };
        *slot = if width.is_nan() {
            *range.start()
        } else {
            width.clamp(*range.start(), *range.end())
        };
    }

    pub fn reset_width(&mut self, side: Side) {
        self.set_width(
            side,
            match side {
                Side::Left => metrics::LAYERS_PANEL_DEFAULT,
                Side::Right => metrics::PROPERTIES_PANEL_DEFAULT,
            },
        );
    }

    /// Whether the Layers column shows, as the full panel or as the rail.
    pub fn layers_shown(&self) -> bool {
        !self.panels_hidden
    }

    /// Whether Layers is collapsed to its rail.
    pub fn layers_collapsed(&self) -> bool {
        self.layers_collapsed
    }

    pub fn properties_shown(&self) -> bool {
        !self.panels_hidden && !self.properties_hidden
    }

    pub fn panels_hidden(&self) -> bool {
        self.panels_hidden
    }

    pub fn set_layers_collapsed(&mut self, collapsed: bool) {
        self.layers_collapsed = collapsed;
    }

    pub fn toggle_layers(&mut self) {
        if self.panels_hidden {
            // Asking for a panel while both are hidden shows them, with Layers expanded.
            self.panels_hidden = false;
            self.layers_collapsed = false;
        } else {
            self.layers_collapsed = !self.layers_collapsed;
        }
    }

    pub fn toggle_properties(&mut self) {
        if self.panels_hidden {
            self.panels_hidden = false;
            self.properties_hidden = false;
        } else {
            self.properties_hidden = !self.properties_hidden;
        }
    }

    pub fn toggle_panels(&mut self) {
        self.panels_hidden = !self.panels_hidden;
    }

    /// The width a column takes in the layout, given its current state.
    pub fn layers_layout_width(&self) -> f32 {
        if self.layers_collapsed {
            metrics::LAYERS_PANEL_RAIL
        } else {
            self.layers_width
        }
    }
}

/// A panel's 34 pt header: `title` in the heading style and, on the right, whatever `trailing`
/// adds.
pub fn panel_header(ui: &mut egui::Ui, title: &str, trailing: impl FnOnce(&mut egui::Ui)) {
    let palette = theme::palette(ui.ctx());
    let width = ui.available_width();
    let header = ui.allocate_ui_with_layout(
        egui::vec2(width, metrics::PANEL_HEADER),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add_space(theme::SPACING[3]);
            ui.label(
                egui::RichText::new(title)
                    .size(theme::HEADING)
                    .strong()
                    .color(palette.text_primary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(theme::SPACING[2]);
                trailing(ui);
            });
        },
    );
    let rect = header.response.rect;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0, palette.border_subtle),
    );
}

/// Drags the strip on a column's inner edge, resizing it; a double click resets the width.
/// `panel` is the column's rect.
pub fn divider(ui: &mut egui::Ui, shell: &mut Shell, side: Side, panel: egui::Rect) {
    let x = match side {
        Side::Left => panel.right(),
        Side::Right => panel.left(),
    };
    let strip =
        egui::Rect::from_x_y_ranges((x - DIVIDER / 2.0)..=(x + DIVIDER / 2.0), panel.y_range());
    let id = ui.id().with(("shell divider", matches!(side, Side::Left)));
    let response = ui
        .interact(strip, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            match side {
                Side::Left => "Layers divider",
                Side::Right => "Properties divider",
            },
        )
    });
    if response.double_clicked() {
        shell.reset_width(side);
    } else if response.dragged() {
        let delta = response.drag_delta().x;
        let width = shell.width(side);
        shell.set_width(
            side,
            match side {
                Side::Left => width + delta,
                Side::Right => width - delta,
            },
        );
    }
    let palette = theme::palette(ui.ctx());
    let color = if response.hovered() || response.dragged() {
        palette.gold
    } else {
        palette.border_subtle
    };
    ui.painter()
        .vline(x, panel.y_range(), egui::Stroke::new(1.0, color));
}

/// The 40 pt rail Layers collapses to. Returns true when clicked, which expands Layers.
pub fn layers_rail(ui: &mut egui::Ui, path: &[String]) -> bool {
    let palette = theme::palette(ui.ctx());
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("layers rail"), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Expand Layers (⌘⌥1)")
    });
    let hovered = response.hovered();
    let tint = if hovered {
        palette.text_primary
    } else {
        palette.text_secondary
    };
    let top = rect.top() + metrics::PANEL_HEADER / 2.0;
    let slot = |y: f32| {
        egui::Rect::from_center_size(
            egui::pos2(rect.center().x, y),
            egui::Vec2::splat(icons::GRID + 8.0),
        )
    };
    icons::paint(ui, slot(top), Icon::CollapseRight, icons::GRID, tint);
    let layers_y = top + metrics::PANEL_HEADER;
    icons::paint(ui, slot(layers_y), Icon::LayerGroup, icons::GRID, tint);
    if !path.is_empty() {
        let text = path.join(" › ");
        let galley = ui.painter().layout_no_wrap(
            text,
            egui::FontId::proportional(theme::BODY),
            palette.text_secondary,
        );
        // Turned a quarter clockwise about its top-left corner, so it reads top to bottom
        // and sits in a column as wide as one line of text.
        let origin = egui::pos2(
            rect.center().x + galley.size().y / 2.0,
            layers_y + icons::GRID + theme::SPACING[3],
        );
        let shape = egui::epaint::TextShape::new(origin, galley, palette.text_secondary)
            .with_angle(std::f32::consts::FRAC_PI_2);
        ui.painter().add(shape);
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// The Selection's names from its Layer down, for the rail. Empty when nothing is selected;
/// several Objects show the count.
pub fn selection_path(map: &Map, selection: &[Id]) -> Vec<String> {
    match selection {
        [] => Vec::new(),
        [id] => {
            let mut path = vec![node_name(map, *id)];
            let mut node = *id;
            while let Some(parent) = map.parent(node) {
                path.push(node_name(map, parent));
                node = parent;
            }
            path.reverse();
            path
        }
        many => vec![format!("{} selected", many.len())],
    }
}

fn kind_noun(map: &Map, id: Id, plural: bool) -> &'static str {
    match (map.node(id), plural) {
        (Some(Node::Layer(_)), false) => "Layer",
        (Some(Node::Layer(_)), true) => "Layers",
        (Some(Node::Object(object)), plural) => match (&object.kind, plural) {
            (ObjectKind::Landmass(_), false) => "Landmass",
            (ObjectKind::Landmass(_), true) => "Landmasses",
            (ObjectKind::Lake(_), false) => "Lake",
            (ObjectKind::Lake(_), true) => "Lakes",
            (ObjectKind::Stamp(_), false) => "Stamp",
            (ObjectKind::Stamp(_), true) => "Stamps",
            (ObjectKind::Label(_), false) => "Label",
            (ObjectKind::Label(_), true) => "Labels",
        },
        (None, false) => "Object",
        (None, true) => "Objects",
    }
}

/// "2 Landmasses", "Stamp · Pine", or "Active Layer: Roads" when nothing is selected.
pub fn selection_summary(map: &Map, selection: &[Id], active_layer: Option<Id>) -> String {
    match selection {
        [] => active_layer
            .map(|layer| format!("Active Layer: {}", node_name(map, layer)))
            .unwrap_or_default(),
        [id] => {
            let noun = kind_noun(map, *id, false);
            let name = match map.node(*id) {
                Some(Node::Object(object)) => match &object.kind {
                    ObjectKind::Stamp(stamp) => Some(title_case(&stamp.asset.asset)),
                    ObjectKind::Label(_) | ObjectKind::Landmass(_) | ObjectKind::Lake(_) => {
                        Some(node_name(map, *id)).filter(|name| name != noun)
                    }
                },
                Some(Node::Layer(layer)) => Some(layer.name.clone()),
                None => None,
            };
            match name {
                Some(name) => format!("{noun} · {name}"),
                None => noun.to_owned(),
            }
        }
        [first, rest @ ..] => {
            let noun = kind_noun(map, *first, true);
            if rest.iter().all(|id| kind_noun(map, *id, true) == noun) {
                format!("{} {noun}", selection.len())
            } else {
                format!("{} Objects", selection.len())
            }
        }
    }
}

fn title_case(id: &str) -> String {
    let mut chars = id.replace(['-', '_'], " ").chars().collect::<Vec<_>>();
    if let Some(first) = chars.first_mut() {
        *first = first.to_ascii_uppercase();
    }
    chars.into_iter().collect()
}

/// A whole number with thousands separators.
fn grouped(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if value < 0 { format!("-{out}") } else { out }
}

/// A Canvas point in pixels and in the Map's real-world units:
/// "X 3,118 Y 1,886 px · 779 × 471 mi".
pub fn cursor_text(canvas: &Canvas, point: [f64; 2]) -> String {
    let unit = match canvas.scale.unit {
        DistanceUnit::Miles => "mi",
        DistanceUnit::Kilometres => "km",
    };
    let per_unit = canvas.scale.distance_per_unit;
    let whole = |value: f64| grouped(value.floor() as i64);
    format!(
        "X {} Y {} px · {} × {} {unit}",
        whole(point[0]),
        whole(point[1]),
        whole(point[0] * per_unit),
        whole(point[1] * per_unit),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssmap_core::Scale;

    #[test]
    fn widths_are_held_to_their_ranges_and_reset_to_the_defaults() {
        let mut shell = Shell::default();
        shell.set_width(Side::Left, 10.0);
        assert_eq!(shell.width(Side::Left), metrics::LAYERS_PANEL_MIN);
        shell.set_width(Side::Left, 9000.0);
        assert_eq!(shell.width(Side::Left), metrics::LAYERS_PANEL_MAX);
        shell.set_width(Side::Right, 10.0);
        assert_eq!(shell.width(Side::Right), metrics::PROPERTIES_PANEL_MIN);
        shell.set_width(Side::Right, 9000.0);
        assert_eq!(shell.width(Side::Right), metrics::PROPERTIES_PANEL_MAX);
        shell.reset_width(Side::Left);
        shell.reset_width(Side::Right);
        assert_eq!(shell, Shell::default());
    }

    #[test]
    fn stored_state_is_clamped_when_loaded_and_survives_a_round_trip() {
        let mut shell = Shell::default();
        shell.set_width(Side::Left, 300.0);
        shell.layers_collapsed = true;
        let json = serde_json::to_string(&shell).unwrap();
        assert_eq!(serde_json::from_str::<Shell>(&json).unwrap(), shell);
        let wild: Shell = serde_json::from_str(r#"{"layers_width": 5000}"#).unwrap();
        assert_eq!(wild.clamped().width(Side::Left), metrics::LAYERS_PANEL_MAX);
    }

    #[test]
    fn hiding_both_panels_keeps_what_the_user_collapsed() {
        let mut shell = Shell::default();
        shell.toggle_layers();
        shell.toggle_properties();
        shell.toggle_panels();
        assert!(!shell.layers_shown() && !shell.properties_shown());
        shell.toggle_panels();
        assert!(shell.layers_collapsed());
        assert!(shell.layers_shown() && !shell.properties_shown());
    }

    #[test]
    fn the_status_bar_gives_real_world_units_for_a_known_scale() {
        let canvas = Canvas::new(4096, 3072, Scale::new(0.25, DistanceUnit::Miles));
        assert_eq!(
            cursor_text(&canvas, [3118.4, 1886.9]),
            "X 3,118 Y 1,886 px · 779 × 471 mi"
        );
        let canvas = Canvas::new(100, 100, Scale::new(2.0, DistanceUnit::Kilometres));
        assert_eq!(
            cursor_text(&canvas, [-5.0, 12.0]),
            "X -5 Y 12 px · -10 × 24 km"
        );
    }

    #[test]
    fn grouping_puts_separators_every_three_digits() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(-1234567), "-1,234,567");
    }
}
