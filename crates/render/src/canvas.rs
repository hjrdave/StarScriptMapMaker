//! The Canvas renderer: an offscreen, multisampled target holding the backdrop and the Map's
//! Landmass and Lake fills.

use lyon::math::Box2D;
use lyon::path::{Path, Winding};
use wgpu::util::DeviceExt;

use crate::color::Color;
use crate::gpu::GpuCapabilities;
use crate::ocean::{OceanPass, OceanUniform};
use crate::shapes::Shapes;
use crate::vector::{FillPath, LyonRenderer, VectorRenderer};
use crate::view::View;

/// The resolved Canvas target's format. Blending happens in linear space, and readback bytes
/// are already sRGB-encoded for PNG.
pub const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// The stencil buffer behind Clip: content clipped to a Landmass draws only where its outline
/// has stamped a 1.
pub(crate) const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

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

/// Everything the Canvas renderer draws.
#[derive(Clone, PartialEq, Debug)]
pub struct CanvasScene {
    /// Canvas size in Canvas units.
    pub canvas_size: [f64; 2],
    pub backdrop: Backdrop,
    /// The area around the Canvas. Editor-only; export never shows it.
    pub pasteboard: Color,
    /// Landmass and Lake fills, drawn over the backdrop.
    pub shapes: Shapes,
}

impl CanvasScene {
    /// A Canvas with only its backdrop.
    pub fn new(canvas: &ssmap_core::Canvas) -> Self {
        Self {
            canvas_size: [f64::from(canvas.width), f64::from(canvas.height)],
            backdrop: Backdrop::default(),
            pasteboard: Color::rgb(0x1c, 0x1c, 0x1f),
            shapes: Shapes::default(),
        }
    }

    /// The whole Map: its Canvas and everything drawn on it, with roughened outlines from
    /// `roughness`.
    pub fn from_map(map: &ssmap_core::Map, roughness: &mut ssmap_core::RoughnessCache) -> Self {
        Self {
            shapes: Shapes::from_map(map, roughness),
            ..Self::new(&map.canvas)
        }
    }

    /// Whether `other` has the same backdrop, ignoring what is drawn over it.
    fn same_backdrop(&self, other: &CanvasScene) -> bool {
        self.canvas_size == other.canvas_size && self.backdrop == other.backdrop
    }
}

pub struct CanvasRenderer {
    capabilities: GpuCapabilities,
    view_uniform: wgpu::Buffer,
    view_bind_group: wgpu::BindGroup,
    /// The water fill beneath the ocean texture.
    vector: LyonRenderer,
    ocean: OceanPass,
    /// Landmass and Lake fills over the ocean.
    shapes: LyonRenderer,
    scene: Option<CanvasScene>,
    target: Option<Target>,
}

struct Target {
    size: [u32; 2],
    /// Multisampled colour, absent when MSAA is unsupported.
    msaa: Option<wgpu::TextureView>,
    stencil: wgpu::TextureView,
    resolved: wgpu::Texture,
    resolved_view: wgpu::TextureView,
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
            vector: LyonRenderer::new(device, &view_layout, samples),
            shapes: LyonRenderer::new(device, &view_layout, samples),
            ocean: OceanPass::new(device, &view_layout, samples),
            capabilities,
            view_uniform,
            view_bind_group,
            scene: None,
            target: None,
        }
    }

    pub fn capabilities(&self) -> &GpuCapabilities {
        &self.capabilities
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
        let previous = self.scene.take();
        if !previous.as_ref().is_some_and(|p| p.same_backdrop(scene)) {
            self.set_backdrop(device, queue, scene);
        }
        if previous.is_none_or(|p| p.shapes != scene.shapes) {
            self.shapes.set_fills(device, &scene.shapes.fill_paths());
        }
        self.scene = Some(scene.clone());
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
        self.vector.draw(&mut pass);
        self.ocean.draw(&mut pass);
        self.shapes.draw(&mut pass);
        drop(pass);
        &target.resolved
    }

    fn set_backdrop(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scene: &CanvasScene) {
        let [w, h] = scene.canvas_size.map(|v| v as f32);
        let mut builder = Path::builder();
        builder.add_rectangle(
            &Box2D::new((0.0, 0.0).into(), (w, h).into()),
            Winding::Positive,
        );
        self.vector.set_fills(
            device,
            &[FillPath {
                path: builder.build(),
                color: scene.backdrop.water,
                stroke_width: None,
                clip: None,
            }],
        );
        self.ocean.set(
            queue,
            OceanUniform {
                canvas_size: [w, h],
                strength: scene.backdrop.texture_strength,
                _pad: 0.0,
            },
        );
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
        self.target = Some(Target {
            size,
            msaa,
            stencil,
            resolved,
            resolved_view,
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

/// How a pipeline uses the stencil buffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StencilUse {
    /// Ignores it: draws everywhere.
    Ignore,
    /// Draws only where the stencil equals the pass's stencil reference.
    Test,
    /// Writes the pass's stencil reference where it covers, and no colour.
    Write,
}

/// A pipeline drawing premultiplied colour into the Canvas target.
pub(crate) fn canvas_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    samples: u32,
) -> wgpu::RenderPipeline {
    canvas_pipeline_stencil(
        device,
        label,
        layout,
        shader,
        buffers,
        samples,
        StencilUse::Ignore,
    )
}

pub(crate) fn canvas_pipeline_stencil(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    samples: u32,
    stencil_use: StencilUse,
) -> wgpu::RenderPipeline {
    let face = |compare, pass_op| wgpu::StencilFaceState {
        compare,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op,
    };
    let stencil_face = match stencil_use {
        StencilUse::Ignore => face(wgpu::CompareFunction::Always, wgpu::StencilOperation::Keep),
        StencilUse::Test => face(wgpu::CompareFunction::Equal, wgpu::StencilOperation::Keep),
        StencilUse::Write => face(
            wgpu::CompareFunction::Always,
            wgpu::StencilOperation::Replace,
        ),
    };
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
                write_mask: if stencil_use == StencilUse::Write {
                    wgpu::ColorWrites::empty()
                } else {
                    wgpu::ColorWrites::ALL
                },
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: STENCIL_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState {
                front: stencil_face,
                back: stencil_face,
                read_mask: 0xff,
                write_mask: 0xff,
            },
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}
