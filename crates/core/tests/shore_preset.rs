//! The Shore preset last picked, kept on a Shore style (issue 651, format 1.22).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use rusqlite::Connection;
use ssmap_core::file::{FORMAT_VERSION, MapFile};
use ssmap_core::{
    Canvas, Command, DistanceUnit, Id, Map, MultiPolygon, Polygon, Scale, ShoreKind, ShoreStyle,
};

fn map_with_land() -> (Map, Id) {
    let mut map = Map::new(Canvas::new(400, 400, Scale::new(1.0, DistanceUnit::Miles)));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Land".into(),
    })
    .unwrap();
    let land = Id::new();
    map.execute(&Command::CreateLandmass {
        id: land,
        parent: Some(layer),
        index: None,
        geometry: MultiPolygon(vec![Polygon {
            exterior: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            holes: vec![],
        }]),
        fill: None,
    })
    .unwrap();
    (map, land)
}

fn meta(path: &std::path::Path, key: &str) -> Option<String> {
    let conn = Connection::open(path).unwrap();
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .ok()
}

#[test]
fn a_picked_preset_carries_its_name_and_still_reads_as_that_preset() {
    let picked = ShoreStyle::picked(ShoreKind::Landmass, "Hand drawn").unwrap();
    assert_eq!(picked.preset.as_deref(), Some("Hand drawn"));
    assert!(picked.same_look(&ShoreStyle::hand_drawn()));
    assert_eq!(picked.preset_name(ShoreKind::Landmass), Some("Hand drawn"));
    // Nothing changed since the pick.
    assert_eq!(picked.changed_from_preset(ShoreKind::Landmass), None);
    // Not a preset for Lakes, and no such preset at all.
    assert_eq!(ShoreStyle::picked(ShoreKind::Lake, "Hand drawn"), None);
    assert_eq!(ShoreStyle::picked(ShoreKind::Landmass, "Stormy"), None);
}

#[test]
fn an_edit_after_a_pick_offers_the_preset_back() {
    let mut style = ShoreStyle::picked(ShoreKind::Landmass, "Soft glow").unwrap();
    style.outline.width = 7.0;
    assert_eq!(style.preset_name(ShoreKind::Landmass), None, "Custom now");
    let back = style.changed_from_preset(ShoreKind::Landmass).unwrap();
    assert!(back.same_look(&ShoreStyle::soft_glow()));
    assert_eq!(back.preset.as_deref(), Some("Soft glow"));

    // A style with no preset picked, or one naming a preset that isn't there, offers nothing.
    let mut plain = ShoreStyle::landmass_default();
    plain.outline.width = 7.0;
    assert_eq!(plain.changed_from_preset(ShoreKind::Landmass), None);
    plain.preset = Some("Stormy".into());
    assert_eq!(plain.changed_from_preset(ShoreKind::Landmass), None);
}

#[test]
fn the_name_is_left_out_of_the_json_until_a_preset_is_picked() {
    let json = serde_json::to_value(ShoreStyle::landmass_default()).unwrap();
    assert!(json.get("preset").is_none());
    let picked = ShoreStyle::picked(ShoreKind::Lake, "Default").unwrap();
    let json = serde_json::to_value(&picked).unwrap();
    assert_eq!(json["preset"], "Default");

    // A style written before the field existed reads with none.
    let mut old = json;
    old.as_object_mut().unwrap().remove("preset");
    let style: ShoreStyle = serde_json::from_value(old).unwrap();
    assert_eq!(style.preset, None);
    assert_eq!(style, ShoreStyle::lake_default());
}

#[test]
fn the_pick_is_part_of_the_set_shore_command_and_undoes_with_it() {
    let (mut map, land) = map_with_land();
    let mut edited = ShoreStyle::picked(ShoreKind::Landmass, "Hand drawn").unwrap();
    edited.outline.width = 9.0;
    let inverses = map
        .execute(&Command::SetShore {
            ids: vec![land],
            shore: Some(edited.clone()),
        })
        .unwrap();
    assert_eq!(map.shore(land), Some(&edited));
    map.revert(inverses);
    assert_eq!(map.shore(land), Some(&ShoreStyle::landmass_default()));
    assert_eq!(map.shore(land).and_then(|s| s.preset.clone()), None);
}

#[test]
fn the_last_preset_survives_save_and_open_on_a_landmass_and_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preset.ssmap");
    let (mut map, land) = map_with_land();
    let mut own = ShoreStyle::picked(ShoreKind::Landmass, "Soft glow").unwrap();
    own.glow.width = 40.0;
    map.execute(&Command::SetShore {
        ids: vec![land],
        shore: Some(own.clone()),
    })
    .unwrap();
    let lake = ShoreStyle::picked(ShoreKind::Lake, "Default").unwrap();
    map.execute(&Command::SetDefaultShore {
        kind: ShoreKind::Lake,
        shore: lake.clone(),
    })
    .unwrap();
    MapFile::create(&path, &map, None).unwrap();
    assert_eq!(meta(&path, "format_version").as_deref(), Some("1.22"));

    let (_, opened) = MapFile::open(&path).unwrap();
    assert_eq!(opened.shore(land), Some(&own));
    assert_eq!(
        opened
            .shore(land)
            .and_then(|s| s.changed_from_preset(ShoreKind::Landmass)),
        ShoreStyle::picked(ShoreKind::Landmass, "Soft glow")
    );
    assert_eq!(opened.defaults().shore_for(ShoreKind::Lake), &lake);
}

#[test]
fn an_older_file_opens_with_no_preset_and_saves_as_the_current_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.ssmap");
    let (mut map, land) = map_with_land();
    // What a format 1.21 app wrote: a Shore style of its own, with no preset name.
    map.execute(&Command::SetShore {
        ids: vec![land],
        shore: Some(ShoreStyle::hand_drawn()),
    })
    .unwrap();
    MapFile::create(&path, &map, None).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE meta SET value = '1.21' WHERE key = 'format_version'",
        [],
    )
    .unwrap();
    drop(conn);

    let (mut file, opened) = MapFile::open(&path).unwrap();
    let shore = opened.shore(land).unwrap();
    assert_eq!(shore.preset, None);
    // It still reads as the preset it matches, with nothing to go back to.
    assert_eq!(shore.preset_name(ShoreKind::Landmass), Some("Hand drawn"));
    assert_eq!(shore.changed_from_preset(ShoreKind::Landmass), None);
    file.save(&opened, None).unwrap();
    assert_eq!(
        meta(&path, "format_version").as_deref(),
        Some(FORMAT_VERSION.to_string().as_str())
    );
}
