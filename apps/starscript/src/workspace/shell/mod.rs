//! The window shell around a Map tab: the Layers and Properties columns, their widths and
//! collapse state (the same for every tab, kept across launches), and the text of the status
//! bar.

use serde::{Deserialize, Serialize};

use crate::icons::Icon;
use crate::theme::metrics;

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
    // A layout saved while the Assets section existed loads as Layers: the Asset picker is a
    // modal now.
    #[default]
    #[serde(alias = "Assets")]
    Layers,
    Map,
    Boards,
    History,
}

impl SidebarSection {
    /// Every section, in the order of the icon strip.
    pub const ALL: [Self; 4] = [Self::Layers, Self::Map, Self::Boards, Self::History];

    pub fn icon(self) -> Icon {
        match self {
            Self::Layers => Icon::LayerGroup,
            Self::Map => Icon::Map,
            Self::Boards => Icon::ExportRegion,
            Self::History => Icon::History,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Layers => "Layers",
            Self::Map => "Map",
            Self::Boards => "Art boards",
            Self::History => "History",
        }
    }

    /// The accessible label and tooltip: the name and the shortcut.
    pub fn label(self) -> &'static str {
        match self {
            Self::Layers => "Layers (⌘⌥1)",
            Self::Map => "Map (⌘⌥6)",
            Self::Boards => "Art boards (⌘⌥3)",
            Self::History => "History (⌘⌥5)",
        }
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
    /// How many boards the shown Map has, to offer the Art boards section only when there are
    /// Art boards besides the Canvas. Set each frame by the window.
    #[serde(skip, default = "one_board")]
    board_count: usize,
    layers_width: f32,
    properties_width: f32,
    /// The user collapsed Layers to its rail.
    layers_collapsed: bool,
    /// The user collapsed Properties to its icon strip.
    #[serde(alias = "properties_hidden")]
    properties_collapsed: bool,
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

fn one_board() -> usize {
    1
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
            board_count: one_board(),
            layers_width: metrics::LAYERS_PANEL_DEFAULT,
            properties_width: metrics::PROPERTIES_PANEL_DEFAULT,
            layers_collapsed: true,
            properties_collapsed: false,
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

    #[allow(clippy::expect_used, reason = "shell state serialises")]
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

    /// Whether the sidebar offers `section` now: Art boards only once the Map has some.
    pub fn section_available(&self, section: SidebarSection) -> bool {
        section != SidebarSection::Boards || self.board_count > 1
    }

    /// Tells the sidebar how many boards the shown Map has. If the Art boards section was
    /// showing and the Map has none now (deleted, undone, or another tab), it goes back to
    /// Layers.
    pub fn set_board_count(&mut self, count: usize) {
        self.board_count = count;
        if !self.section_available(self.sidebar_section) {
            self.sidebar_section = SidebarSection::Layers;
        }
    }

    /// Switches the sidebar to `section`, if it is available.
    pub fn set_sidebar_section(&mut self, section: SidebarSection) {
        if self.section_available(section) {
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

    /// Whether the Properties column shows, as the full panel or as the icon strip.
    pub fn properties_shown(&self) -> bool {
        !self.panels_hidden
    }

    /// Whether Properties is collapsed to its icon strip.
    pub fn properties_collapsed(&self) -> bool {
        self.properties_collapsed
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
            self.properties_collapsed = false;
        } else if self.auto_rail(Side::Right) {
            self.set_properties_overlay(!self.properties_overlay);
        } else {
            self.properties_collapsed = !self.properties_collapsed;
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
            let properties = if self.properties_collapsed || window_width < MINIMUM_WIDTH {
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

    /// The Properties strip was clicked. If the user collapsed Properties and the window has
    /// room, that expands it; otherwise the window is too narrow and it opens over the Canvas.
    pub fn click_properties_rail(&mut self) {
        let mut expanded = self.clone();
        expanded.properties_collapsed = false;
        if expanded.layout(self.window_width).properties == PanelMode::Rail {
            self.set_properties_overlay(!self.properties_overlay);
        } else {
            self.properties_collapsed = false;
        }
    }

    pub fn close_overlays(&mut self) {
        self.layers_overlay = false;
        self.properties_overlay = false;
    }

    pub fn overlay_open(&self) -> bool {
        self.layers_overlay || self.properties_overlay
    }

    /// Whether the window is too narrow for the panel on `side` to show in full, whether or not
    /// the user collapsed it.
    fn auto_rail(&self, side: Side) -> bool {
        let mut expanded = self.clone();
        match side {
            Side::Left => {
                expanded.layers_collapsed = false;
                expanded.current_layout().layers == PanelMode::Rail
            }
            Side::Right => {
                expanded.properties_collapsed = false;
                expanded.current_layout().properties == PanelMode::Rail
            }
        }
    }
}

mod status;
#[cfg(test)]
mod tests;
mod widgets;

pub use status::{cursor_text, selection_path, selection_summary};
pub use widgets::{
    collapse_button, divider, history_list, layers_rail, panel_header, properties_collapse_button,
    properties_rail,
};
