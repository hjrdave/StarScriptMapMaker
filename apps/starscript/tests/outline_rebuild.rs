//! Changing the Map-wide Roughness computes the new outlines on a thread of their own, and the
//! Canvas draws the same coastlines as if they had been computed at once.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use std::time::{Duration, Instant};

use ssmap_app::editor::Editor;
use ssmap_core::{
    Canvas, Command, DistanceUnit, Id, Map, MultiPolygon, Polygon, Roughness, RoughnessCache, Scale,
};
use ssmap_render::{Content, Fonts, MapScene};

fn square(x: f64) -> MultiPolygon {
    MultiPolygon(vec![Polygon {
        exterior: vec![
            [x, 100.0],
            [x + 300.0, 100.0],
            [x + 300.0, 400.0],
            [x, 400.0],
        ],
        holes: Vec::new(),
    }])
}

fn editor() -> Editor {
    let mut editor = Editor::new(Map::new(Canvas::new(
        2000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    )));
    let layer = Id::new();
    editor
        .run(Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Land".into(),
        })
        .unwrap();
    for x in [0.0, 500.0, 1000.0] {
        editor
            .run(Command::CreateLandmass {
                id: Id::new(),
                parent: layer,
                index: None,
                geometry: square(x),
                fill: None,
            })
            .unwrap();
    }
    editor
}

fn fresh(editor: &Editor) -> Content {
    MapScene::new(Fonts::bundled()).content(editor.map(), &mut RoughnessCache::new())
}

fn assert_same(a: &Content, b: &Content) {
    assert_eq!(a.groups().len(), b.groups().len());
    for (x, y) in a.groups().iter().zip(b.groups()) {
        assert_eq!(x, y);
    }
}

fn set_roughness(editor: &mut Editor, amount: f64) {
    editor
        .run(Command::SetDefaultRoughness {
            roughness: Roughness::new(amount, 30.0, 3),
        })
        .unwrap();
}

#[test]
fn without_a_window_the_new_outlines_are_computed_at_once() {
    let mut editor = editor();
    set_roughness(&mut editor, 6.0);
    editor.scene_content();
    set_roughness(&mut editor, 14.0);
    let content = editor.scene_content();
    assert!(!editor.outlines_rebuilding());
    assert_same(&content, &fresh(&editor));
}

#[test]
fn with_a_window_they_are_computed_in_the_background_and_land_as_the_same_coastlines() {
    let mut editor = editor();
    editor.compute_shore_rings_in_background(|| {});
    set_roughness(&mut editor, 6.0);
    editor.scene_content();
    // Several quick changes, as a drag makes: only the last one's outlines are kept.
    for amount in [8.0, 10.0, 12.0, 14.0] {
        set_roughness(&mut editor, amount);
        editor.scene_content();
    }
    let started = Instant::now();
    while editor.outlines_rebuilding() {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the rebuild ends"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let content = editor.scene_content();
    let expected = fresh(&editor);
    // The outlines are the new ones. Rings may still be arriving, so compare the fills.
    let fills = |content: &Content| {
        content
            .groups()
            .iter()
            .filter(|group| !group.effect)
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(fills(&content), fills(&expected));
}
