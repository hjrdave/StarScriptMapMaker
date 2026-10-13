//! Copies the resolved Canvas target into a host's frame, such as egui's render pass.

pub struct Presenter {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    view_format: wgpu::TextureFormat,
    bind_group: Option<(wgpu::Texture, wgpu::BindGroup)>,
}

impl Presenter {
    /// `target_format` is the host frame's format; `samples` its MSAA sample count.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("present"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/present.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(target_format.into())],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        // An sRGB frame decodes and re-encodes; a non-sRGB frame takes the encoded bytes as is.
        let view_format = if target_format.is_srgb() {
            crate::CANVAS_FORMAT
        } else {
            crate::CANVAS_FORMAT.remove_srgb_suffix()
        };
        Self {
            pipeline,
            layout,
            view_format,
            bind_group: None,
        }
    }

    /// Points the presenter at a Canvas target, as returned by `CanvasRenderer::render`.
    pub fn set_source(&mut self, device: &wgpu::Device, texture: &wgpu::Texture) {
        if self.bind_group.as_ref().is_some_and(|(t, _)| t == texture) {
            return;
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        self.bind_group = Some((texture.clone(), bind_group));
    }

    /// Draws the source over the pass's current viewport.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some((_, bind_group)) = &self.bind_group else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
