//! Applies JSON Command scripts headlessly and snapshots what happened (ADR 0004).
//!
//! Each `tests/scripts/*.json` file has optional `setup` Ops, applied directly, and `steps`:
//!
//! - `{"begin": {"label": "...", "selection": [ids]}}`, `{"commit": {"selection": [ids]}}`,
//!   `"rollback"`: transactions. `"rewind"` undoes the open transaction's Commands but keeps
//!   it open, as a drag does each frame.
//! - `{"execute": <Command>}`: run a Command inside the open transaction.
//! - `{"step": <Command>}`: run a Command as its own Undo step.
//! - `"undo"`, `"redo"`.
//! - `{"brush": {"selection": [ids], "active_layer": id, "start": [x, y], "region": <outline>,
//!   "erase": bool, "new_id": id}}`: a finished land brush stroke. The runner records the
//!   Command the brush rules choose and runs it as its own Undo step. Regions are given
//!   directly, since stroking a path uses trigonometry that differs in the last bit by OS.
//! - `"snapshot"`: record the read view at this point. The final view is always recorded.
//!
//! The snapshot lists every step with its outcome. After every step the runner checks that
//! the tree is valid and that no Clip points at a missing Landmass.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests and benches may panic on failure"
)]

use serde::Deserialize;
use serde_json::{Value, json};
use ssmap_core::land::{BrushStroke, brush_command};
use ssmap_core::{
    Canvas, Command, DistanceUnit, History, Id, Map, MultiPolygon, Node, ObjectKind, Op, Point,
    RoughnessCache, Scale, Selection,
};

#[derive(Deserialize)]
struct Script {
    #[serde(default)]
    setup: Vec<Op>,
    steps: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Begin {
        label: String,
        #[serde(default)]
        selection: Selection,
    },
    Commit {
        #[serde(default)]
        selection: Selection,
    },
    Rollback,
    Rewind,
    Execute(Command),
    Step(Command),
    Undo,
    Redo,
    Brush {
        #[serde(default)]
        selection: Selection,
        active_layer: Option<Id>,
        start: Point,
        region: MultiPolygon,
        #[serde(default)]
        erase: bool,
        new_id: Id,
    },
    Snapshot,
}

fn run(script: Script) -> Value {
    let mut map = Map::new(Canvas::new(
        1000,
        1000,
        Scale::new(1.0, DistanceUnit::Miles),
    ));
    for op in script.setup {
        map.apply(op).expect("setup Ops apply");
    }
    let mut history = History::default();
    let mut log = Vec::new();
    for raw in script.steps {
        let action: Action = serde_json::from_value(raw.clone())
            .unwrap_or_else(|err| panic!("bad step {raw}: {err}"));
        let outcome = match action {
            Action::Begin { label, selection } => result(history.begin(label, selection)),
            Action::Commit { selection } => result(history.commit(selection)),
            Action::Rollback => result(history.rollback(&mut map)),
            Action::Rewind => result(history.rewind(&mut map)),
            Action::Execute(command) => result(history.execute(&mut map, &command)),
            Action::Step(command) => result(history.execute_step(&mut map, &command, Vec::new())),
            Action::Undo => restored(history.undo(&mut map)),
            Action::Redo => restored(history.redo(&mut map)),
            Action::Brush {
                selection,
                active_layer,
                start,
                region,
                erase,
                new_id,
            } => {
                let stroke = BrushStroke {
                    region,
                    start,
                    erase,
                };
                let mut roughness = RoughnessCache::new();
                match brush_command(
                    &map,
                    &mut roughness,
                    &selection,
                    active_layer,
                    stroke,
                    new_id,
                ) {
                    Ok(Some(command)) => json!({
                        "command": command,
                        "result": result(history.execute_step(&mut map, &command, selection)),
                    }),
                    Ok(None) => json!("nothing to do"),
                    Err(err) => json!({ "error": err.to_string() }),
                }
            }
            Action::Snapshot => json!({ "view": map.view() }),
        };
        check(&map);
        log.push(json!({ "step": raw, "outcome": outcome }));
    }
    json!({
        "log": log,
        "undo_steps": history.undo_len(),
        "redo_steps": history.redo_len(),
        "view": map.view(),
    })
}

fn result<E: std::fmt::Display>(result: Result<(), E>) -> Value {
    match result {
        Ok(()) => json!("ok"),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

fn restored<E: std::fmt::Display>(result: Result<Option<Selection>, E>) -> Value {
    match result {
        Ok(Some(selection)) => json!({ "selection": selection }),
        Ok(None) => json!("nothing to do"),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

fn check(map: &Map) {
    map.validate().expect("the tree stays valid");
    for id in map.ids() {
        if let Some(Node::Layer(layer)) = map.node(id)
            && let Some(target) = layer.clip
        {
            let is_landmass = matches!(
                map.node(target),
                Some(Node::Object(object)) if matches!(object.kind, ObjectKind::Landmass(_))
            );
            assert!(is_landmass, "Layer {id} clips to missing Landmass {target}");
        }
    }
}

#[test]
fn command_scripts() {
    insta::glob!("scripts/*.json", |path| {
        let text = std::fs::read_to_string(path).unwrap();
        let script: Script =
            serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
        insta::assert_json_snapshot!(run(script));
    });
}
