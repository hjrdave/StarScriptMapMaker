//! Rendering without a window, read back to CPU memory. Export and tests use it.

use crate::canvas::{CanvasRenderer, CanvasScene};
use crate::view::View;

/// Tightly packed 8-bit sRGB RGBA pixels, rows top to bottom.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Renders `scene` through `view` and waits for the pixels.
pub fn render_to_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut CanvasRenderer,
    scene: &CanvasScene,
    view: &View,
) -> RgbaImage {
    let mut encoder = device.create_command_encoder(&Default::default());
    let texture = renderer.render(device, queue, &mut encoder, scene, view);
    let readback = Readback::record(device, &mut encoder, texture);
    queue.submit([encoder.finish()]);
    readback.finish(device)
}

/// Copies an RGBA8 texture's first mip level back to CPU memory, such as an atlas page.
pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> RgbaImage {
    let mut encoder = device.create_command_encoder(&Default::default());
    let readback = Readback::record(device, &mut encoder, texture);
    queue.submit([encoder.finish()]);
    readback.finish(device)
}

/// A texture copy into a mappable buffer, recorded but not yet waited for.
struct Readback {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded: u32,
}

impl Readback {
    fn record(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Self {
        let (width, height) = (texture.width(), texture.height());
        let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Self {
            buffer,
            width,
            height,
            padded,
        }
    }

    /// Waits for the submitted copy and unpads the rows.
    fn finish(self, device: &wgpu::Device) -> RgbaImage {
        let slice = self.buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| {
            result.expect("readback buffer maps");
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the GPU finishes its work");
        let mapped = slice
            .get_mapped_range()
            .expect("the mapped range is readable");
        let unpadded = (self.width * 4) as usize;
        let mut pixels = Vec::with_capacity(unpadded * self.height as usize);
        for row in mapped.chunks(self.padded as usize) {
            pixels.extend_from_slice(&row[..unpadded]);
        }
        drop(mapped);
        self.buffer.unmap();
        RgbaImage {
            width: self.width,
            height: self.height,
            pixels,
        }
    }
}
