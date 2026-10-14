use ssmap_core::{Canvas, Command, DistanceUnit, History, Id, Map, Scale};

fn empty_map() -> Map {
    Map::new(Canvas::new(100, 100, Scale::new(1.0, DistanceUnit::Miles)))
}

fn create(id: Id, name: &str) -> Command {
    Command::CreateLayer {
        id,
        parent: None,
        index: None,
        name: name.into(),
    }
}

#[test]
fn undo_and_redo_restore_the_recorded_selection() {
    let mut map = empty_map();
    let mut history = History::default();
    let layer = Id::new();
    history.begin("Create and select", vec![]).unwrap();
    history.execute(&mut map, &create(layer, "Land")).unwrap();
    history.commit(vec![layer]).unwrap();

    assert_eq!(history.undo_label(), Some("Create and select"));
    assert_eq!(history.undo(&mut map).unwrap(), Some(vec![]));
    assert!(!map.contains(layer));
    assert_eq!(history.redo(&mut map).unwrap(), Some(vec![layer]));
    assert!(map.contains(layer));
}

#[test]
fn a_new_step_clears_redo() {
    let mut map = empty_map();
    let mut history = History::default();
    history
        .execute_step(&mut map, &create(Id::new(), "A"), vec![])
        .unwrap();
    history.undo(&mut map).unwrap();
    assert_eq!(history.redo_len(), 1);
    history
        .execute_step(&mut map, &create(Id::new(), "B"), vec![])
        .unwrap();
    assert_eq!(history.redo_len(), 0);
}

#[test]
fn the_budget_drops_the_oldest_steps_but_keeps_the_newest() {
    let mut map = empty_map();
    // Small enough that only a couple of renames fit.
    let mut history = History::new(600);
    let layer = Id::new();
    history
        .execute_step(&mut map, &create(layer, "Land"), vec![])
        .unwrap();
    for i in 0..20 {
        let rename = Command::RenameLayer {
            id: layer,
            name: format!("Land {i}"),
        };
        history.execute_step(&mut map, &rename, vec![]).unwrap();
    }
    let kept = history.undo_len();
    assert!(
        (1..21).contains(&kept),
        "kept {kept} of 21 steps under the budget"
    );
    while history.undo(&mut map).unwrap().is_some() {}
    // The oldest steps are gone, so the Layer survives with an intermediate name.
    assert!(map.contains(layer));

    let mut tiny = History::new(0);
    tiny.execute_step(&mut map, &create(Id::new(), "Big"), vec![])
        .unwrap();
    assert_eq!(tiny.undo_len(), 1, "the newest step is always kept");
}

#[test]
fn a_failed_step_records_nothing_and_leaves_no_transaction_open() {
    let mut map = empty_map();
    let mut history = History::default();
    let missing = Command::RenameLayer {
        id: Id::new(),
        name: "Nowhere".into(),
    };
    assert!(history.execute_step(&mut map, &missing, vec![]).is_err());
    assert!(!history.is_open());
    assert_eq!(history.undo_len(), 0);
}

#[test]
fn commands_have_a_json_schema() {
    let schema = serde_json::to_value(schemars::schema_for!(Command)).unwrap();
    assert_eq!(schema["title"], "Command");
}

#[test]
fn recorded_ops_replay_commits_undos_and_redos_onto_the_starting_map() {
    let start = empty_map();
    let mut map = start.clone();
    let mut history = History::default();
    history.set_recording(true);
    let (a, b) = (Id::new(), Id::new());
    history
        .execute_step(&mut map, &create(a, "A"), vec![])
        .unwrap();
    history
        .execute_step(&mut map, &create(b, "B"), vec![])
        .unwrap();
    // A gesture's Ops count only once it commits; a rolled-back one never.
    history.begin("Rename", vec![]).unwrap();
    for name in ["A1", "A2"] {
        let rename = Command::RenameLayer {
            id: a,
            name: name.into(),
        };
        history.execute(&mut map, &rename).unwrap();
    }
    history.commit(vec![]).unwrap();
    history.begin("Cancelled", vec![]).unwrap();
    history.execute(&mut map, &create(Id::new(), "C")).unwrap();
    history.rollback(&mut map).unwrap();
    // A rewound gesture counts only what it holds when it commits.
    history.begin("Rewound", vec![]).unwrap();
    history.execute(&mut map, &create(Id::new(), "C")).unwrap();
    history.rewind(&mut map).unwrap();
    history.execute(&mut map, &create(Id::new(), "D")).unwrap();
    history.commit(vec![]).unwrap();
    // A refused Command records nothing.
    let missing = Command::RenameLayer {
        id: Id::new(),
        name: "Nowhere".into(),
    };
    assert!(history.execute_step(&mut map, &missing, vec![]).is_err());
    history.undo(&mut map).unwrap();
    history.undo(&mut map).unwrap();
    history.redo(&mut map).unwrap();

    let mut replayed = start.clone();
    for op in history.take_applied() {
        replayed.apply(op).unwrap();
    }
    assert_eq!(replayed.view(), map.view());
    assert!(
        history.take_applied().is_empty(),
        "taking drains the record"
    );

    // Recording resumes from wherever the Map is now.
    let checkpoint = map.clone();
    history.undo(&mut map).unwrap();
    let mut replayed = checkpoint;
    for op in history.take_applied() {
        replayed.apply(op).unwrap();
    }
    assert_eq!(replayed.view(), map.view());
}

#[test]
fn history_records_nothing_unless_asked() {
    let mut map = empty_map();
    let mut history = History::default();
    history
        .execute_step(&mut map, &create(Id::new(), "A"), vec![])
        .unwrap();
    history.undo(&mut map).unwrap();
    assert!(history.take_applied().is_empty());
}
