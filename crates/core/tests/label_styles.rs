use ssmap_core::file::MapFile;
use ssmap_core::{
    Canvas, Color, Command, DistanceUnit, History, Id, Label, LabelStyle, LabelStyles, Map, Scale,
};

fn map_with_layer() -> (Map, Id) {
    let mut map = Map::new(Canvas::new(
        2000,
        2000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    let layer = Id::new();
    map.execute(&Command::CreateLayer {
        id: layer,
        parent: None,
        index: None,
        name: "Labels".into(),
    })
    .unwrap();
    (map, layer)
}

fn style(name: &str, size: f64) -> LabelStyle {
    LabelStyle {
        size,
        fill: Color::rgb(10, 20, 30),
        ..LabelStyle::new(Id::new(), name)
    }
}

fn place(map: &mut Map, layer: Id, label: Label) -> Id {
    let id = Id::new();
    map.execute(&Command::CreateLabel {
        id,
        parent: layer,
        index: None,
        label: Box::new(Label {
            text: "Name".into(),
            ..label
        }),
    })
    .unwrap();
    id
}

#[test]
fn restyling_a_thousand_labels_is_one_undo_step() {
    let (mut map, layer) = map_with_layer();
    let realm = style("Realm", 80.0);
    map.execute(&Command::CreateLabelStyle {
        style: realm.clone(),
    })
    .unwrap();
    let ids: Vec<Id> = (0..1000)
        .map(|i| {
            place(
                &mut map,
                layer,
                Label {
                    style: realm.id,
                    position: [i as f64, 0.0],
                    ..Label::default()
                },
            )
        })
        .collect();

    let mut history = History::default();
    let bigger = LabelStyle {
        size: 120.0,
        fill: Color::rgb(200, 0, 0),
        ..realm.clone()
    };
    history
        .execute_step(&mut map, &Command::SetLabelStyle { style: bigger }, vec![])
        .unwrap();
    assert_eq!(history.undo_len(), 1);
    for &id in &ids {
        let label = map.resolved_label(id).unwrap();
        assert_eq!(label.size, 120.0);
        assert_eq!(label.fill, Color::rgb(200, 0, 0));
    }

    history.undo(&mut map).unwrap();
    assert!(
        ids.iter()
            .all(|&id| map.resolved_label(id).unwrap().size == 80.0)
    );
    assert_eq!(history.undo_len(), 0);
}

#[test]
fn an_override_wins_over_its_style() {
    let (mut map, layer) = map_with_layer();
    let realm = style("Realm", 80.0);
    map.execute(&Command::CreateLabelStyle {
        style: realm.clone(),
    })
    .unwrap();
    let own = place(
        &mut map,
        layer,
        Label {
            style: realm.id,
            size: Some(30.0),
            ..Label::default()
        },
    );
    let followed = place(
        &mut map,
        layer,
        Label {
            style: realm.id,
            ..Label::default()
        },
    );
    map.execute(&Command::SetLabelStyle {
        style: LabelStyle {
            size: 100.0,
            ..realm
        },
    })
    .unwrap();
    assert_eq!(map.resolved_label(own).unwrap().size, 30.0);
    assert_eq!(map.resolved_label(followed).unwrap().size, 100.0);
}

#[test]
fn deleting_a_style_in_use_changes_nothing_visible_and_undoes_in_one_step() {
    let (mut map, layer) = map_with_layer();
    let realm = style("Realm", 80.0);
    map.execute(&Command::CreateLabelStyle {
        style: realm.clone(),
    })
    .unwrap();
    let plain = place(
        &mut map,
        layer,
        Label {
            style: realm.id,
            ..Label::default()
        },
    );
    let tweaked = place(
        &mut map,
        layer,
        Label {
            style: realm.id,
            size: Some(12.0),
            ..Label::default()
        },
    );
    let elsewhere = place(&mut map, layer, Label::default());
    let before: Vec<_> = [plain, tweaked, elsewhere]
        .map(|id| map.resolved_label(id).unwrap())
        .into();
    assert_eq!(map.labels_using(realm.id).len(), 2);

    let mut history = History::default();
    history
        .execute_step(
            &mut map,
            &Command::DeleteLabelStyle { id: realm.id },
            vec![],
        )
        .unwrap();
    assert_eq!(history.undo_len(), 1);
    let after: Vec<_> = [plain, tweaked, elsewhere]
        .map(|id| map.resolved_label(id).unwrap())
        .into();
    assert_eq!(before, after);
    assert!(map.label_styles().get(realm.id).is_none());
    assert!(map.labels_using(realm.id).is_empty());

    history.undo(&mut map).unwrap();
    assert!(map.label_styles().get(realm.id).is_some());
    assert_eq!(map.labels_using(realm.id).len(), 2);
}

#[test]
fn deleting_a_style_rewrites_locked_labels_too() {
    let (mut map, layer) = map_with_layer();
    let realm = style("Realm", 80.0);
    map.execute(&Command::CreateLabelStyle {
        style: realm.clone(),
    })
    .unwrap();
    let id = place(
        &mut map,
        layer,
        Label {
            style: realm.id,
            ..Label::default()
        },
    );
    map.execute(&Command::SetLocked { id, locked: true })
        .unwrap();
    map.execute(&Command::DeleteLabelStyle { id: realm.id })
        .unwrap();
    assert_eq!(map.resolved_label(id).unwrap().size, 80.0);
}

#[test]
fn the_default_style_stays_and_names_stay_unique() {
    let (mut map, _) = map_with_layer();
    assert!(
        map.execute(&Command::DeleteLabelStyle {
            id: LabelStyle::DEFAULT_ID
        })
        .is_err()
    );
    assert!(
        map.execute(&Command::CreateLabelStyle {
            style: style("Default", 10.0)
        })
        .is_err()
    );
    let city = style("City", 40.0);
    map.execute(&Command::CreateLabelStyle {
        style: city.clone(),
    })
    .unwrap();
    assert!(
        map.execute(&Command::RenameLabelStyle {
            id: city.id,
            name: " Default ".into()
        })
        .is_err()
    );
    map.execute(&Command::RenameLabelStyle {
        id: city.id,
        name: "Town".into(),
    })
    .unwrap();
    assert_eq!(map.label_styles().get(city.id).unwrap().name, "Town");
}

#[test]
fn starter_styles_are_valid_and_survive_a_save() {
    let styles = ssmap_core::LabelStyles::starter();
    let names: Vec<_> = styles.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Default", "Realm", "City", "Sea", "Range"]);
    assert!(styles.iter().all(|s| s.problem().is_none()));
    let json = serde_json::to_string(&styles).unwrap();
    let back: ssmap_core::LabelStyles = serde_json::from_str(&json).unwrap();
    assert_eq!(back, styles);
}

#[test]
fn a_label_must_name_a_known_style() {
    let (mut map, layer) = map_with_layer();
    let result = map.execute(&Command::CreateLabel {
        id: Id::new(),
        parent: layer,
        index: None,
        label: Box::new(Label {
            text: "Lost".into(),
            style: Id::new(),
            ..Label::default()
        }),
    });
    assert!(result.is_err());
}

#[test]
fn styles_survive_a_save_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("styled.ssmap");
    let (mut map, layer) = map_with_layer();
    let realm = style("Realm", 80.0);
    map.execute(&Command::CreateLabelStyle {
        style: realm.clone(),
    })
    .unwrap();
    let label = place(
        &mut map,
        layer,
        Label {
            style: realm.id,
            ..Label::default()
        },
    );
    let (mut file, _) = MapFile::create(&path, &map, None).unwrap();

    map.execute(&Command::SetLabelStyle {
        style: LabelStyle {
            size: 90.0,
            ..realm.clone()
        },
    })
    .unwrap();
    file.save(&map, None).unwrap();

    let (_, reopened) = MapFile::open(&path).unwrap();
    assert_eq!(reopened.label_styles(), map.label_styles());
    assert_eq!(reopened.resolved_label(label).unwrap().size, 90.0);
}

#[test]
fn labels_saved_before_styles_open_as_overrides_of_the_default_style() {
    let json = r##"{
        "text": "Old", "position": [5, 6], "align": "left", "rotation": 0,
        "font": {"family": "Almendra", "weight": 400, "italic": true}, "size": 33,
        "letter_spacing": 0.1, "line_height": 1.5, "all_caps": true, "fill": "#102030",
        "opacity": 0.5, "halo": {"width": 2, "color": "#ffffff"}
    }"##;
    let label: Label = serde_json::from_str(json).unwrap();
    assert_eq!(label.style, LabelStyle::DEFAULT_ID);
    let map = Map::new(Canvas::new(10, 10, Scale::new(1.0, DistanceUnit::Miles)));
    let resolved = label.resolve(map.label_styles());
    assert_eq!(resolved.font.family, "Almendra");
    assert_eq!(resolved.size, 33.0);
    assert!(resolved.all_caps);
    assert_eq!(resolved.halo.width, 2.0);
    assert_eq!(resolved.opacity, 0.5);
    assert_eq!(resolved.arch, 0.0);
}

#[test]
fn arch_overrides_its_style_round_trips_and_is_range_checked() {
    let style = LabelStyle {
        arch: -0.25,
        ..LabelStyle::default()
    };
    let styles = LabelStyles::new(vec![style]);
    let mut label = Label::default();
    assert_eq!(label.resolve(&styles).arch, -0.25);
    label.arch = Some(0.5);
    assert_eq!(label.resolve(&styles).arch, 0.5);

    let json = serde_json::to_string(&label).unwrap();
    let back: Label = serde_json::from_str(&json).unwrap();
    assert_eq!(back.arch, Some(0.5));

    let mut edited = label.resolve(&styles);
    edited.arch = 0.5;
    assert_eq!(label.with_resolved(&styles, &edited).arch, Some(0.5));
    edited.arch = 0.75;
    assert_eq!(label.with_resolved(&styles, &edited).arch, Some(0.75));

    label.text = "x".into();
    assert_eq!(label.problem(), None);
    label.arch = Some(1.5);
    assert!(label.problem().is_some());
}
