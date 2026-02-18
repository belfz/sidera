//! Calibration pipeline for astrophotography.
//!
//! Implements the standard workflow:
//! 1. Master Bias = median stack of bias frames
//! 2. Master Dark = median stack of dark frames − Master Bias
//! 3. Master Flat = median stack of flat frames − Master Bias, normalized to mean=1.0
//! 4. Calibrated Light = (Light − Master Bias − Master Dark) / Master Flat

use crate::bayer::BayerPattern;
use crate::error::{AstroError, Result};
use crate::image::ImageData;
use crate::stacking::{self, StackMethod};
use rayon::prelude::*;

/// Holds the master calibration frames for the pipeline.
#[derive(Debug, Clone)]
pub struct CalibrationFrames {
    pub master_bias: Option<ImageData>,
    pub master_dark: Option<ImageData>,
    pub master_flat: Option<ImageData>,
}

impl CalibrationFrames {
    pub fn new() -> Self {
        CalibrationFrames {
            master_bias: None,
            master_dark: None,
            master_flat: None,
        }
    }
}

impl Default for CalibrationFrames {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a master bias frame by median-stacking the provided bias frames.
///
/// Bias frames should be zero-length exposures that capture the sensor's
/// read noise pattern.
pub fn create_master_bias(frames: &[ImageData]) -> Result<ImageData> {
    if frames.is_empty() {
        return Err(AstroError::NoFrames {
            operation: "master bias creation".into(),
        });
    }

    validate_same_dimensions(frames, "bias")?;

    log::info!("Creating master bias from {} frames", frames.len());
    stacking::stack_images(frames, StackMethod::Median)
}

/// Create a master dark frame by median-stacking the provided dark frames,
/// then subtracting the master bias (if provided).
///
/// Dark frames capture thermal noise and should be taken at the same
/// temperature and exposure time as the light frames.
pub fn create_master_dark(
    frames: &[ImageData],
    master_bias: Option<&ImageData>,
) -> Result<ImageData> {
    if frames.is_empty() {
        return Err(AstroError::NoFrames {
            operation: "master dark creation".into(),
        });
    }

    validate_same_dimensions(frames, "dark")?;

    log::info!("Creating master dark from {} frames", frames.len());

    // If we have a master bias, subtract it from each dark frame first
    let calibrated_frames: Vec<ImageData> = if let Some(bias) = master_bias {
        frames
            .iter()
            .map(|frame| subtract_images(frame, bias))
            .collect::<Result<Vec<_>>>()?
    } else {
        frames.to_vec()
    };

    stacking::stack_images(&calibrated_frames, StackMethod::Median)
}

/// Create a master flat frame by median-stacking the provided flat frames,
/// subtracting the master bias, and normalizing to mean = 1.0.
///
/// Flat frames capture vignetting, dust, and optical imperfections.
/// They should be taken through the same optical path as the lights.
///
/// If a Bayer pattern is provided and the flat is mono, normalization is done
/// per Bayer sub-channel (R, G, B independently) to avoid distorting the
/// sensor's color response. Without this, dividing a Bayer light by a
/// globally-normalized flat systematically boosts R and B while suppressing G,
/// producing a purple/magenta cast.
pub fn create_master_flat(
    frames: &[ImageData],
    master_bias: Option<&ImageData>,
    bayer_pattern: Option<BayerPattern>,
) -> Result<ImageData> {
    if frames.is_empty() {
        return Err(AstroError::NoFrames {
            operation: "master flat creation".into(),
        });
    }

    validate_same_dimensions(frames, "flat")?;

    log::info!("Creating master flat from {} frames", frames.len());

    // Subtract bias from each flat
    let calibrated_frames: Vec<ImageData> = if let Some(bias) = master_bias {
        frames
            .iter()
            .map(|frame| subtract_images(frame, bias))
            .collect::<Result<Vec<_>>>()?
    } else {
        frames.to_vec()
    };

    let mut master = stacking::stack_images(&calibrated_frames, StackMethod::Median)?;

    // Check that the flat has meaningful signal after bias subtraction.
    // If the mean is near zero, the bias may be incorrect (e.g., the user
    // loaded flat frames as biases), which would produce an all-zero flat
    // and destroy every light frame when divided.
    let global_mean: f64 = master.data.iter().map(|&v| v as f64).sum::<f64>()
        / master.data.len().max(1) as f64;
    if global_mean.abs() < 1.0 {
        log::warn!(
            "Master flat has near-zero mean ({:.2}) after bias subtraction — \
             bias frames may be incorrect. Rebuilding flat without bias subtraction.",
            global_mean
        );
        master = stacking::stack_images(frames, StackMethod::Median)?;
    }

    if master.channels == 1 {
        if let Some(pattern) = bayer_pattern {
            normalize_flat_bayer(&mut master, pattern);
        } else {
            normalize_flat(&mut master);
        }
    } else {
        normalize_flat(&mut master);
    }

    Ok(master)
}

/// Calibrate a single light frame using the master calibration frames.
///
/// Formula: calibrated = (light − master_bias − master_dark) / master_flat
///
/// Any missing master frames are simply skipped in the pipeline.
/// If a calibration frame is mono (1-channel) but the light is RGB (3-channel),
/// the mono frame is automatically broadcast across all channels.
pub fn calibrate_light(light: &ImageData, cal: &CalibrationFrames) -> Result<ImageData> {
    let mut result = light.clone();

    // Subtract master bias
    if let Some(ref bias) = cal.master_bias {
        let bias = match_channels(bias, result.channels)?;
        result = subtract_images(&result, &bias)?;
    }

    // Subtract master dark
    if let Some(ref dark) = cal.master_dark {
        let dark = match_channels(dark, result.channels)?;
        result = subtract_images(&result, &dark)?;
    }

    // Divide by master flat
    if let Some(ref flat) = cal.master_flat {
        let flat = match_channels(flat, result.channels)?;
        result = divide_images(&result, &flat)?;
    }

    Ok(result)
}

/// If the calibration frame is mono (1-channel) but we need `target_channels`,
/// broadcast the mono data across all channels. If dimensions already match,
/// return the frame unchanged. This handles the common case where calibration
/// frames (darks, biases) are mono but the lights are RGB.
fn match_channels(frame: &ImageData, target_channels: usize) -> Result<ImageData> {
    if frame.channels == target_channels {
        return Ok(frame.clone());
    }
    if frame.channels == 1 && target_channels > 1 {
        log::info!(
            "Broadcasting mono calibration frame to {} channels",
            target_channels
        );
        let mut result = ImageData::new(frame.width, frame.height, target_channels);
        for i in 0..frame.pixel_count() {
            let v = frame.data[i];
            for c in 0..target_channels {
                result.data[i * target_channels + c] = v;
            }
        }
        return Ok(result);
    }
    // If the calibration frame has MORE channels than the light, extract luminance
    if frame.channels == 3 && target_channels == 1 {
        log::info!("Converting RGB calibration frame to mono for mono light");
        return frame.to_luminance();
    }
    Err(AstroError::DimensionMismatch(format!(
        "Cannot adapt {}ch calibration frame for {}ch light",
        frame.channels, target_channels
    )))
}

/// Calibrate multiple light frames in parallel.
pub fn calibrate_lights(
    lights: &[ImageData],
    cal: &CalibrationFrames,
) -> Result<Vec<ImageData>> {
    if lights.is_empty() {
        return Err(AstroError::NoFrames {
            operation: "light calibration".into(),
        });
    }

    log::info!("Calibrating {} light frames", lights.len());

    lights
        .par_iter()
        .map(|light| calibrate_light(light, cal))
        .collect()
}

// ─── Arithmetic operations ──────────────────────────────────────────────────

/// Subtract image B from image A (pixel-wise): result = A - B.
pub fn subtract_images(a: &ImageData, b: &ImageData) -> Result<ImageData> {
    if !a.same_dimensions(b) {
        return Err(AstroError::DimensionMismatch(format!(
            "Cannot subtract {}x{}x{} from {}x{}x{}",
            b.width, b.height, b.channels, a.width, a.height, a.channels
        )));
    }

    let data: Vec<f32> = a
        .data
        .par_iter()
        .zip(b.data.par_iter())
        .map(|(&av, &bv)| av - bv)
        .collect();

    Ok(ImageData {
        width: a.width,
        height: a.height,
        channels: a.channels,
        data,
    })
}

/// Divide image A by image B (pixel-wise): result = A / B.
/// Pixels where B is near zero are set to 0.0 to avoid infinities.
pub fn divide_images(a: &ImageData, b: &ImageData) -> Result<ImageData> {
    if !a.same_dimensions(b) {
        return Err(AstroError::DimensionMismatch(format!(
            "Cannot divide {}x{}x{} by {}x{}x{}",
            a.width, a.height, a.channels, b.width, b.height, b.channels
        )));
    }

    let epsilon = 1e-10;
    let data: Vec<f32> = a
        .data
        .par_iter()
        .zip(b.data.par_iter())
        .map(|(&av, &bv)| {
            if bv.abs() < epsilon {
                0.0
            } else {
                av / bv
            }
        })
        .collect();

    Ok(ImageData {
        width: a.width,
        height: a.height,
        channels: a.channels,
        data,
    })
}

/// Add image B to image A (pixel-wise): result = A + B.
pub fn add_images(a: &ImageData, b: &ImageData) -> Result<ImageData> {
    if !a.same_dimensions(b) {
        return Err(AstroError::DimensionMismatch(format!(
            "Cannot add {}x{}x{} to {}x{}x{}",
            b.width, b.height, b.channels, a.width, a.height, a.channels
        )));
    }

    let data: Vec<f32> = a
        .data
        .par_iter()
        .zip(b.data.par_iter())
        .map(|(&av, &bv)| av + bv)
        .collect();

    Ok(ImageData {
        width: a.width,
        height: a.height,
        channels: a.channels,
        data,
    })
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Normalize a flat frame so that the mean of each channel equals 1.0.
/// For multi-channel (already demosaiced) flats, this is correct as-is.
fn normalize_flat(flat: &mut ImageData) {
    for c in 0..flat.channels {
        let sum: f64 = (0..flat.pixel_count())
            .map(|i| flat.data[i * flat.channels + c] as f64)
            .sum();
        let mean = sum / flat.pixel_count() as f64;

        if mean > 0.0 {
            for i in 0..flat.pixel_count() {
                let idx = i * flat.channels + c;
                flat.data[idx] = (flat.data[idx] as f64 / mean) as f32;
            }
        }
    }
}

/// Normalize a mono Bayer flat per sub-channel so that each color's mean = 1.0.
///
/// Without this, the global mean is dominated by the brighter green pixels
/// (2× count, higher QE). After global normalization, G values end up > 1.0
/// and R/B values < 1.0. Dividing a light by this flat then suppresses green
/// and boosts red/blue — creating a purple/magenta color cast.
///
/// Per-Bayer-channel normalization ensures the flat only corrects spatial
/// illumination variations (vignetting, dust) within each color, without
/// altering the sensor's native color response.
fn normalize_flat_bayer(flat: &mut ImageData, pattern: BayerPattern) {
    let w = flat.width;
    let h = flat.height;

    // Accumulate sum and count for each Bayer color (0=R, 1=G, 2=B)
    let mut sums = [0.0f64; 3];
    let mut counts = [0u64; 3];

    for row in 0..h {
        for col in 0..w {
            let color = pattern.color_at(row, col);
            let val = flat.get(col, row, 0) as f64;
            sums[color] += val;
            counts[color] += 1;
        }
    }

    let mut means = [0.0f64; 3];
    for color in 0..3 {
        if counts[color] > 0 {
            means[color] = sums[color] / counts[color] as f64;
        }
    }

    log::info!(
        "Flat normalization (Bayer {:?}): R_mean={:.2}, G_mean={:.2}, B_mean={:.2}",
        pattern, means[0], means[1], means[2]
    );

    // Normalize each pixel by the mean of its own color channel
    for row in 0..h {
        for col in 0..w {
            let color = pattern.color_at(row, col);
            let mean = means[color];
            if mean > 0.0 {
                let idx = row * w + col;
                flat.data[idx] = (flat.data[idx] as f64 / mean) as f32;
            }
        }
    }
}

/// Validate that all frames have the same dimensions.
/// Width and height must match. Channel count differences are logged as
/// warnings but not treated as errors (calibration handles channel broadcasting).
fn validate_same_dimensions(frames: &[ImageData], frame_type: &str) -> Result<()> {
    if frames.len() < 2 {
        return Ok(());
    }

    let first = &frames[0];
    for (i, frame) in frames.iter().enumerate().skip(1) {
        if frame.width != first.width || frame.height != first.height {
            return Err(AstroError::DimensionMismatch(format!(
                "{} frame {} is {}x{} but frame 0 is {}x{}",
                frame_type, i, frame.width, frame.height, first.width, first.height
            )));
        }
        if frame.channels != first.channels {
            log::warn!(
                "{} frame {} has {} channels but frame 0 has {} — will be handled during calibration",
                frame_type, i, frame.channels, first.channels
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_image(width: usize, height: usize, value: f32) -> ImageData {
        ImageData {
            width,
            height,
            channels: 1,
            data: vec![value; width * height],
        }
    }

    #[test]
    fn test_subtract_images() {
        let a = make_test_image(10, 10, 100.0);
        let b = make_test_image(10, 10, 30.0);
        let result = subtract_images(&a, &b).unwrap();
        assert!((result.data[0] - 70.0).abs() < 1e-6);
    }

    #[test]
    fn test_divide_images() {
        let a = make_test_image(10, 10, 200.0);
        let b = make_test_image(10, 10, 2.0);
        let result = divide_images(&a, &b).unwrap();
        assert!((result.data[0] - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_divide_by_near_zero() {
        let a = make_test_image(10, 10, 200.0);
        let b = make_test_image(10, 10, 0.0);
        let result = divide_images(&a, &b).unwrap();
        assert_eq!(result.data[0], 0.0);
    }

    #[test]
    fn test_calibrate_light_full_pipeline() {
        let light = make_test_image(10, 10, 1000.0);
        let bias = make_test_image(10, 10, 100.0);
        let dark = make_test_image(10, 10, 50.0);
        // Flat normalized to 1.0
        let flat = make_test_image(10, 10, 1.0);

        let cal = CalibrationFrames {
            master_bias: Some(bias),
            master_dark: Some(dark),
            master_flat: Some(flat),
        };

        let result = calibrate_light(&light, &cal).unwrap();
        // (1000 - 100 - 50) / 1.0 = 850
        assert!((result.data[0] - 850.0).abs() < 1e-3);
    }

    #[test]
    fn test_dimension_mismatch() {
        let a = make_test_image(10, 10, 100.0);
        let b = make_test_image(20, 20, 100.0);
        assert!(subtract_images(&a, &b).is_err());
    }
}
