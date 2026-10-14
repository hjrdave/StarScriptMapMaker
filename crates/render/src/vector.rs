//! Vector fills behind a trait, so the renderer can be swapped (ADR 0001).

use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};
use wgpu::util::DeviceExt;

use crate::color::Color;

/// One filled path in Canvas units, filled even-odd: a subpath inside another is a hole.
#[derive(Clone, Debug)]
pub struct FillPath {
    pub path: Path,
    pub color: Color,
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
    pipeline: wgpu::RenderPipeline,
    mesh: Option<Mesh>,
}

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
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
        let pipeline = crate::canvas::canvas_pipeline(
            device,
            "fill",
            &layout,
            &shader,
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
            })],
            samples,
        );
        Self {
            pipeline,
            mesh: None,
        }
    }
}

impl VectorRenderer for LyonRenderer {
    fn set_fills(&mut self, device: &wgpu::Device, fills: &[FillPath]) {
        let mut geometry: VertexBuffers<Vertex, u32> = VertexBuffers::new();
        let mut tessellator = FillTessellator::new();
        let options = FillOptions::tolerance(TOLERANCE).with_fill_rule(FillRule::EvenOdd);
        for fill in fills {
            let color = fill.color.to_linear_premultiplied();
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
            count: geometry.indices.len() as u32,
        });
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(mesh) = &self.mesh else { return };
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.count, 0, 0..1);
    }
}
