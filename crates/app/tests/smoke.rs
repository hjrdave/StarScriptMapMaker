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
    let mut map = Map::new(Canvas {
        width: 1000,
        height: 1000,
        scale: Scale {
            distance_per_unit: 1.0,
            unit: DistanceUnit::Miles,
        },
    });
    let stamps = (0..100_000)
        .map(|_| Subtree {
            id: Id::new(),
            node: Node::Object(Object::new(ObjectKind::Stamp(Stamp {}))),
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
