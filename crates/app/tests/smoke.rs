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
