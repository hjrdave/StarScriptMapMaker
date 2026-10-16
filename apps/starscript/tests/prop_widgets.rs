//! The Properties widgets in a headless window: a scrub is one Undo step, a reset arrow
//! clears an override, and the gallery draws every state.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::editor::Editor;
use ssmap_app::prop_widgets::{Gallery, LiveEdit, Provenance, ScrubSpec, scrub_field, slider};
use ssmap_core::{Canvas, Command, DistanceUnit, Id, Map, Node, Scale};

struct Fixture {
    editor: Editor,
    layer: Id,
    live: LiveEdit,
}

fn opacity(fixture: &Fixture) -> f32 {
    match fixture.editor.map.node(fixture.layer) {
        Some(Node::Layer(layer)) => layer.opacity,
        _ => panic!("the Layer is gone"),
    }
}

/// A Layer at 100% shown with an opacity field (below 100% counts as overriding the default),
/// whose edits go through the Editor the way a Properties section's would.
fn harness(use_slider: bool) -> Harness<'static, Fixture> {
    let mut editor = Editor::new(Map::new(Canvas::new(
        100,
        100,
        Scale::new(1.0, DistanceUnit::Miles),
    )));
    let layer = editor.new_layer().expect("a Layer");
    let fixture = Fixture {
        editor,
        layer,
        live: LiveEdit::default(),
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(320.0, 200.0))
        .build_ui_state(
            move |ui, state: &mut Fixture| {
                let current = opacity(state);
                let provenance = if current < 1.0 {
                    Provenance::Overridden
                } else {
                    Provenance::Inherited
                };
                let spec = ScrubSpec::new("Opacity")
                    .unit("%")
                    .range(0.0, 100.0)
                    .provenance(provenance);
                let mut value = f64::from(current) * 100.0;
                let edit = if use_slider {
                    slider(ui, &mut value, &spec)
                } else {
                    scrub_field(ui, &mut value, &spec)
                };
                let id = state.layer;
                if edit.reset {
                    state.editor.run(Command::SetOpacity { id, opacity: 1.0 });
                }
                state
                    .live
                    .apply(&mut state.editor, edit, "Change Opacity", || {
                        Command::SetOpacity {
                            id,
                            opacity: (value / 100.0) as f32,
                        }
                    });
            },
            fixture,
        );
    harness.run();
    harness
}

fn pointer(harness: &mut Harness<'_, Fixture>, pos: egui::Pos2, pressed: Option<bool>) {
    harness.event(egui::Event::PointerMoved(pos));
    if let Some(pressed) = pressed {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run();
}

#[test]
fn one_scrub_is_one_undo_step() {
    let mut harness = harness(false);
    let start = harness.get_by_label("Opacity").rect().center();
    let before = harness.state().editor.undo_label().map(str::to_owned);
    assert_eq!(opacity(harness.state()), 1.0);

    pointer(&mut harness, start, Some(true));
    for step in 1..=5 {
        let at = start - egui::vec2(step as f32 * 4.0, 0.0);
        pointer(&mut harness, at, None);
    }
    assert!(
        harness.state().editor.gesture_open(),
        "the drag is a live gesture"
    );
    pointer(&mut harness, start - egui::vec2(20.0, 0.0), Some(false));
    harness.run();

    let state = harness.state_mut();
    assert!(!state.editor.gesture_open());
    assert!(opacity(state) < 1.0, "the drag changed the opacity");
    assert_eq!(state.editor.undo_label(), Some("Change Opacity"));
    // Many live changes, one Undo step: a single undo returns to the start.
    state.editor.undo();
    assert_eq!(opacity(state), 1.0);
    assert_eq!(state.editor.undo_label().map(str::to_owned), before);
}

#[test]
fn one_slider_drag_is_one_undo_step() {
    let mut harness = harness(true);
    let before = harness.state().editor.undo_label().map(str::to_owned);
    let label = harness.get_by_label("Opacity").rect();
    assert_eq!(opacity(harness.state()), 1.0);
    // The slider's accessible node is its track.
    let from = label.center();
    pointer(&mut harness, from, Some(true));
    for step in 1..=4 {
        pointer(
            &mut harness,
            from - egui::vec2(step as f32 * 10.0, 0.0),
            None,
        );
    }
    pointer(&mut harness, from - egui::vec2(40.0, 0.0), Some(false));
    harness.run();

    let state = harness.state_mut();
    assert!(opacity(state) < 1.0);
    assert_eq!(state.editor.undo_label(), Some("Change Opacity"));
    state.editor.undo();
    assert_eq!(opacity(state), 1.0);
    assert_eq!(state.editor.undo_label().map(str::to_owned), before);
}

#[test]
fn the_reset_arrow_clears_an_override() {
    let mut harness = harness(false);
    let id = harness.state().layer;
    harness
        .state_mut()
        .editor
        .run(Command::SetOpacity { id, opacity: 0.4 });
    harness.run();
    harness.get_by_label("Reset Opacity").click();
    harness.run();
    assert_eq!(opacity(harness.state()), 1.0);
    assert!(
        harness.query_by_label("Reset Opacity").is_none(),
        "no reset arrow once nothing is overridden"
    );
}

#[test]
fn the_gallery_shows_every_widget() {
    let mut gallery = Gallery::default();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(320.0, 520.0))
        .build_ui(move |ui| gallery.show(ui));
    harness.run();
    for label in [
        "Gallery section",
        "Add gallery item",
        "Width",
        "Tall field",
        "Inherited angle",
        "Overridden size",
        "Reset Overridden size",
        "Disabled field",
        "Disabled slider",
        "Opacity",
        "Fill",
        "Paint",
        "Erase",
        "Snap to grid",
    ] {
        let _ = harness.get_by_label(label);
    }
}
