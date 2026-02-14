//! Star detection using threshold-based blob detection with centroid refinement.
//!
//! Algorithm:
//! 1. Estimate background level using sigma-clipped median in a grid
//! 2. Threshold: mark pixels significantly above background (> bg + threshold_sigma * sigma)
//! 3. Connected component labeling (flood fill) to find blobs
//! 4. Filter by size (reject too small / too large)
//! 5. Refine positions with center-of-mass centroid calculation
//! 6. Compute Half Flux Radius (HFR) for each star

use crate::error::Result;
use crate::image::ImageData;

/// A detected star with sub-pixel position and quality metrics.
#[derive(Debug, Clone)]
pub struct Star {
    /// Sub-pixel X coordinate (center of mass).
    pub x: f64,
    /// Sub-pixel Y coordinate (center of mass).
    pub y: f64,
    /// Total flux (sum of background-subtracted pixel values).
    pub flux: f64,
    /// Half Flux Radius — radius enclosing half the total flux.
    pub hfr: f64,
    /// Peak pixel value.
    pub peak: f64,
    /// Number of pixels in the detected blob.
    pub pixel_count: usize,
}

/// Parameters for star detection.
#[derive(Debug, Clone)]
pub struct DetectionParams {
    /// Number of sigma above background to threshold. Default: 5.0.
    pub threshold_sigma: f64,
    /// Minimum number of pixels for a valid star. Default: 5.
    pub min_star_size: usize,
    /// Maximum number of pixels for a valid star. Default: 500.
    pub max_star_size: usize,
    /// Maximum HFR for a valid star (rejects large blobs). Default: 20.0.
    pub max_hfr: f64,
    /// Grid size for background estimation. Default: 64.
    pub background_grid_size: usize,
    /// Number of sigma-clip iterations for background estimation. Default: 3.
    pub background_clip_iterations: usize,
    /// Sigma for background clipping. Default: 3.0.
    pub background_clip_sigma: f64,
}

impl Default for DetectionParams {
    fn default() -> Self {
        DetectionParams {
            threshold_sigma: 5.0,
            min_star_size: 5,
            max_star_size: 500,
            max_hfr: 20.0,
            background_grid_size: 64,
            background_clip_iterations: 3,
            background_clip_sigma: 3.0,
        }
    }
}

/// Background model with per-pixel estimated background and noise level.
#[allow(dead_code)]
struct BackgroundModel {
    /// Estimated background level per pixel.
    background: Vec<f32>,
    /// Global noise estimate (sigma).
    sigma: f64,
    width: usize,
    height: usize,
}

/// Detect stars in a mono image (or the luminance of a color image).
pub fn detect_stars(image: &ImageData, params: &DetectionParams) -> Result<Vec<Star>> {
    // Convert to mono luminance if needed
    let mono = if image.channels == 1 {
        image.clone()
    } else {
        image.to_luminance()?
    };

    let w = mono.width;
    let h = mono.height;

    log::info!(
        "Detecting stars in {}x{} image (threshold: {}σ)",
        w,
        h,
        params.threshold_sigma
    );

    // Step 1: Estimate background
    let bg_model = estimate_background(&mono, params)?;

    // Step 2: Create binary mask of pixels above threshold
    let threshold_offset = params.threshold_sigma * bg_model.sigma;
    let mut mask = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let val = mono.get(x, y, 0) as f64;
            let bg = bg_model.background[y * w + x] as f64;
            mask[y * w + x] = val > bg + threshold_offset;
        }
    }

    // Step 3: Connected component labeling (flood fill)
    let components = find_connected_components(&mask, w, h);

    // Step 4 & 5: Filter components and compute star properties
    let mut stars = Vec::new();
    for component in &components {
        if component.len() < params.min_star_size || component.len() > params.max_star_size {
            continue;
        }

        let star = compute_star_properties(component, &mono, &bg_model);

        if star.hfr > params.max_hfr {
            continue;
        }

        // Reject stars too close to the edge (within 5 pixels)
        if star.x < 5.0 || star.x > (w - 5) as f64 || star.y < 5.0 || star.y > (h - 5) as f64 {
            continue;
        }

        stars.push(star);
    }

    // Sort by flux (brightest first)
    stars.sort_by(|a, b| b.flux.partial_cmp(&a.flux).unwrap_or(std::cmp::Ordering::Equal));

    log::info!("Detected {} stars", stars.len());
    Ok(stars)
}

/// Estimate background level using sigma-clipped statistics in a grid.
fn estimate_background(image: &ImageData, params: &DetectionParams) -> Result<BackgroundModel> {
    let w = image.width;
    let h = image.height;
    let grid = params.background_grid_size;

    let grid_w = (w + grid - 1) / grid;
    let grid_h = (h + grid - 1) / grid;

    // Compute median and sigma for each grid cell
    let mut grid_median = vec![0.0f64; grid_w * grid_h];
    let mut grid_sigma = vec![0.0f64; grid_w * grid_h];

    for gy in 0..grid_h {
        for gx in 0..grid_w {
            let x0 = gx * grid;
            let y0 = gy * grid;
            let x1 = (x0 + grid).min(w);
            let y1 = (y0 + grid).min(h);

            // Collect pixel values in this cell
            let mut values: Vec<f64> = Vec::new();
            for y in y0..y1 {
                for x in x0..x1 {
                    values.push(image.get(x, y, 0) as f64);
                }
            }

            // Sigma-clipped statistics
            let (med, sig) = sigma_clipped_stats(
                &mut values,
                params.background_clip_sigma,
                params.background_clip_iterations,
            );
            grid_median[gy * grid_w + gx] = med;
            grid_sigma[gy * grid_w + gx] = sig;
        }
    }

    // Bilinear interpolation of grid values to full resolution
    let background = interpolate_grid(&grid_median, grid_w, grid_h, grid, w, h);

    // Global sigma estimate (median of grid sigmas)
    let mut sigmas: Vec<f64> = grid_sigma.iter().copied().filter(|&s| s > 0.0).collect();
    sigmas.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let global_sigma = if sigmas.is_empty() {
        1.0
    } else {
        sigmas[sigmas.len() / 2]
    };

    Ok(BackgroundModel {
        background,
        sigma: global_sigma,
        width: w,
        height: h,
    })
}

/// Compute sigma-clipped median and standard deviation.
fn sigma_clipped_stats(values: &mut Vec<f64>, clip_sigma: f64, iterations: usize) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }

    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut working = values.clone();

    for _ in 0..iterations {
        if working.is_empty() {
            break;
        }

        let median = working[working.len() / 2];
        let sigma = mad_sigma(&working, median);

        if sigma <= 0.0 {
            break;
        }

        let lo = median - clip_sigma * sigma;
        let hi = median + clip_sigma * sigma;
        working.retain(|&v| v >= lo && v <= hi);
    }

    if working.is_empty() {
        return (0.0, 0.0);
    }

    let median = working[working.len() / 2];
    let sigma = mad_sigma(&working, median);
    (median, sigma)
}

/// Estimate sigma from the Median Absolute Deviation (MAD).
/// sigma ≈ 1.4826 * MAD
fn mad_sigma(values: &[f64], median: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut deviations: Vec<f64> = values.iter().map(|&v| (v - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mad = deviations[deviations.len() / 2];
    1.4826 * mad
}

/// Bilinear interpolation of a low-res grid to full resolution.
fn interpolate_grid(
    grid: &[f64],
    grid_w: usize,
    grid_h: usize,
    cell_size: usize,
    out_w: usize,
    out_h: usize,
) -> Vec<f32> {
    let mut result = vec![0.0f32; out_w * out_h];

    for y in 0..out_h {
        for x in 0..out_w {
            // Map pixel to grid coordinates (center of each cell)
            let gx = (x as f64 + 0.5) / cell_size as f64 - 0.5;
            let gy = (y as f64 + 0.5) / cell_size as f64 - 0.5;

            let gx0 = (gx.floor() as isize).clamp(0, grid_w as isize - 1) as usize;
            let gy0 = (gy.floor() as isize).clamp(0, grid_h as isize - 1) as usize;
            let gx1 = (gx0 + 1).min(grid_w - 1);
            let gy1 = (gy0 + 1).min(grid_h - 1);

            let fx = (gx - gx0 as f64).clamp(0.0, 1.0);
            let fy = (gy - gy0 as f64).clamp(0.0, 1.0);

            let v00 = grid[gy0 * grid_w + gx0];
            let v10 = grid[gy0 * grid_w + gx1];
            let v01 = grid[gy1 * grid_w + gx0];
            let v11 = grid[gy1 * grid_w + gx1];

            let val = v00 * (1.0 - fx) * (1.0 - fy)
                + v10 * fx * (1.0 - fy)
                + v01 * (1.0 - fx) * fy
                + v11 * fx * fy;

            result[y * out_w + x] = val as f32;
        }
    }

    result
}

/// Find connected components in a binary mask using flood fill.
fn find_connected_components(
    mask: &[bool],
    w: usize,
    h: usize,
) -> Vec<Vec<(usize, usize)>> {
    let mut visited = vec![false; w * h];
    let mut components = Vec::new();

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if mask[idx] && !visited[idx] {
                let mut component = Vec::new();
                flood_fill(mask, &mut visited, w, h, x, y, &mut component);
                components.push(component);
            }
        }
    }

    components
}

/// Flood-fill from (start_x, start_y) collecting all connected true pixels.
fn flood_fill(
    mask: &[bool],
    visited: &mut [bool],
    w: usize,
    h: usize,
    start_x: usize,
    start_y: usize,
    component: &mut Vec<(usize, usize)>,
) {
    let mut stack = vec![(start_x, start_y)];

    while let Some((x, y)) = stack.pop() {
        let idx = y * w + x;
        if visited[idx] {
            continue;
        }
        visited[idx] = true;
        component.push((x, y));

        // 8-connected neighbors
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx >= 0 && nx < w as i32 && ny >= 0 && ny < h as i32 {
                    let nidx = ny as usize * w + nx as usize;
                    if mask[nidx] && !visited[nidx] {
                        stack.push((nx as usize, ny as usize));
                    }
                }
            }
        }
    }
}

/// Compute star properties from a connected component of pixels.
fn compute_star_properties(
    pixels: &[(usize, usize)],
    image: &ImageData,
    bg_model: &BackgroundModel,
) -> Star {
    let w = bg_model.width;

    // Compute center-of-mass (flux-weighted centroid)
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut total_flux = 0.0f64;
    let mut peak = 0.0f64;

    for &(x, y) in pixels {
        let val = image.get(x, y, 0) as f64;
        let bg = bg_model.background[y * w + x] as f64;
        let flux = (val - bg).max(0.0);

        sum_x += x as f64 * flux;
        sum_y += y as f64 * flux;
        total_flux += flux;

        if val > peak {
            peak = val;
        }
    }

    let cx = if total_flux > 0.0 {
        sum_x / total_flux
    } else {
        pixels.iter().map(|&(x, _)| x as f64).sum::<f64>() / pixels.len() as f64
    };
    let cy = if total_flux > 0.0 {
        sum_y / total_flux
    } else {
        pixels.iter().map(|&(_, y)| y as f64).sum::<f64>() / pixels.len() as f64
    };

    // Compute HFR (Half Flux Radius)
    let hfr = compute_hfr(pixels, image, bg_model, cx, cy);

    Star {
        x: cx,
        y: cy,
        flux: total_flux,
        hfr,
        peak,
        pixel_count: pixels.len(),
    }
}

/// Compute the Half Flux Radius — the radius from the centroid that
/// encloses exactly half the total flux.
fn compute_hfr(
    pixels: &[(usize, usize)],
    image: &ImageData,
    bg_model: &BackgroundModel,
    cx: f64,
    cy: f64,
) -> f64 {
    let w = bg_model.width;

    // Collect (distance, flux) pairs
    let mut dist_flux: Vec<(f64, f64)> = pixels
        .iter()
        .map(|&(x, y)| {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let val = image.get(x, y, 0) as f64;
            let bg = bg_model.background[y * w + x] as f64;
            let flux = (val - bg).max(0.0);
            (dist, flux)
        })
        .collect();

    // Sort by distance
    dist_flux.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let total_flux: f64 = dist_flux.iter().map(|&(_, f)| f).sum();
    let half_flux = total_flux / 2.0;

    // Find the radius at which cumulative flux reaches half
    let mut cumulative = 0.0;
    for &(dist, flux) in &dist_flux {
        cumulative += flux;
        if cumulative >= half_flux {
            return dist;
        }
    }

    // Fallback
    dist_flux.last().map(|&(d, _)| d).unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sigma_clipped_stats() {
        let mut values: Vec<f64> = (0..100).map(|i| i as f64).collect();
        // Add an outlier
        values.push(10000.0);
        let (median, sigma) = sigma_clipped_stats(&mut values, 3.0, 3);
        // Median of 0..100 should be ~50
        assert!((median - 50.0).abs() < 5.0);
        assert!(sigma > 0.0);
    }

    #[test]
    fn test_detect_stars_on_blank_image() {
        let img = ImageData::new(100, 100, 1);
        let params = DetectionParams::default();
        let stars = detect_stars(&img, &params).unwrap();
        assert!(stars.is_empty(), "No stars should be detected in a blank image");
    }

    #[test]
    fn test_detect_stars_synthetic() {
        // Create an image with a single bright "star" blob
        let mut img = ImageData::new(100, 100, 1);

        // Set a uniform background of 100
        for v in &mut img.data {
            *v = 100.0;
        }

        // Add a Gaussian-like star at (50, 50)
        for dy in -5i32..=5 {
            for dx in -5i32..=5 {
                let r2 = (dx * dx + dy * dy) as f64;
                let intensity = 5000.0 * (-r2 / 8.0).exp();
                let x = (50 + dx) as usize;
                let y = (50 + dy) as usize;
                img.data[y * 100 + x] += intensity as f32;
            }
        }

        let params = DetectionParams {
            threshold_sigma: 3.0,
            min_star_size: 3,
            max_star_size: 200,
            ..Default::default()
        };

        let stars = detect_stars(&img, &params).unwrap();
        assert!(!stars.is_empty(), "Should detect at least one star");

        // The detected star should be near (50, 50)
        let star = &stars[0];
        assert!((star.x - 50.0).abs() < 2.0, "Star X should be near 50, got {}", star.x);
        assert!((star.y - 50.0).abs() < 2.0, "Star Y should be near 50, got {}", star.y);
        assert!(star.flux > 0.0, "Star flux should be positive");
    }

    #[test]
    fn test_flood_fill() {
        let mask = vec![
            false, true, true, false,
            false, true, false, false,
            false, false, false, true,
            false, false, false, false,
        ];
        let components = find_connected_components(&mask, 4, 4);
        // Should find 2 components: {(1,0),(2,0),(1,1)} and {(3,2)}
        assert_eq!(components.len(), 2);
    }
}
