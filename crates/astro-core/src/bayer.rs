//! Bayer demosaicing for one-shot color (OSC) cameras.
//!
//! Supports RGGB, BGGR, GRBG, and GBRG patterns with bilinear interpolation.

use crate::error::{AstroError, Result};
use crate::image::{FitsHeader, ImageData};

/// The four standard Bayer color filter array patterns.
///
/// The pattern name describes the 2×2 superpixel layout:
/// ```text
///   RGGB:     BGGR:     GRBG:     GBRG:
///   R G       B G       G R       G B
///   G B       G R       B G       R G
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BayerPattern {
    RGGB,
    BGGR,
    GRBG,
    GBRG,
}

impl BayerPattern {
    /// Parse from a string (e.g., from FITS BAYERPAT header).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_uppercase().as_str() {
            "RGGB" => Some(BayerPattern::RGGB),
            "BGGR" => Some(BayerPattern::BGGR),
            "GRBG" => Some(BayerPattern::GRBG),
            "GBRG" => Some(BayerPattern::GBRG),
            _ => None,
        }
    }

    /// Detect the Bayer pattern from FITS headers.
    pub fn from_headers(headers: &[FitsHeader]) -> Option<Self> {
        for h in headers {
            if h.keyword == "BAYERPAT" || h.keyword == "COLORTYP" {
                if let Some(s) = h.value.as_str() {
                    return Self::from_str(s);
                }
            }
        }
        None
    }

    /// Returns (r_row, r_col) offset of the red pixel in the 2×2 superpixel.
    fn red_offset(self) -> (usize, usize) {
        match self {
            BayerPattern::RGGB => (0, 0),
            BayerPattern::BGGR => (1, 1),
            BayerPattern::GRBG => (0, 1),
            BayerPattern::GBRG => (1, 0),
        }
    }

    /// Returns (b_row, b_col) offset of the blue pixel in the 2×2 superpixel.
    fn blue_offset(self) -> (usize, usize) {
        match self {
            BayerPattern::RGGB => (1, 1),
            BayerPattern::BGGR => (0, 0),
            BayerPattern::GRBG => (1, 0),
            BayerPattern::GBRG => (0, 1),
        }
    }

    /// Returns which color a given pixel position belongs to.
    /// 0 = Red, 1 = Green, 2 = Blue.
    fn color_at(self, row: usize, col: usize) -> usize {
        let (rr, rc) = self.red_offset();
        let (br, bc) = self.blue_offset();
        let pr = row % 2;
        let pc = col % 2;
        if pr == rr && pc == rc {
            0 // Red
        } else if pr == br && pc == bc {
            2 // Blue
        } else {
            1 // Green
        }
    }
}

/// Demosaic a raw Bayer-patterned mono image into a 3-channel RGB image
/// using bilinear interpolation.
///
/// The input must be a single-channel image. The output will be a 3-channel
/// image with the same dimensions.
pub fn demosaic(image: &ImageData, pattern: BayerPattern) -> Result<ImageData> {
    if image.channels != 1 {
        return Err(AstroError::InvalidDimensions {
            expected: "1 channel (raw Bayer data)".into(),
            got: format!("{} channels", image.channels),
        });
    }

    let w = image.width;
    let h = image.height;
    let mut rgb = ImageData::new(w, h, 3);

    for row in 0..h {
        for col in 0..w {
            let color = pattern.color_at(row, col);
            let raw_val = image.get(col, row, 0);

            match color {
                0 => {
                    // This pixel is Red — interpolate G and B
                    rgb.set(col, row, 0, raw_val);
                    rgb.set(col, row, 1, interp_green_at_rb(image, col, row, w, h));
                    rgb.set(col, row, 2, interp_opposite_at_rb(image, col, row, w, h, pattern, 2));
                }
                1 => {
                    // This pixel is Green — interpolate R and B
                    rgb.set(col, row, 0, interp_rb_at_green(image, col, row, w, h, pattern, 0));
                    rgb.set(col, row, 1, raw_val);
                    rgb.set(col, row, 2, interp_rb_at_green(image, col, row, w, h, pattern, 2));
                }
                2 => {
                    // This pixel is Blue — interpolate R and G
                    rgb.set(col, row, 0, interp_opposite_at_rb(image, col, row, w, h, pattern, 0));
                    rgb.set(col, row, 1, interp_green_at_rb(image, col, row, w, h));
                    rgb.set(col, row, 2, raw_val);
                }
                _ => unreachable!(),
            }
        }
    }

    Ok(rgb)
}

/// Safely get a pixel value with bounds checking, returning 0.0 for out-of-bounds.
#[inline]
fn safe_get(image: &ImageData, col: isize, row: isize, w: usize, h: usize) -> f32 {
    if col >= 0 && col < w as isize && row >= 0 && row < h as isize {
        image.get(col as usize, row as usize, 0)
    } else {
        0.0
    }
}

/// Count valid neighbors for averaging (for border handling).
#[inline]
fn safe_count(col: isize, row: isize, w: usize, h: usize) -> f32 {
    if col >= 0 && col < w as isize && row >= 0 && row < h as isize {
        1.0
    } else {
        0.0
    }
}

/// Interpolate green at a red or blue pixel position.
/// Uses 4-neighbor average (up, down, left, right).
fn interp_green_at_rb(image: &ImageData, col: usize, row: usize, w: usize, h: usize) -> f32 {
    let c = col as isize;
    let r = row as isize;
    let sum = safe_get(image, c - 1, r, w, h)
        + safe_get(image, c + 1, r, w, h)
        + safe_get(image, c, r - 1, w, h)
        + safe_get(image, c, r + 1, w, h);
    let count = safe_count(c - 1, r, w, h)
        + safe_count(c + 1, r, w, h)
        + safe_count(c, r - 1, w, h)
        + safe_count(c, r + 1, w, h);
    if count > 0.0 {
        sum / count
    } else {
        0.0
    }
}

/// Interpolate the opposite color (R at B, or B at R) using diagonal neighbors.
fn interp_opposite_at_rb(
    image: &ImageData,
    col: usize,
    row: usize,
    w: usize,
    h: usize,
    _pattern: BayerPattern,
    _target_color: usize,
) -> f32 {
    let c = col as isize;
    let r = row as isize;
    // Diagonal neighbors contain the opposite color
    let sum = safe_get(image, c - 1, r - 1, w, h)
        + safe_get(image, c + 1, r - 1, w, h)
        + safe_get(image, c - 1, r + 1, w, h)
        + safe_get(image, c + 1, r + 1, w, h);
    let count = safe_count(c - 1, r - 1, w, h)
        + safe_count(c + 1, r - 1, w, h)
        + safe_count(c - 1, r + 1, w, h)
        + safe_count(c + 1, r + 1, w, h);
    if count > 0.0 {
        sum / count
    } else {
        0.0
    }
}

/// Interpolate red or blue at a green pixel position.
/// Green pixels are adjacent to both red and blue — need to pick the right axis.
fn interp_rb_at_green(
    image: &ImageData,
    col: usize,
    row: usize,
    w: usize,
    h: usize,
    pattern: BayerPattern,
    target_color: usize,
) -> f32 {
    let c = col as isize;
    let r = row as isize;

    // Determine which axis has the target color neighbors.
    // For a green pixel, the target color is on either the horizontal or vertical axis.
    let (rr, rc) = if target_color == 0 {
        pattern.red_offset()
    } else {
        pattern.blue_offset()
    };

    // The target color pixels are on the axis where the offset matches
    let row_parity = row % 2;
    let col_parity = col % 2;

    if row_parity == rr {
        // Same row as target color → target is to left and right
        let sum = safe_get(image, c - 1, r, w, h) + safe_get(image, c + 1, r, w, h);
        let count = safe_count(c - 1, r, w, h) + safe_count(c + 1, r, w, h);
        if count > 0.0 {
            sum / count
        } else {
            0.0
        }
    } else if col_parity == rc {
        // Same column as target color → target is above and below
        let sum = safe_get(image, c, r - 1, w, h) + safe_get(image, c, r + 1, w, h);
        let count = safe_count(c, r - 1, w, h) + safe_count(c, r + 1, w, h);
        if count > 0.0 {
            sum / count
        } else {
            0.0
        }
    } else {
        // Fallback: average all 4 cardinal neighbors
        let sum = safe_get(image, c - 1, r, w, h)
            + safe_get(image, c + 1, r, w, h)
            + safe_get(image, c, r - 1, w, h)
            + safe_get(image, c, r + 1, w, h);
        let count = safe_count(c - 1, r, w, h)
            + safe_count(c + 1, r, w, h)
            + safe_count(c, r - 1, w, h)
            + safe_count(c, r + 1, w, h);
        if count > 0.0 {
            sum / count
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bayer_pattern_from_str() {
        assert_eq!(BayerPattern::from_str("RGGB"), Some(BayerPattern::RGGB));
        assert_eq!(BayerPattern::from_str("bggr"), Some(BayerPattern::BGGR));
        assert_eq!(BayerPattern::from_str("GRBG"), Some(BayerPattern::GRBG));
        assert_eq!(BayerPattern::from_str("GBRG"), Some(BayerPattern::GBRG));
        assert_eq!(BayerPattern::from_str("XXXX"), None);
    }

    #[test]
    fn test_color_at_rggb() {
        let p = BayerPattern::RGGB;
        assert_eq!(p.color_at(0, 0), 0); // R
        assert_eq!(p.color_at(0, 1), 1); // G
        assert_eq!(p.color_at(1, 0), 1); // G
        assert_eq!(p.color_at(1, 1), 2); // B
    }

    #[test]
    fn test_demosaic_dimensions() {
        let img = ImageData::new(100, 100, 1);
        let rgb = demosaic(&img, BayerPattern::RGGB).unwrap();
        assert_eq!(rgb.width, 100);
        assert_eq!(rgb.height, 100);
        assert_eq!(rgb.channels, 3);
    }

    #[test]
    fn test_demosaic_rejects_multichannel() {
        let img = ImageData::new(100, 100, 3);
        assert!(demosaic(&img, BayerPattern::RGGB).is_err());
    }
}
