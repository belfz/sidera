//! # astro-core
//!
//! Pure-Rust astrophotography image processing library.

pub mod alignment;
pub mod bayer;
pub mod calibration;
pub mod error;
pub mod fits_io;
pub mod gradient;
pub mod histogram;
pub mod image;
pub mod output;
pub mod stacking;
pub mod star_detection;

pub use error::{AstroError, Result};
pub use image::{FitsHeader, FitsValue, FrameMetadata, FrameType, ImageData};

use std::path::PathBuf;

/// Full processing pipeline.
pub struct Pipeline {
    pub bias_paths: Vec<PathBuf>,
    pub dark_paths: Vec<PathBuf>,
    pub flat_paths: Vec<PathBuf>,
    pub light_paths: Vec<PathBuf>,
    pub bayer_pattern: Option<bayer::BayerPattern>,
    pub skip_demosaic: bool,
    pub stack_method: stacking::StackMethod,
    pub alignment_params: alignment::AlignmentParams,
    pub detection_params: star_detection::DetectionParams,
}

impl Default for Pipeline {
    fn default() -> Self {
        Pipeline {
            bias_paths: Vec::new(),
            dark_paths: Vec::new(),
            flat_paths: Vec::new(),
            light_paths: Vec::new(),
            bayer_pattern: None,
            skip_demosaic: false,
            stack_method: stacking::StackMethod::default(),
            alignment_params: alignment::AlignmentParams::default(),
            detection_params: star_detection::DetectionParams::default(),
        }
    }
}

pub type ProgressCallback = Box<dyn Fn(PipelineStage, f32) + Send + Sync>;

#[derive(Debug, Clone, Copy)]
pub enum PipelineStage {
    LoadingBias,
    LoadingDarks,
    LoadingFlats,
    LoadingLights,
    CreatingMasterBias,
    CreatingMasterDark,
    CreatingMasterFlat,
    Registering,
    Stacking,
    PostProcessing,
    Complete,
}

impl std::fmt::Display for PipelineStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineStage::LoadingBias => write!(f, "Loading bias frames"),
            PipelineStage::LoadingDarks => write!(f, "Loading dark frames"),
            PipelineStage::LoadingFlats => write!(f, "Loading flat frames"),
            PipelineStage::LoadingLights => write!(f, "Loading light frames"),
            PipelineStage::CreatingMasterBias => write!(f, "Creating master bias"),
            PipelineStage::CreatingMasterDark => write!(f, "Creating master dark"),
            PipelineStage::CreatingMasterFlat => write!(f, "Creating master flat"),
            PipelineStage::Registering => write!(f, "Registering frames"),
            PipelineStage::Stacking => write!(f, "Stacking"),
            PipelineStage::PostProcessing => write!(f, "Post-processing"),
            PipelineStage::Complete => write!(f, "Complete"),
        }
    }
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run the full pipeline using a memory-efficient two-pass approach.
    ///
    /// **Pass 1 (Registration):** Load each light frame one at a time,
    /// calibrate, demosaic, detect stars. Store only the star lists (small).
    /// Then compute alignment transforms.
    ///
    /// **Pass 2 (Stacking):** Load each light frame again, calibrate,
    /// demosaic, warp with the stored transform, and accumulate into a
    /// running sum. Memory usage: ~2 frames at a time regardless of count.
    ///
    /// **Post-processing:** Background gradient extraction, color neutralization,
    /// border cropping, noise reduction.
    pub fn run(&self, progress: Option<ProgressCallback>) -> Result<ImageData> {
        let report = |stage: PipelineStage, pct: f32| {
            if let Some(ref cb) = progress {
                cb(stage, pct);
            }
        };

        let effective_bayer = self.resolve_bayer_pattern()?;
        log::info!("Bayer pattern: {:?}", effective_bayer);

        // ─── Load calibration frames & create masters ────────────────────
        let cal = self.create_calibration_masters(&report, effective_bayer)?;

        // ─── PASS 1: Registration ────────────────────────────────────────
        // Load each light one at a time, calibrate+demosaic, detect stars.
        report(PipelineStage::Registering, 0.0);
        let n_lights = self.light_paths.len();
        let mut star_lists: Vec<Option<Vec<star_detection::Star>>> = Vec::with_capacity(n_lights);
        let mut ref_width = 0usize;
        let mut ref_height = 0usize;
        let mut ref_channels = 0usize;

        for (i, path) in self.light_paths.iter().enumerate() {
            let pct = i as f32 / n_lights as f32 * 0.5; // 0-50% of registration phase
            report(PipelineStage::Registering, pct);

            match self.load_one_frame(path, &cal, effective_bayer) {
                Ok(frame) => {
                    if i == 0 {
                        ref_width = frame.width;
                        ref_height = frame.height;
                        ref_channels = frame.channels;
                    }
                    match star_detection::detect_stars(&frame, &self.detection_params) {
                        Ok(stars) => {
                            log::debug!("Frame {}: {} stars detected", i, stars.len());
                            star_lists.push(Some(stars));
                        }
                        Err(e) => {
                            log::warn!("Frame {}: star detection failed: {}", i, e);
                            star_lists.push(None);
                        }
                    }
                    // `frame` is dropped here — memory freed
                }
                Err(e) => {
                    log::warn!("Frame {}: load failed: {}", i, e);
                    star_lists.push(None);
                }
            }
        }

        // Choose reference: frame with most detected stars
        let ref_idx = star_lists.iter().enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|stars| (i, stars.len())))
            .max_by_key(|(_, count)| *count)
            .map(|(i, _)| i)
            .ok_or_else(|| AstroError::NoFrames {
                operation: "registration (no frames with detectable stars)".into(),
            })?;

        let ref_stars = star_lists[ref_idx].as_ref().unwrap();
        log::info!(
            "Reference frame: {} ({} stars)",
            ref_idx,
            ref_stars.len()
        );

        // Compute alignment transforms
        let mut transforms: Vec<Option<alignment::Transform>> = Vec::with_capacity(n_lights);
        let mut valid_count = 0;
        for (i, stars_opt) in star_lists.iter().enumerate() {
            let pct = 0.5 + (i as f32 / n_lights as f32 * 0.5); // 50-100%
            report(PipelineStage::Registering, pct);

            if i == ref_idx {
                transforms.push(Some(alignment::Transform::identity()));
                valid_count += 1;
                continue;
            }

            match stars_opt {
                Some(stars) if stars.len() >= 3 => {
                    match alignment::compute_alignment(ref_stars, stars, &self.alignment_params) {
                        Ok(t) => {
                            transforms.push(Some(t));
                            valid_count += 1;
                        }
                        Err(e) => {
                            log::warn!("Frame {}: alignment failed: {}", i, e);
                            transforms.push(None);
                        }
                    }
                }
                _ => transforms.push(None),
            }
        }

        log::info!("{} of {} frames aligned successfully", valid_count, n_lights);
        if valid_count < 2 {
            return Err(AstroError::Alignment(
                "Fewer than 2 frames aligned successfully".into(),
            ));
        }

        // Drop star lists — no longer needed
        drop(star_lists);
        report(PipelineStage::Registering, 1.0);

        // ─── PASS 2: Incremental stacking ────────────────────────────────
        // Load each frame again, warp, and accumulate into running sum.
        // Memory: 1 frame + accumulator (~200MB total for 3856×2180×3).
        report(PipelineStage::Stacking, 0.0);

        let w = ref_width;
        let h = ref_height;
        let ch = ref_channels;
        let total = w * h * ch;

        // Running sum and count (per-pixel, per-channel)
        let mut sum = vec![0.0f64; total];
        let mut count = vec![0u32; total];

        // Compute per-channel reference backgrounds for equalization
        let ref_frame = self.load_one_frame(&self.light_paths[ref_idx], &cal, effective_bayer)?;
        let ref_bgs = estimate_frame_background_per_channel(&ref_frame);
        log::info!("Reference frame backgrounds: {:?}", ref_bgs);
        drop(ref_frame);

        let mut stacked_count = 0u32;
        for (i, path) in self.light_paths.iter().enumerate() {
            let pct = i as f32 / n_lights as f32;
            report(PipelineStage::Stacking, pct);

            if transforms[i].is_none() {
                continue; // Skip frames that failed alignment
            }
            let transform = transforms[i].as_ref().unwrap();

            let frame = match self.load_one_frame(path, &cal, effective_bayer) {
                Ok(f) => f,
                Err(_) => continue,
            };

            // Warp
            let warped = match alignment::apply_transform(&frame, transform) {
                Ok(w) => w,
                Err(_) => continue,
            };
            drop(frame);

            // Per-channel background equalization
            let frame_bgs = estimate_frame_background_per_channel(&warped);
            let bg_deltas: Vec<f32> = ref_bgs.iter()
                .zip(frame_bgs.iter())
                .map(|(r, f)| r - f)
                .collect();

            // Accumulate (skip zero-fill pixels from alignment borders)
            for j in 0..total {
                let v = warped.data[j];
                if v.abs() > 0.5 {
                    let c = j % ch;
                    let delta = if c < bg_deltas.len() { bg_deltas[c] } else { 0.0 };
                    sum[j] += (v + delta) as f64;
                    count[j] += 1;
                }
            }

            stacked_count += 1;
        }

        log::info!("Stacked {} frames", stacked_count);
        report(PipelineStage::Stacking, 1.0);

        // Compute mean
        let data: Vec<f32> = sum.iter().zip(count.iter())
            .map(|(&s, &c)| if c > 0 { (s / c as f64) as f32 } else { 0.0 })
            .collect();

        let mut result = ImageData::from_data(w, h, ch, data)?;

        // Log per-channel statistics of the stacked result
        let stacked_bgs = estimate_frame_background_per_channel(&result);
        log::info!("Stacked result per-channel backgrounds: {:?}", stacked_bgs);

        // ─── POST-PROCESSING ─────────────────────────────────────────────
        report(PipelineStage::PostProcessing, 0.0);

        // 1. Smart crop — remove low-coverage borders FIRST so that
        //    gradient extraction and neutralization operate on clean data
        //    without edge artifacts that skew their statistics.
        log::info!("Cropping borders (coverage-based)...");
        result = smart_crop_by_coverage(&result, &count, w, h, ch);
        report(PipelineStage::PostProcessing, 0.2);

        // 2. Background gradient extraction
        log::info!("Extracting background gradient...");
        result = gradient::extract_gradient(&result);
        let post_gradient_bgs = estimate_frame_background_per_channel(&result);
        log::info!("Post-gradient per-channel backgrounds: {:?}", post_gradient_bgs);
        report(PipelineStage::PostProcessing, 0.4);

        // 3. Background neutralization (white balance)
        if result.channels == 3 {
            log::info!("Neutralizing background color...");
            neutralize_background(&mut result);
            let post_neutral_bgs = estimate_frame_background_per_channel(&result);
            log::info!("Post-neutralize per-channel backgrounds: {:?}", post_neutral_bgs);
        }
        report(PipelineStage::PostProcessing, 0.6);

        // 4. SCNR — remove residual magenta cast
        if result.channels == 3 {
            log::info!("Applying SCNR (magenta removal)...");
            scnr_average_neutral(&mut result);
        }
        report(PipelineStage::PostProcessing, 0.8);

        // 5. Noise reduction
        log::info!("Reducing noise...");
        result = reduce_noise(&result);
        report(PipelineStage::PostProcessing, 1.0);

        report(PipelineStage::Complete, 1.0);
        Ok(result)
    }

    /// Resolve Bayer pattern (explicit > auto-detect from FITS > none).
    fn resolve_bayer_pattern(&self) -> Result<Option<bayer::BayerPattern>> {
        if let Some(pat) = self.bayer_pattern {
            return Ok(Some(pat));
        }
        if self.skip_demosaic {
            return Ok(None);
        }
        if let Some(first_path) = self.light_paths.first() {
            let fits = fits_io::read_fits(first_path)?;
            if let Some(ref pat_str) = fits.metadata.bayer_pattern {
                if let Some(pat) = bayer::BayerPattern::from_str(pat_str) {
                    log::info!("Auto-detected Bayer pattern: {}", pat_str);
                    return Ok(Some(pat));
                }
            }
            if fits.image.channels > 1 {
                log::info!("Image already has {} channels, skipping demosaic", fits.image.channels);
                return Ok(None);
            }
        }
        Ok(None)
    }

    /// Create master calibration frames.
    fn create_calibration_masters(
        &self,
        report: &dyn Fn(PipelineStage, f32),
        effective_bayer: Option<bayer::BayerPattern>,
    ) -> Result<calibration::CalibrationFrames> {
        let load_raw = |paths: &[PathBuf], stage: PipelineStage| -> Result<Vec<ImageData>> {
            let mut frames = Vec::new();
            for (i, path) in paths.iter().enumerate() {
                report(stage, i as f32 / paths.len().max(1) as f32);
                let fits = fits_io::read_fits(path)?;
                frames.push(fits.image);
            }
            Ok(frames)
        };

        let bias_frames = load_raw(&self.bias_paths, PipelineStage::LoadingBias)?;
        let dark_frames = load_raw(&self.dark_paths, PipelineStage::LoadingDarks)?;
        let flat_frames = load_raw(&self.flat_paths, PipelineStage::LoadingFlats)?;

        report(PipelineStage::CreatingMasterBias, 0.0);
        let master_bias = if !bias_frames.is_empty() {
            Some(calibration::create_master_bias(&bias_frames)?)
        } else { None };
        report(PipelineStage::CreatingMasterBias, 1.0);

        report(PipelineStage::CreatingMasterDark, 0.0);
        let master_dark = if !dark_frames.is_empty() {
            Some(calibration::create_master_dark(&dark_frames, master_bias.as_ref())?)
        } else { None };
        report(PipelineStage::CreatingMasterDark, 1.0);

        report(PipelineStage::CreatingMasterFlat, 0.0);
        let master_flat = if !flat_frames.is_empty() {
            Some(calibration::create_master_flat(&flat_frames, master_bias.as_ref(), effective_bayer)?)
        } else { None };
        report(PipelineStage::CreatingMasterFlat, 1.0);

        Ok(calibration::CalibrationFrames { master_bias, master_dark, master_flat })
    }

    /// Load a single frame: read FITS → calibrate → demosaic.
    fn load_one_frame(
        &self,
        path: &PathBuf,
        cal: &calibration::CalibrationFrames,
        bayer: Option<bayer::BayerPattern>,
    ) -> Result<ImageData> {
        let fits = fits_io::read_fits(path)?;
        let raw = fits.image;

        let calibrated = calibration::calibrate_light(&raw, cal)?;

        if let Some(pattern) = bayer {
            if calibrated.channels == 1 {
                return bayer::demosaic(&calibrated, pattern);
            }
        }
        Ok(calibrated)
    }
}

// ─── Post-processing helpers ─────────────────────────────────────────────────

/// Crop to the region with sufficient stacking coverage, then trim any
/// remaining columns/rows with anomalous per-channel backgrounds.
///
/// Two-stage approach:
/// 1. Coverage crop: find the bounding box where ≥95% of frames contributed.
/// 2. Background anomaly scan: scan columns/rows from each edge inward,
///    trimming until the per-channel background matches the interior.
///    This catches the color banding that survives coverage cropping.
fn smart_crop_by_coverage(
    image: &ImageData,
    count: &[u32],
    orig_w: usize,
    orig_h: usize,
    ch: usize,
) -> ImageData {
    let max_count = count.iter().copied().max().unwrap_or(1);
    let threshold = (max_count as f64 * 0.95) as u32;

    // Per-pixel coverage: minimum count across all channels
    let mut covered = vec![false; orig_w * orig_h];
    for y in 0..orig_h {
        for x in 0..orig_w {
            let base = (y * orig_w + x) * ch;
            let min_ch_count = (0..ch)
                .map(|c| count[base + c])
                .min()
                .unwrap_or(0);
            covered[y * orig_w + x] = min_ch_count >= threshold;
        }
    }

    // Find bounding box of covered region
    let mut left = orig_w;
    let mut right = 0usize;
    let mut top = orig_h;
    let mut bottom = 0usize;

    for y in 0..orig_h {
        for x in 0..orig_w {
            if covered[y * orig_w + x] {
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }

    // Fall back to 5% crop if coverage analysis fails
    if right <= left || bottom <= top {
        let margin_w = orig_w / 20;
        let margin_h = orig_h / 20;
        left = margin_w;
        right = orig_w.saturating_sub(margin_w);
        top = margin_h;
        bottom = orig_h.saturating_sub(margin_h);
    }

    // Small margin to trim partial-coverage edge pixels
    let margin = 8;
    left = (left + margin).min(right.saturating_sub(1));
    top = (top + margin).min(bottom.saturating_sub(1));
    right = right.saturating_sub(margin).max(left + 1);
    bottom = bottom.saturating_sub(margin).max(top + 1);

    log::info!(
        "Coverage crop: {}x{} → L={} T={} R={} B={} (threshold: {}/{} frames)",
        orig_w, orig_h, left, top, orig_w - right, orig_h - bottom,
        threshold, max_count,
    );

    // Stage 2: trim columns/rows with anomalous backgrounds.
    // Compute the "interior" reference background from the central 50% of
    // the coverage-cropped region, then scan from each edge inward.
    let crop_w = right - left;
    let crop_h = bottom - top;

    if ch >= 3 && crop_w > 100 && crop_h > 100 {
        let (trim_l, trim_r) = detect_banding_columns(
            image, left, top, crop_w, crop_h, ch,
        );
        let (trim_t, trim_b) = detect_banding_rows(
            image, left, top, crop_w, crop_h, ch,
        );

        if trim_l > 0 || trim_r > 0 || trim_t > 0 || trim_b > 0 {
            log::info!(
                "Banding trim: L+={} R+={} T+={} B+={}",
                trim_l, trim_r, trim_t, trim_b,
            );
            left += trim_l;
            right -= trim_r;
            top += trim_t;
            bottom -= trim_b;
        }
    }

    let new_w = right - left;
    let new_h = bottom - top;

    log::info!(
        "Final crop: {}x{} → {}x{}",
        orig_w, orig_h, new_w, new_h,
    );

    let mut cropped = ImageData::new(new_w, new_h, ch);
    for y in 0..new_h {
        for x in 0..new_w {
            for c in 0..ch {
                cropped.set(x, y, c, image.get(x + left, y + top, c));
            }
        }
    }
    cropped
}

/// Compute the sigma-clipped median of a slice of f32 values.
fn sigma_clipped_median_vec(values: &mut Vec<f32>) -> f32 {
    if values.is_empty() { return 0.0; }
    for _ in 0..3 {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = values.len();
        if n < 10 { break; }
        let median = values[n / 2];
        let mut devs: Vec<f32> = values.iter().map(|&v| (v - median).abs()).collect();
        devs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let sigma = 1.4826 * devs[n / 2];
        let lo = median - 2.5 * sigma;
        let hi = median + 2.5 * sigma;
        values.retain(|&v| v >= lo && v <= hi);
    }
    if values.is_empty() { return 0.0; }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

/// Compute the per-channel background median for a single column within the
/// given crop region.
fn column_background(
    image: &ImageData, col: usize, y_start: usize, height: usize, ch: usize,
) -> Vec<f32> {
    (0..ch.min(3)).map(|c| {
        let mut vals: Vec<f32> = (0..height)
            .map(|dy| image.get(col, y_start + dy, c))
            .filter(|v| v.abs() > 0.5)
            .collect();
        sigma_clipped_median_vec(&mut vals)
    }).collect()
}

/// Compute the per-channel background median for a single row within the
/// given crop region.
fn row_background(
    image: &ImageData, row: usize, x_start: usize, width: usize, ch: usize,
) -> Vec<f32> {
    (0..ch.min(3)).map(|c| {
        let mut vals: Vec<f32> = (0..width)
            .map(|dx| image.get(x_start + dx, row, c))
            .filter(|v| v.abs() > 0.5)
            .collect();
        sigma_clipped_median_vec(&mut vals)
    }).collect()
}

/// Scan columns from left and right edges inward, detecting where the
/// per-channel background deviates from the interior. Returns (trim_left,
/// trim_right) — the number of additional columns to remove from each side.
fn detect_banding_columns(
    image: &ImageData,
    x0: usize, y0: usize, w: usize, h: usize, ch: usize,
) -> (usize, usize) {
    // Reference: median background of columns in the central 50%
    let center_start = w / 4;
    let center_end = w * 3 / 4;
    let n_sample = 20.min(center_end - center_start);
    let step = (center_end - center_start) / n_sample.max(1);

    let mut ref_bg = vec![0.0f64; ch.min(3)];
    let mut n_ref = 0u32;
    for i in 0..n_sample {
        let col = x0 + center_start + i * step;
        let bg = column_background(image, col, y0, h, ch);
        for (c, &v) in bg.iter().enumerate() {
            ref_bg[c] += v as f64;
        }
        n_ref += 1;
    }
    if n_ref > 0 {
        for v in &mut ref_bg { *v /= n_ref as f64; }
    }

    // Compute MAD of interior column backgrounds for the tolerance threshold
    let mut col_diffs: Vec<f32> = Vec::new();
    for i in 0..n_sample {
        let col = x0 + center_start + i * step;
        let bg = column_background(image, col, y0, h, ch);
        let max_diff: f32 = bg.iter().enumerate()
            .map(|(c, &v)| (v - ref_bg[c] as f32).abs())
            .fold(0.0f32, f32::max);
        col_diffs.push(max_diff);
    }
    col_diffs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let interior_mad = if col_diffs.is_empty() { 1.0 } else { col_diffs[col_diffs.len() / 2] };
    let tolerance = (interior_mad * 5.0).max(0.5);

    let max_trim = w / 4; // never trim more than 25% from each side

    // Scan from left
    let mut trim_left = 0;
    for dx in 0..max_trim {
        let bg = column_background(image, x0 + dx, y0, h, ch);
        let max_diff: f32 = bg.iter().enumerate()
            .map(|(c, &v)| (v - ref_bg[c] as f32).abs())
            .fold(0.0f32, f32::max);
        if max_diff > tolerance {
            trim_left = dx + 1;
        } else {
            break;
        }
    }

    // Scan from right
    let mut trim_right = 0;
    for dx in 0..max_trim {
        let col = x0 + w - 1 - dx;
        let bg = column_background(image, col, y0, h, ch);
        let max_diff: f32 = bg.iter().enumerate()
            .map(|(c, &v)| (v - ref_bg[c] as f32).abs())
            .fold(0.0f32, f32::max);
        if max_diff > tolerance {
            trim_right = dx + 1;
        } else {
            break;
        }
    }

    (trim_left, trim_right)
}

/// Scan rows from top and bottom edges inward, detecting where the
/// per-channel background deviates from the interior. Returns (trim_top,
/// trim_bottom).
fn detect_banding_rows(
    image: &ImageData,
    x0: usize, y0: usize, w: usize, h: usize, ch: usize,
) -> (usize, usize) {
    let center_start = h / 4;
    let center_end = h * 3 / 4;
    let n_sample = 20.min(center_end - center_start);
    let step = (center_end - center_start) / n_sample.max(1);

    let mut ref_bg = vec![0.0f64; ch.min(3)];
    let mut n_ref = 0u32;
    for i in 0..n_sample {
        let row = y0 + center_start + i * step;
        let bg = row_background(image, row, x0, w, ch);
        for (c, &v) in bg.iter().enumerate() {
            ref_bg[c] += v as f64;
        }
        n_ref += 1;
    }
    if n_ref > 0 {
        for v in &mut ref_bg { *v /= n_ref as f64; }
    }

    let mut row_diffs: Vec<f32> = Vec::new();
    for i in 0..n_sample {
        let row = y0 + center_start + i * step;
        let bg = row_background(image, row, x0, w, ch);
        let max_diff: f32 = bg.iter().enumerate()
            .map(|(c, &v)| (v - ref_bg[c] as f32).abs())
            .fold(0.0f32, f32::max);
        row_diffs.push(max_diff);
    }
    row_diffs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let interior_mad = if row_diffs.is_empty() { 1.0 } else { row_diffs[row_diffs.len() / 2] };
    let tolerance = (interior_mad * 5.0).max(0.5);

    let max_trim = h / 4;

    let mut trim_top = 0;
    for dy in 0..max_trim {
        let bg = row_background(image, y0 + dy, x0, w, ch);
        let max_diff: f32 = bg.iter().enumerate()
            .map(|(c, &v)| (v - ref_bg[c] as f32).abs())
            .fold(0.0f32, f32::max);
        if max_diff > tolerance {
            trim_top = dy + 1;
        } else {
            break;
        }
    }

    let mut trim_bottom = 0;
    for dy in 0..max_trim {
        let row = y0 + h - 1 - dy;
        let bg = row_background(image, row, x0, w, ch);
        let max_diff: f32 = bg.iter().enumerate()
            .map(|(c, &v)| (v - ref_bg[c] as f32).abs())
            .fold(0.0f32, f32::max);
        if max_diff > tolerance {
            trim_bottom = dy + 1;
        } else {
            break;
        }
    }

    (trim_top, trim_bottom)
}

/// Neutralize background color using additive correction.
///
/// Computes the sigma-clipped median of each RGB channel's background,
/// then shifts each channel so all backgrounds equal a common reference level.
/// Additive correction preserves signal colors — a star with R=500, G=450,
/// B=480 keeps those relative ratios, unlike multiplicative scaling which
/// would distort them.
fn neutralize_background(image: &mut ImageData) {
    if image.channels < 3 { return; }

    let mut channel_medians = Vec::new();
    for c in 0..image.channels.min(3) {
        let mut values: Vec<f32> = (0..image.pixel_count())
            .map(|i| image.data[i * image.channels + c])
            .filter(|v| *v > 0.5)
            .collect();
        for _ in 0..3 {
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = values.len();
            if n < 10 { break; }
            let median = values[n / 2];
            let mut devs: Vec<f32> = values.iter().map(|&v| (v - median).abs()).collect();
            devs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let sigma = 1.4826 * devs[n / 2];
            let lo = median - 3.0 * sigma;
            let hi = median + 3.0 * sigma;
            values.retain(|&v| v >= lo && v <= hi);
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let med = if values.is_empty() { 1.0 } else { values[values.len() / 2] };
        channel_medians.push(med);
    }

    // Use the green channel as reference — it has the best SNR (most
    // photosites in Bayer pattern, highest QE) and is the standard
    // reference in tools like Siril and PixInsight.
    let ref_level = channel_medians[1];
    log::info!("Neutralize (additive): R={:.2}, G={:.2}, B={:.2} → ref=G={:.2}",
        channel_medians[0], channel_medians[1], channel_medians[2], ref_level);

    for c in 0..image.channels.min(3) {
        let offset = ref_level - channel_medians[c];
        for i in 0..image.pixel_count() {
            image.data[i * image.channels + c] += offset;
        }
    }
}

/// Subtractive Chromatic Noise Reduction — "Average Neutral" method.
///
/// For each pixel, if R and B both exceed G (i.e. the pixel appears
/// magenta), reduce R and B toward their average with G. This targets
/// only the chromatic noise / cast without affecting pixels that are
/// genuinely red or blue.
///
/// The protection factor ensures only clearly magenta pixels are
/// affected; borderline or faintly tinted pixels are left alone.
fn scnr_average_neutral(image: &mut ImageData) {
    if image.channels < 3 { return; }

    let mut affected = 0u64;
    let total = image.pixel_count() as u64;

    for i in 0..image.pixel_count() {
        let base = i * image.channels;
        let r = image.data[base];
        let g = image.data[base + 1];
        let b = image.data[base + 2];

        // Only act on pixels where both R and B exceed G (magenta).
        // A pixel that's simply red (R > G but B < G) or blue (B > G
        // but R < G) is left untouched.
        if r > g && b > g {
            // "Average neutral": set the excess channels to the average
            // of the original value and the green value. This is gentler
            // than full SCNR which would clamp to G outright.
            image.data[base]     = (r + g) * 0.5;
            image.data[base + 2] = (b + g) * 0.5;
            affected += 1;
        }
    }

    let pct = if total > 0 { affected as f64 / total as f64 * 100.0 } else { 0.0 };
    log::info!("SCNR: adjusted {}/{} pixels ({:.1}%)", affected, total, pct);
}

/// Estimate per-channel frame background using sigma-clipped median.
///
/// Returns a Vec of per-channel medians. For a 3-channel RGB image this
/// returns [R_bg, G_bg, B_bg]. Using per-channel backgrounds prevents the
/// luminance-weighted estimate from systematically favoring green (71.5%
/// weight), which was causing R and B backgrounds to be under-corrected
/// during stacking — producing a green color cast in the final image.
fn estimate_frame_background_per_channel(image: &ImageData) -> Vec<f32> {
    let ch = image.channels;
    (0..ch).map(|c| {
        let mut values: Vec<f32> = (0..image.pixel_count())
            .map(|i| image.data[i * ch + c])
            .filter(|v| *v > 0.5)
            .collect();

        for _ in 0..3 {
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let n = values.len();
            if n < 10 { break; }
            let median = values[n / 2];
            let mut devs: Vec<f32> = values.iter().map(|&v| (v - median).abs()).collect();
            devs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let sigma = 1.4826 * devs[n / 2];
            values.retain(|&v| v >= median - 3.0 * sigma && v <= median + 3.0 * sigma);
        }

        if values.is_empty() { return 0.0; }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        values[values.len() / 2]
    }).collect()
}

/// Estimate frame background as a single scalar (luminance-based).
/// Used for noise reduction threshold, where a single value suffices.
fn estimate_frame_background(image: &ImageData) -> f32 {
    let bgs = estimate_frame_background_per_channel(image);
    if bgs.len() >= 3 {
        0.2126 * bgs[0] + 0.7152 * bgs[1] + 0.0722 * bgs[2]
    } else {
        bgs[0]
    }
}

/// Simple noise reduction: selective median filter.
///
/// Applies a 3×3 median filter only to "background" pixels (below a threshold
/// above the sky level). Bright pixels (stars, galaxy cores) are left untouched
/// to preserve detail.
fn reduce_noise(image: &ImageData) -> ImageData {
    let w = image.width;
    let h = image.height;
    let ch = image.channels;

    // Compute the background level + noise to set the threshold
    let bg = estimate_frame_background(image);
    // Compute noise level (MAD of luminance)
    let mut lum_values: Vec<f32> = if ch >= 3 {
        (0..image.pixel_count())
            .map(|i| {
                0.2126 * image.data[i * ch]
                    + 0.7152 * image.data[i * ch + 1]
                    + 0.0722 * image.data[i * ch + 2]
            })
            .filter(|v| *v > 0.5)
            .collect()
    } else {
        image.data.iter().copied().filter(|v| *v > 0.5).collect()
    };
    lum_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = if lum_values.is_empty() { 0.0 } else { lum_values[lum_values.len() / 2] };
    let mut devs: Vec<f32> = lum_values.iter().map(|&v| (v - median).abs()).collect();
    devs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sigma = if devs.is_empty() { 1.0 } else { 1.4826 * devs[devs.len() / 2] };

    // Only filter pixels below background + 5σ (sky + faint signal)
    let threshold = bg + 5.0 * sigma;

    log::info!("Noise reduction: bg={:.2}, sigma={:.4}, threshold={:.2}", bg, sigma, threshold);

    let mut result = image.clone();

    for y in 1..(h - 1) {
        for x in 1..(w - 1) {
            // Check luminance
            let lum = if ch >= 3 {
                0.2126 * image.get(x, y, 0)
                    + 0.7152 * image.get(x, y, 1)
                    + 0.0722 * image.get(x, y, 2)
            } else {
                image.get(x, y, 0)
            };

            if lum > threshold {
                continue; // Don't filter bright pixels
            }

            // Apply 3×3 median per channel
            for c in 0..ch {
                let mut neighbors = [
                    image.get(x - 1, y - 1, c), image.get(x, y - 1, c), image.get(x + 1, y - 1, c),
                    image.get(x - 1, y, c),     image.get(x, y, c),     image.get(x + 1, y, c),
                    image.get(x - 1, y + 1, c), image.get(x, y + 1, c), image.get(x + 1, y + 1, c),
                ];
                neighbors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                result.set(x, y, c, neighbors[4]); // median of 9
            }
        }
    }

    result
}
