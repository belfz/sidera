//! Background gradient extraction for deep-sky images.
//!
//! Models and subtracts the smooth sky background gradient caused by
//! light pollution, atmospheric extinction, and vignetting. This is
//! equivalent to Siril's "Background Extraction" step and is essential
//! for revealing faint extended objects.
//!
//! Algorithm:
//! 1. Divide the image into a grid of sample cells
//! 2. For each cell, compute sigma-clipped median per channel (rejecting stars)
//! 3. Bilinearly interpolate between grid points to create a smooth background model
//! 4. Subtract the model from the image

use crate::image::ImageData;

/// Number of grid divisions along each axis for background sampling.
const GRID_SIZE: usize = 16;

/// Extract and subtract the background gradient from an image.
///
/// Returns a new image with the gradient removed. The minimum background
/// level is preserved (not shifted to zero) so that the data remains positive.
pub fn extract_gradient(image: &ImageData) -> ImageData {
    let w = image.width;
    let h = image.height;
    let ch = image.channels;

    // Step 1: Sample the background in a grid
    let grid_w = GRID_SIZE;
    let grid_h = (GRID_SIZE as f64 * h as f64 / w as f64).round() as usize;
    let grid_h = grid_h.max(4);

    let cell_w = w as f64 / grid_w as f64;
    let cell_h = h as f64 / grid_h as f64;

    // For each grid cell, compute sigma-clipped median per channel
    let mut grid = vec![vec![0.0f32; ch]; grid_w * grid_h];

    for gy in 0..grid_h {
        for gx in 0..grid_w {
            let x0 = (gx as f64 * cell_w) as usize;
            let y0 = (gy as f64 * cell_h) as usize;
            let x1 = ((gx + 1) as f64 * cell_w) as usize;
            let y1 = ((gy + 1) as f64 * cell_h) as usize;
            let x1 = x1.min(w);
            let y1 = y1.min(h);

            for c in 0..ch {
                let bg = sigma_clipped_median(image, x0, y0, x1, y1, c);
                grid[gy * grid_w + gx][c] = bg;
            }
        }
    }

    // Step 2: Build the smooth background model by bilinear interpolation
    let mut model = ImageData::new(w, h, ch);

    for y in 0..h {
        for x in 0..w {
            // Find the grid cell this pixel falls in
            let gx_f = (x as f64 / cell_w) - 0.5;
            let gy_f = (y as f64 / cell_h) - 0.5;

            let gx0 = (gx_f.floor() as isize).max(0) as usize;
            let gy0 = (gy_f.floor() as isize).max(0) as usize;
            let gx1 = (gx0 + 1).min(grid_w - 1);
            let gy1 = (gy0 + 1).min(grid_h - 1);

            let fx = (gx_f - gx0 as f64).clamp(0.0, 1.0) as f32;
            let fy = (gy_f - gy0 as f64).clamp(0.0, 1.0) as f32;

            for c in 0..ch {
                let v00 = grid[gy0 * grid_w + gx0][c];
                let v10 = grid[gy0 * grid_w + gx1][c];
                let v01 = grid[gy1 * grid_w + gx0][c];
                let v11 = grid[gy1 * grid_w + gx1][c];

                let bg = v00 * (1.0 - fx) * (1.0 - fy)
                    + v10 * fx * (1.0 - fy)
                    + v01 * (1.0 - fx) * fy
                    + v11 * fx * fy;

                model.set(x, y, c, bg);
            }
        }
    }

    // Step 3: Find the minimum background level (to preserve as baseline)
    let mut min_bg = f32::INFINITY;
    for v in &model.data {
        if *v < min_bg {
            min_bg = *v;
        }
    }

    // Step 4: Subtract the model, keeping the minimum background level
    let mut result = image.clone();
    for i in 0..result.data.len() {
        result.data[i] = result.data[i] - model.data[i] + min_bg;
    }

    log::info!(
        "Gradient extraction: grid={}x{}, min_bg={:.2}",
        grid_w, grid_h, min_bg
    );

    result
}

/// Compute the sigma-clipped median for a rectangular region of one channel.
/// Iteratively rejects pixels more than 2σ from the median (stars, hot pixels).
fn sigma_clipped_median(
    image: &ImageData,
    x0: usize, y0: usize, x1: usize, y1: usize,
    channel: usize,
) -> f32 {
    let mut values: Vec<f32> = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            let v = image.get(x, y, channel);
            if v.is_finite() && v.abs() > 0.5 {
                // Skip zero/near-zero pixels (alignment artifacts)
                values.push(v);
            }
        }
    }

    if values.is_empty() {
        return 0.0;
    }

    // 3 iterations of sigma clipping
    for _ in 0..3 {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = values.len();
        if n < 5 {
            break;
        }

        let median = values[n / 2];
        let mut devs: Vec<f32> = values.iter().map(|&v| (v - median).abs()).collect();
        devs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let sigma = 1.4826 * devs[n / 2];

        let lo = median - 2.0 * sigma;
        let hi = median + 2.0 * sigma;
        values.retain(|&v| v >= lo && v <= hi);
    }

    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}
