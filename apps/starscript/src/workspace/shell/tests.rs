use super::status::grouped;
use super::*;
use ssmap_core::{Canvas, DistanceUnit, Scale};

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
fn the_map_section_is_wider_by_default_and_keeps_its_own_width() {
    let mut shell = Shell::default();
    assert_eq!(shell.width(Side::Left), metrics::LAYERS_PANEL_DEFAULT);
    shell.set_sidebar_section(SidebarSection::Map);
    assert_eq!(shell.width(Side::Left), metrics::MAP_PANEL_DEFAULT);
    shell.set_width(Side::Left, 9000.0);
    assert_eq!(shell.width(Side::Left), metrics::MAP_PANEL_MAX);
    shell.reset_width(Side::Left);
    assert_eq!(shell.width(Side::Left), metrics::MAP_PANEL_DEFAULT);
    // Resizing it leaves the other sections' width alone.
    shell.set_width(Side::Left, 400.0);
    shell.set_sidebar_section(SidebarSection::Layers);
    assert_eq!(shell.width(Side::Left), metrics::LAYERS_PANEL_DEFAULT);
    // In a narrower window the Map section still gets more than the other sections.
    shell.layers_collapsed = false;
    shell.set_sidebar_section(SidebarSection::Layers);
    let narrow = shell.layout(1280.0).layers.width();
    shell.set_sidebar_section(SidebarSection::Map);
    assert!(shell.layout(1280.0).layers.width() > narrow);
    // A layout saved before the Map section existed gets the default.
    let old: Shell = serde_json::from_str(r#"{"layers_width": 300}"#).unwrap();
    assert_eq!(old.map_width, metrics::MAP_PANEL_DEFAULT);
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

fn expanded() -> Shell {
    let mut shell = Shell::default();
    shell.set_layers_collapsed(false);
    shell
}

#[test]
fn the_left_sidebar_starts_collapsed_to_its_rail() {
    let shell = Shell::default();
    assert!(shell.layers_collapsed());
    assert_eq!(shell.layout(1500.0).layers, PanelMode::Rail);
}

#[test]
fn hiding_both_panels_keeps_what_the_user_collapsed() {
    let mut shell = expanded();
    shell.toggle_layers();
    shell.toggle_properties();
    shell.toggle_panels();
    assert!(!shell.layers_shown() && !shell.properties_shown());
    shell.toggle_panels();
    assert!(shell.layers_collapsed());
    assert!(shell.layers_shown() && shell.properties_collapsed());
}

fn full(width: f32) -> PanelMode {
    PanelMode::Full(width)
}

#[test]
fn at_1360_and_wider_both_panels_show_in_full() {
    let layout = expanded().layout(1360.0);
    assert_eq!(layout.layers, full(232.0));
    assert_eq!(layout.properties, full(288.0));
    assert!(layout.layers_footer && layout.real_world_coordinates);
    assert_eq!(expanded().layout(2400.0).layers, full(232.0));
}

#[test]
fn from_1180_to_1359_layers_narrows_and_loses_its_footer() {
    for width in [1180.0, 1359.0] {
        let layout = expanded().layout(width);
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
    let mut shell = expanded();
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
    let mut shell = expanded();
    shell.toggle_layers();
    shell.toggle_properties();
    for width in [1024.0, 1250.0, 1400.0, 3000.0] {
        let layout = shell.layout(width);
        assert_eq!(layout.layers, PanelMode::Rail, "{width}");
        assert_eq!(layout.properties, PanelMode::Rail, "{width}");
    }
    shell.toggle_panels();
    assert_eq!(shell.layout(1500.0).layers, PanelMode::Hidden);
}

#[test]
fn narrowing_the_window_leaves_the_users_state_alone() {
    let shell = expanded();
    let before = shell.clone();
    let _ = shell.layout(1024.0);
    assert_eq!(shell, before);
    // And growing back restores the full layout.
    assert_eq!(shell.layout(1024.0).layers, PanelMode::Rail);
    assert_eq!(shell.layout(1400.0).layers, full(232.0));
}

#[test]
fn the_layers_shortcut_opens_the_overlay_when_the_window_is_too_narrow() {
    let mut shell = expanded();
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
fn a_shortcut_opens_the_overlay_on_the_first_press_while_the_sidebar_is_collapsed() {
    // The sidebar starts collapsed. In a window too narrow for it, the shortcut opens it over
    // the Canvas at once and leaves the user's collapse alone.
    let mut shell = Shell::default();
    assert!(shell.layers_collapsed());
    shell.set_window_width(1100.0);
    shell.toggle_layers();
    assert!(shell.layers_overlay() && shell.layers_collapsed());
    shell.toggle_layers();
    assert!(!shell.layers_overlay() && shell.layers_collapsed());

    for section in SidebarSection::ALL {
        let mut shell = Shell::default();
        shell.set_window_width(1100.0);
        // Art boards is offered once the Map has a board besides the Canvas.
        shell.board_count = 2;
        shell.show_section(section);
        assert!(
            shell.layers_overlay() && shell.layers_collapsed(),
            "{section:?}"
        );
        assert_eq!(shell.sidebar_section(), section);
    }

    // The same for Properties on the right.
    // Properties gives way to a strip below 1100.
    let mut shell = Shell::default();
    shell.set_window_width(1000.0);
    shell.properties_collapsed = true;
    shell.toggle_properties();
    assert!(shell.properties_overlay() && shell.properties_collapsed());
    shell.toggle_properties();
    assert!(!shell.properties_overlay() && shell.properties_collapsed());
}

#[test]
fn in_a_window_with_room_a_shortcut_still_expands_a_collapsed_sidebar() {
    let mut shell = Shell::default();
    shell.set_window_width(1500.0);
    shell.toggle_layers();
    assert!(!shell.layers_collapsed() && !shell.layers_overlay());
    shell.set_layers_collapsed(true);
    shell.show_section(SidebarSection::Boards);
    assert!(!shell.layers_collapsed() && !shell.layers_overlay());
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
