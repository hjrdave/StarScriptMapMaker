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

/// A section of the left sidebar. One shows at a time; the icon strip switches between them.
/// A new section is a variant here and one row in [`SidebarSection::ALL`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarSection {
    #[default]
    Layers,
    History,
}

impl SidebarSection {
    /// Every section, in the order of the icon strip.
    pub const ALL: [Self; 2] = [Self::Layers, Self::History];

    pub fn icon(self) -> Icon {
        match self {
            Self::Layers => Icon::LayerGroup,
            Self::History => Icon::History,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Layers => "Layers",
            Self::History => "History",
        }
    }

    /// The accessible label and tooltip: the name and the shortcut.
    pub fn label(self) -> &'static str {
        match self {
            Self::Layers => "Layers (⌘⌥1)",
            Self::History => "History (⌘⌥5)",
        }
    }

    /// Whether the section is offered now. Sections that depend on the Map (such as Art
    /// boards) will return false when they have nothing to show.
    pub fn available(self) -> bool {
        true
    }
}

/// What a click on the Layers rail asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailClick {
    /// The collapse control: expand the sidebar to the section it last showed.
    Expand,
    /// A section icon: expand the sidebar to that section.
    Section(SidebarSection),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shell {
    /// The sidebar section the left bar shows.
    sidebar_section: SidebarSection,
    layers_width: f32,
    properties_width: f32,
    /// The user collapsed Layers to its rail.
    layers_collapsed: bool,
    /// The user hid Properties.
    properties_hidden: bool,
    /// ⇥ hid both panels. Kept apart from the two flags above so that showing them again
    /// restores exactly what the user had collapsed.
    panels_hidden: bool,
    /// The window's width, in points, as of the last frame.
    #[serde(skip, default = "default_window_width")]
    window_width: f32,
    /// A rail was clicked and its panel shows over the Canvas. Closes on an outside click
    /// or Esc.
    #[serde(skip)]
    layers_overlay: bool,
    #[serde(skip)]
    properties_overlay: bool,
}

fn default_window_width() -> f32 {
    FULL_WIDTH
}

/// The window width, in points, from which both panels show in full.
pub const FULL_WIDTH: f32 = 1360.0;
/// From here Layers narrows; below it, Layers is the rail.
pub const NARROW_WIDTH: f32 = 1180.0;
/// Below this Properties is a rail too, opening as a floating inspector. The window cannot be
/// made smaller than `theme::metrics::MIN_WINDOW`.
pub const MINIMUM_WIDTH: f32 = 1100.0;
/// The Canvas is never narrower than this, in points.
pub const CANVAS_MIN_WIDTH: f32 = 480.0;
/// The Layers width in the 1180–1359 band.
const LAYERS_NARROW: f32 = 200.0;

/// How a panel takes part in the layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelMode {
    /// A column of this width.
    Full(f32),
    /// The 40 pt rail.
    Rail,
    /// Not shown at all.
    Hidden,
}

impl PanelMode {
    /// The width the panel takes from the Canvas.
    pub fn width(self) -> f32 {
        match self {
            Self::Full(width) => width,
            Self::Rail => metrics::LAYERS_PANEL_RAIL,
            Self::Hidden => 0.0,
        }
    }
}

/// What the window shows at a given width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub layers: PanelMode,
    pub properties: PanelMode,
    /// The Object and Layer counts under the Layers list.
    pub layers_footer: bool,
    /// Real-world distances in the status bar.
    pub real_world_coordinates: bool,
}

impl Layout {
    /// The width left to the Canvas in a window `window_width` wide.
    pub fn canvas_width(&self, window_width: f32) -> f32 {
        window_width - self.layers.width() - self.properties.width()
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            sidebar_section: SidebarSection::default(),
            layers_width: metrics::LAYERS_PANEL_DEFAULT,
            properties_width: metrics::PROPERTIES_PANEL_DEFAULT,
            layers_collapsed: false,
            properties_hidden: false,
            panels_hidden: false,
            window_width: FULL_WIDTH,
            layers_overlay: false,
            properties_overlay: false,
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

    /// The section the sidebar shows.
    pub fn sidebar_section(&self) -> SidebarSection {
        self.sidebar_section
    }

    /// Switches the sidebar to `section`, if it is available.
    pub fn set_sidebar_section(&mut self, section: SidebarSection) {
        if section.available() {
            self.sidebar_section = section;
        }
    }

    /// Shows `section` in the sidebar, bringing the sidebar back first if it is collapsed or
    /// hidden. In a window too narrow for the panel it opens over the Canvas.
    pub fn show_section(&mut self, section: SidebarSection) {
        self.set_sidebar_section(section);
        if self.panels_hidden {
            self.panels_hidden = false;
            self.layers_collapsed = false;
        } else if self.auto_rail(Side::Left) {
            self.set_layers_overlay(true);
        } else {
            self.layers_collapsed = false;
        }
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
        } else if self.auto_rail(Side::Left) {
            // The window is too narrow for the panel: the shortcut opens it over the Canvas.
            self.set_layers_overlay(!self.layers_overlay);
        } else {
            self.layers_collapsed = !self.layers_collapsed;
        }
    }

    pub fn toggle_properties(&mut self) {
        if self.panels_hidden {
            self.panels_hidden = false;
            self.properties_hidden = false;
        } else if self.auto_rail(Side::Right) {
            self.set_properties_overlay(!self.properties_overlay);
        } else {
            self.properties_hidden = !self.properties_hidden;
        }
    }

    pub fn toggle_panels(&mut self) {
        self.panels_hidden = !self.panels_hidden;
    }

    /// What the window shows at `window_width`: a pure function of the width and of what the
    /// user chose. A panel the user collapsed stays collapsed at every width; the narrower
    /// bands collapse panels only in the layout, never in the user's state, so growing the
    /// window brings back exactly what they had.
    pub fn layout(&self, window_width: f32) -> Layout {
        let (mut layers, mut properties) = if self.panels_hidden {
            (PanelMode::Hidden, PanelMode::Hidden)
        } else {
            let layers = if self.layers_collapsed || window_width < NARROW_WIDTH {
                PanelMode::Rail
            } else if window_width < FULL_WIDTH {
                PanelMode::Full(LAYERS_NARROW)
            } else {
                PanelMode::Full(self.layers_width)
            };
            let properties = if self.properties_hidden {
                PanelMode::Hidden
            } else if window_width < MINIMUM_WIDTH {
                PanelMode::Rail
            } else {
                PanelMode::Full(self.properties_width)
            };
            (layers, properties)
        };
        // The Canvas keeps its room: Properties gives way first, then Layers.
        let canvas = |layers: PanelMode, properties: PanelMode| {
            window_width - layers.width() - properties.width()
        };
        if canvas(layers, properties) < CANVAS_MIN_WIDTH && matches!(properties, PanelMode::Full(_))
        {
            properties = PanelMode::Rail;
        }
        if canvas(layers, properties) < CANVAS_MIN_WIDTH && matches!(layers, PanelMode::Full(_)) {
            layers = PanelMode::Rail;
        }
        Layout {
            layers,
            properties,
            layers_footer: window_width >= FULL_WIDTH,
            real_world_coordinates: window_width >= NARROW_WIDTH,
        }
    }

    /// Records the window's width at the start of a frame.
    pub fn set_window_width(&mut self, width: f32) {
        self.window_width = width;
    }

    /// The layout at the window's current width.
    pub fn current_layout(&self) -> Layout {
        self.layout(self.window_width)
    }

    pub fn layers_overlay(&self) -> bool {
        self.layers_overlay
    }

    pub fn properties_overlay(&self) -> bool {
        self.properties_overlay
    }

    pub fn set_layers_overlay(&mut self, open: bool) {
        self.layers_overlay = open;
        if open {
            self.properties_overlay = false;
        }
    }

    pub fn set_properties_overlay(&mut self, open: bool) {
        self.properties_overlay = open;
        if open {
            self.layers_overlay = false;
        }
    }

    /// The Layers rail was clicked. If the user collapsed Layers and the window has room, that
    /// expands it; otherwise the window is too narrow and Layers opens over the Canvas.
    pub fn click_layers_rail(&mut self, click: RailClick) {
        if let RailClick::Section(section) = click {
            self.set_sidebar_section(section);
        }
        let mut expanded = self.clone();
        expanded.layers_collapsed = false;
        if expanded.layout(self.window_width).layers == PanelMode::Rail {
            self.set_layers_overlay(!self.layers_overlay);
        } else {
            self.layers_collapsed = false;
        }
    }

    pub fn close_overlays(&mut self) {
        self.layers_overlay = false;
        self.properties_overlay = false;
    }

    pub fn overlay_open(&self) -> bool {
        self.layers_overlay || self.properties_overlay
    }

    /// Whether the window's width, not the user, makes the panel on `side` a rail.
    fn auto_rail(&self, side: Side) -> bool {
        let layout = self.current_layout();
        match side {
            Side::Left => !self.layers_collapsed && layout.layers == PanelMode::Rail,
            Side::Right => !self.properties_hidden && layout.properties == PanelMode::Rail,
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

/// The undo history as a list: "Original" and then every step, oldest first. `applied` steps
/// are in effect; the rest are dimmed and can be redone. Returns how many steps should be
/// applied after a click on an entry.
pub fn history_list(ui: &mut egui::Ui, steps: &[String], applied: usize) -> Option<usize> {
    let palette = theme::palette(ui.ctx());
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let entries = std::iter::once("Original").chain(steps.iter().map(String::as_str));
            for (position, label) in entries.enumerate() {
                let color = if position > applied {
                    palette.text_secondary
                } else {
                    palette.text_primary
                };
                let text = egui::RichText::new(label).color(color);
                let response = ui.add_sized(
                    [ui.available_width(), metrics::PANEL_HEADER * 0.8],
                    egui::Button::selectable(position == applied, text).frame_when_inactive(false),
                );
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                    && position != applied
                {
                    clicked = Some(position);
                }
            }
        });
    clicked
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

/// The Layers collapse control: a double chevron that points left while the panel is
/// expanded and is mirrored to point right once `collapsed`. Its place is the bottom of the
/// panel in both states.
pub fn collapse_button(ui: &mut egui::Ui, collapsed: bool) -> egui::Response {
    let label = if collapsed {
        "Expand Layers (⌘⌥1)"
    } else {
        "Collapse Layers (⌘⌥1)"
    };
    icons::icon_button_flipped(ui, Icon::CollapseDouble, label, collapsed)
}

/// The 40 pt rail Layers collapses to: only the section icons, with the expand control pinned
/// to the bottom. Returns what was clicked, which expands the sidebar.
pub fn layers_rail(
    ui: &mut egui::Ui,
    path: &[String],
    current: SidebarSection,
) -> Option<RailClick> {
    let palette = theme::palette(ui.ctx());
    let rect = ui.max_rect();
    let mut clicked = None;
    let step = icons::GRID + 8.0 + theme::SPACING[1];
    let mut layers_y = rect.top() + metrics::PANEL_HEADER / 2.0;
    let mut count = 0.0;
    for section in SidebarSection::ALL.into_iter().filter(|s| s.available()) {
        let slot = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, layers_y + count * step),
            egui::Vec2::splat(icons::GRID + 8.0),
        );
        let response = ui.interact(
            slot,
            ui.id().with(("sidebar section", section.title())),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, section.label())
        });
        let tint = if section == current || response.hovered() {
            palette.text_primary
        } else {
            palette.text_secondary
        };
        icons::paint(ui, slot, section.icon(), icons::GRID, tint);
        if response
            .on_hover_text(section.label())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            clicked = Some(RailClick::Section(section));
        }
        count += 1.0;
    }
    layers_y += (count - 1.0).max(0.0) * step;
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
    let bar = egui::Rect::from_min_max(
        egui::pos2(
            rect.left(),
            rect.bottom() - crate::layer_tree::FOOTER_HEIGHT,
        ),
        rect.right_bottom(),
    );
    let mut bottom =
        ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(
            egui::Layout::top_down(egui::Align::Center).with_main_align(egui::Align::Center),
        ));
    if collapse_button(&mut bottom, true)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        clicked = Some(RailClick::Expand);
    }
    clicked
}

/// The 40 pt rail Properties narrows to at the smallest window. Returns true when clicked,
/// which opens the floating inspector.
pub fn properties_rail(ui: &mut egui::Ui) -> bool {
    let palette = theme::palette(ui.ctx());
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("properties rail"), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Open Properties (⌘⌥2)")
    });
    let tint = if response.hovered() {
        palette.text_primary
    } else {
        palette.text_secondary
    };
    let top = rect.top() + metrics::PANEL_HEADER / 2.0;
    for (i, icon) in [Icon::CollapseLeft, Icon::Shape].into_iter().enumerate() {
        let slot = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, top + i as f32 * metrics::PANEL_HEADER),
            egui::Vec2::splat(icons::GRID + 8.0),
        );
        icons::paint(ui, slot, icon, icons::GRID, tint);
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
/// "X 3,118 Y 1,886 px · 779 × 471 mi". Without `real_world` only the pixels are given.
pub fn cursor_text(canvas: &Canvas, point: [f64; 2], real_world: bool) -> String {
    let unit = match canvas.scale.unit {
        DistanceUnit::Miles => "mi",
        DistanceUnit::Kilometres => "km",
    };
    let per_unit = canvas.scale.distance_per_unit;
    let whole = |value: f64| grouped(value.floor() as i64);
    if !real_world {
        return format!("X {} Y {} px", whole(point[0]), whole(point[1]));
    }
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
    fn the_sidebar_section_defaults_to_layers_and_is_stored() {
        let old: Shell = serde_json::from_str(r#"{"layers_width": 300}"#).unwrap();
        assert_eq!(old.sidebar_section(), SidebarSection::Layers);
        let json = serde_json::to_string(&Shell::default()).unwrap();
        assert!(json.contains("sidebar_section"));
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

    fn full(width: f32) -> PanelMode {
        PanelMode::Full(width)
    }

    #[test]
    fn at_1360_and_wider_both_panels_show_in_full() {
        let layout = Shell::default().layout(1360.0);
        assert_eq!(layout.layers, full(232.0));
        assert_eq!(layout.properties, full(288.0));
        assert!(layout.layers_footer && layout.real_world_coordinates);
        assert_eq!(Shell::default().layout(2400.0).layers, full(232.0));
    }

    #[test]
    fn from_1180_to_1359_layers_narrows_and_loses_its_footer() {
        for width in [1180.0, 1359.0] {
            let layout = Shell::default().layout(width);
            assert_eq!(layout.layers, full(200.0));
            assert_eq!(layout.properties, full(288.0));
            assert!(!layout.layers_footer && layout.real_world_coordinates);
        }
    }

    #[test]
    fn from_1024_to_1179_layers_is_the_rail_and_the_status_bar_drops_distances() {
        for width in [1100.0, 1179.0] {
            let layout = Shell::default().layout(width);
            assert_eq!(layout.layers, PanelMode::Rail);
            assert_eq!(layout.properties, full(288.0));
            assert!(!layout.real_world_coordinates);
        }
    }

    #[test]
    fn at_the_minimum_both_panels_are_rails() {
        let layout = Shell::default().layout(1024.0);
        assert_eq!(layout.layers, PanelMode::Rail);
        assert_eq!(layout.properties, PanelMode::Rail);
        assert_eq!(layout.canvas_width(1024.0), 944.0);
    }

    #[test]
    fn the_canvas_keeps_480_points() {
        let mut shell = Shell::default();
        shell.set_width(Side::Right, 360.0);
        for width in [600.0, 700.0, 900.0, 1024.0, 1250.0, 1500.0] {
            let layout = shell.layout(width);
            assert!(layout.canvas_width(width) >= CANVAS_MIN_WIDTH, "{width}");
        }
        // Properties gives way before Layers.
        let layout = shell.layout(1360.0);
        assert_eq!(
            (layout.layers, layout.properties),
            (full(232.0), full(360.0))
        );
        let layout = Shell::default().layout(700.0);
        assert_eq!(layout.layers, PanelMode::Rail);
    }

    #[test]
    fn a_panel_the_user_collapsed_stays_collapsed_when_the_window_grows() {
        let mut shell = Shell::default();
        shell.toggle_layers();
        shell.toggle_properties();
        for width in [1024.0, 1250.0, 1400.0, 3000.0] {
            let layout = shell.layout(width);
            assert_eq!(layout.layers, PanelMode::Rail, "{width}");
            assert_eq!(layout.properties, PanelMode::Hidden, "{width}");
        }
        shell.toggle_panels();
        assert_eq!(shell.layout(1500.0).layers, PanelMode::Hidden);
    }

    #[test]
    fn narrowing_the_window_leaves_the_users_state_alone() {
        let shell = Shell::default();
        let before = shell.clone();
        let _ = shell.layout(1024.0);
        assert_eq!(shell, before);
        // And growing back restores the full layout.
        assert_eq!(shell.layout(1024.0).layers, PanelMode::Rail);
        assert_eq!(shell.layout(1400.0).layers, full(232.0));
    }

    #[test]
    fn the_layers_shortcut_opens_the_overlay_when_the_window_is_too_narrow() {
        let mut shell = Shell::default();
        shell.set_window_width(1100.0);
        shell.toggle_layers();
        assert!(shell.layers_overlay() && !shell.layers_collapsed());
        shell.toggle_layers();
        assert!(!shell.layers_overlay());
        shell.set_window_width(1400.0);
        shell.toggle_layers();
        assert!(shell.layers_collapsed() && !shell.layers_overlay());
    }

    #[test]
    fn only_one_overlay_is_open_at_a_time() {
        let mut shell = Shell::default();
        shell.set_layers_overlay(true);
        shell.set_properties_overlay(true);
        assert!(shell.properties_overlay() && !shell.layers_overlay());
        shell.close_overlays();
        assert!(!shell.overlay_open());
    }

    #[test]
    fn the_status_bar_gives_real_world_units_for_a_known_scale() {
        let canvas = Canvas::new(4096, 3072, Scale::new(0.25, DistanceUnit::Miles));
        assert_eq!(
            cursor_text(&canvas, [3118.4, 1886.9], true),
            "X 3,118 Y 1,886 px · 779 × 471 mi"
        );
        assert_eq!(
            cursor_text(&canvas, [3118.4, 1886.9], false),
            "X 3,118 Y 1,886 px"
        );
        let canvas = Canvas::new(100, 100, Scale::new(2.0, DistanceUnit::Kilometres));
        assert_eq!(
            cursor_text(&canvas, [-5.0, 12.0], true),
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
