//! Paged texture atlases for Stamp art (ADR 0001).
//!
//! Each page is a square texture. The rectangle allocator (`etagere`) is private to this
//! module, so it can be swapped without touching callers.

use crate::raster::Raster;

/// The smallest page. wgpu guarantees every adapter supports 2D textures this large.
pub const MIN_PAGE_SIZE: u32 = 2048;
/// The largest page: 64 MiB of RGBA8. Bigger pages would waste memory on mostly-empty space.
pub const MAX_PAGE_SIZE: u32 = 4096;

/// Transparent pixels around every entry, so filtering never samples a neighbour.
const GUTTER: u32 = 1;

/// Premultiplied, sRGB-encoded RGBA, matching [`Raster`].
pub const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Where one raster lives in the atlas.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AtlasEntry {
    pub page: u32,
    /// The raster's pixels within the page, excluding the gutter.
    pub rect: AtlasRect,
    id: etagere::AllocId,
}

impl AtlasEntry {
    /// `[u0, v0, u1, v1]` texture coordinates of the entry on its page.
    pub fn uv(&self, page_size: u32) -> [f32; 4] {
        let size = page_size as f32;
        let AtlasRect {
            x,
            y,
            width,
            height,
        } = self.rect;
        [
            x as f32 / size,
            y as f32 / size,
            (x + width) as f32 / size,
            (y + height) as f32 / size,
        ]
    }
}

/// A rectangle of pixels on a page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AtlasRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasRect {
    pub fn intersects(&self, other: &AtlasRect) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum AtlasError {
    /// The raster, with its gutter, is larger than a page.
    #[error("a {width}x{height} raster does not fit on a {page_size}x{page_size} atlas page")]
    TooLarge {
        width: u32,
        height: u32,
        page_size: u32,
    },
}

/// The page size for a device: its largest 2D texture, between [`MIN_PAGE_SIZE`] and
/// [`MAX_PAGE_SIZE`].
pub fn page_size_for(limits: &wgpu::Limits) -> u32 {
    limits
        .max_texture_dimension_2d
        .clamp(MIN_PAGE_SIZE, MAX_PAGE_SIZE)
}

/// Rectangle packing across pages, without any GPU state.
struct Packer {
    page_size: u32,
    pages: Vec<etagere::AtlasAllocator>,
}

impl Packer {
    fn new(page_size: u32) -> Self {
        Self {
            page_size,
            pages: Vec::new(),
        }
    }

    /// Finds room for `width` x `height` pixels plus the gutter, opening a page if none has
    /// room. Returns the entry and whether a page was opened.
    fn allocate(&mut self, width: u32, height: u32) -> Result<(AtlasEntry, bool), AtlasError> {
        let (padded_width, padded_height) = (width + 2 * GUTTER, height + 2 * GUTTER);
        if width == 0
            || height == 0
            || padded_width > self.page_size
            || padded_height > self.page_size
        {
            return Err(AtlasError::TooLarge {
                width,
                height,
                page_size: self.page_size,
            });
        }
        let size = etagere::size2(padded_width as i32, padded_height as i32);
        let entry = |page: usize, allocation: etagere::Allocation| AtlasEntry {
            page: page as u32,
            rect: AtlasRect {
                x: allocation.rectangle.min.x as u32 + GUTTER,
                y: allocation.rectangle.min.y as u32 + GUTTER,
                width,
                height,
            },
            id: allocation.id,
        };
        for (page, allocator) in self.pages.iter_mut().enumerate() {
            if let Some(allocation) = allocator.allocate(size) {
                return Ok((entry(page, allocation), false));
            }
        }
        let side = self.page_size as i32;
        // Two columns of shelves pack mixed sizes far tighter than one (79% vs 56% in the
        // golden test), but an entry wider than a column needs a page with just one.
        let num_columns = if padded_width * 2 <= self.page_size {
            2
        } else {
            1
        };
        let mut allocator = etagere::AtlasAllocator::with_options(
            etagere::size2(side, side),
            &etagere::AllocatorOptions {
                num_columns,
                ..Default::default()
            },
        );
        let allocation = allocator
            .allocate(size)
            .expect("an empty page fits the entry");
        self.pages.push(allocator);
        Ok((entry(self.pages.len() - 1, allocation), true))
    }

    fn deallocate(&mut self, entry: AtlasEntry) {
        self.pages[entry.page as usize].deallocate(entry.id);
    }
}

/// Paged atlas textures. Pages are created as entries need them.
pub struct Atlas {
    packer: Packer,
    pages: Vec<Page>,
}

struct Page {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Atlas {
    /// `page_size` usually comes from [`page_size_for`].
    pub fn new(page_size: u32) -> Self {
        Self {
            packer: Packer::new(page_size),
            pages: Vec::new(),
        }
    }

    pub fn page_size(&self) -> u32 {
        self.packer.page_size
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page_texture(&self, page: u32) -> &wgpu::Texture {
        &self.pages[page as usize].texture
    }

    pub fn page_view(&self, page: u32) -> &wgpu::TextureView {
        &self.pages[page as usize].view
    }

    /// Copies `raster` into the atlas, opening a page if needed.
    pub fn insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        raster: &Raster,
    ) -> Result<AtlasEntry, AtlasError> {
        let (entry, new_page) = self.packer.allocate(raster.width, raster.height)?;
        if new_page {
            let size = self.packer.page_size;
            let scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let page = Page::new(device, size, entry.page);
            if let Some(err) = pollster::block_on(scope.pop()) {
                eprintln!(
                    "GPU allocation failed: atlas page {} ({size}x{size}, {:?}) on {}: {err}",
                    entry.page,
                    ATLAS_FORMAT,
                    crate::gpu::describe_limits("the GPU", &device.limits()),
                );
            }
            self.pages.push(page);
        }
        // Write the gutter too: a reused rectangle may hold an old entry's pixels.
        let padded_width = raster.width + 2 * GUTTER;
        let padded_height = raster.height + 2 * GUTTER;
        let mut padded = vec![0u8; (padded_width * padded_height * 4) as usize];
        let row = (raster.width * 4) as usize;
        for (y, source) in raster.pixels.chunks_exact(row).enumerate() {
            let start = (((y as u32 + GUTTER) * padded_width + GUTTER) * 4) as usize;
            padded[start..start + row].copy_from_slice(source);
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.pages[entry.page as usize].texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: entry.rect.x - GUTTER,
                    y: entry.rect.y - GUTTER,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &padded,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_width * 4),
                rows_per_image: Some(padded_height),
            },
            wgpu::Extent3d {
                width: padded_width,
                height: padded_height,
                depth_or_array_layers: 1,
            },
        );
        Ok(entry)
    }

    /// Frees an entry's space for later inserts. Its pixels stay until overwritten.
    pub fn remove(&mut self, entry: AtlasEntry) {
        self.packer.deallocate(entry);
    }
}

impl Page {
    fn new(device: &wgpu::Device, size: u32, index: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("atlas page {index}")),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ATLAS_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_size_follows_the_device_within_bounds() {
        let limits = |max| wgpu::Limits {
            max_texture_dimension_2d: max,
            ..wgpu::Limits::downlevel_defaults()
        };
        assert_eq!(page_size_for(&limits(2048)), 2048);
        assert_eq!(page_size_for(&limits(3000)), 3000);
        assert_eq!(page_size_for(&limits(16384)), MAX_PAGE_SIZE);
    }

    #[test]
    fn entries_never_overlap_and_spill_onto_new_pages() {
        let mut packer = Packer::new(256);
        let mut entries = Vec::new();
        for i in 0..200u32 {
            let (entry, _) = packer.allocate(10 + i % 37, 8 + i % 23).unwrap();
            entries.push(entry);
        }
        assert!(packer.pages.len() > 1);
        for (i, a) in entries.iter().enumerate() {
            assert!(a.rect.x >= GUTTER && a.rect.x + a.rect.width + GUTTER <= 256);
            assert!(a.rect.y >= GUTTER && a.rect.y + a.rect.height + GUTTER <= 256);
            for b in &entries[i + 1..] {
                if a.page == b.page {
                    let grown = AtlasRect {
                        x: a.rect.x - GUTTER,
                        y: a.rect.y - GUTTER,
                        width: a.rect.width + 2 * GUTTER,
                        height: a.rect.height + 2 * GUTTER,
                    };
                    assert!(!grown.intersects(&b.rect), "{a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn freed_space_is_reused() {
        let mut packer = Packer::new(64);
        let (first, opened) = packer.allocate(62, 62).unwrap();
        assert!(opened);
        packer.deallocate(first);
        let (second, opened) = packer.allocate(62, 62).unwrap();
        assert!(!opened);
        assert_eq!(second.page, 0);
    }

    #[test]
    fn oversized_rasters_are_refused() {
        let mut packer = Packer::new(64);
        assert_eq!(
            packer.allocate(63, 10),
            Err(AtlasError::TooLarge {
                width: 63,
                height: 10,
                page_size: 64
            })
        );
        assert!(packer.allocate(0, 10).is_err());
    }

    #[test]
    fn uvs_cover_the_entry() {
        let mut packer = Packer::new(64);
        let (entry, _) = packer.allocate(16, 32).unwrap();
        assert_eq!(
            entry.uv(64),
            [1.0 / 64.0, 1.0 / 64.0, 17.0 / 64.0, 33.0 / 64.0]
        );
    }
}
