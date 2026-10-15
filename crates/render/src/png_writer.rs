//! Streaming PNG writer for export (ADR 0007): 8-bit sRGB RGBA, one row at a time.
//!
//! The image is written to a temporary file beside the destination and renamed on
//! [`PngWriter::finish`], so dropping the writer (Cancel, or any error) leaves no partial file.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

const METRES_PER_INCH: f64 = 0.0254;

#[derive(Debug, Error)]
pub enum PngWriteError {
    #[error("could not write the PNG: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not encode the PNG: {0}")]
    Encode(#[from] png::EncodingError),
    #[error("a row must be {expected} bytes (width x 4), got {got}")]
    RowLength { expected: usize, got: usize },
    #[error("the image has {expected} rows, but {got} were written")]
    RowCount { expected: u32, got: u32 },
}

/// What to embed besides the pixels.
#[derive(Debug, Clone, Default)]
pub struct PngMeta {
    /// Print density; written as `pHYs` in pixels per metre.
    pub dpi: Option<u32>,
    /// Text chunks (keyword, text), e.g. `Software` and credits.
    pub text: Vec<(String, String)>,
}

impl PngMeta {
    /// Adds the app name and version as the `Software` text chunk.
    pub fn with_software(mut self, name: &str, version: &str) -> Self {
        self.text
            .push(("Software".to_owned(), format!("{name} {version}")));
        self
    }

    /// Adds the credits as a text chunk under `keyword`.
    pub fn with_credits(mut self, keyword: &str, credits: &str) -> Self {
        self.text.push((keyword.to_owned(), credits.to_owned()));
        self
    }
}

pub struct PngWriter {
    stream: Option<png::StreamWriter<'static, BufWriter<File>>>,
    temp: PathBuf,
    dest: PathBuf,
    width: u32,
    height: u32,
    rows: u32,
    done: bool,
}

impl PngWriter {
    pub fn create(
        dest: &Path,
        width: u32,
        height: u32,
        meta: &PngMeta,
    ) -> Result<Self, PngWriteError> {
        let mut name = dest.file_name().unwrap_or_default().to_os_string();
        name.push(".part");
        let temp = dest.with_file_name(name);
        let file = File::create(&temp)?;
        // From here on, Drop removes the temp file.
        let mut writer = Self {
            stream: None,
            temp,
            dest: dest.to_owned(),
            width,
            height,
            rows: 0,
            done: false,
        };
        let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        if let Some(dpi) = meta.dpi {
            let ppu = (f64::from(dpi) / METRES_PER_INCH).round() as u32;
            encoder.set_pixel_dims(Some(png::PixelDimensions {
                xppu: ppu,
                yppu: ppu,
                unit: png::Unit::Meter,
            }));
        }
        for (key, text) in &meta.text {
            encoder.add_text_chunk(key.clone(), text.clone())?;
        }
        writer.stream = Some(encoder.write_header()?.into_stream_writer()?);
        Ok(writer)
    }

    /// Appends one row of `width * 4` RGBA bytes.
    pub fn write_row(&mut self, row: &[u8]) -> Result<(), PngWriteError> {
        let expected = self.width as usize * 4;
        if row.len() != expected {
            return Err(PngWriteError::RowLength {
                expected,
                got: row.len(),
            });
        }
        if self.rows >= self.height {
            return Err(PngWriteError::RowCount {
                expected: self.height,
                got: self.rows + 1,
            });
        }
        let stream = self.stream.as_mut().expect("stream is open until finish");
        stream.write_all(row)?;
        self.rows += 1;
        Ok(())
    }

    /// Completes the image and moves it into place. Fails if rows are missing.
    pub fn finish(mut self) -> Result<(), PngWriteError> {
        if self.rows != self.height {
            return Err(PngWriteError::RowCount {
                expected: self.height,
                got: self.rows,
            });
        }
        let stream = self.stream.take().expect("stream is open until finish");
        stream.finish()?;
        std::fs::rename(&self.temp, &self.dest)?;
        self.done = true;
        Ok(())
    }
}

impl Drop for PngWriter {
    fn drop(&mut self) {
        if !self.done {
            self.stream = None;
            let _ = std::fs::remove_file(&self.temp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(y: u32, width: u32) -> Vec<u8> {
        (0..width)
            .flat_map(|x| [(x % 251) as u8, (y % 241) as u8, 7, 255])
            .collect()
    }

    fn decode(path: &Path) -> (png::OutputInfo, Vec<u8>, png::Info<'static>) {
        let mut reader = png::Decoder::new(std::io::BufReader::new(File::open(path).unwrap()))
            .read_info()
            .unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        (info, buf, reader.info().clone())
    }

    #[test]
    fn round_trips_pixels_and_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.png");
        let meta = PngMeta {
            dpi: Some(300),
            ..PngMeta::default()
        }
        .with_software("Star Script Map Maker", "1.2.3");
        let mut w = PngWriter::create(&path, 5, 4, &meta).unwrap();
        for y in 0..4 {
            w.write_row(&row(y, 5)).unwrap();
        }
        w.finish().unwrap();
        assert!(!dir.path().join("out.png.part").exists());

        let (frame, pixels, info) = decode(&path);
        assert_eq!((frame.width, frame.height), (5, 4));
        assert_eq!(frame.color_type, png::ColorType::Rgba);
        let expected: Vec<u8> = (0..4).flat_map(|y| row(y, 5)).collect();
        assert_eq!(pixels, expected);
        let dims = info.pixel_dims.unwrap();
        assert_eq!((dims.xppu, dims.unit), (11811, png::Unit::Meter));
        assert!(
            info.uncompressed_latin1_text
                .iter()
                .any(|t| t.keyword == "Software" && t.text == "Star Script Map Maker 1.2.3")
        );
    }

    #[test]
    fn credits_are_written_as_a_text_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.png");
        let meta = PngMeta::default().with_credits("Credits", "Sword by Lorc\nShield by Lorc\n");
        let mut w = PngWriter::create(&path, 1, 1, &meta).unwrap();
        w.write_row(&[1, 2, 3, 4]).unwrap();
        w.finish().unwrap();
        let (.., info) = decode(&path);
        assert!(
            info.uncompressed_latin1_text
                .iter()
                .any(|t| t.keyword == "Credits" && t.text == "Sword by Lorc\nShield by Lorc\n")
        );
    }

    #[test]
    fn no_phys_chunk_without_dpi() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.png");
        let mut w = PngWriter::create(&path, 1, 1, &PngMeta::default()).unwrap();
        w.write_row(&[1, 2, 3, 4]).unwrap();
        w.finish().unwrap();
        assert!(decode(&path).2.pixel_dims.is_none());
    }

    #[test]
    fn dropping_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.png");
        let mut w = PngWriter::create(&path, 3, 3, &PngMeta::default()).unwrap();
        w.write_row(&row(0, 3)).unwrap();
        drop(w);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn short_image_is_an_error_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.png");
        let mut w = PngWriter::create(&path, 3, 3, &PngMeta::default()).unwrap();
        w.write_row(&row(0, 3)).unwrap();
        assert!(matches!(w.finish(), Err(PngWriteError::RowCount { .. })));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn rejects_wrong_row_length() {
        let dir = tempfile::tempdir().unwrap();
        let mut w =
            PngWriter::create(&dir.path().join("o.png"), 3, 1, &PngMeta::default()).unwrap();
        assert!(matches!(
            w.write_row(&[0; 4]),
            Err(PngWriteError::RowLength { .. })
        ));
    }

    /// 16k wide, streamed from a synthetic row source; the writer never holds more than a row.
    #[test]
    fn sixteen_k_wide_streams_and_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        let (width, height) = (16_384, 64);
        let mut w = PngWriter::create(&path, width, height, &PngMeta::default()).unwrap();
        let mut buf = vec![0u8; width as usize * 4];
        for y in 0..height {
            for (x, px) in buf.chunks_exact_mut(4).enumerate() {
                px.copy_from_slice(&[(x % 256) as u8, y as u8, 0, 255]);
            }
            w.write_row(&buf).unwrap();
        }
        w.finish().unwrap();
        let (frame, pixels, _) = decode(&path);
        assert_eq!((frame.width, frame.height), (width, height));
        let last = &pixels[(height as usize - 1) * width as usize * 4..][..4];
        assert_eq!(last, &[0, 63, 0, 255]);
    }
}
