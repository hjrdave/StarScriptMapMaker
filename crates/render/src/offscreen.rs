//! Rendering without a window, read back to CPU memory. Export and tests use it.

use crate::canvas::{CanvasRenderer, CanvasScene};
use crate::view::View;
use std::sync::mpsc;
use thiserror::Error;

/// Why the pixels of a render could not be read back from the GPU.
#[derive(Debug, Error)]
pub enum ReadbackError {
    #[error("the readback buffer could not be mapped")]
    Map(#[source] wgpu::BufferAsyncError),
    #[error("the GPU did not finish its work")]
    Poll(#[source] wgpu::PollError),
    #[error("the readback buffer was never mapped")]
    NotMapped,
    #[error("the mapped readback range is not readable")]
    Range(#[source] wgpu::MapRangeError),
}

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
) -> Result<RgbaImage, ReadbackError> {
    let mut encoder = device.create_command_encoder(&Default::default());
    // An image shows everything that is ready, so it applies all of it, not one frame's share.
    let budget = renderer.set_apply_budget(None);
    let texture = renderer.render(device, queue, &mut encoder, scene, view);
    let readback = Readback::record(device, &mut encoder, texture);
    renderer.set_apply_budget(budget);
    queue.submit([encoder.finish()]);
    readback.finish(device)
}

/// Copies an RGBA8 texture's first mip level back to CPU memory, such as an atlas page.
pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<RgbaImage, ReadbackError> {
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
    fn finish(self, device: &wgpu::Device) -> Result<RgbaImage, ReadbackError> {
        let slice = self.buffer.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            // The receiver lives until `finish` returns, so a failed send can be ignored.
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(ReadbackError::Poll)?;
        receiver
            .try_recv()
            .map_err(|_| ReadbackError::NotMapped)?
            .map_err(ReadbackError::Map)?;
        let mapped = slice.get_mapped_range().map_err(ReadbackError::Range)?;
        let unpadded = (self.width * 4) as usize;
        let mut pixels = Vec::with_capacity(unpadded * self.height as usize);
        for row in mapped.chunks(self.padded as usize) {
            pixels.extend_from_slice(&row[..unpadded]);
        }
        drop(mapped);
        self.buffer.unmap();
        Ok(RgbaImage {
            width: self.width,
            height: self.height,
            pixels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export_job::ExportError;

    #[test]
    fn a_readback_failure_reaches_an_export_as_an_error() {
        let error = ExportError::from(ReadbackError::NotMapped);
        assert!(matches!(
            error,
            ExportError::Readback(ReadbackError::NotMapped)
        ));
        assert_eq!(error.to_string(), "the readback buffer was never mapped");
    }
}
