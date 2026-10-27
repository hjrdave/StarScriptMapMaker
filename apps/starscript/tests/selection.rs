//! Multi-select on the Canvas (box-select, Shift-click), its sync with the Layers panel, and
//! the right-click menu. The real app, driven headlessly; the Canvas draws its placeholder.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_core::{Canvas, DistanceUnit, Id, Scale};

/// Canvas points of the three Labels: two near each other, one far off, and a spot on the
/// off-Canvas outside the Canvas.
const A: [f64; 2] = [1000.0, 1000.0];
const B: [f64; 2] = [2000.0, 1000.0];
const C: [f64; 2] = [3000.0, 2500.0];

fn app() -> (Harness<'static, MapMakerApp>, [Id; 3]) {
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
    let editor = harness.state_mut().active_editor_mut().unwrap();
    let ids = [A, B, C].map(|at| {
        let (id, gesture) = editor.place_label(at).expect("a Label is placed");
        editor.end_gesture(gesture);
        id
    });
    editor.select(Vec::new());
    harness.run();
    (harness, ids)
}

fn screen(harness: &Harness<'static, MapMakerApp>, point: [f64; 2]) -> egui::Pos2 {
    harness.state().canvas_to_screen(point).expect("drawn")
}

fn selection(harness: &Harness<'static, MapMakerApp>) -> Vec<Id> {
    let mut ids = harness.state().editor().selection().to_vec();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<Id>) -> Vec<Id> {
    ids.sort();
    ids
}

fn button(
    harness: &mut Harness<'static, MapMakerApp>,
    pos: egui::Pos2,
    button: egui::PointerButton,
    pressed: bool,
    modifiers: egui::Modifiers,
) {
    harness.event(egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers,
    });
    harness.run();
}

fn hold(harness: &mut Harness<'static, MapMakerApp>, modifiers: egui::Modifiers) {
    harness.event(egui::Event::ModifiersChanged(modifiers));
    harness.run();
}

fn click(
    harness: &mut Harness<'static, MapMakerApp>,
    point: [f64; 2],
    modifiers: egui::Modifiers,
    which: egui::PointerButton,
) {
    let pos = screen(harness, point);
    hold(harness, modifiers);
    harness.event(egui::Event::PointerMoved(pos));
    harness.run();
    button(harness, pos, which, true, modifiers);
    button(harness, pos, which, false, modifiers);
    hold(harness, egui::Modifiers::NONE);
}

fn click_at(harness: &mut Harness<'static, MapMakerApp>, point: [f64; 2], shift: bool) {
    let modifiers = if shift {
        egui::Modifiers::SHIFT
    } else {
        egui::Modifiers::NONE
    };
    click(harness, point, modifiers, egui::PointerButton::Primary);
}

fn right_click(harness: &mut Harness<'static, MapMakerApp>, point: [f64; 2]) {
    click(
        harness,
        point,
        egui::Modifiers::NONE,
        egui::PointerButton::Secondary,
    );
}

/// Drags a box from `from` to `to` over a few frames.
fn box_drag(
    harness: &mut Harness<'static, MapMakerApp>,
    from: egui::Pos2,
    to: egui::Pos2,
    modifiers: egui::Modifiers,
) {
    hold(harness, modifiers);
    harness.event(egui::Event::PointerMoved(from));
    harness.run();
    button(harness, from, egui::PointerButton::Primary, true, modifiers);
    for step in 1..=5 {
        harness.event(egui::Event::PointerMoved(
            from + (to - from) * (step as f32 / 5.0),
        ));
        harness.run();
    }
    button(harness, to, egui::PointerButton::Primary, false, modifiers);
    hold(harness, egui::Modifiers::NONE);
}

/// The box from just above-left of `top_left` to just below-right of `bottom_right`.
fn around(
    harness: &Harness<'static, MapMakerApp>,
    top_left: [f64; 2],
    bottom_right: [f64; 2],
) -> (egui::Pos2, egui::Pos2) {
    let pad = egui::vec2(30.0, 30.0);
    (
        screen(harness, top_left) - pad,
        screen(harness, bottom_right) + pad,
    )
}

#[test]
fn a_box_drag_selects_what_it_touches() {
    let (mut harness, [a, b, _]) = app();
    let (from, to) = around(&harness, A, B);
    box_drag(&mut harness, from, to, egui::Modifiers::NONE);
    assert_eq!(selection(&harness), sorted(vec![a, b]));
    // An empty box clears the Selection, as a click on empty Canvas does.
    let empty = screen(&harness, [3500.0, 200.0]);
    box_drag(
        &mut harness,
        empty,
        empty + egui::vec2(20.0, 20.0),
        egui::Modifiers::NONE,
    );
    assert!(selection(&harness).is_empty());
}

#[test]
fn a_box_drag_can_start_on_the_off_canvas() {
    let (mut harness, [a, b, _]) = app();
    // Starting outside the Canvas edge, up and to the left of everything.
    let origin = screen(&harness, [0.0, 0.0]) - egui::vec2(20.0, 20.0);
    let to = screen(&harness, B) + egui::vec2(30.0, 30.0);
    box_drag(&mut harness, origin, to, egui::Modifiers::NONE);
    assert_eq!(selection(&harness), sorted(vec![a, b]));
}

#[test]
fn shift_adds_to_the_selection_for_a_box_and_for_a_click() {
    let (mut harness, [a, b, c]) = app();
    click_at(&mut harness, C, false);
    assert_eq!(selection(&harness), vec![c]);
    // Shift-box keeps C and adds A and B.
    let (from, to) = around(&harness, A, B);
    box_drag(&mut harness, from, to, egui::Modifiers::SHIFT);
    assert_eq!(selection(&harness), sorted(vec![a, b, c]));
    // Shift-click toggles one out and back in.
    click_at(&mut harness, B, true);
    assert_eq!(selection(&harness), sorted(vec![a, c]));
    click_at(&mut harness, B, true);
    assert_eq!(selection(&harness), sorted(vec![a, b, c]));
}

#[test]
fn the_canvas_and_the_layers_panel_share_one_selection() {
    let (mut harness, [a, b, _]) = app();
    let (from, to) = around(&harness, A, B);
    box_drag(&mut harness, from, to, egui::Modifiers::NONE);
    // Both Labels sit in the Labels Layer, which the panel shows with the selection's
    // active Layer: grouping from the panel's menu acts on what the Canvas selected.
    let layer = harness.state().editor().map().parent(a).unwrap();
    assert_eq!(harness.state().editor().map().parent(b), Some(layer));
    assert_eq!(harness.state().editor().active_layer(), Some(layer));
    // And the other way: a row clicked in the panel is what the Canvas now has selected.
    harness.get_by_label("Labels").click();
    harness.run();
    assert_eq!(selection(&harness), vec![layer]);
}

#[test]
fn right_click_on_an_unselected_object_selects_it_and_opens_the_menu() {
    let (mut harness, [a, b, _]) = app();
    click_at(&mut harness, A, false);
    right_click(&mut harness, B);
    assert_eq!(selection(&harness), vec![b]);
    for item in ["Copy", "Cut", "Paste", "Group", "Delete"] {
        harness.get_by_label(item);
    }
    assert!(harness.state().editor().map().contains(a));
}

#[test]
fn the_menu_copies_cuts_and_pastes() {
    let (mut harness, [a, b, _]) = app();
    let (from, to) = around(&harness, A, B);
    box_drag(&mut harness, from, to, egui::Modifiers::NONE);

    right_click(&mut harness, A);
    harness.get_by_label("Copy").click();
    harness.run();
    assert!(harness.state().editor().has_clipboard());
    assert!(harness.state().editor().map().contains(a));

    right_click(&mut harness, A);
    harness.get_by_label("Cut").click();
    harness.run();
    assert!(!harness.state().editor().map().contains(a));
    assert!(!harness.state().editor().map().contains(b));

    // Paste lands the pair back, with their middle under the right-click.
    let target = [2500.0, 2000.0];
    right_click(&mut harness, target);
    harness.get_by_label("Paste").click();
    harness.run();
    let editor = harness.state().editor();
    let pasted = editor.selection().to_vec();
    assert_eq!(pasted.len(), 2);
    let positions: Vec<[f64; 2]> = pasted
        .iter()
        .map(|&id| editor.label(id).expect("a Label").position)
        .collect();
    let middle = [
        (positions[0][0] + positions[1][0]) / 2.0,
        (positions[0][1] + positions[1][1]) / 2.0,
    ];
    assert!(
        (middle[0] - target[0]).abs() < 60.0 && (middle[1] - target[1]).abs() < 60.0,
        "{middle:?}"
    );
}

#[test]
fn the_menu_groups_and_deletes() {
    let (mut harness, [a, b, _]) = app();
    let (from, to) = around(&harness, A, B);
    box_drag(&mut harness, from, to, egui::Modifiers::NONE);

    right_click(&mut harness, A);
    harness.get_by_label("Group").click();
    harness.run();
    let editor = harness.state().editor();
    let [group] = editor.selection() else {
        panic!("the new Group is selected")
    };
    assert_eq!(editor.map().children(Some(*group)).len(), 2);
    assert_eq!(editor.undo_label(), Some("Group"));

    // Undo, then delete the pair.
    harness.state_mut().active_editor_mut().unwrap().undo();
    let (from, to) = around(&harness, A, B);
    box_drag(&mut harness, from, to, egui::Modifiers::NONE);
    right_click(&mut harness, A);
    harness.get_by_label("Delete").click();
    harness.run();
    let editor = harness.state().editor();
    assert!(!editor.map().contains(a) && !editor.map().contains(b));
    assert_eq!(editor.undo_label(), Some("Delete"));
}

#[test]
fn the_panel_menu_pastes_what_the_canvas_copied() {
    let (mut harness, [a, _, _]) = app();
    click_at(&mut harness, A, false);
    right_click(&mut harness, A);
    harness.get_by_label("Copy").click();
    harness.run();

    let layer = harness.state().editor().map().parent(a).unwrap();
    let before = harness.state().editor().map().children(Some(layer)).len();
    harness.get_by_label("Labels").click_secondary();
    harness.run();
    harness.get_by_label("Paste").click();
    harness.run();
    harness.run();
    let editor = harness.state().editor();
    assert_eq!(editor.map().children(Some(layer)).len(), before + 1);
}
