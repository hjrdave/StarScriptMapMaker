//! A setting that differs across a multi-selection shows "Mixed"; dragging the slider or typing
//! a value sets every selected item, as one Undo step.

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_core::{
    Canvas, Command, DistanceUnit, Id, MultiPolygon, Polygon, Roughness, Scale, ShoreStyle,
};

fn square(min: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![
            [min, min],
            [min + 100.0, min],
            [min + 100.0, min + 100.0],
            [min, min + 100.0],
        ],
        holes: Vec::new(),
    }])
}

/// Two Landmasses selected, with Roughness amounts 10 and 30 and Ripple counts 2 and 4.
fn app() -> (Harness<'static, MapMakerApp>, [Id; 2]) {
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
    let (layer, a, b) = (Id::new(), Id::new(), Id::new());
    let editor = harness.state_mut().active_editor_mut().unwrap();
    editor
        .run(Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        })
        .unwrap();
    for (i, (id, min)) in [(a, 0.0), (b, 500.0)].into_iter().enumerate() {
        editor
            .run(Command::CreateLandmass {
                id,
                parent: layer,
                index: None,
                geometry: square(min),
                fill: None,
            })
            .unwrap();
        editor
            .run(Command::SetRoughness {
                ids: vec![id],
                roughness: Some(Roughness::new(10.0 + 20.0 * i as f64, 40.0, 7)),
            })
            .unwrap();
        let mut shore = ShoreStyle::landmass_default();
        shore.ripples.count = 2 + 2 * i as u32;
        editor
            .run(Command::SetShore {
                ids: vec![id],
                shore: Some(shore),
            })
            .unwrap();
    }
    editor.select(vec![a, b]);
    harness.run();
    (harness, [a, b])
}

fn amounts(harness: &Harness<'static, MapMakerApp>, ids: [Id; 2]) -> [f64; 2] {
    ids.map(|id| {
        harness
            .state()
            .editor()
            .map()
            .roughness(id)
            .expect("a Landmass")
            .amount
    })
}

fn counts(harness: &Harness<'static, MapMakerApp>, ids: [Id; 2]) -> [u32; 2] {
    ids.map(|id| {
        harness
            .state()
            .editor()
            .map()
            .shore(id)
            .expect("a Landmass")
            .ripples
            .count
    })
}

fn undo(harness: &mut Harness<'static, MapMakerApp>) {
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
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

fn drag_to_the_end(harness: &mut Harness<'static, MapMakerApp>, label: &str) {
    let track = harness.get_by_role_and_label(Role::Slider, label).rect();
    let y = track.center().y;
    press(harness, track.center(), true);
    for step in 1..=4 {
        let x = track.center().x + step as f32 * track.width() / 8.0;
        harness.event(egui::Event::PointerMoved(egui::pos2(x, y)));
        harness.run();
    }
    press(harness, egui::pos2(track.right() + 40.0, y), false);
}

/// Types into the first box that shows `shown`; Roughness sits above Shore style on the panel.
fn type_into(harness: &mut Harness<'static, MapMakerApp>, shown: &str, text: &str) {
    harness
        .get_all_by_value(shown)
        .next()
        .expect("a box showing that value")
        .click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    harness
        .get_by(|n| n.role() == Role::TextInput && n.is_focused())
        .type_text(text);
    harness.key_press(egui::Key::Enter);
    harness.run();
}

#[test]
fn a_setting_that_differs_shows_mixed_and_one_that_matches_shows_its_value() {
    let (harness, ids) = app();
    assert_eq!(amounts(&harness, ids), [10.0, 30.0]);
    // Amount and the Ripple count differ; Scale is the same on both.
    assert_eq!(harness.get_all_by_value("Mixed").count(), 2);
    assert!(harness.query_by_value("40.0 u").is_some());
}

#[test]
fn dragging_a_mixed_slider_sets_every_item_as_one_undo_step() {
    let (mut harness, ids) = app();
    let steps = harness.state().editor().history_steps().0.len();
    drag_to_the_end(&mut harness, "Amount");
    assert_eq!(amounts(&harness, ids), [50.0, 50.0]);
    let (after, _) = harness.state().editor().history_steps();
    assert_eq!(after.len(), steps + 1);
    // The other settings of each item were kept.
    assert_eq!(counts(&harness, ids), [2, 4]);
    undo(&mut harness);
    assert_eq!(amounts(&harness, ids), [10.0, 30.0]);
}

#[test]
fn a_typed_value_sets_every_item_and_undo_restores_each_own_value() {
    let (mut harness, ids) = app();
    type_into(&mut harness, "Mixed", "25");
    assert_eq!(amounts(&harness, ids), [25.0, 25.0]);
    assert_eq!(counts(&harness, ids), [2, 4]);
    undo(&mut harness);
    assert_eq!(amounts(&harness, ids), [10.0, 30.0]);
}

#[test]
fn the_ripple_count_number_box_is_mixed_too() {
    let (mut harness, ids) = app();
    assert_eq!(counts(&harness, ids), [2, 4]);
    // Fix the Amount first so that only the Count box says Mixed.
    type_into(&mut harness, "Mixed", "20");
    assert_eq!(amounts(&harness, ids), [20.0, 20.0]);
    type_into(&mut harness, "Mixed", "3");
    assert_eq!(counts(&harness, ids), [3, 3]);
    undo(&mut harness);
    assert_eq!(counts(&harness, ids), [2, 4]);
    assert_eq!(amounts(&harness, ids), [20.0, 20.0]);
}
