//! The window shell: the left sidebar's rail and the panel it opens, collapsing Properties,
//! hiding both with Tab, divider drags and the status bar (no window needed).

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ssmap_app::MapMakerApp;
use ssmap_app::shell::Side;
use ssmap_app::theme::metrics;
use ssmap_core::{Canvas, DistanceUnit, Scale};

/// Wide enough for both panels in full.
const WIDTH: f32 = 1440.0;

/// The Properties header with nothing selected and the Select tool active: the tool's name.
const PANEL_HEADER: &str = "Select";

fn app() -> Harness<'static, MapMakerApp> {
    app_with_step(0.25)
}

fn app_with_step(step_dt: f32) -> Harness<'static, MapMakerApp> {
    app_sized(step_dt, WIDTH)
}

fn app_sized(step_dt: f32, width: f32) -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_step_dt(step_dt)
        .with_size(egui::vec2(width, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    // The app starts on Home with no Map; these tests need one open for the panels.
    harness.state_mut().new_map(Canvas::new(
        4096,
        3072,
        Scale::new(0.25, DistanceUnit::Miles),
    ));
    harness.run();
    // The sidebar's panel starts closed, with only the rail; most tests here begin with it
    // open. A narrow window shows the rail whatever the user chose.
    if width >= 1180.0 {
        harness.get_by_label(RAIL_LAYERS).click();
        harness.run();
        // Let the panel finish opening, as the step is tiny in some tests.
        for _ in 0..60 {
            harness.step();
        }
    }
    harness
}

/// The Layers icon on the sidebar's rail.
const RAIL_LAYERS: &str = "Layers (⌘⌥1)";

const BOTH: egui::Modifiers = egui::Modifiers {
    alt: true,
    command: true,
    ctrl: false,
    shift: false,
    mac_cmd: false,
};

#[test]
fn the_layers_shortcut_closes_the_panel_and_opens_it_again() {
    let mut harness = app();
    assert!(harness.query_by_label("New Layer (⇧⌘N)").is_some());
    harness.key_press_modifiers(BOTH, egui::Key::Num1);
    harness.run();
    assert!(harness.state().shell().layers_collapsed());
    assert!(harness.query_by_label("New Layer (⇧⌘N)").is_none());
    // The rail stays on screen with the panel closed.
    assert!(harness.query_by_label(RAIL_LAYERS).is_some());
    harness.key_press_modifiers(BOTH, egui::Key::Num1);
    harness.run();
    assert!(!harness.state().shell().layers_collapsed());
}

#[test]
fn the_active_rail_icon_closes_the_panel_and_an_icon_opens_its_section() {
    let mut harness = app();
    // The open section's icon closes the panel; there is no collapse button any more.
    assert!(harness.query_by_label("Collapse Layers (⌘⌥1)").is_none());
    harness.get_by_label(RAIL_LAYERS).click();
    harness.run();
    assert!(harness.state().shell().layers_collapsed());
    harness.get_by_label(RAIL_LAYERS).click();
    harness.run();
    assert!(!harness.state().shell().layers_collapsed());
    // Another icon switches the open panel to its section instead of closing it.
    harness.get_by_label("History (⌘⌥5)").click();
    harness.run();
    assert!(!harness.state().shell().layers_collapsed());
    assert_eq!(
        harness.state().shell().sidebar_section(),
        ssmap_app::shell::SidebarSection::History
    );
}

#[test]
fn the_rail_stays_put_when_the_panel_opens_and_closes_and_the_canvas_makes_room() {
    let mut harness = app();
    let open = harness.get_by_label(RAIL_LAYERS).rect();
    let wide = harness.state().shell().current_layout().canvas_width(WIDTH);
    harness.get_by_label(RAIL_LAYERS).click();
    harness.run();
    assert_eq!(harness.get_by_label(RAIL_LAYERS).rect(), open);
    let narrow = harness.state().shell().current_layout().canvas_width(WIDTH);
    // The panel pushes the Canvas over rather than floating above it.
    assert!(narrow > wide, "{narrow} {wide}");
}

#[test]
fn the_properties_shortcut_hides_and_shows_properties() {
    let mut harness = app();
    assert!(harness.query_by_label(PANEL_HEADER).is_some());
    harness.key_press_modifiers(BOTH, egui::Key::Num2);
    harness.run();
    assert!(harness.state().shell().properties_collapsed());
    assert!(harness.query_by_label(PANEL_HEADER).is_none());
    harness.key_press_modifiers(BOTH, egui::Key::Num2);
    harness.run();
    assert!(!harness.state().shell().properties_collapsed());
    assert!(harness.query_by_label(PANEL_HEADER).is_some());
}

#[test]
fn the_collapse_button_collapses_properties_to_a_strip_and_the_strip_expands_it() {
    let mut harness = app();
    harness.get_by_label("Collapse Properties (⌘⌥2)").click();
    harness.run();
    assert!(harness.state().shell().properties_collapsed());
    assert!(harness.query_by_label(PANEL_HEADER).is_none());
    assert!(harness.query_by_label("Open Properties (⌘⌥2)").is_some());
    harness.get_by_label("Expand Properties (⌘⌥2)").click();
    harness.run();
    assert!(!harness.state().shell().properties_collapsed());
    assert!(harness.query_by_label(PANEL_HEADER).is_some());
}

#[test]
fn clicking_the_properties_icon_expands_the_strip() {
    let mut harness = app();
    harness.key_press_modifiers(BOTH, egui::Key::Num2);
    harness.run();
    harness.get_by_label("Open Properties (⌘⌥2)").click();
    harness.run();
    assert!(!harness.state().shell().properties_collapsed());
    assert!(
        harness
            .query_by_label("Collapse Properties (⌘⌥2)")
            .is_some()
    );
}

#[test]
fn the_properties_control_sits_at_the_bottom_and_stays_put_when_toggled() {
    let mut harness = app();
    let expanded = harness.get_by_label("Collapse Properties (⌘⌥2)").rect();
    assert!(expanded.center().y > 600.0);
    harness.get_by_label("Collapse Properties (⌘⌥2)").click();
    harness.run();
    let collapsed = harness.get_by_label("Expand Properties (⌘⌥2)").rect();
    assert_eq!(expanded.center().y, collapsed.center().y);
}

#[test]
fn tab_hides_both_panels_and_brings_back_what_was_collapsed() {
    let mut harness = app();
    harness.key_press_modifiers(BOTH, egui::Key::Num1);
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(harness.state().shell().panels_hidden());
    assert!(harness.query_by_label(PANEL_HEADER).is_none());
    assert!(harness.query_by_label(RAIL_LAYERS).is_none());
    harness.key_press(egui::Key::Tab);
    harness.run();
    // Layers is back as the rail it was collapsed to, not as the full panel.
    assert!(harness.state().shell().layers_shown());
    assert!(harness.state().shell().layers_collapsed());
    assert!(harness.query_by_label(PANEL_HEADER).is_some());
}

#[test]
fn dragging_a_divider_resizes_within_its_range() {
    let mut harness = app();
    let y = 400.0;
    // The panel's edge is just right of the rail at the window's left edge.
    let rail = metrics::LAYERS_PANEL_RAIL;
    drag(
        &mut harness,
        rail + metrics::LAYERS_PANEL_DEFAULT,
        5000.0,
        y,
    );
    assert_eq!(
        harness.state().shell().width(Side::Left),
        metrics::LAYERS_PANEL_MAX
    );
    drag(&mut harness, rail + metrics::LAYERS_PANEL_MAX, 0.0, y);
    assert_eq!(
        harness.state().shell().width(Side::Left),
        metrics::LAYERS_PANEL_MIN
    );
    // Properties is anchored to the right edge of the window.
    let edge = WIDTH - metrics::PROPERTIES_PANEL_DEFAULT;
    drag(&mut harness, edge, 0.0, y);
    assert_eq!(
        harness.state().shell().width(Side::Right),
        metrics::PROPERTIES_PANEL_MAX
    );
}

#[test]
fn double_clicking_a_divider_resets_its_width() {
    // Frames a few milliseconds apart, so two clicks make a double click.
    let mut harness = app_with_step(0.01);
    let y = 400.0;
    let rail = metrics::LAYERS_PANEL_RAIL;
    drag(&mut harness, rail + metrics::LAYERS_PANEL_DEFAULT, 340.0, y);
    let width = harness.state().shell().width(Side::Left);
    assert_ne!(width, metrics::LAYERS_PANEL_DEFAULT);
    let at = egui::pos2(rail + width, y);
    harness.hover_at(at);
    harness.run();
    for _ in 0..2 {
        harness.drag_at(at);
        harness.step();
        harness.drop_at(at);
        harness.step();
    }
    harness.run();
    assert_eq!(
        harness.state().shell().width(Side::Left),
        metrics::LAYERS_PANEL_DEFAULT
    );
}

/// Presses at `from` on the row `y`, moves to `to` over a few frames and releases.
fn drag(harness: &mut Harness<'_, MapMakerApp>, from: f32, to: f32, y: f32) {
    let (from, to) = (egui::pos2(from, y), egui::pos2(to, y));
    harness.hover_at(from);
    harness.run();
    harness.drag_at(from);
    harness.run();
    for step in 1..=4 {
        harness.hover_at(from + (to - from) * (step as f32 / 4.0));
        harness.run();
    }
    harness.drop_at(to);
    harness.run();
}

#[test]
fn the_status_bar_shows_the_cursor_in_pixels_and_real_world_units() {
    let mut harness = app();
    harness.hover_at(egui::pos2(640.0, 400.0));
    harness.run();
    // The starter Map is 4096 × 3072 px at 0.25 mi per px.
    let node = harness.get_by_label_contains(" px · ");
    let label = node.accesskit_node().value().unwrap();
    assert!(label.starts_with("X ") && label.ends_with(" mi"), "{label}");
}

#[test]
fn a_narrow_window_shows_the_rail_and_its_panel_opens_over_the_canvas() {
    let mut harness = app_sized(0.25, 1100.0);
    assert!(harness.query_by_label("New Layer (⇧⌘N)").is_none());
    harness.get_by_label(RAIL_LAYERS).click();
    harness.run();
    // Open over the Canvas; the user's own choice (closed, the default) is untouched.
    assert!(harness.state().shell().layers_overlay());
    assert!(harness.state().shell().layers_collapsed());
    assert!(harness.query_by_label("New Layer (⇧⌘N)").is_some());
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().shell().layers_overlay());
    assert!(harness.query_by_label("New Layer (⇧⌘N)").is_none());
}

#[test]
fn the_rail_shows_the_section_icons_and_a_click_opens_that_section() {
    let mut harness = app();
    harness.get_by_label(RAIL_LAYERS).click();
    harness.run();
    assert!(harness.state().shell().layers_collapsed());
    // The icons are still there with the panel closed.
    for label in ["Layers (⌘⌥1)", "Map (⌘⌥6)", "History (⌘⌥5)"] {
        assert!(harness.query_by_label(label).is_some(), "{label}");
    }
    harness.get_by_label("Map (⌘⌥6)").click();
    harness.run();
    assert!(!harness.state().shell().layers_collapsed());
    assert_eq!(
        harness.state().shell().sidebar_section(),
        ssmap_app::shell::SidebarSection::Map
    );
}

#[test]
fn a_press_outside_the_overlay_closes_it() {
    let mut harness = app_sized(0.25, 1100.0);
    harness.get_by_label(RAIL_LAYERS).click();
    harness.run();
    assert!(harness.state().shell().layers_overlay());
    harness.hover_at(egui::pos2(800.0, 500.0));
    harness.run();
    harness.drag_at(egui::pos2(800.0, 500.0));
    harness.run();
    harness.drop_at(egui::pos2(800.0, 500.0));
    harness.run();
    assert!(!harness.state().shell().layers_overlay());
}

#[test]
fn at_the_minimum_properties_is_a_rail_that_opens_a_floating_inspector() {
    let mut harness = app_sized(0.25, 1024.0);
    assert!(harness.query_by_label(PANEL_HEADER).is_none());
    harness.get_by_label("Open Properties (⌘⌥2)").click();
    harness.run();
    assert!(harness.state().shell().properties_overlay());
    assert!(harness.query_by_label(PANEL_HEADER).is_some());
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().shell().properties_overlay());
}

#[test]
fn the_status_bar_drops_real_world_units_below_1180() {
    let mut harness = app_sized(0.25, 1100.0);
    harness.hover_at(egui::pos2(640.0, 400.0));
    harness.run();
    let node = harness.get_by_label_contains(" px");
    let label = node.accesskit_node().value().unwrap();
    assert!(label.starts_with("X ") && label.ends_with(" px"), "{label}");
}

#[test]
fn the_status_bar_summarises_the_selection() {
    let mut harness = app();
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::N,
    );
    harness.run();
    assert!(harness.query_by_label_contains("Layer · ").is_some());
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.query_by_label_contains("Active Layer: ").is_some());
}

#[test]
fn the_panel_shortcuts_are_not_also_taken_as_tab_shortcuts() {
    // egui ignores an extra Alt, so Cmd+Alt+1 would match Cmd+1 (Home) if the tab shortcuts ran
    // first. Ctrl+Tab cycles tabs and must not also hide the panels, which plain Tab does.
    let mut harness = app();
    assert!(!harness.state().home_active());
    harness.key_press_modifiers(BOTH, egui::Key::Num1);
    harness.run();
    assert!(harness.state().shell().layers_collapsed());
    assert!(!harness.state().home_active());

    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Tab);
    harness.run();
    assert!(harness.state().home_active());
    assert!(harness.state().shell().properties_shown());
}

/// Switches the sidebar with its ⌘⌥ shortcut and returns the section it shows.
fn show_section(
    harness: &mut Harness<'static, MapMakerApp>,
    key: egui::Key,
) -> ssmap_app::shell::SidebarSection {
    harness.key_press_modifiers(BOTH, key);
    harness.run();
    harness.state().shell().sidebar_section()
}

const SECTION_KEYS: [(ssmap_app::shell::SidebarSection, egui::Key); 3] = [
    (ssmap_app::shell::SidebarSection::Layers, egui::Key::Num1),
    (ssmap_app::shell::SidebarSection::Boards, egui::Key::Num3),
    (ssmap_app::shell::SidebarSection::History, egui::Key::Num5),
];

#[test]
fn the_active_icon_closes_the_panel_in_every_sidebar_section() {
    let mut harness = app();
    harness
        .state_mut()
        .run_command(ssmap_app::menu::Command::NewArtBoard);
    harness.run();
    for (section, key) in SECTION_KEYS {
        // The Layers shortcut closes the panel when Layers is already showing.
        if harness.state().shell().sidebar_section() != section {
            assert_eq!(show_section(&mut harness, key), section);
        }
        harness.get_by_label(section.label()).click();
        harness.run();
        assert!(harness.state().shell().layers_collapsed(), "{section:?}");
        // The shortcut opens it again, on the same section.
        assert_eq!(show_section(&mut harness, key), section);
        assert!(!harness.state().shell().layers_collapsed(), "{section:?}");
    }
}

#[test]
fn the_overlay_closes_from_every_sidebar_section() {
    for (section, key) in SECTION_KEYS {
        let mut harness = app_sized(0.25, 1100.0);
        harness
            .state_mut()
            .run_command(ssmap_app::menu::Command::NewArtBoard);
        harness.run();
        // The panel starts closed, and in a narrow window the shortcut opens it over the
        // Canvas on the first press, leaving the user's choice as it was (#492).
        assert!(harness.state().shell().layers_collapsed(), "{section:?}");
        assert_eq!(show_section(&mut harness, key), section);
        assert!(harness.state().shell().layers_overlay(), "{section:?}");
        assert!(harness.state().shell().layers_collapsed(), "{section:?}");
        harness.get_by_label(section.label()).click();
        harness.run();
        assert!(!harness.state().shell().layers_overlay(), "{section:?}");
        assert!(harness.state().shell().layers_collapsed(), "{section:?}");
    }
}
