//! The starter template and the default pack flag.

use std::path::Path;

use ssmap_core::asset::{PackLibrary, PackSource, parse_manifest};
use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Color, Command, DistanceUnit, Id, LabelStyles, Landmass, MapTemplate, MultiPolygon,
    Node, Scale, ShoreStyle, TemplateError,
};

fn canvas() -> Canvas {
    Canvas::new(2000, 1000, Scale::new(5.0, DistanceUnit::Kilometres))
}

#[test]
fn the_starter_template_is_a_ready_layer_stack() {
    let map = MapTemplate::starter().unwrap().new_map(canvas());
    let expected = Canvas {
        template: Some("Ocean".to_owned()),
        ..canvas()
    };
    assert_eq!(map.canvas(), &expected);
    map.validate().unwrap();

    let layers: Vec<(String, bool)> = map
        .children(None)
        .iter()
        .map(|&id| match map.node(id) {
            Some(Node::Layer(layer)) => (layer.name.clone(), layer.depth_sort),
            _ => panic!("an Object at the top level"),
        })
        .collect();
    assert_eq!(
        layers,
        [
            ("Backdrop".to_owned(), false),
            ("Land".to_owned(), false),
            ("Assets".to_owned(), true),
            ("Labels".to_owned(), false),
        ]
    );
}

#[test]
fn the_starter_template_sets_the_map_wide_defaults() {
    let map = MapTemplate::starter().unwrap().new_map(canvas());
    let defaults = map.defaults();
    assert_eq!(defaults.land_fill, Landmass::DEFAULT_FILL);
    assert_eq!(defaults.shore, ShoreStyle::landmass_default());
    assert_eq!(defaults.lake_shore, ShoreStyle::lake_default());
    assert_eq!(defaults.roughness, Default::default());
    assert_eq!(*map.label_styles(), LabelStyles::starter());
    let names: Vec<&str> = map.label_styles().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Default", "Realm", "City", "Sea", "Range"]);
}

#[test]
fn land_painted_into_a_new_map_follows_the_default_land_colour() {
    let mut map = MapTemplate::starter().unwrap().new_map(canvas());
    let land = map.children(None)[1];
    let square = ssmap_core::Polygon {
        exterior: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        holes: Vec::new(),
    };
    let id = Id::new();
    map.execute(&Command::CreateLandmass {
        id,
        parent: land,
        index: None,
        geometry: MultiPolygon(vec![square]),
        fill: None,
    })
    .unwrap();
    let fill = |map: &ssmap_core::Map| match map.node(id) {
        Some(Node::Object(o)) => o.kind.fill(),
        _ => None,
    };
    assert_eq!(fill(&map), Some(Landmass::DEFAULT_FILL));
}

#[test]
fn a_template_can_set_the_land_colour() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/templates/starter.json"),
    )
    .unwrap()
    .replace("#d8c49a", "#102030");
    let map = MapTemplate::parse(&text).unwrap().new_map(canvas());
    assert_eq!(map.defaults().land_fill, Color::rgb(0x10, 0x20, 0x30));
}

#[test]
fn a_map_from_the_template_saves_and_reopens() {
    let map = MapTemplate::starter().unwrap().new_map(canvas());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.ssmap");
    MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.view(), map.view());
}

#[test]
fn every_bundled_template_loads_validates_and_round_trips() {
    let (templates, problems) = MapTemplate::load_all();
    assert!(problems.is_empty(), "{problems:?}");
    let ids: Vec<&str> = templates.iter().map(|t| t.id()).collect();
    assert_eq!(
        ids,
        [
            "starter",
            "dark-ink",
            "flat-colour",
            "parchment",
            "tropical",
            "winter"
        ]
    );
    let names: Vec<&str> = templates.iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        [
            "Ocean",
            "Dark Ink",
            "Flat Colour",
            "Parchment",
            "Tropical",
            "Winter"
        ]
    );

    let dir = tempfile::tempdir().unwrap();
    let mut backgrounds = Vec::new();
    for template in &templates {
        let map = template.new_map(canvas());
        map.validate().unwrap();
        assert_eq!(map.canvas().width, 2000);
        let path = dir.path().join(format!("{}.ssmap", template.id()));
        MapFile::create(&path, &map, None).unwrap();
        let (_, reopened) = MapFile::open(&path).unwrap();
        assert_eq!(reopened.view(), map.view(), "{}", template.name());
        assert_eq!(reopened.canvas().background, map.canvas().background);
        backgrounds.push(map.canvas().background.clone());
    }
    for (i, background) in backgrounds.iter().enumerate() {
        assert!(!backgrounds[i + 1..].contains(background));
    }
}

#[test]
fn a_map_takes_the_templates_background_and_the_chosen_canvas() {
    let (templates, _) = MapTemplate::load_all();
    let flat = templates.iter().find(|t| t.id() == "flat-colour").unwrap();
    let map = flat.new_map(canvas());
    assert_eq!(map.canvas().width, 2000);
    assert_eq!(map.canvas().height, 1000);
    assert_eq!(map.canvas().background.texture.strength, 0.0);
    assert_eq!(map.canvas().background.water, Color::rgb(0x6f, 0x9f, 0xc0));
}

#[test]
fn a_map_remembers_its_template_and_a_background_change_keeps_it() {
    let templates = MapTemplate::load_all().0;
    let (ocean, parchment) = (&templates[0], &templates[3]);
    let mut map = ocean.new_map(canvas());
    assert_eq!(map.canvas().template.as_deref(), Some("Ocean"));

    let mut history = ssmap_core::History::default();
    let original = map.canvas().background.clone();
    let background = parchment.background().clone();
    history
        .execute_step(&mut map, &Command::SetBackground { background }, Vec::new())
        .unwrap();
    assert_eq!(map.canvas().background, *parchment.background());
    assert_eq!(map.canvas().template.as_deref(), Some("Ocean"));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changed.ssmap");
    MapFile::create(&path, &map, None).unwrap();
    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.canvas().background, *parchment.background());
    assert_eq!(reopened.canvas().template.as_deref(), Some("Ocean"));

    history.undo(&mut map).unwrap();
    assert_eq!(map.canvas().background, original);
}

#[test]
fn a_background_with_a_texture_strength_out_of_range_is_refused() {
    let mut map = MapTemplate::starter().unwrap().new_map(canvas());
    let mut background = map.canvas().background.clone();
    background.texture.strength = 1.5;
    assert!(map.execute(&Command::SetBackground { background }).is_err());
}

#[test]
fn a_broken_template_is_reported_and_the_rest_still_load() {
    let dir = tempfile::tempdir().unwrap();
    let starter = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/templates/starter.json"),
    )
    .unwrap();
    std::fs::write(dir.path().join("a-broken.json"), "{}").unwrap();
    std::fs::write(dir.path().join("z-fine.json"), &starter).unwrap();
    std::fs::write(dir.path().join("starter.json"), &starter).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
    let (templates, problems) = MapTemplate::load_dir(dir.path());
    let ids: Vec<&str> = templates.iter().map(|t| t.id()).collect();
    assert_eq!(ids, ["starter", "z-fine"]);
    assert_eq!(problems.len(), 1);
}

#[test]
fn bad_templates_are_refused() {
    let starter = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/templates/starter.json"),
    )
    .unwrap();
    let newer = starter.replace("\"1.3\"", "\"2.0\"");
    assert!(matches!(
        MapTemplate::parse(&newer),
        Err(TemplateError::UnsupportedVersion { .. })
    ));
    let garbled = starter.replace("\"1.3\"", "\"one\"");
    assert!(matches!(
        MapTemplate::parse(&garbled),
        Err(TemplateError::BadVersion(_))
    ));
    assert!(matches!(
        MapTemplate::parse("{}"),
        Err(TemplateError::Parse(_))
    ));
    let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-template.json");
    assert!(matches!(
        MapTemplate::load(missing),
        Err(TemplateError::Read { .. })
    ));
}

fn pack(extra: &str) -> String {
    format!(
        "[pack]\nid = \"p\"\nname = \"P\"\nlicence = \"CC0-1.0\"\ncredits = \"Nobody.\"\n{extra}"
    )
}

#[test]
fn the_default_pack_is_the_one_flagged() {
    let plain = parse_manifest(Path::new("a/pack.toml"), &pack(""), PackSource::BuiltIn).unwrap();
    assert!(!plain.default);
    let flagged = parse_manifest(
        Path::new("b/pack.toml"),
        &pack("default = true\n").replace("id = \"p\"", "id = \"q\""),
        PackSource::BuiltIn,
    )
    .unwrap();
    assert!(flagged.default);

    let mut library = PackLibrary::default();
    let mut problems = Vec::new();
    library.add(plain, &mut problems);
    assert!(library.default_pack().is_none(), "no pack is flagged yet");
    library.add(flagged, &mut problems);
    assert_eq!(library.default_pack().map(|p| p.id.as_str()), Some("q"));
}

#[test]
fn the_default_flag_must_be_a_boolean() {
    let err = parse_manifest(
        Path::new("a/pack.toml"),
        &pack("default = \"yes\"\n"),
        PackSource::BuiltIn,
    )
    .unwrap_err();
    assert!(err.to_string().contains("pack.default"), "{err}");
}
