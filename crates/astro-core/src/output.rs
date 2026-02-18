//! Output/export functionality — save processed images as FITS, TIFF, or PNG.

use crate::error::{AstroError, Result};
use crate::fits_io;
use crate::histogram::{self, StretchParams};
use crate::image::{FitsValue, ImageData};
use image::{ImageBuffer, Rgb, Luma};
use std::collections::HashMap;
use std::path::Path;

/// Supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// FITS file with 32-bit floating point data.
    Fits,
    /// TIFF file with 16-bit unsigned integer data.
    Tiff,
    /// PNG file with 8-bit unsigned integer data (stretch applied).
    Png,
}

impl OutputFormat {
    /// Get the file extension for this format.
    pub fn extension(&self) -> &str {
        match self {
            OutputFormat::Fits => "fits",
            OutputFormat::Tiff => "tiff",
            OutputFormat::Png => "png",
        }
    }

    /// Detect format from file extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| match ext.to_lowercase().as_str() {
                "fits" | "fit" | "fts" => Some(OutputFormat::Fits),
                "tiff" | "tif" => Some(OutputFormat::Tiff),
                "png" => Some(OutputFormat::Png),
                _ => None,
            })
    }
}

/// Save an image to disk in the specified format.
///
/// For PNG output, an auto-stretch is applied if no stretch params are provided.
/// FITS and TIFF preserve the full dynamic range of the data.
pub fn save_image(
    path: &Path,
    image: &ImageData,
    format: OutputFormat,
    stretch: Option<&StretchParams>,
) -> Result<()> {
    match format {
        OutputFormat::Fits => save_fits(path, image),
        OutputFormat::Tiff => save_tiff(path, image),
        OutputFormat::Png => save_png(path, image, stretch),
    }
}

/// Save as FITS (32-bit float).
pub fn save_fits(path: &Path, image: &ImageData) -> Result<()> {
    log::info!("Saving FITS: {}", path.display());

    let mut headers = HashMap::new();
    headers.insert(
        "CREATOR".to_string(),
        FitsValue::String("sidera".to_string()),
    );

    fits_io::write_fits(path, image, &headers)
}

/// Save as 16-bit TIFF.
pub fn save_tiff(path: &Path, image: &ImageData) -> Result<()> {
    log::info!("Saving TIFF: {}", path.display());

    let (min, max) = image.min_max();
    let range = max - min;

    if image.channels == 1 {
        let mut buf: ImageBuffer<Luma<u16>, Vec<u16>> =
            ImageBuffer::new(image.width as u32, image.height as u32);

        for y in 0..image.height {
            for x in 0..image.width {
                let v = image.get(x, y, 0);
                let normalized = if range > 0.0 {
                    (v - min) / range
                } else {
                    0.0
                };
                let u16_val = (normalized * 65535.0).clamp(0.0, 65535.0) as u16;
                buf.put_pixel(x as u32, y as u32, Luma([u16_val]));
            }
        }

        buf.save(path)
            .map_err(|e| AstroError::Encoding(format!("Failed to save TIFF: {e}")))?;
    } else if image.channels >= 3 {
        let mut buf: ImageBuffer<Rgb<u16>, Vec<u16>> =
            ImageBuffer::new(image.width as u32, image.height as u32);

        for y in 0..image.height {
            for x in 0..image.width {
                let r = image.get(x, y, 0);
                let g = image.get(x, y, 1);
                let b = image.get(x, y, 2);

                let scale = |v: f32| -> u16 {
                    let normalized = if range > 0.0 {
                        (v - min) / range
                    } else {
                        0.0
                    };
                    (normalized * 65535.0).clamp(0.0, 65535.0) as u16
                };

                buf.put_pixel(x as u32, y as u32, Rgb([scale(r), scale(g), scale(b)]));
            }
        }

        buf.save(path)
            .map_err(|e| AstroError::Encoding(format!("Failed to save TIFF: {e}")))?;
    } else {
        return Err(AstroError::Encoding(format!(
            "Unsupported channel count for TIFF: {}",
            image.channels
        )));
    }

    Ok(())
}

/// Save as 8-bit PNG with stretch applied.
pub fn save_png(path: &Path, image: &ImageData, stretch: Option<&StretchParams>) -> Result<()> {
    log::info!("Saving PNG: {}", path.display());

    // Apply stretch (auto if not provided)
    let params = stretch.cloned().unwrap_or_else(|| histogram::auto_stretch(image));
    let stretched = histogram::apply_stretch(image, &params);

    if stretched.channels == 1 {
        let mut buf: ImageBuffer<Luma<u8>, Vec<u8>> =
            ImageBuffer::new(stretched.width as u32, stretched.height as u32);

        for y in 0..stretched.height {
            for x in 0..stretched.width {
                let v = (stretched.get(x, y, 0) * 255.0).clamp(0.0, 255.0) as u8;
                buf.put_pixel(x as u32, y as u32, Luma([v]));
            }
        }

        buf.save(path)
            .map_err(|e| AstroError::Encoding(format!("Failed to save PNG: {e}")))?;
    } else if stretched.channels >= 3 {
        let mut buf: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::new(stretched.width as u32, stretched.height as u32);

        for y in 0..stretched.height {
            for x in 0..stretched.width {
                let r = (stretched.get(x, y, 0) * 255.0).clamp(0.0, 255.0) as u8;
                let g = (stretched.get(x, y, 1) * 255.0).clamp(0.0, 255.0) as u8;
                let b = (stretched.get(x, y, 2) * 255.0).clamp(0.0, 255.0) as u8;
                buf.put_pixel(x as u32, y as u32, Rgb([r, g, b]));
            }
        }

        buf.save(path)
            .map_err(|e| AstroError::Encoding(format!("Failed to save PNG: {e}")))?;
    } else {
        return Err(AstroError::Encoding(format!(
            "Unsupported channel count for PNG: {}",
            stretched.channels
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_from_path() {
        assert_eq!(
            OutputFormat::from_path(Path::new("test.fits")),
            Some(OutputFormat::Fits)
        );
        assert_eq!(
            OutputFormat::from_path(Path::new("test.tiff")),
            Some(OutputFormat::Tiff)
        );
        assert_eq!(
            OutputFormat::from_path(Path::new("test.png")),
            Some(OutputFormat::Png)
        );
        assert_eq!(
            OutputFormat::from_path(Path::new("test.fit")),
            Some(OutputFormat::Fits)
        );
        assert_eq!(OutputFormat::from_path(Path::new("test.jpg")), None);
    }

    #[test]
    fn test_output_format_extension() {
        assert_eq!(OutputFormat::Fits.extension(), "fits");
        assert_eq!(OutputFormat::Tiff.extension(), "tiff");
        assert_eq!(OutputFormat::Png.extension(), "png");
    }
}
