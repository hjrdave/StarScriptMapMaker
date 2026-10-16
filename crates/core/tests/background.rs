//! The Canvas's background block: defaults, round-trip and older files.

use ssmap_core::{Background, Canvas, Color, DistanceUnit, Scale};

fn canvas() -> Canvas {
    Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles))
}

#[test]
fn a_canvas_without_a_background_block_gets_the_default() {
    let old = r#"{"width":800,"height":600,"scale":{"distance_per_unit":1.0,"unit":"miles"}}"#;
    let parsed: Canvas = serde_json::from_str(old).unwrap();
    assert_eq!(parsed.background, Background::default());
}

#[test]
fn the_default_background_is_left_out_of_the_file() {
    let json = serde_json::to_value(canvas()).unwrap();
    assert!(json.get("background").is_none());
}

#[test]
fn a_custom_background_round_trips() {
    let mut c = canvas();
    c.background.water = Color::rgb(0xe8, 0xd8, 0xb0);
    c.background.texture.strength = 0.0;
    c.background.texture.scale = 2.5;
    c.background.texture.dark = Color::rgb(0x40, 0x30, 0x10);
    let json = serde_json::to_string(&c).unwrap();
    assert!(json.contains("\"background\""));
    let back: Canvas = serde_json::from_str(&json).unwrap();
    assert_eq!(back, c);
}

#[test]
fn a_partial_texture_block_takes_defaults_for_the_rest() {
    let text = r##"{"water":"#112233","texture":{"strength":0.5}}"##;
    let background: Background = serde_json::from_str(text).unwrap();
    assert_eq!(background.water, Color::rgb(0x11, 0x22, 0x33));
    assert_eq!(background.texture.strength, 0.5);
    assert_eq!(background.texture.scale, 1.0);
}
