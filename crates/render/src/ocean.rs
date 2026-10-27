//! The ocean texture drawn over the Canvas background fill.

use bytemuck::Zeroable as _;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct OceanUniform {
    pub(crate) canvas_size: [f32; 2],
    pub(crate) strength: f32,
    pub(crate) scale: f32,
    /// The Canvas corner in workspace coordinates.
    pub(crate) origin: [f32; 2],
    pub(crate) _pad: [f32; 2],
    /// Linear, premultiplied colours the noise lightens and darkens towards.
    pub(crate) light: [f32; 4],
    pub(crate) dark: [f32; 4],
}

/// The ocean texture of every board in view: one uniform and bind group each.
pub(crate) struct OceanPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    boards: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    /// How many of `boards` the last [`OceanPass::set`] filled in.
    count: usize,
}

impl OceanPass {
    pub(crate) fn new(
        device: &wgpu::Device,
        view_layout: &wgpu::BindGroupLayout,
        samples: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(
                    include_str!("shaders/view.wgsl"),
                    include_str!("shaders/ocean.wgsl")
                )
                .into(),
            ),
        });
        let layout = crate::canvas::uniform_layout(device, "ocean");
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ocean"),
            bind_group_layouts: &[Some(view_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = crate::canvas::canvas_pipeline(
            device,
            "ocean",
            &pipeline_layout,
            &shader,
            &[],
            samples,
            crate::canvas::StencilUse::Ignore,
        );
        Self {
            pipeline,
            layout,
            boards: Vec::new(),
            count: 0,
        }
    }

    /// Sets the ocean of each board, in the order they draw.
    pub(crate) fn set(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uniforms: &[OceanUniform],
    ) {
        while self.boards.len() < uniforms.len() {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ocean"),
                contents: bytemuck::bytes_of(&OceanUniform::zeroed()),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ocean"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            self.boards.push((buffer, bind_group));
        }
        for ((buffer, _), uniform) in self.boards.iter().zip(uniforms) {
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(uniform));
        }
        self.count = uniforms.len();
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        for (_, bind_group) in self.boards.iter().take(self.count) {
            pass.set_bind_group(1, bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}
