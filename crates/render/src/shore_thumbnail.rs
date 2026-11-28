//! Shore preset thumbnails (issue 651): a small render of a Shore style on a sample coast, for
//! the preset row in Properties. The sample is a Map of its own, built through Commands and
//! drawn by the same renderer as the Canvas, so a thumbnail shows what the preset draws.

use ssmap_core::{
    Background, Canvas, Color, Command, DistanceUnit, ExecuteError, History, HistoryError, Id, Map,
    MultiPolygon, Polygon, RoughnessCache, Scale, ShoreKind, ShoreStyle,
};
use thiserror::Error;

use crate::{
    CanvasRenderer, CanvasScene, Fonts, GpuCapabilities, MapScene, ReadbackError, RgbaImage, View,
};

/// The sample Canvas, in Canvas units. A thumbnail is [`SHORE_THUMBNAIL_SCALE`] pixels per unit,
/// so Shore widths look as they do on a Map at 100% on a high-density screen.
pub const SHORE_THUMBNAIL_SIZE: [u32; 2] = [112, 72];

/// Pixels per Canvas unit in a thumbnail.
pub const SHORE_THUMBNAIL_SCALE: u32 = 2;

/// The colours of the Map a thumbnail stands for. Thumbnails differ only by these, the kind
/// and the style, so they can be made once for each.
#[derive(Clone, PartialEq, Debug)]
pub struct ShoreSample {
    /// The land fill.
    pub land: Color,
    /// The ocean.
    pub background: Background,
}

impl ShoreSample {
    /// The colours of `map`: its default land fill and its Canvas background.
    pub fn of(map: &Map) -> Self {
        Self {
            land: map.defaults().land_fill,
            background: map.canvas().background.clone(),
        }
    }
}

/// Why a thumbnail could not be made.
#[derive(Debug, Error)]
pub enum ShoreThumbnailError {
    /// The sample Map refused a Command, such as a Shore style that is out of range.
    #[error("the sample coast could not be built")]
    Sample(#[from] ExecuteError),
    /// The sample Map's Undo step could not be opened or closed.
    #[error("the sample coast could not be built")]
    History(#[from] HistoryError),
    /// The pixels could not be read back.
    #[error(transparent)]
    Readback(#[from] ReadbackError),
}

/// A coast across the sample Canvas: land on the left, with a gentle bay and headland, water
/// on the right.
fn coast() -> Polygon {
    let [w, h] = SHORE_THUMBNAIL_SIZE.map(f64::from);
    let steps = 48;
    let mut exterior = vec![[-20.0, -20.0]];
    exterior.extend((0..=steps).map(|i| {
        let t = f64::from(i) / f64::from(steps);
        let y = -20.0 + (h + 40.0) * t;
        let x = w * (0.4 + 0.06 * (t * std::f64::consts::TAU).sin());
        [x, y]
    }));
    exterior.push([-20.0, h + 20.0]);
    Polygon {
        exterior,
        holes: vec![],
    }
}

/// A Lake in the middle of the sample, which is all land.
fn lake() -> Polygon {
    let [w, h] = SHORE_THUMBNAIL_SIZE.map(f64::from);
    let steps = 24;
    let exterior = (0..steps)
        .map(|i| {
            let angle = std::f64::consts::TAU * f64::from(i) / f64::from(steps);
            // A little uneven, so it reads as a Lake rather than a circle.
            let wobble = 1.0 + 0.12 * (3.0 * angle).sin();
            [
                w * 0.5 + w * 0.3 * wobble * angle.cos(),
                h * 0.5 + h * 0.3 * wobble * angle.sin(),
            ]
        })
        .collect();
    Polygon {
        exterior,
        holes: vec![],
    }
}

/// The sample Map for `style` on a `kind` of Object: a coast for a Landmass, or a Lake in
/// land for a Lake.
pub fn shore_sample_map(
    kind: ShoreKind,
    style: &ShoreStyle,
    sample: &ShoreSample,
) -> Result<Map, ShoreThumbnailError> {
    let [width, height] = SHORE_THUMBNAIL_SIZE;
    let mut canvas = Canvas::new(width, height, Scale::new(1.0, DistanceUnit::Miles));
    canvas.background = sample.background.clone();
    let mut map = Map::new(canvas);
    let layer = Id::new();
    let land = Id::new();
    let mut commands = vec![
        Command::CreateLayer {
            id: layer,
            parent: None,
            index: None,
            name: "Sample".into(),
        },
        Command::CreateLandmass {
            id: land,
            parent: Some(layer),
            index: None,
            geometry: MultiPolygon(vec![coast()]),
            fill: Some(sample.land),
        },
    ];
    let shored = match kind {
        ShoreKind::Landmass => land,
        ShoreKind::Lake => {
            let [w, h] = SHORE_THUMBNAIL_SIZE.map(f64::from);
            let lake_id = Id::new();
            // All land under the Lake, so only its own shore shows.
            commands[1] = Command::CreateLandmass {
                id: land,
                parent: Some(layer),
                index: None,
                geometry: MultiPolygon(vec![Polygon {
                    exterior: vec![
                        [-20.0, -20.0],
                        [w + 20.0, -20.0],
                        [w + 20.0, h + 20.0],
                        [-20.0, h + 20.0],
                    ],
                    holes: vec![],
                }]),
                fill: Some(sample.land),
            };
            commands.push(Command::CreateLake {
                id: lake_id,
                parent: Some(layer),
                index: None,
                geometry: MultiPolygon(vec![lake()]),
                fill: None,
            });
            lake_id
        }
    };
    commands.push(Command::SetShore {
        ids: vec![shored],
        shore: Some(style.clone()),
    });
    // A Map changes only through Commands (ADR 0004), so the sample is built as one Undo step.
    let mut history = History::default();
    history.begin("Sample coast", Vec::new())?;
    for command in &commands {
        history.execute(&mut map, command)?;
    }
    history.commit(Vec::new())?;
    Ok(map)
}

/// Draws thumbnails with a Canvas renderer of its own, so the screen's is left alone.
pub struct ShoreThumbnailer {
    renderer: CanvasRenderer,
    scene: MapScene,
    roughness: RoughnessCache,
}

impl ShoreThumbnailer {
    /// A thumbnailer on `device`.
    pub fn new(device: &wgpu::Device, capabilities: GpuCapabilities) -> Self {
        Self {
            renderer: CanvasRenderer::new(device, capabilities),
            scene: MapScene::new(Fonts::bundled()),
            roughness: RoughnessCache::new(),
        }
    }

    /// `style` on the sample coast for `kind`: [`SHORE_THUMBNAIL_SIZE`] times
    /// [`SHORE_THUMBNAIL_SCALE`] pixels.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        kind: ShoreKind,
        style: &ShoreStyle,
        sample: &ShoreSample,
    ) -> Result<RgbaImage, ShoreThumbnailError> {
        let map = shore_sample_map(kind, style, sample)?;
        // Every ring is computed now: a thumbnail is drawn once and kept.
        let content = self.scene.complete_content(&map, &mut self.roughness);
        let scene = CanvasScene::new(map.canvas()).with_content(content);
        let [width, height] = SHORE_THUMBNAIL_SIZE;
        let longest = width.max(height) * SHORE_THUMBNAIL_SCALE;
        let view = View::whole_canvas(scene.canvas_size, longest);
        Ok(crate::render_to_image(
            device,
            queue,
            &mut self.renderer,
            &scene,
            &view,
        )?)
    }
}
