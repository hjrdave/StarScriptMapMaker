//! Drawing Stamps: one instanced quad each, from the atlas pages (ADR 0003).

use std::collections::HashMap;
use std::ops::Range;

use wgpu::util::DeviceExt;

use crate::atlas::Atlas;
use crate::canvas::StencilUse;

/// One Stamp ready to draw. Everything is in Canvas units or already linear.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StampInstance {
    /// Where the Anchor lands.
    pub position: [f32; 2],
    /// The quad's size, scale included.
    pub size: [f32; 2],
    /// The Anchor as fractions of the quad.
    pub anchor: [f32; 2],
    /// Radians clockwise, then +1 or -1 to flip left to right.
    pub turn: [f32; 2],
    pub art_uv: [f32; 4],
    pub silhouette_uv: [f32; 4],
    /// Linear tint in rgb, opacity in alpha.
    pub tint: [f32; 4],
    /// Linear and premultiplied.
    pub fill: [f32; 4],
}

/// A Stamp and the atlas pages its art and Silhouette are on.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StampDraw {
    pub art_page: u32,
    pub silhouette_page: u32,
    pub instance: StampInstance,
}

/// A run of instances sharing the same two pages, drawn in one call.
struct Batch {
    pages: (u32, u32),
    instances: Range<u32>,
}

pub struct StampPass {
    pipeline: wgpu::RenderPipeline,
    art_layout: wgpu::BindGroupLayout,
    silhouette_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    instances: Option<wgpu::Buffer>,
    /// Per run of the Content, the batches to draw.
    runs: Vec<Vec<Batch>>,
    art_groups: HashMap<u32, wgpu::BindGroup>,
    silhouette_groups: HashMap<u32, wgpu::BindGroup>,
}

impl StampPass {
    pub(crate) fn new(
        device: &wgpu::Device,
        view_layout: &wgpu::BindGroupLayout,
        samples: u32,
    ) -> Self {
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
        let art_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("stamp art"),
            entries: &[
                texture_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let silhouette_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("stamp silhouette"),
            entries: &[texture_entry(0)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("stamp"),
            bind_group_layouts: &[
                Some(view_layout),
                Some(&art_layout),
                Some(&silhouette_layout),
            ],
            immediate_size: 0,
        });
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
        let pipeline = crate::canvas::canvas_pipeline(
            device,
            "stamp",
            &layout,
            &shader,
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<StampInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2,
                    4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
                ],
            })],
            samples,
            StencilUse::Ignore,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("stamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            art_layout,
            silhouette_layout,
            sampler,
            instances: None,
            runs: Vec::new(),
            art_groups: HashMap::new(),
            silhouette_groups: HashMap::new(),
        }
    }

    /// Replaces what is drawn: `runs[i]` are the Stamps of the Content's i-th run, in order.
    pub(crate) fn set_runs(&mut self, device: &wgpu::Device, runs: &[&[StampDraw]]) {
        let mut instances: Vec<StampInstance> = Vec::new();
        self.runs = runs
            .iter()
            .map(|run| {
                let mut batches: Vec<Batch> = Vec::new();
                for draw in *run {
                    let pages = (draw.art_page, draw.silhouette_page);
                    let at = instances.len() as u32;
                    instances.push(draw.instance);
                    match batches.last_mut() {
                        Some(last) if last.pages == pages => last.instances.end = at + 1,
                        _ => batches.push(Batch {
                            pages,
                            instances: at..at + 1,
                        }),
                    }
                }
                batches
            })
            .collect();
        self.instances = (!instances.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("stamp instances"),
                contents: bytemuck::cast_slice(&instances),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
    }

    /// Makes the bind groups for every page the current runs use. Call before the pass.
    pub(crate) fn prepare(&mut self, device: &wgpu::Device, atlas: &Atlas) {
        for &(art, silhouette) in self.runs.iter().flatten().map(|batch| &batch.pages) {
            self.art_groups.entry(art).or_insert_with(|| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("stamp art"),
                    layout: &self.art_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(atlas.page_view(art)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                })
            });
            self.silhouette_groups.entry(silhouette).or_insert_with(|| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("stamp silhouette"),
                    layout: &self.silhouette_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(atlas.page_view(silhouette)),
                    }],
                })
            });
        }
    }

    /// Draws the Stamps of run `run`. [`StampPass::prepare`] must have run since the last
    /// [`StampPass::set_runs`].
    pub(crate) fn draw_run(&self, pass: &mut wgpu::RenderPass<'_>, run: usize) {
        let (Some(instances), Some(batches)) = (&self.instances, self.runs.get(run)) else {
            return;
        };
        if batches.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, instances.slice(..));
        for batch in batches {
            pass.set_bind_group(1, &self.art_groups[&batch.pages.0], &[]);
            pass.set_bind_group(2, &self.silhouette_groups[&batch.pages.1], &[]);
            pass.draw(0..6, batch.instances.clone());
        }
    }
}
