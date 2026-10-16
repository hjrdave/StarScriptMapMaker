//! Drawing Stamps: instanced quads sampling atlas pages (ADR 0003). The Silhouette is filled
//! under the line art, so overlapping Stamps hide each other's lines.

use std::collections::HashMap;
use std::ops::Range;

use ssmap_core::asset::AssetRef;
use ssmap_core::{Color, Point};
use wgpu::util::DeviceExt;

use crate::atlas::{Atlas, AtlasEntry};
use crate::canvas::{CLIP_INSIDE, StencilUse};
use crate::color::ColorExt as _;
use crate::sprite::{SpriteState, SpriteStore};
use crate::vector::overlaps;

/// One Stamp as the Canvas draws it.
#[derive(Clone, PartialEq, Debug)]
pub struct StampDraw {
    pub asset: AssetRef,
    pub position: Point,
    pub scale: f64,
    /// Degrees clockwise about the Anchor.
    pub rotation: f64,
    pub flip: bool,
    pub tint: Option<Color>,
    /// The Silhouette's fill.
    pub fill: Color,
    /// Already faded by the Layers holding the Stamp.
    pub opacity: f32,
}

/// A Stamp and where it sits in the draw order: it draws over the first `after` vector groups
/// of the content, and under the rest.
#[derive(Clone, PartialEq, Debug)]
pub struct PlacedStamp {
    pub after: usize,
    /// The index of the content's Clip scope it shows inside, if any.
    pub clip: Option<usize>,
    /// The Stamp is on the pasteboard: drawn in full, not clipped at the Canvas edge.
    pub pasteboard: bool,
    pub draw: StampDraw,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    anchor: [f32; 2],
    column_x: [f32; 2],
    column_y: [f32; 2],
    rect_min: [f32; 2],
    rect_size: [f32; 2],
    art_uv: [f32; 4],
    silhouette_uv: [f32; 4],
    tint: [f32; 4],
    fill: [f32; 4],
    opacity: f32,
    _pad: [f32; 3],
}

/// The box around a Stamp's quad: the corners of its art rectangle, carried to the Canvas by
/// the instance's columns.
fn quad_bounds(
    anchor: [f32; 2],
    column_x: [f32; 2],
    column_y: [f32; 2],
    rect_min: [f32; 2],
    rect_size: [f32; 2],
) -> [f32; 4] {
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for corner in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
        let x = rect_min[0] + corner[0] * rect_size[0];
        let y = rect_min[1] + corner[1] * rect_size[1];
        let cx = anchor[0] + column_x[0] * x + column_y[0] * y;
        let cy = anchor[1] + column_x[1] * x + column_y[1] * y;
        bounds = [
            bounds[0].min(cx),
            bounds[1].min(cy),
            bounds[2].max(cx),
            bounds[3].max(cy),
        ];
    }
    bounds
}

/// An Asset's art on atlas pages.
#[derive(Clone, Copy)]
struct Uploaded {
    art: AtlasEntry,
    silhouette: AtlasEntry,
    size: [f64; 2],
    anchor: [f64; 2],
}

/// Instances drawn with the same pair of atlas pages.
pub(crate) struct Batch {
    pages: (u32, u32),
    instances: Range<u32>,
}

/// The Stamps of one Clip scope drawn after the first `after` vector groups.
struct Run {
    after: usize,
    clip: Option<usize>,
    pasteboard: bool,
    batches: Vec<Batch>,
}

pub(crate) struct StampRenderer {
    pipeline: wgpu::RenderPipeline,
    clipped: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: Atlas,
    store: Option<SpriteStore>,
    /// The store's generation the uploaded art came from.
    generation: u64,
    /// `None` records an Asset that has no usable art. An Asset whose art is still being
    /// rasterised is not recorded.
    uploaded: HashMap<AssetRef, Option<Uploaded>>,
    /// The stand-in box for Assets with no art, once uploaded.
    placeholder: Option<Option<Uploaded>>,
    /// The store's finished-raster count when the Stamps were last prepared.
    completed: u64,
    /// Whether the last prepare skipped a Stamp for art that was not ready.
    skipped_pending: bool,
    bind_groups: HashMap<(u32, u32), wgpu::BindGroup>,
    instances: Option<wgpu::Buffer>,
    /// The Canvas area each instance covers, as `[min_x, min_y, max_x, max_y]`, so a frame
    /// draws only the Stamps in view.
    bounds: Vec<[f32; 4]>,
    runs: Vec<Run>,
}

impl StampRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        view_layout: &wgpu::BindGroupLayout,
        samples: u32,
        page_size: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stamp"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("shaders/view.wgsl"),
                    include_str!("shaders/stamp.wgsl")
                )
                .into(),
            ),
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("stamp textures"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("stamp"),
            bind_group_layouts: &[Some(view_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let pipeline = |label, stencil| {
            crate::canvas::canvas_pipeline(
                device,
                label,
                &layout,
                &shader,
                &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2,
                        4 => Float32x2, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
                        8 => Float32x4, 9 => Float32
                    ],
                })],
                samples,
                stencil,
            )
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("stamp atlas"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline: pipeline("stamp", StencilUse::Ignore),
            clipped: pipeline("stamp clipped", StencilUse::Clipped),
            texture_layout,
            sampler,
            atlas: Atlas::new(page_size),
            store: None,
            generation: 0,
            uploaded: HashMap::new(),
            placeholder: None,
            completed: 0,
            skipped_pending: false,
            bind_groups: HashMap::new(),
            instances: None,
            bounds: Vec::new(),
            runs: Vec::new(),
        }
    }

    pub(crate) fn store(&self) -> Option<&SpriteStore> {
        self.store.as_ref()
    }

    /// Where Asset art comes from. Replacing the store forgets everything uploaded.
    pub(crate) fn set_store(&mut self, store: SpriteStore) {
        self.generation = store.generation();
        self.store = Some(store);
        self.forget_uploads();
    }

    /// Forgets everything uploaded if the store's embedded Assets changed since. Returns
    /// whether Stamps must be prepared again: they did, or art that a Stamp was skipped for
    /// has been rasterised since.
    pub(crate) fn refresh_store(&mut self) -> bool {
        let Some(store) = &self.store else {
            return false;
        };
        let generation = store.generation();
        if generation != self.generation {
            self.generation = generation;
            self.forget_uploads();
            return true;
        }
        self.skipped_pending && store.completed() != self.completed
    }

    fn forget_uploads(&mut self) {
        self.atlas = Atlas::new(self.atlas.page_size());
        self.uploaded.clear();
        self.placeholder = None;
        self.bind_groups.clear();
    }

    /// Uploads any art `stamps` need and builds their instances. Stamps whose Asset has no
    /// usable art draw nothing, and neither do those whose art is still being rasterised: they
    /// appear when [`StampRenderer::refresh_store`] next asks for them to be prepared again.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        stamps: &[PlacedStamp],
    ) {
        // Read before polling: a raster that lands during the loop is then seen as new.
        self.completed = self.store.as_ref().map_or(0, SpriteStore::completed);
        self.skipped_pending = false;
        let page_size = self.atlas.page_size();
        let mut instances: Vec<Instance> = Vec::with_capacity(stamps.len());
        let mut bounds: Vec<[f32; 4]> = Vec::with_capacity(stamps.len());
        let mut runs: Vec<Run> = Vec::new();
        for placed in stamps {
            let Some(uploaded) = self.upload(device, queue, &placed.draw.asset) else {
                continue;
            };
            let draw = &placed.draw;
            let (sin, cos) = draw.rotation.to_radians().sin_cos();
            let flip = if draw.flip { -1.0 } else { 1.0 };
            let [width, height] = uploaded.size;
            let column_x = [
                (draw.scale * flip * cos) as f32,
                (draw.scale * flip * sin) as f32,
            ];
            let column_y = [(-draw.scale * sin) as f32, (draw.scale * cos) as f32];
            let rect_min = [
                (-uploaded.anchor[0] * width) as f32,
                (-uploaded.anchor[1] * height) as f32,
            ];
            let rect_size = [width as f32, height as f32];
            let anchor: [f32; 2] = draw.position.map(|v| v as f32);
            bounds.push(quad_bounds(anchor, column_x, column_y, rect_min, rect_size));
            instances.push(Instance {
                anchor,
                column_x,
                column_y,
                rect_min,
                rect_size,
                art_uv: uploaded.art.uv(page_size),
                silhouette_uv: uploaded.silhouette.uv(page_size),
                tint: draw
                    .tint
                    .map_or([1.0; 4], |tint| tint.to_linear_premultiplied()),
                fill: draw.fill.to_linear_premultiplied(),
                opacity: draw.opacity,
                _pad: [0.0; 3],
            });
            let index = instances.len() as u32 - 1;
            let pages = (uploaded.art.page, uploaded.silhouette.page);
            if runs.last().is_none_or(|run| {
                (run.after, run.clip, run.pasteboard)
                    != (placed.after, placed.clip, placed.pasteboard)
            }) {
                runs.push(Run {
                    after: placed.after,
                    clip: placed.clip,
                    pasteboard: placed.pasteboard,
                    batches: Vec::new(),
                });
            }
            let run = runs.last_mut().expect("pushed above");
            match run.batches.last_mut() {
                Some(batch) if batch.pages == pages => batch.instances.end = index + 1,
                _ => run.batches.push(Batch {
                    pages,
                    instances: index..index + 1,
                }),
            }
            self.bind_group(device, pages);
        }
        self.instances = (!instances.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("stamp instances"),
                contents: bytemuck::cast_slice(&instances),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        self.bounds = bounds;
        self.runs = runs;
    }

    /// The vector-group counts that Stamps draw after, in order.
    pub(crate) fn cuts(&self) -> impl Iterator<Item = usize> + '_ {
        self.runs.iter().map(|run| run.after)
    }

    /// The Stamps placed after `after` vector groups, in draw order: for each Clip scope
    /// they show inside, whether they are on the pasteboard, and its batches.
    pub(crate) fn runs_after(
        &self,
        after: usize,
    ) -> impl Iterator<Item = (Option<usize>, bool, &[Batch])> + '_ {
        self.runs
            .iter()
            .filter(move |run| run.after == after)
            .map(|run| (run.clip, run.pasteboard, run.batches.as_slice()))
    }

    /// Draws batches from [`StampRenderer::runs_after`], only inside the active Clip if
    /// `clipped`, and only the Stamps that reach `visible` (`[min_x, min_y, max_x, max_y]`).
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        batches: &[Batch],
        clipped: bool,
        visible: [f32; 4],
    ) {
        let Some(instances) = &self.instances else {
            return;
        };
        pass.set_pipeline(if clipped {
            &self.clipped
        } else {
            &self.pipeline
        });
        pass.set_stencil_reference(if clipped { CLIP_INSIDE } else { 0 });
        pass.set_vertex_buffer(0, instances.slice(..));
        for batch in batches {
            let mut bound = false;
            // One draw per stretch of consecutive visible instances.
            let mut start = None;
            for index in batch
                .instances
                .clone()
                .chain(std::iter::once(batch.instances.end))
            {
                let shown =
                    index < batch.instances.end && overlaps(self.bounds[index as usize], visible);
                match (shown, start) {
                    (true, None) => start = Some(index),
                    (false, Some(first)) => {
                        if !bound {
                            pass.set_bind_group(1, &self.bind_groups[&batch.pages], &[]);
                            bound = true;
                        }
                        pass.draw(0..6, first..index);
                        start = None;
                    }
                    _ => {}
                }
            }
        }
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset: &AssetRef,
    ) -> Option<Uploaded> {
        if let Some(uploaded) = self.uploaded.get(asset) {
            return *uploaded;
        }
        let uploaded = match self.store.as_ref().map(|store| store.poll(asset)) {
            Some(SpriteState::Ready(sprite)) => (|| {
                Some(Uploaded {
                    art: self.atlas.insert(device, queue, &sprite.art).ok()?,
                    silhouette: self.atlas.insert(device, queue, &sprite.silhouette).ok()?,
                    size: sprite.shape.size,
                    anchor: sprite.shape.anchor,
                })
            })(),
            Some(SpriteState::Pending) => {
                // Not cached: ask again once the raster lands.
                self.skipped_pending = true;
                return None;
            }
            // The Asset is not installed (a pack was deleted, say): a plain box stands in.
            Some(SpriteState::Missing) => self.upload_placeholder(device, queue),
            None => None,
        };
        self.uploaded.insert(asset.clone(), uploaded);
        uploaded
    }

    /// The box drawn for a Stamp with no art, uploaded once and shared.
    fn upload_placeholder(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<Uploaded> {
        if self.placeholder.is_none() {
            let sprite = crate::sprite::placeholder();
            self.placeholder = Some((|| {
                Some(Uploaded {
                    art: self.atlas.insert(device, queue, &sprite.art).ok()?,
                    silhouette: self.atlas.insert(device, queue, &sprite.silhouette).ok()?,
                    size: sprite.shape.size,
                    anchor: sprite.shape.anchor,
                })
            })());
        }
        self.placeholder.flatten()
    }

    fn bind_group(&mut self, device: &wgpu::Device, pages: (u32, u32)) {
        let (atlas, layout, sampler) = (&self.atlas, &self.texture_layout, &self.sampler);
        self.bind_groups.entry(pages).or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("stamp textures"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(atlas.page_view(pages.0)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(atlas.page_view(pages.1)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        });
    }
}
