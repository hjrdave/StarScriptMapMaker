//! Vector fills behind a trait, so the renderer can be swapped (ADR 0001).

use std::sync::Arc;

use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, LineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use wgpu::util::DeviceExt;

use crate::canvas::StencilUse;
use crate::color::Color;

/// One path in Canvas units: filled even-odd (a subpath inside another is a hole), or with
/// `stroke_width` set, stroked along its subpaths at that width.
#[derive(Clone, Debug)]
pub struct FillPath {
    pub path: Path,
    pub color: Color,
    pub stroke_width: Option<f32>,
    /// Where this may draw: only inside this region (filled even-odd), or anywhere. Paths
    /// that follow one another with the same region (the same `Arc`) share one stencil.
    pub clip: Option<Arc<Path>>,
}

/// Draws vector fills into a Canvas render pass whose group 0 is the view uniform.
pub trait VectorRenderer {
    /// Replaces what is drawn. Fills draw in order, first lowest.
    fn set_fills(&mut self, device: &wgpu::Device, fills: &[FillPath]);

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>);
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

/// lyon tessellation drawn as plain triangles; antialiasing comes from the target's MSAA.
pub struct LyonRenderer {
    /// Draws everywhere.
    pipeline: wgpu::RenderPipeline,
    /// Draws where the stencil is 1.
    clipped: wgpu::RenderPipeline,
    /// Stamps the stencil, with no colour.
    stamp: wgpu::RenderPipeline,
    mesh: Option<Mesh>,
}

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    steps: Vec<Step>,
}

/// One draw call of a mesh, in order.
enum Step {
    /// Triangles over everything, or only where the stencil is 1.
    Draw {
        indices: std::ops::Range<u32>,
        clipped: bool,
    },
    /// Triangles that set the stencil to `value` where they cover.
    Stamp {
        indices: std::ops::Range<u32>,
        value: u32,
    },
}

/// Curve flattening tolerance in Canvas units.
const TOLERANCE: f32 = 0.05;

impl LyonRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        view_layout: &wgpu::BindGroupLayout,
        samples: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fill"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("shaders/view.wgsl"),
                    include_str!("shaders/fill.wgsl")
                )
                .into(),
            ),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fill"),
            bind_group_layouts: &[Some(view_layout)],
            immediate_size: 0,
        });
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
        })];
        let pipeline = |label: &str, stencil_use| {
            crate::canvas::canvas_pipeline_stencil(
                device,
                label,
                &layout,
                &shader,
                &buffers,
                samples,
                stencil_use,
            )
        };
        Self {
            pipeline: pipeline("fill", StencilUse::Ignore),
            clipped: pipeline("clipped fill", StencilUse::Test),
            stamp: pipeline("clip stamp", StencilUse::Write),
            mesh: None,
        }
    }
}

impl VectorRenderer for LyonRenderer {
    fn set_fills(&mut self, device: &wgpu::Device, fills: &[FillPath]) {
        type Stamped = Option<(Arc<Path>, std::ops::Range<u32>)>;
        let mut geometry: VertexBuffers<Vertex, u32> = VertexBuffers::new();
        let mut tessellator = FillTessellator::new();
        let mut stroker = StrokeTessellator::new();
        let options = FillOptions::tolerance(TOLERANCE).with_fill_rule(FillRule::EvenOdd);
        let mut steps: Vec<Step> = Vec::new();
        // The clip region stamped now, and where its triangles are, to unstamp it afterwards.
        let mut stamped: Stamped = None;
        let mut drawn_from = 0u32;

        let close = |steps: &mut Vec<Step>,
                     geometry: &VertexBuffers<Vertex, u32>,
                     drawn_from: &mut u32,
                     stamped: &mut Stamped| {
            let end = geometry.indices.len() as u32;
            if end > *drawn_from {
                steps.push(Step::Draw {
                    indices: *drawn_from..end,
                    clipped: stamped.is_some(),
                });
            }
            if let Some((_, region)) = stamped.take() {
                steps.push(Step::Stamp {
                    indices: region,
                    value: 0,
                });
            }
            *drawn_from = end;
        };

        for fill in fills {
            let same_clip = match (&stamped, &fill.clip) {
                (None, None) => true,
                (Some((current, _)), Some(wanted)) => Arc::ptr_eq(current, wanted),
                _ => false,
            };
            if !same_clip {
                close(&mut steps, &geometry, &mut drawn_from, &mut stamped);
                if let Some(region) = &fill.clip {
                    let start = geometry.indices.len() as u32;
                    // A region lyon cannot fill stamps nothing, so what it clips vanishes.
                    let _ = tessellator.tessellate_path(
                        &**region,
                        &options,
                        &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| Vertex {
                            position: vertex.position().to_array(),
                            color: [0.0; 4],
                        }),
                    );
                    let end = geometry.indices.len() as u32;
                    steps.push(Step::Stamp {
                        indices: start..end,
                        value: 1,
                    });
                    stamped = Some((region.clone(), start..end));
                    drawn_from = end;
                }
            }
            let color = fill.color.to_linear_premultiplied();
            if let Some(width) = fill.stroke_width {
                let options = StrokeOptions::tolerance(TOLERANCE)
                    .with_line_width(width)
                    .with_line_join(LineJoin::Round);
                let _ = stroker.tessellate_path(
                    &fill.path,
                    &options,
                    &mut BuffersBuilder::new(&mut geometry, |vertex: StrokeVertex| Vertex {
                        position: vertex.position().to_array(),
                        color,
                    }),
                );
                continue;
            }
            // A path lyon cannot fill draws nothing rather than failing the frame.
            let _ = tessellator.tessellate_path(
                &fill.path,
                &options,
                &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| Vertex {
                    position: vertex.position().to_array(),
                    color,
                }),
            );
        }
        close(&mut steps, &geometry, &mut drawn_from, &mut stamped);
        self.mesh = (!geometry.indices.is_empty()).then(|| Mesh {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fill vertices"),
                contents: bytemuck::cast_slice(&geometry.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fill indices"),
                contents: bytemuck::cast_slice(&geometry.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            steps,
        });
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(mesh) = &self.mesh else { return };
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        for step in &mesh.steps {
            match step {
                Step::Draw { indices, clipped } => {
                    pass.set_pipeline(if *clipped {
                        &self.clipped
                    } else {
                        &self.pipeline
                    });
                    pass.set_stencil_reference(1);
                    pass.draw_indexed(indices.clone(), 0, 0..1);
                }
                Step::Stamp { indices, value } => {
                    pass.set_pipeline(&self.stamp);
                    pass.set_stencil_reference(*value);
                    pass.draw_indexed(indices.clone(), 0, 0..1);
                }
            }
        }
    }
}
