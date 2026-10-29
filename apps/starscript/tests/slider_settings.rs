//! The Stamp scatter settings and the Label text settings are scrub fields. Scatter settings are
//! tool settings, so they make no Undo step; each drag on a Label setting is one named step.

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::tools::ToolId;
use ssmap_core::{Canvas, DistanceUnit, Id, Scale};

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness.state_mut().new_map(Canvas::new(
        4096,
        3072,
        Scale::new(0.25, DistanceUnit::Miles),
    ));
    harness.run();
    harness
}

fn undo_label(harness: &Harness<'static, MapMakerApp>) -> Option<String> {
    harness.state().editor().undo_label().map(str::to_owned)
}

fn press(harness: &mut Harness<'static, MapMakerApp>, pos: egui::Pos2, pressed: bool) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.run();
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
}

fn type_into(harness: &mut Harness<'static, MapMakerApp>, shown: &str, text: &str) {
    harness.get_by_value(shown).click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    harness
        .get_by(|n| n.role() == Role::TextInput && n.is_focused())
        .type_text(text);
    harness.key_press(egui::Key::Enter);
    harness.run();
}

fn stamp_app() -> Harness<'static, MapMakerApp> {
    let mut harness = app();
    let tool = ToolId::Stamp;
    harness
        .get_by_label(&format!("{} ({})", tool.label(), tool.shortcut_text()))
        .click();
    harness.run();
    assert_eq!(harness.state().editor().active_tool, tool);
    harness
}

/// Drags the scrub handle of the field called `label` by `dx` points.
fn scrub(harness: &mut Harness<'static, MapMakerApp>, label: &str, dx: f32) {
    let handle = harness
        .get_by_role_and_label(Role::SpinButton, label)
        .rect();
    let start = handle.center();
    press(harness, start, true);
    for step in 1..=4 {
        let x = start.x + step as f32 * dx / 4.0;
        harness.event(egui::Event::PointerMoved(egui::pos2(x, start.y)));
        harness.run();
    }
    press(harness, egui::pos2(start.x + dx, start.y), false);
}

/// Drags the slider called `label` from the middle of its track by `dx` points.
fn drag_slider(harness: &mut Harness<'static, MapMakerApp>, label: &str, dx: f32) {
    let track = harness.get_by_role_and_label(Role::Slider, label).rect();
    let start = track.center();
    press(harness, start, true);
    for step in 1..=4 {
        let x = start.x + step as f32 * dx / 4.0;
        harness.event(egui::Event::PointerMoved(egui::pos2(x, start.y)));
        harness.run();
    }
    press(harness, egui::pos2(start.x + dx, start.y), false);
}

#[test]
fn each_scatter_field_takes_a_scrub_without_an_undo_step() {
    for label in ["Density", "Spacing", "Size jitter", "Tilt jitter"] {
        let mut harness = stamp_app();
        let before = undo_label(&harness);
        let scatter = harness.state().editor().scatter.clone();
        drag_slider(&mut harness, label, 40.0);
        assert_ne!(harness.state().editor().scatter, scatter, "{label}");
        assert_eq!(undo_label(&harness), before, "{label}");
    }
}

#[test]
fn the_scatter_fields_stop_at_their_range_ends() {
    let mut harness = stamp_app();
    for label in ["Density", "Spacing", "Size jitter", "Tilt jitter"] {
        drag_slider(&mut harness, label, 4000.0);
    }
    let scatter = &harness.state().editor().scatter;
    assert_eq!(scatter.density, 100.0);
    // The track ends at 200; only typing goes on to the hard maximum.
    assert_eq!(scatter.spacing, 200.0);
    assert!((scatter.size_jitter - 0.9).abs() < 1e-9);
    assert_eq!(scatter.rotation_jitter, 180.0);
}

#[test]
fn a_typed_spacing_stops_at_the_hard_maximum() {
    let mut harness = stamp_app();
    type_into(&mut harness, "10", "500");
    assert_eq!(harness.state().editor().scatter.spacing, 500.0);
    type_into(&mut harness, "500", "99999");
    assert_eq!(harness.state().editor().scatter.spacing, 2000.0);
}

/// Places a Label with the Label tool and leaves it selected.
fn label_app() -> (Harness<'static, MapMakerApp>, Id) {
    let mut harness = app();
    harness.key_press(egui::Key::T);
    harness.run();
    let canvas = egui::pos2(640.0, 400.0);
    harness.event(egui::Event::PointerMoved(canvas));
    harness.run();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: canvas,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
    }
    assert!(harness.state().editing_label_text());
    harness.event(egui::Event::Text("Ærendál".into()));
    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.run();
    let [id] = harness.state().editor().selection() else {
        panic!("the new Label is selected")
    };
    let id = *id;
    (harness, id)
}

#[test]
fn each_label_field_is_one_named_undo_step_per_scrub() {
    for (label, name, dx) in [
        ("Size", "Change Label Size", 40.0),
        ("Letter spacing", "Change Label Letter Spacing", 40.0),
        ("Line height", "Change Label Line Height", 40.0),
        ("Halo", "Change Label Halo", 40.0),
        // Opacity starts at 100%, so it can only be dragged down.
        ("Opacity", "Change Label Opacity", -40.0),
        ("Arch", "Change Label Arch", 40.0),
    ] {
        let (mut harness, id) = label_app();
        let before = harness.state().editor().label(id).cloned().unwrap();
        assert_eq!(undo_label(&harness).as_deref(), Some("Add Label"));
        scrub(&mut harness, label, dx);
        let after = harness.state().editor().label(id).cloned().unwrap();
        assert_ne!(after, before, "{label}");
        assert_eq!(undo_label(&harness).as_deref(), Some(name), "{label}");
        // One step: a single Undo returns to the placed Label.
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.run();
        assert_eq!(
            harness.state().editor().label(id).cloned().unwrap(),
            before,
            "{label}"
        );
        assert_eq!(
            undo_label(&harness).as_deref(),
            Some("Add Label"),
            "{label}"
        );
    }
}

#[test]
fn the_label_weight_is_a_scrub_field_without_a_slider() {
    let (mut harness, id) = label_app();
    assert!(
        harness
            .query_by_role_and_label(Role::Slider, "Weight")
            .is_none()
    );
    // Weight moves in hundreds, so two points is two steps.
    scrub(&mut harness, "Weight", 2.0);
    assert_eq!(undo_label(&harness).as_deref(), Some("Change Label Weight"));
    let weight = harness.state().editor().label(id).unwrap().font.clone();
    assert!(weight.is_some());
}

#[test]
fn a_typed_label_size_may_pass_the_slider_end() {
    let (mut harness, id) = label_app();
    type_into(&mut harness, "48.0 u", "5000");
    let label = harness.state().editor().label(id).unwrap();
    assert_eq!(label.size, Some(5000.0));
}
