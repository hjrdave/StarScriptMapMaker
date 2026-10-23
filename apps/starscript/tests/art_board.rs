//! The Art board tool: select the Canvas by its edge or label (not its inside), resize it from
//! its handles with Shift and Alt, the cursor HUD, one Undo step per resize, and that no
//! Object moves. The real app, driven headlessly.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;
use ssmap_app::tools::ToolId;
use ssmap_core::{Canvas, DistanceUnit, Id, Scale};

const WIDTH: f64 = 4096.0;
const HEIGHT: f64 = 3072.0;
const LABEL_AT: [f64; 2] = [1500.0, 1200.0];

fn app() -> (Harness<'static, MapMakerApp>, Id) {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_recovery_dir(cc, None));
    harness.run();
    harness.state_mut().new_map(Canvas::new(
        WIDTH as u32,
        HEIGHT as u32,
        Scale::new(0.25, DistanceUnit::Miles),
    ));
    harness.run();
    let editor = harness.state_mut().active_editor_mut().unwrap();
    let (id, gesture) = editor.place_label(LABEL_AT).expect("a Label is placed");
    editor.end_gesture(gesture);
    editor.select(Vec::new());
    editor.active_tool = ToolId::ArtBoard;
    harness.run();
    (harness, id)
}

fn screen(harness: &Harness<'static, MapMakerApp>, point: [f64; 2]) -> egui::Pos2 {
    harness.state().canvas_to_screen(point).expect("drawn")
}

fn button(
    harness: &mut Harness<'static, MapMakerApp>,
    pos: egui::Pos2,
    pressed: bool,
    modifiers: egui::Modifiers,
) {
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    });
    harness.run();
}

fn click_screen(harness: &mut Harness<'static, MapMakerApp>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.run();
    button(harness, pos, true, egui::Modifiers::NONE);
    button(harness, pos, false, egui::Modifiers::NONE);
}

fn click_canvas(harness: &mut Harness<'static, MapMakerApp>, point: [f64; 2]) {
    let pos = screen(harness, point);
    click_screen(harness, pos);
}

fn select_board(harness: &mut Harness<'static, MapMakerApp>) {
    click_canvas(harness, [0.0, HEIGHT / 2.0]);
    assert!(harness.state().editor().board_selected());
}

/// Presses on `from`, drags to `to` and returns with the button still down.
fn press_and_drag(
    harness: &mut Harness<'static, MapMakerApp>,
    from: egui::Pos2,
    to: egui::Pos2,
    modifiers: egui::Modifiers,
) {
    harness.event(egui::Event::ModifiersChanged(modifiers));
    harness.event(egui::Event::PointerMoved(from));
    harness.run();
    button(harness, from, true, modifiers);
    for step in 1..=5 {
        harness.event(egui::Event::PointerMoved(
            from + (to - from) * (step as f32 / 5.0),
        ));
        harness.run();
    }
}

fn release(harness: &mut Harness<'static, MapMakerApp>, at: egui::Pos2) {
    button(harness, at, false, egui::Modifiers::NONE);
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run();
}

/// Drags the board's bottom-right handle to the Canvas point `to`.
fn drag_corner(
    harness: &mut Harness<'static, MapMakerApp>,
    to: [f64; 2],
    modifiers: egui::Modifiers,
) {
    select_board(harness);
    let from = screen(harness, [WIDTH, HEIGHT]);
    let target = screen(harness, to);
    press_and_drag(harness, from, target, modifiers);
    release(harness, target);
}

fn canvas(harness: &Harness<'static, MapMakerApp>) -> (f64, u32, u32) {
    let canvas = &harness.state().editor().map.canvas;
    (canvas.origin[0], canvas.width, canvas.height)
}

fn near(actual: u32, expected: f64) -> bool {
    (f64::from(actual) - expected).abs() <= 3.0
}

#[test]
fn clicking_the_edge_or_the_label_selects_the_board_but_not_the_inside() {
    let (mut harness, _) = app();
    click_canvas(&mut harness, [WIDTH / 2.0, HEIGHT / 2.0]);
    assert!(!harness.state().editor().board_selected());

    click_canvas(&mut harness, [0.0, HEIGHT / 2.0]);
    assert!(harness.state().editor().board_selected());

    // Clicking away deselects; the label (above the top-left corner) selects it again.
    click_canvas(&mut harness, [WIDTH / 2.0, HEIGHT / 2.0]);
    assert!(!harness.state().editor().board_selected());
    let corner = screen(&harness, [0.0, 0.0]);
    click_screen(&mut harness, corner + egui::vec2(10.0, -12.0));
    assert!(harness.state().editor().board_selected());
}

#[test]
fn the_board_shows_its_width_and_height_in_properties() {
    let (mut harness, _) = app();
    select_board(&mut harness);
    harness.run();
    // The same stepper buttons the Map section's fields have.
    harness.get_by_label("Increase Canvas width").click();
    harness.run();
    assert_eq!(canvas(&harness).1, WIDTH as u32 + 10);
    harness.get_by_label("Increase Canvas height").click();
    harness.run();
    assert_eq!(canvas(&harness).2, HEIGHT as u32 + 10);
}

#[test]
fn dragging_a_handle_resizes_the_board_and_shows_the_hud() {
    let (mut harness, label) = app();
    let before = format!("{:?}", harness.state().editor().map.node(label));
    select_board(&mut harness);
    let from = screen(&harness, [WIDTH, HEIGHT]);
    let target = screen(&harness, [WIDTH - 500.0, HEIGHT - 400.0]);
    press_and_drag(&mut harness, from, target, egui::Modifiers::NONE);
    let (_, width, height) = canvas(&harness);
    assert!(
        harness
            .query_by_label(&format!("Board · {width} × {height}"))
            .is_some()
    );
    release(&mut harness, target);

    let (origin, width, height) = canvas(&harness);
    assert_eq!(origin, 0.0);
    assert!(near(width, WIDTH - 500.0), "{width}");
    assert!(near(height, HEIGHT - 400.0), "{height}");
    assert!(harness.query_by_label_contains("Board · ").is_none());
    assert_eq!(
        format!("{:?}", harness.state().editor().map.node(label)),
        before
    );
}

#[test]
fn shift_keeps_the_aspect_ratio() {
    let (mut harness, _) = app();
    drag_corner(
        &mut harness,
        [WIDTH - 800.0, HEIGHT],
        egui::Modifiers::SHIFT,
    );
    let (_, width, height) = canvas(&harness);
    assert!(near(width, WIDTH - 800.0), "{width}");
    let ratio = f64::from(width) / f64::from(height);
    assert!((ratio - WIDTH / HEIGHT).abs() < 0.01, "{ratio}");
}

#[test]
fn alt_resizes_from_the_centre() {
    let (mut harness, label) = app();
    let before = format!("{:?}", harness.state().editor().map.node(label));
    drag_corner(
        &mut harness,
        [WIDTH - 500.0, HEIGHT - 400.0],
        egui::Modifiers::ALT,
    );
    let (origin, width, height) = canvas(&harness);
    assert!(near(width, WIDTH - 1000.0), "{width}");
    assert!(near(height, HEIGHT - 800.0), "{height}");
    assert!((origin - 500.0).abs() <= 3.0, "{origin}");
    assert_eq!(
        format!("{:?}", harness.state().editor().map.node(label)),
        before
    );
}

#[test]
fn each_resize_is_one_undo_step() {
    let (mut harness, _) = app();
    drag_corner(
        &mut harness,
        [WIDTH - 500.0, HEIGHT - 400.0],
        egui::Modifiers::NONE,
    );
    assert_eq!(harness.state().editor().undo_label(), Some("Resize Canvas"));
    harness.state_mut().active_editor_mut().unwrap().undo();
    harness.run();
    assert_eq!(canvas(&harness), (0.0, WIDTH as u32, HEIGHT as u32));
    assert_ne!(harness.state().editor().undo_label(), Some("Resize Canvas"));
}
