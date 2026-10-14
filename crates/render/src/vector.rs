//! Vector fills and strokes behind a trait, so the renderer can be swapped (ADR 0001).

use std::ops::Range;

use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use wgpu::util::DeviceExt;

pub use lyon::tessellation::FillRule;

use crate::canvas::{CLIP_INSIDE, StencilUse};
use crate::color::{Color, ColorExt as _};

/// How a path is painted.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Paint {
    Fill(FillRule),
    /// A stroke centred on the path, `width` Canvas units across, with round joins and caps.
    Stroke {
        width: f32,
    },
}

/// One painted path in Canvas units.
#[derive(Clone, Debug)]
pub struct VectorShape {
    pub path: Path,
    pub color: Color,
    pub paint: Paint,
}

/// Shapes painted as one, such as a Label's fill and its halo. Every sample takes the colour
/// of the first shape in the group that covers it, so the group never blends with itself:
/// a translucent stroke doesn't darken where it overlaps itself, and a halo listed after the
/// fill shows only around it.
#[derive(Clone, Debug, Default)]
pub struct VectorGroup {
    pub shapes: Vec<VectorShape>,
}

impl VectorGroup {
    pub fn fill(path: Path, color: Color, rule: FillRule) -> Self {
        Self {
            shapes: vec![VectorShape {
                path,
                color,
                paint: Paint::Fill(rule),
            }],
        }
    }

    /// Whether drawing the group's triangles plainly could cover a sample twice. Fill
    /// tessellation never overlaps itself; strokes and stacked shapes do.
    fn may_overlap(&self) -> bool {
        match self.shapes.as_slice() {
            [] => false,
            [shape] => matches!(shape.paint, Paint::Stroke { .. }),
            _ => true,
        }
    }
}

/// Draws vector groups into a Canvas render pass whose group 0 is the view uniform and which
/// has a stencil attachment cleared to 0.
pub trait VectorRenderer {
    /// Replaces what is drawn. Groups draw in order, first lowest.
    fn set_groups(&mut self, device: &wgpu::Device, groups: &[VectorGroup]);

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
    /// Draws without touching the stencil.
    plain: wgpu::RenderPipeline,
    /// Draws only inside the active Clip.
    clipped: wgpu::RenderPipeline,
    /// Draws only where the stencil equals the reference, and marks what it draws.
    once: wgpu::RenderPipeline,
    /// Draws no colour, and clears the marks `once` left.
    reset: wgpu::RenderPipeline,
    /// Draws no colour, and sets the stencil to the reference.
    mask: wgpu::RenderPipeline,
    mesh: Option<Mesh>,
}

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    draws: Vec<Draw>,
    /// Ranges of `draws`, one per run (see [`LyonRenderer::set_runs`]).
    runs: Vec<Range<usize>>,
}

/// A run of indices: either several groups that can't overlap themselves, drawn plainly,
/// or one group drawn once per sample.
struct Draw {
    indices: Range<u32>,
    once: bool,
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
        let pipeline = |label, stencil| {
            crate::canvas::canvas_pipeline(
                device,
                label,
                &layout,
                &shader,
                &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
                samples,
                stencil,
            )
        };
        Self {
            plain: pipeline("fill", StencilUse::Ignore),
            clipped: pipeline("fill clipped", StencilUse::Clipped),
            once: pipeline("fill once", StencilUse::PaintOnce),
            reset: pipeline("fill reset", StencilUse::Reset),
            mask: pipeline("fill mask", StencilUse::Mask),
            mesh: None,
        }
    }
}

impl LyonRenderer {
    /// Like [`VectorRenderer::set_groups`], but also splits the groups into runs that
    /// [`LyonRenderer::draw_run`] draws separately, so something else can draw between them.
    /// `cuts` lists, in increasing order, the group counts to split at: run `i` ends before
    /// group `cuts[i]`, and the last run takes the rest. There is one more run than cuts.
    pub(crate) fn set_runs(
        &mut self,
        device: &wgpu::Device,
        groups: &[VectorGroup],
        cuts: &[usize],
    ) {
        let mut geometry: VertexBuffers<Vertex, u32> = VertexBuffers::new();
        let mut fill = FillTessellator::new();
        let mut stroke = StrokeTessellator::new();
        let mut draws: Vec<Draw> = Vec::new();
        let mut runs: Vec<Range<usize>> = Vec::new();
        let mut run_start = 0;
        let mut remaining_cuts = cuts.iter().peekable();
        // The first group after a cut starts a new draw, so no draw spans two runs.
        let mut split = false;
        for (position, group) in groups.iter().enumerate() {
            while remaining_cuts.next_if(|&&cut| cut <= position).is_some() {
                runs.push(run_start..draws.len());
                run_start = draws.len();
                split = true;
            }
            let start = geometry.indices.len() as u32;
            for shape in &group.shapes {
                tessellate(shape, &mut geometry, &mut fill, &mut stroke);
            }
            let end = geometry.indices.len() as u32;
            if start == end {
                continue;
            }
            let once = group.may_overlap();
            match draws.last_mut() {
                Some(last) if !split && !once && !last.once => last.indices.end = end,
                _ => draws.push(Draw {
                    indices: start..end,
                    once,
                }),
            }
            split = false;
        }
        for _ in remaining_cuts {
            runs.push(run_start..draws.len());
            run_start = draws.len();
        }
        runs.push(run_start..draws.len());
        self.mesh = (!geometry.indices.is_empty()).then(|| Mesh {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vector vertices"),
                contents: bytemuck::cast_slice(&geometry.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vector indices"),
                contents: bytemuck::cast_slice(&geometry.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            draws,
            runs,
        });
    }

    /// Draws run `run` of the groups last given to [`LyonRenderer::set_runs`], only inside the
    /// active Clip if `clipped`.
    pub(crate) fn draw_run(&self, pass: &mut wgpu::RenderPass<'_>, run: usize, clipped: bool) {
        let Some(mesh) = &self.mesh else { return };
        let Some(range) = mesh.runs.get(run) else {
            return;
        };
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_stencil_reference(if clipped { CLIP_INSIDE } else { 0 });
        let plain = if clipped { &self.clipped } else { &self.plain };
        for draw in &mesh.draws[range.clone()] {
            if draw.once {
                pass.set_pipeline(&self.once);
                pass.draw_indexed(draw.indices.clone(), 0, 0..1);
                pass.set_pipeline(&self.reset);
                pass.draw_indexed(draw.indices.clone(), 0, 0..1);
            } else {
                pass.set_pipeline(plain);
                pass.draw_indexed(draw.indices.clone(), 0, 0..1);
            }
        }
    }

    /// Sets the stencil to `value` under run `run` of the groups last given to
    /// [`LyonRenderer::set_runs`], without drawing colour.
    pub(crate) fn draw_mask(&self, pass: &mut wgpu::RenderPass<'_>, run: usize, value: u32) {
        let Some(mesh) = &self.mesh else { return };
        let Some(range) = mesh.runs.get(run) else {
            return;
        };
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_stencil_reference(value);
        pass.set_pipeline(&self.mask);
        for draw in &mesh.draws[range.clone()] {
            pass.draw_indexed(draw.indices.clone(), 0, 0..1);
        }
    }
}

impl VectorRenderer for LyonRenderer {
    fn set_groups(&mut self, device: &wgpu::Device, groups: &[VectorGroup]) {
        self.set_runs(device, groups, &[]);
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_run(pass, 0, false);
    }
}

/// Adds one shape's triangles. A path lyon cannot tessellate draws nothing rather than
/// failing the frame.
fn tessellate(
    shape: &VectorShape,
    geometry: &mut VertexBuffers<Vertex, u32>,
    fill: &mut FillTessellator,
    stroke: &mut StrokeTessellator,
) {
    let color = shape.color.to_linear_premultiplied();
    match shape.paint {
        Paint::Fill(rule) => {
            let options = FillOptions::tolerance(TOLERANCE).with_fill_rule(rule);
            let _ = fill.tessellate_path(
                &shape.path,
                &options,
                &mut BuffersBuilder::new(geometry, |vertex: FillVertex| Vertex {
                    position: vertex.position().to_array(),
                    color,
                }),
            );
        }
        Paint::Stroke { width } => {
            let options = StrokeOptions::tolerance(TOLERANCE)
                .with_line_width(width)
                .with_line_join(LineJoin::Round)
                .with_line_cap(LineCap::Round);
            let _ = stroke.tessellate_path(
                &shape.path,
                &options,
                &mut BuffersBuilder::new(geometry, |vertex: StrokeVertex| Vertex {
                    position: vertex.position().to_array(),
                    color,
                }),
            );
        }
    }
}
