//! The Canvas renderer: an offscreen, multisampled target holding the backdrop and the Map's
//! Objects.

use std::sync::Arc;

use lyon::math::Box2D;
use lyon::path::{Path, Winding};
use wgpu::util::DeviceExt;

use crate::color::{Color, ColorExt as _};
use crate::gpu::GpuCapabilities;
use crate::ocean::{OceanPass, OceanUniform};
use crate::stamp_pass::{StampDraw, StampPass};
use crate::stamps::StampAtlas;
use crate::vector::{FillRule, LyonRenderer, VectorGroup, VectorRenderer};
use crate::view::View;

/// The resolved Canvas target's format. Blending happens in linear space, and readback bytes
/// are already sRGB-encoded for PNG.
pub const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// The Canvas pass's stencil, which lets a [`VectorGroup`] paint each sample once.
const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;

/// The ocean: a background fill plus a texture over it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Backdrop {
    pub water: Color,
    /// How strongly the ocean texture lightens and darkens the water, from 0 to 1.
    pub texture_strength: f32,
}

impl Default for Backdrop {
    fn default() -> Self {
        Self {
            water: Color::rgb(0x3d, 0x6e, 0x8c),
            texture_strength: 0.3,
        }
    }
}

/// One thing the Canvas draws for an Object.
#[derive(Clone, Debug)]
pub enum Item {
    Vector(VectorGroup),
    Stamp(StampDraw),
}

/// Consecutive items of one kind, drawn together.
#[derive(Clone, Debug)]
enum Run {
    Vector(Vec<VectorGroup>),
    Stamps(Vec<StampDraw>),
}

/// The Map's Objects as drawn items, first drawn lowest, as [`MapScene`](crate::MapScene)
/// builds them. Two are equal only if they are the same build (or both empty), so comparing
/// scenes every frame stays cheap.
#[derive(Clone, Debug, Default)]
pub struct Content(Arc<[Run]>);

impl Content {
    pub fn new(groups: Vec<VectorGroup>) -> Self {
        Self::from_items(groups.into_iter().map(Item::Vector).collect())
    }

    /// Items in Stacking order. Vector groups and Stamps keep that order against each other.
    pub fn from_items(items: Vec<Item>) -> Self {
        let mut runs: Vec<Run> = Vec::new();
        for item in items {
            match (item, runs.last_mut()) {
                (Item::Vector(group), Some(Run::Vector(groups))) => groups.push(group),
                (Item::Stamp(draw), Some(Run::Stamps(draws))) => draws.push(draw),
                (Item::Vector(group), _) => runs.push(Run::Vector(vec![group])),
                (Item::Stamp(draw), _) => runs.push(Run::Stamps(vec![draw])),
            }
        }
        Self(runs.into())
    }

    /// The vector groups, first lowest.
    pub fn groups(&self) -> impl Iterator<Item = &VectorGroup> {
        self.0.iter().flat_map(|run| match run {
            Run::Vector(groups) => groups.as_slice(),
            Run::Stamps(_) => &[],
        })
    }

    /// The Stamps, first lowest.
    pub fn stamps(&self) -> impl Iterator<Item = &StampDraw> {
        self.0.iter().flat_map(|run| match run {
            Run::Stamps(draws) => draws.as_slice(),
            Run::Vector(_) => &[],
        })
    }
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || (self.0.is_empty() && other.0.is_empty())
    }
}

/// Everything the Canvas renderer draws.
#[derive(Clone, PartialEq, Debug)]
pub struct CanvasScene {
    /// Canvas size in Canvas units.
    pub canvas_size: [f64; 2],
    pub backdrop: Backdrop,
    /// The area around the Canvas. Editor-only; export never shows it.
    pub pasteboard: Color,
    /// Drawn over the backdrop.
    pub content: Content,
}

impl CanvasScene {
    pub fn new(canvas: &ssmap_core::Canvas) -> Self {
        Self {
            canvas_size: [f64::from(canvas.width), f64::from(canvas.height)],
            backdrop: Backdrop::default(),
            pasteboard: Color::rgb(0x1c, 0x1c, 0x1f),
            content: Content::default(),
        }
    }

    pub fn with_content(mut self, content: Content) -> Self {
        self.content = content;
        self
    }
}

/// The parts of a scene that change the backdrop.
#[derive(Clone, PartialEq, Debug)]
struct BackdropKey {
    canvas_size: [f64; 2],
    backdrop: Backdrop,
}

pub struct CanvasRenderer {
    capabilities: GpuCapabilities,
    view_uniform: wgpu::Buffer,
    view_bind_group: wgpu::BindGroup,
    /// The water under the ocean texture.
    backdrop: LyonRenderer,
    ocean: OceanPass,
    /// The Map's Objects, over the ocean.
    content: LyonRenderer,
    stamp_pass: StampPass,
    stamp_atlas: StampAtlas,
    drawn_backdrop: Option<BackdropKey>,
    drawn_content: Option<Content>,
    target: Option<Target>,
}

struct Target {
    size: [u32; 2],
    /// Multisampled colour, absent when MSAA is unsupported.
    msaa: Option<wgpu::TextureView>,
    resolved: wgpu::Texture,
    resolved_view: wgpu::TextureView,
    /// At the multisampled size; its contents are never kept between frames.
    stencil: wgpu::TextureView,
}

impl CanvasRenderer {
    pub fn new(device: &wgpu::Device, capabilities: GpuCapabilities) -> Self {
        let samples = capabilities.msaa_samples;
        let view_layout = uniform_layout(device, "view");
        let view_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("view"),
            contents: bytemuck::bytes_of(&View::fit([1.0, 1.0], [1, 1], 0.0).uniform()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view"),
            layout: &view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: view_uniform.as_entire_binding(),
            }],
        });
        Self {
            backdrop: LyonRenderer::new(device, &view_layout, samples),
            ocean: OceanPass::new(device, &view_layout, samples),
            content: LyonRenderer::new(device, &view_layout, samples),
            stamp_pass: StampPass::new(device, &view_layout, samples),
            stamp_atlas: StampAtlas::new(&capabilities.limits),
            capabilities,
            view_uniform,
            view_bind_group,
            drawn_backdrop: None,
            drawn_content: None,
            target: None,
        }
    }

    pub fn capabilities(&self) -> &GpuCapabilities {
        &self.capabilities
    }

    /// The atlas of Stamp art, which [`MapScene`](crate::MapScene) fills as it builds content.
    pub fn stamp_atlas(&mut self) -> &mut StampAtlas {
        &mut self.stamp_atlas
    }

    /// Records the Canvas render into `encoder` and returns the resolved target, which holds
    /// the result once the encoder is submitted.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &CanvasScene,
        view: &View,
    ) -> &wgpu::Texture {
        let backdrop = BackdropKey {
            canvas_size: scene.canvas_size,
            backdrop: scene.backdrop,
        };
        if self.drawn_backdrop.as_ref() != Some(&backdrop) {
            self.set_backdrop(device, queue, backdrop);
        }
        if self.drawn_content.as_ref() != Some(&scene.content) {
            let no_groups: &[VectorGroup] = &[];
            let no_stamps: &[StampDraw] = &[];
            let (mut vectors, mut stamps) = (Vec::new(), Vec::new());
            for run in scene.content.0.iter() {
                match run {
                    Run::Vector(groups) => {
                        vectors.push(groups.as_slice());
                        stamps.push(no_stamps);
                    }
                    Run::Stamps(draws) => {
                        vectors.push(no_groups);
                        stamps.push(draws.as_slice());
                    }
                }
            }
            self.content.set_runs(device, &vectors);
            self.stamp_pass.set_runs(device, &stamps);
            self.drawn_content = Some(scene.content.clone());
        }
        self.stamp_pass.prepare(device, self.stamp_atlas.atlas());
        self.ensure_target(device, view.size);
        queue.write_buffer(&self.view_uniform, 0, bytemuck::bytes_of(&view.uniform()));

        let target = self.target.as_ref().expect("ensured above");
        let (attachment, resolve_target) = match &target.msaa {
            Some(msaa) => (msaa, Some(&target.resolved_view)),
            None => (&target.resolved_view, None),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("canvas"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: attachment,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(scene.pasteboard.to_wgpu()),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.stencil,
                depth_ops: None,
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Discard,
                }),
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.view_bind_group, &[]);
        self.backdrop.draw(&mut pass);
        self.ocean.draw(&mut pass);
        for run in 0..scene.content.0.len() {
            self.content.draw_run(&mut pass, run);
            self.stamp_pass.draw_run(&mut pass, run);
        }
        drop(pass);
        &target.resolved
    }

    fn set_backdrop(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scene: BackdropKey) {
        let [w, h] = scene.canvas_size.map(|v| v as f32);
        let mut builder = Path::builder();
        builder.add_rectangle(
            &Box2D::new((0.0, 0.0).into(), (w, h).into()),
            Winding::Positive,
        );
        self.backdrop.set_groups(
            device,
            &[VectorGroup::fill(
                builder.build(),
                scene.backdrop.water,
                FillRule::NonZero,
            )],
        );
        self.ocean.set(
            queue,
            OceanUniform {
                canvas_size: [w, h],
                strength: scene.backdrop.texture_strength,
                _pad: 0.0,
            },
        );
        self.drawn_backdrop = Some(scene);
    }

    fn ensure_target(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = size.map(|v| v.max(1));
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let samples = self.capabilities.msaa_samples;
        let msaa = (samples > 1).then(|| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("canvas msaa"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: CANVAS_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        let resolved = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("canvas"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANVAS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            // Hosts with a non-sRGB frame read the encoded bytes through this view.
            view_formats: &[CANVAS_FORMAT.remove_srgb_suffix()],
        });
        let resolved_view = resolved.create_view(&Default::default());
        let stencil = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("canvas stencil"),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format: STENCIL_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        self.target = Some(Target {
            size,
            msaa,
            resolved,
            resolved_view,
            stencil,
        });
    }
}

pub(crate) fn uniform_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// How a Canvas pipeline uses the stencil, which starts each frame at 0.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StencilUse {
    Ignore,
    /// Draws only where the stencil is 0 and marks what it draws, so overlapping triangles
    /// in one draw paint each sample once.
    PaintOnce,
    /// Draws no colour and clears marks, ready for the next `PaintOnce` draw.
    Reset,
}

impl StencilUse {
    fn state(self) -> wgpu::DepthStencilState {
        let face = |compare, pass_op| wgpu::StencilFaceState {
            compare,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op,
        };
        let (face, write_mask) = match self {
            StencilUse::Ignore => (wgpu::StencilFaceState::IGNORE, 0),
            StencilUse::PaintOnce => (
                face(
                    wgpu::CompareFunction::Equal,
                    wgpu::StencilOperation::IncrementClamp,
                ),
                0xff,
            ),
            StencilUse::Reset => (
                face(
                    wgpu::CompareFunction::NotEqual,
                    wgpu::StencilOperation::Zero,
                ),
                0xff,
            ),
        };
        wgpu::DepthStencilState {
            format: STENCIL_FORMAT,
            depth_write_enabled: None,
            depth_compare: None,
            stencil: wgpu::StencilState {
                front: face,
                back: face,
                read_mask: 0xff,
                write_mask,
            },
            bias: Default::default(),
        }
    }
}

/// A pipeline drawing premultiplied colour into the Canvas target.
pub(crate) fn canvas_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    samples: u32,
    stencil: StencilUse,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers,
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: CANVAS_FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: if stencil == StencilUse::Reset {
                    wgpu::ColorWrites::empty()
                } else {
                    wgpu::ColorWrites::ALL
                },
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: Some(stencil.state()),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}
