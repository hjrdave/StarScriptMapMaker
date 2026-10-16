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

pub(crate) struct OceanPass {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
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
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean"),
            contents: bytemuck::bytes_of(&OceanUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layout = crate::canvas::uniform_layout(device, "ocean");
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
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
            uniform,
            bind_group,
        }
    }

    pub(crate) fn set(&self, queue: &wgpu::Queue, uniform: OceanUniform) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}
