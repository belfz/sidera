//! Histogram computation and midtone transfer function (MTF) stretching.
//!
//! Implements the PixInsight-style Screen Transfer Function (STF) auto-stretch
//! algorithm, which maps a linear image to a visually pleasing non-linear
//! representation.

use crate::image::ImageData;

/// Histogram data for a single channel.
#[derive(Debug, Clone)]
pub struct Histogram {
    /// Bin counts.
    pub bins: Vec<u32>,
    /// Minimum value in the data.
    pub min: f32,
    /// Maximum value in the data.
    pub max: f32,
    /// Number of bins.
    pub bin_count: usize,
}

/// Histogram data for all channels of an image.
#[derive(Debug, Clone)]
pub struct ImageHistogram {
    /// Per-channel histograms.
    pub channels: Vec<Histogram>,
    /// Combined luminance histogram (if multi-channel).
    pub luminance: Option<Histogram>,
}

/// Parameters for the midtone transfer function stretch.
#[derive(Debug, Clone, Copy)]
pub struct StretchParams {
    /// Shadow clipping point [0, 1]. Pixels below this are clipped to black.
    pub shadows: f32,
    /// Midtone balance [0, 1]. Controls the overall brightness curve.
    /// Lower values = brighter midtones, higher = darker.
    pub midtones: f32,
    /// Highlight clipping point [0, 1]. Pixels above this are clipped to white.
    pub highlights: f32,
}

impl Default for StretchParams {
    fn default() -> Self {
        StretchParams {
            shadows: 0.0,
            midtones: 0.5,
            highlights: 1.0,
        }
    }
}

/// Compute a histogram from image data for a single channel.
pub fn compute_channel_histogram(
    image: &ImageData,
    channel: usize,
    bin_count: usize,
) -> Histogram {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;

    for i in 0..image.pixel_count() {
        let v = image.data[i * image.channels + channel];
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }

    let range = max - min;
    let mut bins = vec![0u32; bin_count];

    if range <= 0.0 {
        // All pixels have the same value
        bins[0] = image.pixel_count() as u32;
        return Histogram {
            bins,
            min,
            max,
            bin_count,
        };
    }

    for i in 0..image.pixel_count() {
        let v = image.data[i * image.channels + channel];
        let normalized = (v - min) / range;
        let bin = ((normalized * (bin_count - 1) as f32) as usize).min(bin_count - 1);
        bins[bin] += 1;
    }

    Histogram {
        bins,
        min,
        max,
        bin_count,
    }
}

/// Compute histograms for all channels of an image.
pub fn compute_histogram(image: &ImageData, bin_count: usize) -> ImageHistogram {
    let channels: Vec<Histogram> = (0..image.channels)
        .map(|c| compute_channel_histogram(image, c, bin_count))
        .collect();

    let luminance = if image.channels == 3 {
        // Compute a luminance histogram from the RGB data
        let lum = image.to_luminance().ok();
        lum.map(|l| compute_channel_histogram(&l, 0, bin_count))
    } else {
        None
    };

    ImageHistogram {
        channels,
        luminance,
    }
}

/// Compute robust normalization bounds using median/MAD on luminance.
///
/// For multi-channel (color) images, computes statistics on LUMINANCE instead
/// of mixing R/G/B values. This is critical because the three channels have
/// different offsets (due to Bayer filter response), and mixing them creates
/// an artificially wide distribution that makes the normalization too timid.
///
/// The LOW bound is the noise floor (median − 2.8σ) and the HIGH bound is
/// the 99.9th percentile. This clips the negative noise tail from dark
/// subtraction and extreme star peaks, keeping the useful dynamic range.
fn robust_normalize_bounds(image: &ImageData) -> (f32, f32) {
    // Use LUMINANCE for computing bounds (avoids mixing R/G/B offsets).
    // Also filter out zero/near-zero pixels — these are typically invalid data
    // from alignment borders (zero-fill where frames don't overlap).
    let zero_threshold = 1.0; // Values below this are considered "empty"
    let values: Vec<f32> = if image.channels == 1 {
        image.data.iter().copied()
            .filter(|v| v.is_finite() && *v > zero_threshold)
            .collect()
    } else {
        (0..image.pixel_count())
            .filter_map(|i| {
                let r = image.data[i * image.channels];
                let g = if image.channels >= 2 { image.data[i * image.channels + 1] } else { r };
                let b = if image.channels >= 3 { image.data[i * image.channels + 2] } else { r };
                let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                // Skip invalid/zero-fill pixels (alignment border artifacts)
                if lum.is_finite() && lum > zero_threshold { Some(lum) } else { None }
            })
            .collect()
    };

    if values.is_empty() {
        return (0.0, 1.0);
    }

    let mut sorted = values;
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();

    // Compute median (this IS the sky background for a typical astro image)
    let median = sorted[n / 2];

    // Compute MAD (Median Absolute Deviation) as a robust noise estimate
    let mut deviations: Vec<f32> = sorted.iter().map(|&v| (v - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mad = deviations[n / 2];
    let sigma = 1.4826 * mad;

    // LOW bound: noise floor = median − 2.8σ
    let low = median - 2.8 * sigma;

    // HIGH bound: use median + 100σ to capture the full useful dynamic range.
    // P99.9% is too close to the sky median for deep-sky images (stars are
    // only 0.01% of pixels), which compresses the range and makes the sky
    // appear at ~0.4 in normalized space instead of near 0.
    // Using a sigma-based bound ensures we include stars and preserve
    // enough headroom for the stretch to work aggressively.
    let sigma_high = median + 100.0 * sigma;
    // Cap at the actual 99.99th percentile to avoid extreme outliers
    let p_high_idx = ((n as f64 * 0.9999) as usize).min(n - 1);
    let p_high = sorted[p_high_idx];
    let high = sigma_high.min(p_high).max(median + 10.0 * sigma);

    log::info!(
        "Robust normalize (luminance): median={:.2}, MAD={:.4}, sigma={:.4}, low={:.2}, high={:.2}",
        median, mad, sigma, low, high
    );

    if high <= low {
        let p_low = sorted[((n as f64 * 0.001) as usize).min(n - 1)];
        let p_high = sorted[((n as f64 * 0.999) as usize).min(n - 1)];
        if p_high > p_low {
            return (p_low, p_high);
        }
        return (sorted[0], sorted[n - 1]);
    }

    (low, high)
}

/// Apply robust percentile-based normalization to an image, returning a new
/// normalized copy. This is used consistently by both auto_stretch and
/// apply_stretch to ensure they agree on the same data mapping.
fn robust_normalize(image: &ImageData) -> ImageData {
    let (low, high) = robust_normalize_bounds(image);
    let range = high - low;

    let mut result = image.clone();
    if range > 0.0 {
        for v in &mut result.data {
            *v = ((*v - low) / range).clamp(0.0, 1.0);
        }
    } else {
        for v in &mut result.data {
            *v = 0.0;
        }
    }
    result
}

/// Auto-stretch: compute optimal stretch parameters using the
/// Median + MAD method (similar to PixInsight's STF auto-stretch).
///
/// The algorithm:
/// 1. Robustly normalize the image using percentile clipping (not min/max)
///    to avoid extreme outliers from dark subtraction, hot pixels, etc.
/// 2. Compute the median of the luminance (sky background estimate)
/// 3. Compute the MAD (Median Absolute Deviation) as noise estimate
/// 4. Set shadows = median - 2.8 * MAD_sigma (clips the noise floor)
/// 5. Set highlights = 1.0
/// 6. Set midtones such that the median maps to a target brightness (~0.25)
pub fn auto_stretch(image: &ImageData) -> StretchParams {
    // Robustly normalize using percentile clipping instead of min/max.
    // This prevents extreme negative values (from dark subtraction) or
    // hot pixels from compressing the useful signal into a narrow range.
    let work = robust_normalize(image);

    // Get the luminance channel for statistics
    let lum_data: Vec<f32> = if work.channels == 1 {
        work.data.clone()
    } else {
        // Compute luminance inline to avoid allocating a full ImageData
        (0..work.pixel_count())
            .map(|i| {
                let r = work.data[i * work.channels];
                let g = work.data[i * work.channels + 1];
                let b = if work.channels >= 3 {
                    work.data[i * work.channels + 2]
                } else {
                    0.0
                };
                0.2126 * r + 0.7152 * g + 0.0722 * b
            })
            .collect()
    };

    if lum_data.is_empty() {
        return StretchParams::default();
    }

    // Compute median
    let mut sorted = lum_data;
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let median = sorted[n / 2];

    // Compute MAD (Median Absolute Deviation)
    let mut deviations: Vec<f32> = sorted.iter().map(|&v| (v - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mad = deviations[n / 2];
    // Scale factor to convert MAD to standard deviation equivalent
    let mad_sigma = 1.4826 * mad;

    log::info!(
        "Auto-stretch stats: median={:.6}, MAD={:.6}, MAD_sigma={:.6}",
        median, mad, mad_sigma
    );

    // Shadow clipping: clip at median - 2.8 * sigma
    // Clamp to [0, median) so we never clip INTO the signal
    let shadows = (median - 2.8 * mad_sigma).clamp(0.0, median);

    // Highlights: typically 1.0 for linear data
    let highlights = 1.0f32;

    // Midtone transfer function target: map the sky background to ~10%
    // brightness. This is more aggressive than the traditional 0.25 target
    // and better reveals faint extended objects (galaxies, nebulae) by
    // pushing the sky darker and boosting the signal-to-background contrast.
    let target_median = 0.10f32;
    let normalized_median = if highlights - shadows > 0.0 {
        ((median - shadows) / (highlights - shadows)).clamp(0.001, 0.999)
    } else {
        0.5
    };

    // Solve for midtone balance 'm' such that MTF(normalized_median, m) = target_median
    // MTF(x, m) = (m-1)*x / ((2m-1)*x - m)
    // Solving: m = x*(target - 1) / (2*target*x - target - x)
    let midtones = if normalized_median > 0.001 && normalized_median < 0.999 {
        let x = normalized_median;
        let t = target_median;
        let m = x * (t - 1.0) / (2.0 * t * x - t - x);
        m.clamp(0.001, 0.999)
    } else {
        0.5
    };

    log::info!(
        "Auto-stretch result: shadows={:.6}, midtones={:.6}, highlights={:.6}, normalized_median={:.6}",
        shadows, midtones, highlights, normalized_median
    );

    StretchParams {
        shadows,
        midtones,
        highlights,
    }
}

/// Apply the midtone transfer function (MTF) stretch to an image.
///
/// The MTF maps [shadows, highlights] to [0, 1] with a non-linear curve
/// controlled by the midtone balance parameter.
///
/// MTF(x, m) = (m - 1) * x / ((2m - 1) * x - m)
/// where x ∈ [0, 1] and m ∈ (0, 1).
///
/// The image is first robustly normalized to [0, 1] using percentile clipping
/// (matching the normalization used in auto_stretch), then the stretch is applied.
pub fn apply_stretch(image: &ImageData, params: &StretchParams) -> ImageData {
    // Use the same robust percentile-based normalization as auto_stretch
    // so that the stretch parameters are consistent with the data mapping
    let mut result = robust_normalize(image);

    let shadows = params.shadows;
    let highlights = params.highlights;
    let midtones = params.midtones;
    let range = highlights - shadows;

    for v in &mut result.data {
        // Clip and normalize to [shadows, highlights] range
        let x = if range > 0.0 {
            ((*v - shadows) / range).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Apply MTF
        *v = mtf(x, midtones);
    }

    result
}

/// Midtone Transfer Function.
/// Maps x ∈ [0, 1] to [0, 1] with midtone balance m ∈ (0, 1).
///
/// MTF(x, m) = (m - 1) * x / ((2m - 1) * x - m)
///
/// Properties:
/// - MTF(0, m) = 0
/// - MTF(1, m) = 1
/// - MTF(m, m) = 0.5  (the midtone balance maps to exactly 0.5)
#[inline]
pub fn mtf(x: f32, m: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    if (m - 0.5).abs() < 1e-6 {
        return x; // Linear when m = 0.5
    }

    let numerator = (m - 1.0) * x;
    let denominator = (2.0 * m - 1.0) * x - m;

    if denominator.abs() < 1e-10 {
        return x;
    }

    (numerator / denominator).clamp(0.0, 1.0)
}

/// Generate RGBA pixel data suitable for display on an HTML Canvas.
/// Returns a Vec<u8> of length width * height * 4 (RGBA).
///
/// If stretch params are provided, applies the stretch before conversion.
/// Otherwise, applies auto-stretch.
pub fn to_rgba_preview(
    image: &ImageData,
    stretch: Option<&StretchParams>,
) -> Vec<u8> {
    let params = stretch.cloned().unwrap_or_else(|| auto_stretch(image));
    let stretched = apply_stretch(image, &params);

    let pixel_count = stretched.width * stretched.height;
    let mut rgba = vec![255u8; pixel_count * 4]; // Alpha = 255

    for i in 0..pixel_count {
        if stretched.channels == 1 {
            let v = (stretched.data[i] * 255.0).clamp(0.0, 255.0) as u8;
            rgba[i * 4] = v;
            rgba[i * 4 + 1] = v;
            rgba[i * 4 + 2] = v;
        } else if stretched.channels >= 3 {
            rgba[i * 4] = (stretched.data[i * stretched.channels] * 255.0).clamp(0.0, 255.0) as u8;
            rgba[i * 4 + 1] =
                (stretched.data[i * stretched.channels + 1] * 255.0).clamp(0.0, 255.0) as u8;
            rgba[i * 4 + 2] =
                (stretched.data[i * stretched.channels + 2] * 255.0).clamp(0.0, 255.0) as u8;
        }
        // Alpha stays 255
    }

    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mtf_identity() {
        // m = 0.5 should be identity
        assert!((mtf(0.0, 0.5) - 0.0).abs() < 1e-6);
        assert!((mtf(0.5, 0.5) - 0.5).abs() < 1e-6);
        assert!((mtf(1.0, 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_mtf_endpoints() {
        for m in [0.1f32, 0.3, 0.5, 0.7, 0.9] {
            assert!((mtf(0.0, m) - 0.0).abs() < 1e-6, "MTF(0, {}) should be 0", m);
            assert!((mtf(1.0, m) - 1.0).abs() < 1e-6, "MTF(1, {}) should be 1", m);
        }
    }

    #[test]
    fn test_mtf_midpoint() {
        // MTF(m, m) should equal 0.5
        for m in [0.1f32, 0.3, 0.7, 0.9] {
            let result = mtf(m, m);
            assert!(
                (result - 0.5).abs() < 0.01,
                "MTF({0}, {0}) should be ~0.5, got {1}",
                m,
                result
            );
        }
    }

    #[test]
    fn test_compute_histogram() {
        let mut img = ImageData::new(100, 100, 1);
        for (i, v) in img.data.iter_mut().enumerate() {
            *v = (i % 256) as f32;
        }
        let hist = compute_channel_histogram(&img, 0, 256);
        assert_eq!(hist.bin_count, 256);
        assert!(hist.min >= 0.0);
        assert!(hist.max <= 255.0);
        // Total counts should equal pixel count
        let total: u32 = hist.bins.iter().sum();
        assert_eq!(total, 10000);
    }

    #[test]
    fn test_auto_stretch() {
        let mut img = ImageData::new(100, 100, 1);
        // Simulate a typical linear astro image: most pixels near 0, few bright
        for (i, v) in img.data.iter_mut().enumerate() {
            *v = (i as f32 / 10000.0) * 0.1; // Low stretch
        }
        let params = auto_stretch(&img);
        assert!(params.shadows >= 0.0);
        assert!(params.highlights <= 1.0);
        assert!(params.midtones > 0.0 && params.midtones < 1.0);
    }

    #[test]
    fn test_to_rgba_preview() {
        let img = ImageData::new(10, 10, 1);
        let rgba = to_rgba_preview(&img, None);
        assert_eq!(rgba.len(), 10 * 10 * 4);
        // All alpha values should be 255
        for i in 0..100 {
            assert_eq!(rgba[i * 4 + 3], 255);
        }
    }
}
