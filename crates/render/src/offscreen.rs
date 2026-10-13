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
    let [width, height] = view.size;
    let unpadded = width * 4;
    let padded =
        unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    let texture = renderer.render(device, queue, &mut encoder, scene, view);
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
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| {
        result.expect("readback buffer maps");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the GPU finishes the render");
    let mapped = slice
        .get_mapped_range()
        .expect("the mapped range is readable");
    let mut pixels = Vec::with_capacity((unpadded * height) as usize);
    for row in mapped.chunks(padded as usize) {
        pixels.extend_from_slice(&row[..unpadded as usize]);
    }
    drop(mapped);
    buffer.unmap();
    RgbaImage {
        width,
        height,
        pixels,
    }
}
