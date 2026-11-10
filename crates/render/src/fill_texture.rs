//! Texture images for Fill styles (ADR 0015): which texture a fill vertex names, and the GPU
//! array that holds the images.
//!
//! A vertex can only carry a byte for its texture, and tessellated vertices are cached per
//! Object, so the byte has to mean the same thing for as long as the process runs. [`slot_for`]
//! hands out those bytes: the first texture asked for gets 1, the next 2, and so on, and a slot
//! is never reused. The renderer keeps one layer of a texture array per slot ([`FillTextures`]),
//! loading each image from the installed packs. Slot 0 means no texture.
//!
//! Every image is kept at [`TEXTURE_SIZE`] pixels square, with mip levels, so a zoomed-out
//! fill averages the image rather than shimmering. An image of another size is resampled with
//! wrap-around, which keeps a seamless texture seamless. A texture that is not installed or
//! cannot be read becomes mid-grey, which leaves the fill as it was.

use std::sync::{Mutex, PoisonError};

use ssmap_core::asset::{AssetRef, PackLibrary};

use crate::raster::Art;
use crate::sprite::SpriteStore;

/// Pixels on a side of every texture layer.
pub(crate) const TEXTURE_SIZE: u32 = 256;

/// Mip levels from [`TEXTURE_SIZE`] down to one pixel.
const MIP_LEVELS: u32 = 9;

/// The most textures at once; a vertex names one in a byte, and 0 is none.
pub(crate) const MAX_SLOTS: usize = 255;

/// The textures asked for so far, in slot order (slot `i + 1` is entry `i`).
static SLOTS: Mutex<Vec<AssetRef>> = Mutex::new(Vec::new());

/// The slot (from 1) of `texture`, given out on first use. 0 if all [`MAX_SLOTS`] are taken,
/// in which case the fill is drawn without a texture.
pub(crate) fn slot_for(texture: &AssetRef) -> u8 {
    let mut slots = SLOTS.lock().unwrap_or_else(PoisonError::into_inner);
    let index = match slots.iter().position(|slot| slot == texture) {
        Some(index) => index,
        None if slots.len() < MAX_SLOTS => {
            slots.push(texture.clone());
            slots.len() - 1
        }
        None => return 0,
    };
    u8::try_from(index + 1).unwrap_or(0)
}

/// The textures given a slot so far, in slot order.
fn slots() -> Vec<AssetRef> {
    SLOTS.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The texture array every fill pipeline samples, as bind group 1.
pub(crate) struct FillTextures {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    store: Option<SpriteStore>,
    /// The slots and the library generation the array was built from.
    built: (usize, u64),
}

impl FillTextures {
    /// Starts with one layer that is never drawn from: no vertex names a slot until one has been
    /// given, and [`FillTextures::refresh`] loads it before it is drawn.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fill textures"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fill textures"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let bind_group = build(device, None, &layout, &sampler, &[]);
        Self {
            layout,
            sampler,
            bind_group,
            store: None,
            built: (0, 0),
        }
    }

    pub(crate) fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Where the images come from. Forgets what was loaded.
    pub(crate) fn set_store(&mut self, store: SpriteStore) {
        self.store = Some(store);
        self.built = (0, u64::MAX);
    }

    /// Loads the textures given a slot since the last call, or all again if the installed packs
    /// changed. Call it after everything that can give out a slot (tessellation) and before
    /// drawing. Returns whether the bind group is new.
    pub(crate) fn refresh(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let slots = slots();
        let generation = self.store.as_ref().map_or(0, SpriteStore::generation);
        if (slots.len(), generation) == self.built {
            return false;
        }
        let library = self.store.as_ref().map(SpriteStore::library);
        let layers: Vec<Vec<Vec<u8>>> = slots
            .iter()
            .map(|texture| load_mips(library.as_deref(), texture))
            .collect();
        self.bind_group = build(device, Some(queue), &self.layout, &self.sampler, &layers);
        self.built = (slots.len(), generation);
        true
    }
}

/// Uploads `layers` (each a list of mip levels of RGBA8 pixels) as a texture array, and binds
/// it. With no layers the array has one that is left empty, so the bind group is still valid.
fn build(
    device: &wgpu::Device,
    queue: Option<&wgpu::Queue>,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    layers: &[Vec<Vec<u8>>],
) -> wgpu::BindGroup {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fill textures"),
        size: wgpu::Extent3d {
            width: TEXTURE_SIZE,
            height: TEXTURE_SIZE,
            depth_or_array_layers: layers.len().max(1) as u32,
        },
        mip_level_count: MIP_LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // Not sRGB: the shader blends the encoded values, with mid-grey meaning no change.
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, levels) in layers.iter().enumerate() {
        let Some(queue) = queue else { break };
        for (level, pixels) in levels.iter().enumerate() {
            let side = (TEXTURE_SIZE >> level).max(1);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(side * 4),
                    rows_per_image: Some(side),
                },
                wgpu::Extent3d {
                    width: side,
                    height: side,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fill textures"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// Mid-grey: what a missing texture is, and what multiplies a fill by 1.
fn neutral_pixels() -> Vec<u8> {
    vec![128; (TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize]
}

/// The mip levels of `texture` from the installed packs, or of mid-grey if it cannot be had.
fn load_mips(library: Option<&PackLibrary>, texture: &AssetRef) -> Vec<Vec<u8>> {
    mips(&load(library, texture).unwrap_or_else(neutral_pixels))
}

/// `TEXTURE_SIZE` square RGBA pixels of the texture, opaque, if it is installed and readable.
fn load(library: Option<&PackLibrary>, texture: &AssetRef) -> Option<Vec<u8>> {
    let pack = library?.pack(&texture.pack)?;
    let entry = pack.texture(&texture.asset)?;
    let Art::Png(raster) = Art::load(&pack.path(&entry.file)).ok()? else {
        return None;
    };
    let rgba = raster.unpremultiplied();
    Some(resample(&rgba, raster.width, raster.height))
}

/// `rgba` (`width` by `height`) as `TEXTURE_SIZE` square pixels with alpha dropped. Bilinear
/// and wrapping at the edges, so the result tiles if the source does.
fn resample(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (width, height) = (width.max(1), height.max(1));
    let at = |x: i64, y: i64, c: usize| -> f32 {
        let x = x.rem_euclid(i64::from(width)) as usize;
        let y = y.rem_euclid(i64::from(height)) as usize;
        f32::from(rgba[(y * width as usize + x) * 4 + c])
    };
    let mut out = Vec::with_capacity((TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize);
    for oy in 0..TEXTURE_SIZE {
        for ox in 0..TEXTURE_SIZE {
            // The source position of this pixel's centre, then the four pixels around it.
            let sx = (ox as f32 + 0.5) * width as f32 / TEXTURE_SIZE as f32 - 0.5;
            let sy = (oy as f32 + 0.5) * height as f32 / TEXTURE_SIZE as f32 - 0.5;
            let (x0, y0) = (sx.floor(), sy.floor());
            let (fx, fy) = (sx - x0, sy - y0);
            for c in 0..3 {
                let (x0, y0) = (x0 as i64, y0 as i64);
                let top = at(x0, y0, c) * (1.0 - fx) + at(x0 + 1, y0, c) * fx;
                let bottom = at(x0, y0 + 1, c) * (1.0 - fx) + at(x0 + 1, y0 + 1, c) * fx;
                out.push((top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8);
            }
            out.push(255);
        }
    }
    out
}

/// `base` and every smaller level, each a 2x2 box average of the one before.
fn mips(base: &[u8]) -> Vec<Vec<u8>> {
    let mut levels = vec![base.to_vec()];
    for level in 1..MIP_LEVELS {
        let side = (TEXTURE_SIZE >> level).max(1) as usize;
        let above = levels[level as usize - 1].as_slice();
        let above_side = side * 2;
        let mut next = Vec::with_capacity(side * side * 4);
        for y in 0..side {
            for x in 0..side {
                for c in 0..4 {
                    let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                        .iter()
                        .map(|(dx, dy)| {
                            u32::from(above[((y * 2 + dy) * above_side + x * 2 + dx) * 4 + c])
                        })
                        .sum();
                    next.push(((sum + 2) / 4) as u8);
                }
            }
        }
        levels.push(next);
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_stable_and_start_at_one() {
        let a: AssetRef = "slots-test/a".parse().unwrap();
        let b: AssetRef = "slots-test/b".parse().unwrap();
        let first = slot_for(&a);
        assert_ne!(first, 0, "0 means no texture");
        assert_eq!(slot_for(&a), first);
        let second = slot_for(&b);
        assert!(second != 0 && second != first);
        assert_eq!(slots()[usize::from(first) - 1], a);
    }

    #[test]
    fn resampling_wraps_and_keeps_a_uniform_image_uniform() {
        let grey = vec![200; 4 * 4 * 4];
        let out = resample(&grey, 4, 4);
        assert_eq!(out.len(), (TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize);
        assert!(out.chunks(4).all(|px| px == [200, 200, 200, 255]));
        // A 2x1 image of black then white blends across the wrap from white back to black.
        let stripes = [0, 0, 0, 255, 255, 255, 255, 255];
        let out = resample(&stripes, 2, 1);
        let at = |x: u32| out[(x * 4) as usize];
        // The source pixel centres land near output x = 64 (black) and 192 (white).
        assert!(at(TEXTURE_SIZE / 4) < 8 && at(TEXTURE_SIZE * 3 / 4) > 247);
        // Across the wrap the two edge pixels sit either side of the midpoint, with no seam.
        let (last, first) = (i32::from(at(TEXTURE_SIZE - 1)), i32::from(at(0)));
        assert!((100..=156).contains(&last) && (100..=156).contains(&first));
        assert!((last - first).abs() <= 8);
        assert!(at(TEXTURE_SIZE / 2 - 1) < at(TEXTURE_SIZE / 2 + 8));
    }

    #[test]
    fn mips_average_down_to_one_pixel() {
        let mut base = neutral_pixels();
        // One half black, one half white.
        for (i, px) in base.chunks_mut(4).enumerate() {
            let x = i as u32 % TEXTURE_SIZE;
            px[..3].fill(if x < TEXTURE_SIZE / 2 { 0 } else { 255 });
        }
        let levels = mips(&base);
        assert_eq!(levels.len(), MIP_LEVELS as usize);
        assert_eq!(levels[8].len(), 4);
        assert!((126..=129).contains(&levels[8][0]));
        assert_eq!(levels[1].len(), 128 * 128 * 4);
    }
}
