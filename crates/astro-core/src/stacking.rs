//! Image stacking (integration) algorithms.
//!
//! Combines multiple aligned frames into a single result with improved
//! signal-to-noise ratio. Supports mean, median, and sigma-clipped variants.

use crate::error::{AstroError, Result};
use crate::image::ImageData;
use rayon::prelude::*;

/// Available stacking methods.
#[derive(Debug, Clone)]
pub enum StackMethod {
    /// Simple mean (average) of all frames.
    Mean,

    /// Median of all frames — robust to outliers.
    Median,

    /// Iterative sigma-clipped mean: reject pixels > kappa sigma from the mean,
    /// then recompute. Good balance of noise rejection and SNR.
    SigmaClippedMean {
        /// Rejection threshold in sigma units. Default: 3.0.
        kappa: f64,
        /// Maximum number of rejection iterations. Default: 5.
        iterations: usize,
    },

    /// Iterative sigma-clipped median: reject outliers, then take median.
    /// Most robust to artifacts (satellite trails, cosmic rays).
    SigmaClippedMedian {
        /// Rejection threshold in sigma units. Default: 3.0.
        kappa: f64,
        /// Maximum number of rejection iterations. Default: 5.
        iterations: usize,
    },
}

impl Default for StackMethod {
    fn default() -> Self {
        StackMethod::SigmaClippedMean {
            kappa: 3.0,
            iterations: 5,
        }
    }
}

/// Stack multiple images using the specified method.
///
/// All images must have identical dimensions. The result is a single image
/// with the same dimensions.
pub fn stack_images(images: &[ImageData], method: StackMethod) -> Result<ImageData> {
    if images.is_empty() {
        return Err(AstroError::NoFrames {
            operation: "stacking".into(),
        });
    }

    // Validate dimensions
    let first = &images[0];
    for (i, img) in images.iter().enumerate().skip(1) {
        if !first.same_dimensions(img) {
            return Err(AstroError::DimensionMismatch(format!(
                "Image {} is {}x{}x{} but image 0 is {}x{}x{}",
                i, img.width, img.height, img.channels, first.width, first.height, first.channels
            )));
        }
    }

    let width = first.width;
    let height = first.height;
    let channels = first.channels;
    let total = width * height * channels;
    let n = images.len();

    log::info!(
        "Stacking {} images ({}x{}x{}) with {:?}",
        n,
        width,
        height,
        channels,
        method
    );

    if n == 1 {
        return Ok(images[0].clone());
    }

    // Collect pixel values across frames for each position, then apply the stacking function
    let data: Vec<f32> = (0..total)
        .into_par_iter()
        .map(|idx| {
            let mut values: Vec<f32> = images.iter().map(|img| img.data[idx]).collect();
            match &method {
                StackMethod::Mean => stack_mean(&values),
                StackMethod::Median => stack_median(&mut values),
                StackMethod::SigmaClippedMean { kappa, iterations } => {
                    stack_sigma_clipped_mean(&mut values, *kappa, *iterations)
                }
                StackMethod::SigmaClippedMedian { kappa, iterations } => {
                    stack_sigma_clipped_median(&mut values, *kappa, *iterations)
                }
            }
        })
        .collect();

    Ok(ImageData {
        width,
        height,
        channels,
        data,
    })
}

/// Simple mean of all values.
fn stack_mean(values: &[f32]) -> f32 {
    let sum: f64 = values.iter().map(|&v| v as f64).sum();
    (sum / values.len() as f64) as f32
}

/// Median of values.
fn stack_median(values: &mut [f32]) -> f32 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    if n % 2 == 0 {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    } else {
        values[n / 2]
    }
}

/// Sigma-clipped mean: iteratively reject outliers, then take mean.
fn stack_sigma_clipped_mean(values: &mut Vec<f32>, kappa: f64, max_iterations: usize) -> f32 {
    if values.len() <= 2 {
        return stack_mean(values);
    }

    let mut working: Vec<f32> = values.clone();

    for _ in 0..max_iterations {
        if working.len() <= 2 {
            break;
        }

        let mean = working.iter().map(|&v| v as f64).sum::<f64>() / working.len() as f64;
        let variance = working
            .iter()
            .map(|&v| {
                let d = v as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / working.len() as f64;
        let sigma = variance.sqrt();

        if sigma < 1e-10 {
            break;
        }

        let lo = (mean - kappa * sigma) as f32;
        let hi = (mean + kappa * sigma) as f32;
        let new_len = working.len();
        working.retain(|&v| v >= lo && v <= hi);

        if working.len() == new_len {
            break; // No more rejections
        }
    }

    if working.is_empty() {
        stack_mean(values)
    } else {
        stack_mean(&working)
    }
}

/// Sigma-clipped median: iteratively reject outliers, then take median.
fn stack_sigma_clipped_median(values: &mut Vec<f32>, kappa: f64, max_iterations: usize) -> f32 {
    if values.len() <= 2 {
        return stack_median(values);
    }

    let mut working: Vec<f32> = values.clone();

    for _ in 0..max_iterations {
        if working.len() <= 2 {
            break;
        }

        working.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = working[working.len() / 2];

        // Estimate sigma via MAD (Median Absolute Deviation)
        let mut deviations: Vec<f64> = working.iter().map(|&v| (v as f64 - median as f64).abs()).collect();
        deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mad = deviations[deviations.len() / 2];
        let sigma = 1.4826 * mad;

        if sigma < 1e-10 {
            break;
        }

        let lo = (median as f64 - kappa * sigma) as f32;
        let hi = (median as f64 + kappa * sigma) as f32;
        let new_len = working.len();
        working.retain(|&v| v >= lo && v <= hi);

        if working.len() == new_len {
            break;
        }
    }

    if working.is_empty() {
        stack_median(values)
    } else {
        stack_median(&mut working)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_const_image(w: usize, h: usize, val: f32) -> ImageData {
        ImageData {
            width: w,
            height: h,
            channels: 1,
            data: vec![val; w * h],
        }
    }

    #[test]
    fn test_stack_mean_basic() {
        let images = vec![
            make_const_image(10, 10, 100.0),
            make_const_image(10, 10, 200.0),
            make_const_image(10, 10, 300.0),
        ];
        let result = stack_images(&images, StackMethod::Mean).unwrap();
        assert!((result.data[0] - 200.0).abs() < 1e-3);
    }

    #[test]
    fn test_stack_median_basic() {
        let images = vec![
            make_const_image(10, 10, 100.0),
            make_const_image(10, 10, 200.0),
            make_const_image(10, 10, 10000.0), // Outlier
        ];
        let result = stack_images(&images, StackMethod::Median).unwrap();
        assert!((result.data[0] - 200.0).abs() < 1e-3);
    }

    #[test]
    fn test_stack_sigma_clip_rejects_outlier() {
        let mut images = vec![
            make_const_image(10, 10, 100.0),
            make_const_image(10, 10, 100.0),
            make_const_image(10, 10, 100.0),
            make_const_image(10, 10, 100.0),
            make_const_image(10, 10, 100.0),
        ];
        // Add one frame with a huge outlier value
        images.push(make_const_image(10, 10, 50000.0));

        let result = stack_images(
            &images,
            StackMethod::SigmaClippedMean {
                kappa: 2.0,
                iterations: 5,
            },
        )
        .unwrap();

        // The outlier should be rejected, so the result should be close to 100
        assert!(
            (result.data[0] - 100.0).abs() < 10.0,
            "Expected ~100, got {}",
            result.data[0]
        );
    }

    #[test]
    fn test_stack_single_image() {
        let images = vec![make_const_image(10, 10, 42.0)];
        let result = stack_images(&images, StackMethod::Mean).unwrap();
        assert!((result.data[0] - 42.0).abs() < 1e-6);
    }

    #[test]
    fn test_stack_empty() {
        let images: Vec<ImageData> = vec![];
        assert!(stack_images(&images, StackMethod::Mean).is_err());
    }

    #[test]
    fn test_stack_dimension_mismatch() {
        let images = vec![
            make_const_image(10, 10, 100.0),
            make_const_image(20, 20, 100.0),
        ];
        assert!(stack_images(&images, StackMethod::Mean).is_err());
    }
}
