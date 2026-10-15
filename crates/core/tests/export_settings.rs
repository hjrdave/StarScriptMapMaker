//! PNG export settings: size maths and persistence.

use rusqlite::Connection;
use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, CreditsOptions, DistanceUnit, ExportRegion, ExportSettings, ExportSizeError, Map, Rect,
    Scale, SizeMode,
};

fn canvas(width: u32, height: u32) -> Canvas {
    Canvas::new(width, height, Scale::new(1.0, DistanceUnit::Miles))
}

fn settings(size: SizeMode) -> ExportSettings {
    ExportSettings {
        size,
        ..ExportSettings::default()
    }
}

#[test]
fn default_is_1x_of_the_whole_canvas() {
    let size = ExportSettings::default()
        .output_size(&canvas(4000, 3000))
        .unwrap();
    assert_eq!((size.width, size.height, size.dpi), (4000, 3000, None));
    assert!(!size.warn_large);
}

#[test]
fn scale_multiplies_the_one_x_size() {
    let c = canvas(4000, 3000);
    for (factor, expected) in [
        (2.0, (8000, 6000)),
        (0.5, (2000, 1500)),
        (1.25, (5000, 3750)),
    ] {
        let size = settings(SizeMode::Scale { factor })
            .output_size(&c)
            .unwrap();
        assert_eq!((size.width, size.height), expected);
    }
}

#[test]
fn target_width_keeps_the_aspect() {
    let size = settings(SizeMode::Width { pixels: 2000 })
        .output_size(&canvas(4000, 3000))
        .unwrap();
    assert_eq!((size.width, size.height, size.dpi), (2000, 1500, None));
    // The height rounds to the nearest pixel.
    let size = settings(SizeMode::Width { pixels: 1000 })
        .output_size(&canvas(3, 2))
        .unwrap();
    assert_eq!((size.width, size.height), (1000, 667));
}

#[test]
fn inches_times_dpi_gives_pixels_and_records_the_dpi() {
    let size = settings(SizeMode::Print {
        width_inches: 24.0,
        dpi: 300,
    })
    .output_size(&canvas(4000, 3000))
    .unwrap();
    assert_eq!((size.width, size.height, size.dpi), (7200, 5400, Some(300)));
}

#[test]
fn a_custom_region_sets_the_aspect_and_the_one_x_size() {
    let mut s = settings(SizeMode::Scale { factor: 2.0 });
    s.region = ExportRegion::Custom {
        rect: Rect {
            min: [100.0, 100.0],
            max: [600.0, 400.0],
        },
    };
    let size = s.output_size(&canvas(4000, 3000)).unwrap();
    assert_eq!((size.width, size.height), (1000, 600));
    s.size = SizeMode::Width { pixels: 100 };
    let size = s.output_size(&canvas(4000, 3000)).unwrap();
    assert_eq!((size.width, size.height), (100, 60));
}

#[test]
fn the_cap_is_32768_per_side() {
    let c = canvas(16_384, 8_192);
    let ok = settings(SizeMode::Scale { factor: 2.0 })
        .output_size(&c)
        .unwrap();
    assert_eq!((ok.width, ok.height), (32_768, 16_384));
    let err = settings(SizeMode::Width { pixels: 32_769 })
        .output_size(&c)
        .unwrap_err();
    assert_eq!(
        err,
        ExportSizeError::TooLarge {
            width: 32_769,
            height: 16_385
        }
    );
    // The long side may be the height.
    let tall = settings(SizeMode::Scale { factor: 4.0 })
        .output_size(&canvas(1000, 8193))
        .unwrap_err();
    assert!(matches!(tall, ExportSizeError::TooLarge { .. }));
    let print = settings(SizeMode::Print {
        width_inches: 200.0,
        dpi: 300,
    })
    .output_size(&c);
    assert!(matches!(print, Err(ExportSizeError::TooLarge { .. })));
}

#[test]
fn above_16k_warns_but_16k_does_not() {
    let c = canvas(16_384, 100);
    let at = settings(SizeMode::Scale { factor: 1.0 })
        .output_size(&c)
        .unwrap();
    assert!(!at.warn_large);
    let over = settings(SizeMode::Width { pixels: 16_385 })
        .output_size(&c)
        .unwrap();
    assert!(over.warn_large);
    let tall = settings(SizeMode::Scale { factor: 2.0 })
        .output_size(&canvas(100, 8_193))
        .unwrap();
    assert!(tall.warn_large);
}

#[test]
fn nonsense_sizes_are_errors() {
    let c = canvas(100, 100);
    for size in [
        SizeMode::Scale { factor: 0.0 },
        SizeMode::Scale { factor: f64::NAN },
        SizeMode::Scale { factor: -1.0 },
        SizeMode::Scale { factor: 0.001 },
        SizeMode::Width { pixels: 0 },
        SizeMode::Print {
            width_inches: 0.0,
            dpi: 300,
        },
        SizeMode::Print {
            width_inches: 4.0,
            dpi: 0,
        },
    ] {
        assert_eq!(
            settings(size).output_size(&c),
            Err(ExportSizeError::InvalidSize)
        );
    }
    let empty = ExportSettings {
        region: ExportRegion::Custom {
            rect: Rect {
                min: [5.0, 5.0],
                max: [5.0, 50.0],
            },
        },
        ..ExportSettings::default()
    };
    assert_eq!(empty.output_size(&c), Err(ExportSizeError::EmptyRegion));
}

#[test]
fn settings_round_trip_through_a_saved_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("world.ssmap");
    let mut map = Map::new(canvas(4000, 3000));
    map.export_settings = ExportSettings {
        size: SizeMode::Print {
            width_inches: 36.0,
            dpi: 150,
        },
        region: ExportRegion::Custom {
            rect: Rect {
                min: [10.0, 20.0],
                max: [1010.0, 820.0],
            },
        },
        transparent_background: true,
        credits: CreditsOptions {
            text_file: false,
            png_chunk: true,
        },
        file_name: Some("poster.png".into()),
        ..ExportSettings::default()
    };
    let (mut file, _) = MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.export_settings, map.export_settings);
    assert_eq!(reopened.view().export_settings, map.export_settings);

    // A later change is saved too.
    map.export_settings.transparent_background = false;
    file.save(&map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert!(!reopened.export_settings.transparent_background);
}

#[test]
fn an_older_file_loads_with_default_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.ssmap");
    let mut map = Map::new(canvas(800, 600));
    map.export_settings.transparent_background = true;
    MapFile::create(&path, &map, None).unwrap();

    // Make it look like a format 1.2 file, which had no export settings.
    let conn = Connection::open(&path).unwrap();
    conn.execute("DELETE FROM meta WHERE key = 'export_settings'", [])
        .unwrap();
    conn.execute(
        "UPDATE meta SET value = '1.2' WHERE key = 'format_version'",
        [],
    )
    .unwrap();
    drop(conn);

    let (_, loaded) = MapFile::open(&path).unwrap();
    assert_eq!(loaded.export_settings, ExportSettings::default());
}

#[test]
fn unknown_fields_survive_and_no_path_is_stored() {
    let json = r#"{"size":{"mode":"scale","factor":2.0},"region":{"kind":"canvas"},
        "transparent_background":false,"credits":{"text_file":true,"png_chunk":true},
        "file_name":"a.png","future":7}"#;
    let parsed: ExportSettings = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.extra["future"], 7);
    let again = serde_json::to_value(&parsed).unwrap();
    assert_eq!(again["future"], 7);
    assert_eq!(again["file_name"], "a.png");
}
