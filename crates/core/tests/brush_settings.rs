//! Brush settings: saved with the Map, outside Commands (ADR 0020).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rusqlite::Connection;
use ssmap_core::file::MapFile;
use ssmap_core::{
    BrushSettings, Canvas, Color, DistanceUnit, LandBrushSettings, Map, MapTemplate,
    PaintBrushSettings, Scale,
};

fn map() -> Map {
    Map::new(Canvas::new(800, 600, Scale::new(1.0, DistanceUnit::Miles)))
}

fn changed() -> BrushSettings {
    BrushSettings {
        land_paint: LandBrushSettings {
            size: 120.0,
            irregularity: 0.4,
        },
        land_erase: LandBrushSettings {
            size: 20.0,
            irregularity: 0.0,
        },
        water_paint_size: 64.0,
        water_erase_size: 12.0,
        paint_color: Color::rgb(0x11, 0x22, 0x33),
        painting: PaintBrushSettings {
            size: 30.0,
            softness: 0.8,
            opacity: 0.5,
        },
        erasing: PaintBrushSettings {
            size: 90.0,
            softness: 0.1,
            opacity: 1.0,
        },
        ..BrushSettings::default()
    }
}

fn meta(path: &std::path::Path, key: &str) -> Option<String> {
    let conn = Connection::open(path).unwrap();
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .ok()
}

#[test]
fn a_new_map_starts_with_the_defaults() {
    assert_eq!(map().brush_settings, BrushSettings::default());
    assert_eq!(map().view().brush_settings, BrushSettings::default());
}

#[test]
fn settings_round_trip_through_a_saved_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ssmap");
    let mut map = map();
    map.brush_settings = changed();
    let (mut file, _) = MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.brush_settings, changed());
    assert_eq!(reopened.view().brush_settings, changed());

    // A later change is saved too, and so is going back to the defaults.
    map.brush_settings.land_erase.size = 7.0;
    file.save(&map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.brush_settings.land_erase.size, 7.0);
    map.brush_settings = BrushSettings::default();
    file.save(&map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.brush_settings, BrushSettings::default());

    // Save As keeps them.
    map.brush_settings = changed();
    let copy = dir.path().join("copy.ssmap");
    file.save_as(&copy, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&copy).unwrap();
    assert_eq!(reopened.brush_settings, changed());
}

#[test]
fn an_older_file_without_the_key_opens_with_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.ssmap");
    let mut map = map();
    map.brush_settings = changed();
    MapFile::create(&path, &map, None).unwrap();
    assert!(meta(&path, "brush_settings").is_some());

    // Make it look like a format 1.20 file, which had no brush settings.
    let conn = Connection::open(&path).unwrap();
    conn.execute("DELETE FROM meta WHERE key = 'brush_settings'", [])
        .unwrap();
    conn.execute(
        "UPDATE meta SET value = '1.20' WHERE key = 'format_version'",
        [],
    )
    .unwrap();
    drop(conn);

    let (mut file, loaded) = MapFile::open(&path).unwrap();
    assert_eq!(loaded.brush_settings, BrushSettings::default());
    // Its next save writes the current version.
    file.save(&loaded, None).unwrap();
    assert_eq!(meta(&path, "format_version").as_deref(), Some("1.21"));
}

#[test]
fn unknown_fields_in_the_saved_settings_survive_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("newer.ssmap");
    let mut map = map();
    map.brush_settings = changed();
    MapFile::create(&path, &map, None).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE meta SET value = json_set(value, '$.pencil', json('{\"size\":3}'))
         WHERE key = 'brush_settings'",
        [],
    )
    .unwrap();
    drop(conn);

    let (mut file, mut opened) = MapFile::open(&path).unwrap();
    opened.brush_settings.water_paint_size = 99.0;
    file.save(&opened, None).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&meta(&path, "brush_settings").unwrap()).unwrap();
    assert_eq!(saved["pencil"]["size"], 3);
    assert_eq!(saved["water_paint_size"], 99.0);
}

#[test]
fn a_template_made_from_a_map_does_not_keep_its_brush_settings() {
    let mut map = map();
    map.brush_settings = changed();
    let template = MapTemplate::from_map("Mine", &map, Default::default());
    let made = template.new_map(Canvas::new(400, 300, Scale::new(1.0, DistanceUnit::Miles)));
    assert_eq!(made.brush_settings, BrushSettings::default());
}
