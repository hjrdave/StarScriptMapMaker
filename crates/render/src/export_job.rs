//! A PNG export that advances one small step per call, so the app can run it a frame at a time
//! behind a progress dialog and stop it with Cancel (ADR 0007).
//!
//! The job never blocks: while derived data is rebuilding, a step only reports that it is
//! waiting. Then each step renders one tile, and the last step of a tile row writes that row
//! to a temporary file that is renamed over the destination when the image is complete.
//! Dropping the job, or [`ExportJob::cancel`], at any point removes the temporary file, so a
//! cancelled or failed export leaves no partial PNG.

use std::path::Path;

use ssmap_core::{ExportSettings, ExportSize, ExportSizeError, Map, Rect, RoughnessCache};
use thiserror::Error;

use crate::canvas::{CanvasRenderer, CanvasScene, Content};
use crate::offscreen::render_to_image;
use crate::png_writer::{PngMeta, PngWriteError, PngWriter};
use crate::scene::MapScene;
use crate::tiled::{ExportStatus, TilePlan, copy_tile};
use crate::view::View;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error(transparent)]
    Size(#[from] ExportSizeError),
    #[error(transparent)]
    Png(#[from] PngWriteError),
}

/// What one [`ExportJob::step`] did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportStep {
    /// The export goes on; call `step` again.
    Working(ExportStatus),
    /// The file is written and in place.
    Finished(ExportSize),
}

/// What an export draws from. The same Map, scene and caches must be passed to every step.
pub struct ExportContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub renderer: &'a mut CanvasRenderer,
    pub scene: &'a mut MapScene,
    pub map: &'a Map,
    pub roughness: &'a mut RoughnessCache,
}

struct Rendering {
    scene: CanvasScene,
    plan: TilePlan,
    writer: PngWriter,
    region: Rect,
    zoom: f64,
    column: u32,
    row: u32,
    done: u32,
    /// The output rows of the current tile row.
    band: Vec<u8>,
}

enum Phase {
    /// Waiting for Roughness outlines, then for the rest of the derived data.
    Waiting {
        content: Option<Content>,
    },
    Rendering(Box<Rendering>),
    Finished,
}

pub struct ExportJob {
    settings: ExportSettings,
    output: ExportSize,
    meta: PngMeta,
    dest: std::path::PathBuf,
    max_tile_side: Option<u32>,
    phase: Phase,
}

impl ExportJob {
    /// Prepares an export of `map` to the PNG at `dest`. Nothing is created on disk until
    /// rendering starts.
    pub fn new(
        map: &Map,
        settings: &ExportSettings,
        meta: PngMeta,
        dest: &Path,
    ) -> Result<Self, ExportError> {
        let output = settings.output_size(&map.canvas)?;
        Ok(Self {
            settings: settings.clone(),
            output,
            meta: PngMeta {
                dpi: output.dpi.or(meta.dpi),
                ..meta
            },
            dest: dest.to_owned(),
            max_tile_side: None,
            phase: Phase::Waiting { content: None },
        })
    }

    /// Overrides the device's texture limit, which tests use to force many small tiles.
    pub fn with_max_tile_side(mut self, max_tile_side: u32) -> Self {
        self.max_tile_side = Some(max_tile_side);
        self
    }

    pub fn output_size(&self) -> ExportSize {
        self.output
    }

    /// Stops the export and removes its temporary file. Any file already at the destination
    /// is untouched.
    pub fn cancel(self) {}

    /// Advances the export by one wait check or one tile. On an error the job is spent and
    /// leaves no file behind.
    pub fn step(&mut self, cx: &mut ExportContext) -> Result<ExportStep, ExportError> {
        let result = self.advance(cx);
        if result.is_err() {
            self.phase = Phase::Finished;
        }
        result
    }

    fn advance(&mut self, cx: &mut ExportContext) -> Result<ExportStep, ExportError> {
        match &mut self.phase {
            Phase::Finished => Ok(ExportStep::Finished(self.output)),
            Phase::Waiting { content } => {
                if cx.roughness.is_rebuilding() {
                    return Ok(ExportStep::Working(ExportStatus::WaitingForDerivedData));
                }
                let content = content.get_or_insert_with(|| {
                    // Content built while outlines were placeholders, or around rings still
                    // being computed, must not be reused.
                    cx.scene.complete_content(cx.map, cx.roughness)
                });
                if !cx.renderer.is_ready(content) {
                    return Ok(ExportStep::Working(ExportStatus::WaitingForDerivedData));
                }
                let mut scene = CanvasScene::new(&cx.map.canvas).with_content(content.clone());
                // A region reaching past the Canvas shows water, not the editor's pasteboard.
                scene.pasteboard = scene.backdrop.water;
                // Only the Canvas is exported: its members are clipped at the edge and
                // pasteboard Objects are left out.
                scene.pasteboard_objects = false;
                let region = self.settings.region_rect(&cx.map.canvas);
                let max_side = self
                    .max_tile_side
                    .unwrap_or(cx.renderer.capabilities().limits.max_texture_dimension_2d);
                let plan = TilePlan::new([self.output.width, self.output.height], max_side);
                let writer = PngWriter::create(
                    &self.dest,
                    self.output.width,
                    self.output.height,
                    &self.meta,
                )?;
                let total = plan.columns() * plan.rows();
                self.phase = Phase::Rendering(Box::new(Rendering {
                    scene,
                    plan,
                    writer,
                    region,
                    zoom: f64::from(self.output.width) / region.width(),
                    column: 0,
                    row: 0,
                    done: 0,
                    band: Vec::new(),
                }));
                Ok(ExportStep::Working(ExportStatus::Rendering {
                    done: 0,
                    total,
                }))
            }
            Phase::Rendering(job) => {
                let total = job.plan.columns() * job.plan.rows();
                let stride = self.output.width as usize * 4;
                let tile = job.plan.tile(job.column, job.row);
                if job.column == 0 {
                    job.band.clear();
                    job.band.resize(stride * tile.height as usize, 0);
                }
                let margin = f64::from(job.plan.margin);
                let view = View {
                    origin: [
                        job.region.min[0] + (f64::from(tile.x) - margin) / job.zoom,
                        job.region.min[1] + (f64::from(tile.y) - margin) / job.zoom,
                    ],
                    zoom: job.zoom,
                    size: job.plan.render_size(),
                };
                let image = render_to_image(cx.device, cx.queue, cx.renderer, &job.scene, &view);
                copy_tile(&image, &job.plan, tile, &mut job.band, stride);
                job.done += 1;
                job.column += 1;
                if job.column == job.plan.columns() {
                    for line in job.band.chunks_exact(stride) {
                        job.writer.write_row(line)?;
                    }
                    job.column = 0;
                    job.row += 1;
                }
                if job.row == job.plan.rows() {
                    let Phase::Rendering(job) = std::mem::replace(&mut self.phase, Phase::Finished)
                    else {
                        unreachable!("the phase was Rendering");
                    };
                    job.writer.finish()?;
                    return Ok(ExportStep::Finished(self.output));
                }
                Ok(ExportStep::Working(ExportStatus::Rendering {
                    done: job.done,
                    total,
                }))
            }
        }
    }
}
