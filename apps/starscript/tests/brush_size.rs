//! The Size slider of the Land Brush, Lake Brush and Stamp Eraser: a logarithmic slider over
//! 2–200 with a number box that types up to 2000. Tool settings are not part of the Map, so
//! none of this makes an Undo step.

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::tools::ToolId;
use ssmap_core::{Canvas, DistanceUnit, Scale};

const TOOLS: [ToolId; 3] = [ToolId::LandBrush, ToolId::WaterBrush, ToolId::StampEraser];

fn app(tool: ToolId) -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_step_dt(0.25)
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
        .get_by_label(&format!("{} ({})", tool.label(), tool.shortcut_text()))
        .click();
    harness.run();
    assert_eq!(harness.state().editor().active_tool, tool);
    harness
}

fn size(harness: &Harness<'static, MapMakerApp>) -> f64 {
    harness.state().editor().brush_size
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

fn type_size(harness: &mut Harness<'static, MapMakerApp>, text: &str) {
    let shown = format!("{}", size(harness));
    harness.get_by_value(&shown).click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    harness
        .get_by(|n| n.role() == Role::TextInput && n.value().as_deref() == Some(&shown))
        .type_text(text);
    harness.key_press(egui::Key::Enter);
    harness.run();
}

#[test]
fn a_slider_click_lands_on_the_log_track_without_an_undo_step() {
    for tool in TOOLS {
        let mut harness = app(tool);
        let before = undo_label(&harness);
        let track = harness.get_by_role_and_label(Role::Slider, "Size").rect();
        // The middle of a logarithmic 2–200 track is about 20, far below the linear 101.
        press(&mut harness, track.center(), true);
        press(&mut harness, track.center(), false);
        let value = size(&harness);
        assert!((10.0..=40.0).contains(&value), "{tool:?}: {value}");
        assert_eq!(undo_label(&harness), before, "{tool:?}");
    }
}

#[test]
fn dragging_the_thumb_to_the_end_reaches_the_slider_maximum() {
    let mut harness = app(ToolId::LandBrush);
    let track = harness.get_by_role_and_label(Role::Slider, "Size").rect();
    press(&mut harness, track.center(), true);
    press(
        &mut harness,
        egui::pos2(track.right() + 40.0, track.center().y),
        true,
    );
    press(
        &mut harness,
        egui::pos2(track.right() + 40.0, track.center().y),
        false,
    );
    assert_eq!(size(&harness), 200.0);
}

#[test]
fn the_arrow_steppers_nudge_the_size_by_one() {
    for tool in TOOLS {
        let mut harness = app(tool);
        let start = size(&harness);
        harness.get_by_label("Increase Size").click();
        harness.run();
        assert_eq!(size(&harness), start + 1.0, "{tool:?}");
        harness.get_by_label("Decrease Size").click();
        harness.run();
        harness.get_by_label("Decrease Size").click();
        harness.run();
        assert_eq!(size(&harness), start - 1.0, "{tool:?}");
    }
}

#[test]
fn a_typed_size_may_pass_the_slider_end_up_to_the_hard_maximum() {
    for tool in TOOLS {
        let mut harness = app(tool);
        let before = undo_label(&harness);
        type_size(&mut harness, "500");
        assert_eq!(size(&harness), 500.0, "{tool:?}");
        type_size(&mut harness, "99999");
        assert_eq!(size(&harness), 2000.0, "{tool:?}");
        type_size(&mut harness, "1");
        assert_eq!(size(&harness), 2.0, "{tool:?}");
        assert_eq!(undo_label(&harness), before, "{tool:?}");
    }
}
