//! egui_kittest smoke tests: the real app, driven headlessly through its accessibility tree.
//! There is no GPU here, so the Canvas draws its flat placeholder.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ssmap_app::MapMakerApp;

fn app() -> Harness<'static, MapMakerApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::new(cc));
    harness.run();
    harness
}

#[test]
fn launches_and_creates_a_layer() {
    let mut harness = app();
    assert!(harness.state().editor().map.children(None).is_empty());

    harness.get_by_label("New Layer").click();
    harness.run();
    let map = &harness.state().editor().map;
    assert_eq!(map.children(None).len(), 1);
    // The new Layer shows in the tree and is selected.
    harness.get_by_label("Layer 1");
    assert_eq!(
        harness.state().editor().selection(),
        harness.state().editor().map.children(None)
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert!(harness.state().editor().map.children(None).is_empty());
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
    harness.run();
    assert_eq!(harness.state().editor().map.children(None).len(), 1);
}

#[test]
fn a_locked_layer_refuses_edits_with_a_visible_message() {
    let mut harness = app();
    harness.get_by_label("New Layer").click();
    harness.run();
    harness.get_by_label("Lock").click();
    harness.run();
    let layer = harness.state().editor().map.children(None)[0];
    assert!(harness.state().editor().map.node(layer).unwrap().locked());

    // The hide toggle is refused, and the reason shows in the status bar.
    harness.get_by_label("Hide").click();
    harness.run();
    let editor = harness.state().editor();
    let message = "“Layer 1” is locked";
    assert_eq!(editor.notice().map(|n| n.text.as_str()), Some(message));
    harness.get_by_label(message);
    assert!(matches!(
        editor.map.node(layer),
        Some(ssmap_core::Node::Layer(l)) if l.visible
    ));
}

#[test]
fn a_layer_with_100k_objects_lays_out_only_visible_rows() {
    use ssmap_core::{
        Canvas, DistanceUnit, Id, Layer, Map, Node, Object, ObjectKind, Op, Scale, Stamp, Subtree,
    };
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let stamps = (0..100_000)
        .map(|_| Subtree {
            id: Id::new(),
            node: Node::Object(Object::new(ObjectKind::Stamp(Stamp::default()))),
            children: Vec::new(),
        })
        .collect();
    map.apply(Op::Insert {
        parent: None,
        index: 0,
        subtree: Subtree {
            id: Id::new(),
            node: Node::Layer(Layer::new("Forest")),
            children: stamps,
        },
    })
    .unwrap();

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(|cc| MapMakerApp::with_map(cc, map));
    let started = std::time::Instant::now();
    harness.run();
    eprintln!("first frames with 100k rows: {:?}", started.elapsed());
    harness.get_by_label("Forest");
    // Only rows that fit on screen exist in the UI.
    let stamp_rows = harness.query_all_by_label("Stamp").count();
    assert!(
        (1..100).contains(&stamp_rows),
        "{stamp_rows} Stamp rows laid out"
    );
}

#[test]
fn save_as_then_open_round_trips_the_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world");
    let mut harness = app();
    assert_eq!(harness.state().title(), "Untitled — StarScript Map Maker");
    harness.get_by_label("New Layer").click();
    harness.run();
    assert!(harness.state().title().starts_with("Untitled •"));

    assert!(harness.state_mut().save_as_path(&path));
    harness.run();
    let saved = dir.path().join("world.ssmap");
    let editor = harness.state().editor();
    assert_eq!(harness.state().document().path(), Some(saved.as_path()));
    assert_eq!(
        editor.notice().map(|n| n.text.as_str()),
        Some("Saved “world.ssmap”")
    );
    assert_eq!(
        harness.state().title(),
        "world.ssmap — StarScript Map Maker"
    );
    let saved_view = editor.map.view();

    // More edits, then open the saved file again: they are gone.
    harness.get_by_label("New Layer").click();
    harness.run();
    assert!(harness.state().title().starts_with("world.ssmap •"));
    assert!(harness.state_mut().open_path(&saved));
    harness.run();
    assert_eq!(harness.state().editor().map.view(), saved_view);
    assert!(!harness.state().editor().can_undo());
    harness.get_by_label("Layer 1");
    assert_eq!(harness.state().recent_files(), std::slice::from_ref(&saved));

    // Ctrl+S saves to the open file.
    harness.get_by_label("New Layer").click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    harness.run();
    assert_eq!(
        harness.state().title(),
        "world.ssmap — StarScript Map Maker"
    );
    let (_, reopened) = ssmap_core::file::MapFile::open(&saved).unwrap();
    assert_eq!(reopened.children(None).len(), 2);
}

#[test]
fn saving_an_untitled_map_asks_where() {
    let mut harness = app();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    harness.run();
    assert!(harness.state().document().path().is_none());
    harness.get_by_label("Save Map As");
}

#[test]
fn a_file_that_fails_to_open_shows_why() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.ssmap");
    std::fs::write(&path, "plain text ".repeat(100)).unwrap();
    let mut harness = app();
    assert!(!harness.state_mut().open_path(&path));
    harness.run();
    let message = "Couldn’t open “notes.ssmap”: this is not a StarScript Map file";
    assert_eq!(
        harness.state().editor().notice().map(|n| n.text.as_str()),
        Some(message)
    );
    harness.get_by_label(message);
    assert!(harness.state().document().path().is_none());
}

/// Clicks the Canvas at a screen point.
fn click_at(harness: &mut Harness<'static, MapMakerApp>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.run();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
    }
}

#[test]
fn the_label_tool_places_a_label_and_types_its_text_in_one_undo_step() {
    let mut harness = app();
    harness.key_press(egui::Key::T);
    harness.run();
    // The middle of the Canvas panel, between the Layers and Properties panels.
    let canvas = egui::pos2(640.0, 400.0);
    click_at(&mut harness, canvas);
    assert!(harness.state().editing_label_text());
    // The placeholder text is selected, so typing replaces it.
    harness.event(egui::Event::Text("Ærendál".into()));
    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!harness.state().editing_label_text());

    let editor = harness.state().editor();
    let [id] = editor.selection() else {
        panic!("the new Label is selected")
    };
    let id = *id;
    assert_eq!(editor.label(id).map(|l| l.text.as_str()), Some("Ærendál"));
    assert_eq!(editor.undo_label(), Some("Add Label"));
    // Its first line names it in the Layers panel.
    harness.get_by_label("Ærendál");

    // Clicking empty Canvas deselects it, and clicking it picks it again.
    click_at(&mut harness, egui::pos2(300.0, 120.0));
    assert!(harness.state().editor().selection().is_empty());
    click_at(&mut harness, canvas);
    assert_eq!(harness.state().editor().selection(), &[id]);

    // Dragging it moves it, as one Undo step.
    let before = harness.state().editor().label(id).unwrap().position;
    harness.event(egui::Event::PointerButton {
        pos: canvas,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    for step in 1..=5 {
        harness.event(egui::Event::PointerMoved(
            canvas + egui::vec2(10.0 * step as f32, 0.0),
        ));
        harness.run();
    }
    harness.event(egui::Event::PointerButton {
        pos: canvas + egui::vec2(50.0, 0.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    let editor = harness.state().editor();
    let after = editor.label(id).unwrap().position;
    assert!(
        after[0] > before[0] && after[1] == before[1],
        "{before:?} -> {after:?}"
    );
    assert_eq!(editor.undo_label(), Some("Move"));

    for _ in 0..2 {
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        harness.run();
    }
    let editor = harness.state().editor();
    assert!(
        editor.map.children(None).is_empty(),
        "the Layer it made goes too"
    );
}
