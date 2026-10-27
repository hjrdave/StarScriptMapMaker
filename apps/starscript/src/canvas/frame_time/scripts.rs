//! Named interaction scripts for `--frame-time` (issue #245): a fixed sequence of edits, one per
//! frame, applied to a generated large Map. Everything but the new ids is deterministic, so runs
//! compare before and after a change.
//!
//! Each frame stands for one burst of input: one stretch of a brush stroke, one step of a drag
//! or one slider value. The harness times the edit, the scene rebuild and the GPU work apart.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "existing invariant panics; replace with error handling over time (#351)"
)]

use std::f64::consts::TAU;

use ssmap_core::land::{self, BrushStroke};
use ssmap_core::scatter::{ScatterBrush, ScatterStroke, Terrain, scatter_command};
use ssmap_core::{
    Command, History, Id, Map, Node, ObjectKind, PickIndex, Point, Rect, Roughness, RoughnessCache,
    Selection, ShoreKind, ShoreStyle, Stamp,
};
use ssmap_render::View;

use super::{LANDMASSES, PHASE_FRAMES, TARGET};

/// Runs `command` on `map` as its own Undo step, the way the editor does. `what` says why it
/// cannot fail.
pub(super) fn run(map: &mut Map, command: &Command, what: &str) {
    History::default()
        .execute_step(map, command, Selection::default())
        .expect(what);
}

/// Frames of an interaction script, about a second and a half of dragging at 60 fps.
pub(super) const EDIT_FRAMES: usize = 90;

/// The Map-wide Roughness (amount, scale, seed) of the generated Map: rough enough to be a
/// worst case.
const ROUGHNESS: (f64, f64, u32) = (60.0, 200.0, 7);

/// Stamps moved per frame by `drag-stamps`.
const DRAGGED_STAMPS: usize = 2_000;

/// The landmass the edits work on, near the middle of the grid.
const TARGET_LANDMASS: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Script {
    PanZoom,
    Paint,
    Erase,
    DragLandmass,
    DragStamps,
    Scatter,
    SliderAmount,
    SliderScale,
    SliderShore,
    Marquee,
}

impl Script {
    pub(super) const ALL: [Script; 10] = [
        Script::PanZoom,
        Script::Paint,
        Script::Erase,
        Script::DragLandmass,
        Script::DragStamps,
        Script::Scatter,
        Script::SliderAmount,
        Script::SliderScale,
        Script::SliderShore,
        Script::Marquee,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Script::PanZoom => "pan-zoom",
            Script::Paint => "paint",
            Script::Erase => "erase",
            Script::DragLandmass => "drag-landmass",
            Script::DragStamps => "drag-stamps",
            Script::Scatter => "scatter",
            Script::SliderAmount => "slider-amount",
            Script::SliderScale => "slider-scale",
            Script::SliderShore => "slider-shore",
            Script::Marquee => "marquee",
        }
    }

    pub(super) fn parse(name: &str) -> Option<Script> {
        Script::ALL.into_iter().find(|script| script.name() == name)
    }

    /// Whether the script runs on a Map with a rough default coastline (every script but pan
    /// and zoom, which keeps the Map of issue #94).
    fn rough(self) -> bool {
        self != Script::PanZoom
    }
}

/// The names of all scripts, for messages.
pub(super) fn names() -> String {
    Script::ALL
        .iter()
        .map(|script| script.name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The Map a script runs on, with what the edits need to find.
pub(super) struct World {
    pub map: Map,
    pub cache: RoughnessCache,
    layer: Id,
    landmasses: Vec<Id>,
    stamps: Vec<Id>,
    terrain: Terrain,
    pick: Option<PickIndex>,
    base_stamp: Stamp,
}

impl World {
    /// Wraps a generated Map: `layer` holds its `landmasses`, then its Stamps. For a script
    /// that needs it, the Map gets a high Map-wide Roughness first.
    pub(super) fn new(mut map: Map, layer: Id, landmasses: usize, script: Script) -> Self {
        let children = map.children(Some(layer)).to_vec();
        let (land, stamps) = children.split_at(landmasses.min(children.len()));
        let (land, stamps) = (land.to_vec(), stamps.to_vec());
        if script.rough() {
            let (amount, scale, seed) = ROUGHNESS;
            run(
                &mut map,
                &Command::SetDefaultRoughness {
                    roughness: Roughness::new(amount, scale, seed),
                },
                "a valid Roughness",
            );
        }
        let base_stamp = stamps
            .first()
            .and_then(|&id| stamp_of(&map, id))
            .unwrap_or_default();
        let mut cache = RoughnessCache::new();
        let terrain = Terrain::of(&map, &mut cache);
        let pick = (script == Script::Marquee).then(|| PickIndex::build(&map, &mut cache));
        Self {
            map,
            cache,
            layer,
            landmasses: land,
            stamps,
            terrain,
            pick,
            base_stamp,
        }
    }

    fn target(&self) -> Id {
        self.landmasses[TARGET_LANDMASS.min(self.landmasses.len() - 1)]
    }
}

fn stamp_of(map: &Map, id: Id) -> Option<Stamp> {
    match map.node(id)? {
        Node::Object(object) => match &object.kind {
            ObjectKind::Stamp(stamp) => Some(stamp.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Where landmass `i` of the generated Map sits.
pub(super) fn centre_of(i: usize) -> Point {
    [(i % 5) as f64 * 3_000.0, (i / 5) as f64 * 3_000.0]
}

/// The edited landmass's centre.
fn work_centre() -> Point {
    centre_of(TARGET_LANDMASS.min(LANDMASSES - 1))
}

/// The views a script renders, the first being the warm-up. Pan and zoom moves; the rest hold
/// one view framing the edited landmass (or, for the marquee, the whole Canvas).
pub(super) fn views(script: Script, canvas: [f64; 2]) -> Vec<View> {
    match script {
        Script::PanZoom => super::path(canvas),
        Script::Marquee => vec![View::fit(canvas, TARGET, 20.0)],
        _ => {
            let centre = work_centre();
            let zoom = f64::from(TARGET[1]) / 2_600.0;
            vec![View {
                origin: [
                    centre[0] - f64::from(TARGET[0]) / (2.0 * zoom),
                    centre[1] - f64::from(TARGET[1]) / (2.0 * zoom),
                ],
                zoom,
                size: TARGET,
            }]
        }
    }
}

/// How many frames the script measures.
pub(super) fn frames(script: Script) -> usize {
    match script {
        Script::PanZoom => 3 * PHASE_FRAMES,
        _ => EDIT_FRAMES,
    }
}

/// The edit of frame `frame` (counted from 0), applied to the Map. Pan and zoom changes
/// nothing, and the marquee only queries.
pub(super) fn edit(script: Script, frame: usize, world: &mut World) {
    let t = frame as f64 / EDIT_FRAMES as f64;
    let centre = work_centre();
    let target = world.target();
    match script {
        Script::PanZoom => {}
        Script::Paint | Script::Erase => {
            let erase = script == Script::Erase;
            // Paint along the coast, so every segment extends the shore; erase along a
            // circle inside the land.
            let radius = if erase { 600.0 } else { 1_000.0 };
            let at = |t: f64| {
                [
                    centre[0] + radius * (t * TAU * 0.5).cos(),
                    centre[1] + radius * (t * TAU * 0.5).sin(),
                ]
            };
            let (from, to) = (at(t), at(t + 1.0 / EDIT_FRAMES as f64));
            let tolerance = land::simplify_tolerance(120.0, 1.0);
            let stroke = BrushStroke {
                region: land::brush_region(&[from, to], 120.0, tolerance),
                start: from,
                erase,
            };
            let selection: &[Id] = if erase { &[] } else { &[target] };
            let command = land::brush_command(
                &world.map,
                &mut world.cache,
                selection,
                Some(world.layer),
                stroke,
                Id::new(),
            )
            .expect("the layer exists");
            if let Some(command) = command {
                run(&mut world.map, &command, "the stroke applies");
            }
        }
        Script::DragLandmass => {
            run(
                &mut world.map,
                &Command::Translate {
                    ids: vec![target],
                    offset: [6.0, 4.0],
                },
                "the landmass moves",
            );
        }
        Script::DragStamps => {
            for &id in world.stamps.iter().take(DRAGGED_STAMPS) {
                let Some(mut stamp) = stamp_of(&world.map, id) else {
                    continue;
                };
                stamp.position = [stamp.position[0] + 3.0, stamp.position[1] + 2.0];
                run(
                    &mut world.map,
                    &Command::SetStamp {
                        id,
                        stamp: Box::new(stamp),
                    },
                    "the Stamp moves",
                );
            }
        }
        Script::Scatter => {
            let points: Vec<Point> = (0..4)
                .map(|i| {
                    let t = t + i as f64 * 0.002;
                    [
                        centre[0] - 900.0 + 1_800.0 * t,
                        centre[1] + 500.0 * (t * TAU * 2.0).sin(),
                    ]
                })
                .collect();
            let brush = ScatterBrush {
                density: 40.0,
                ..ScatterBrush::default()
            };
            let command = scatter_command(
                &brush,
                ScatterStroke {
                    points: &points,
                    radius: 150.0,
                    parent: world.layer,
                    base: &world.base_stamp,
                    variants: &[],
                },
                &world.terrain,
                frame as u64,
                Id::new,
            );
            if let Some(command) = command {
                run(&mut world.map, &command, "the Stamps place");
            }
        }
        Script::SliderAmount | Script::SliderScale => {
            let (amount, scale, seed) = ROUGHNESS;
            let roughness = if script == Script::SliderAmount {
                Roughness::new(amount * (0.25 + 1.5 * t), scale, seed)
            } else {
                Roughness::new(amount, scale * (0.5 + 1.5 * t), seed)
            };
            run(
                &mut world.map,
                &Command::SetDefaultRoughness { roughness },
                "a valid Roughness",
            );
        }
        Script::SliderShore => {
            let mut shore = ShoreStyle::landmass_default();
            shore.outline.width = 1.0 + 20.0 * t;
            shore.ripples.spacing = 8.0 + 40.0 * t;
            shore.shading.width = 10.0 + 120.0 * t;
            run(
                &mut world.map,
                &Command::SetDefaultShore {
                    kind: ShoreKind::Landmass,
                    shore,
                },
                "a valid Shore style",
            );
        }
        Script::Marquee => {
            let Some(pick) = &world.pick else { return };
            // A rectangle dragged from the middle of the grid out to the whole Canvas.
            let reach = 1_000.0 + 14_000.0 * t;
            let rect = Rect {
                min: [6_000.0 - reach, 6_000.0 - reach],
                max: [6_000.0 + reach, 6_000.0 + reach],
            };
            std::hint::black_box(pick.intersecting(&rect));
        }
    }
}
