//! Turning Asset art into pixels (ADR 0003): SVG through resvg at whatever width is needed,
//! PNG at its native resolution.

use std::path::{Path, PathBuf};

use resvg::{tiny_skia, usvg};
use ssmap_core::asset::ArtFormat;

/// The longest side [`Art::rasterise`] produces, so a runaway zoom can't exhaust memory.
pub const MAX_RASTER_SIZE: u32 = 8192;

/// Premultiplied-alpha, sRGB-encoded RGBA8 pixels, rows top to bottom. Atlases store this.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Raster {
    /// A fully transparent raster.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.pixels[i..i + 4].try_into().expect("four channels")
    }

    /// The same pixels with straight alpha, as PNG files store them.
    pub fn unpremultiplied(&self) -> Vec<u8> {
        let mut pixels = self.pixels.clone();
        for px in pixels.chunks_exact_mut(4) {
            let alpha = u32::from(px[3]);
            if alpha > 0 && alpha < 255 {
                for channel in &mut px[..3] {
                    *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
        pixels
    }
}

/// Asset art that could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum ArtError {
    #[error("is not a valid SVG")]
    Svg(#[from] usvg::Error),
    #[error("is not a valid PNG")]
    Png(#[from] image::ImageError),
    #[error("{}: cannot be read", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{}: is neither an .svg nor a .png file", path.display())]
    UnknownFormat { path: PathBuf },
    /// Art read from a file that then failed to parse.
    #[error("{}: {source}", path.display())]
    File {
        path: PathBuf,
        #[source]
        source: Box<ArtError>,
    },
}

/// One Asset's art (or Silhouette), parsed and ready to rasterise.
pub enum Art {
    Svg(Box<usvg::Tree>),
    Png(Raster),
}

impl Art {
    /// Loads an `.svg` or `.png` file.
    pub fn load(path: &Path) -> Result<Self, ArtError> {
        let format = ArtFormat::from_path(path).ok_or_else(|| ArtError::UnknownFormat {
            path: path.to_path_buf(),
        })?;
        let data = std::fs::read(path).map_err(|source| ArtError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        match format {
            ArtFormat::Svg => Self::from_svg(&data),
            ArtFormat::Png => Self::from_png(&data),
        }
        .map_err(|source| ArtError::File {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
    }

    /// Parses art held in memory, such as an embedded copy, telling a PNG from an SVG by its
    /// signature.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ArtError> {
        if data.starts_with(b"\x89PNG") {
            Self::from_png(data)
        } else {
            Self::from_svg(data)
        }
    }

    pub fn from_svg(data: &[u8]) -> Result<Self, ArtError> {
        let tree = usvg::Tree::from_data(data, &usvg::Options::default())?;
        Ok(Art::Svg(Box::new(tree)))
    }

    pub fn from_png(data: &[u8]) -> Result<Self, ArtError> {
        let image =
            image::load_from_memory_with_format(data, image::ImageFormat::Png)?.into_rgba8();
        let (width, height) = image.dimensions();
        let mut pixels = image.into_raw();
        for px in pixels.chunks_exact_mut(4) {
            let alpha = u16::from(px[3]);
            for channel in &mut px[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        Ok(Art::Png(Raster {
            width,
            height,
            pixels,
        }))
    }

    pub fn format(&self) -> ArtFormat {
        match self {
            Art::Svg(_) => ArtFormat::Svg,
            Art::Png(_) => ArtFormat::Png,
        }
    }

    /// The art's own size: an SVG's width and height, or a PNG's pixels.
    pub fn size(&self) -> [f32; 2] {
        match self {
            Art::Svg(tree) => [tree.size().width(), tree.size().height()],
            Art::Png(raster) => [raster.width as f32, raster.height as f32],
        }
    }

    /// Rasterises an SVG `width` pixels wide, with the height following its aspect ratio,
    /// both capped at [`MAX_RASTER_SIZE`]. A PNG ignores `width` and keeps its own pixels.
    pub fn rasterise(&self, width: u32) -> Raster {
        let tree = match self {
            Art::Png(raster) => return raster.clone(),
            Art::Svg(tree) => tree,
        };
        let [art_width, art_height] = self.size();
        let mut width = width.clamp(1, MAX_RASTER_SIZE);
        let mut height = ((width as f32 * art_height / art_width).round() as u32).max(1);
        if height > MAX_RASTER_SIZE {
            width = ((width as f32 * MAX_RASTER_SIZE as f32 / height as f32).round() as u32).max(1);
            height = MAX_RASTER_SIZE;
        }
        let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("a non-empty size");
        let transform =
            tiny_skia::Transform::from_scale(width as f32 / art_width, height as f32 / art_height);
        resvg::render(tree, transform, &mut pixmap.as_mut());
        Raster {
            width,
            height,
            pixels: pixmap.take(),
        }
    }

    /// Rasterises so the longer side is `size` pixels. A PNG keeps its own pixels.
    pub fn rasterise_fit(&self, size: u32) -> Raster {
        let [width, height] = self.size();
        let scale = size as f32 / width.max(height);
        self.rasterise((width * scale).round() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RING: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
        <rect x="1" y="1" width="18" height="8" fill="none" stroke="black" stroke-width="2"/>
    </svg>"#;

    #[test]
    fn svgs_rasterise_at_the_requested_width() {
        let art = Art::from_svg(RING.as_bytes()).unwrap();
        assert_eq!(art.size(), [20.0, 10.0]);
        let raster = art.rasterise(200);
        assert_eq!([raster.width, raster.height], [200, 100]);
        assert_eq!(raster.pixel(5, 50), [0, 0, 0, 255], "the stroke is ink");
        assert_eq!(raster.pixel(100, 50), [0, 0, 0, 0], "the inside is empty");
        let fitted = art.rasterise_fit(40);
        assert_eq!([fitted.width, fitted.height], [40, 20]);
    }

    #[test]
    fn huge_rasters_are_capped() {
        let art = Art::from_svg(RING.as_bytes()).unwrap();
        let raster = art.rasterise(u32::MAX);
        assert_eq!(
            [raster.width, raster.height],
            [MAX_RASTER_SIZE, MAX_RASTER_SIZE / 2]
        );
    }

    #[test]
    fn pngs_keep_their_pixels_and_are_premultiplied() {
        let mut png = Vec::new();
        image::write_buffer_with_format(
            &mut std::io::Cursor::new(&mut png),
            &[200, 100, 50, 128, 10, 20, 30, 255],
            2,
            1,
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        let art = Art::from_png(&png).unwrap();
        let raster = art.rasterise(1000);
        assert_eq!([raster.width, raster.height], [2, 1]);
        assert_eq!(raster.pixel(0, 0), [100, 50, 25, 128]);
        assert_eq!(raster.pixel(1, 0), [10, 20, 30, 255]);
        assert_eq!(&raster.unpremultiplied()[..4], &[199, 100, 50, 128]);
    }

    #[test]
    fn bad_files_name_themselves() {
        let error = Art::load(Path::new("nowhere/ghost.svg")).err().unwrap();
        assert!(
            error
                .to_string()
                .starts_with("nowhere/ghost.svg: cannot be read")
        );
        assert!(Art::from_svg(b"<not svg").is_err());
    }

    #[test]
    fn errors_keep_the_underlying_cause() {
        use std::error::Error;
        let svg = Art::from_svg(b"<not svg").err().unwrap();
        assert!(matches!(svg, ArtError::Svg(_)));
        assert!(svg.source().is_some());
        let png = Art::from_png(b"not a png").err().unwrap();
        assert!(matches!(png, ArtError::Png(_)));
        assert!(png.source().is_some());
        let io = Art::load(Path::new("nowhere/ghost.png")).err().unwrap();
        assert!(matches!(io, ArtError::Io { .. }));
        assert!(io.source().is_some());
    }
}
