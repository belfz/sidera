//! Image alignment via star pattern matching and homography estimation.
//!
//! Algorithm:
//! 1. Form triangles from the brightest stars in each image
//! 2. Match triangles between reference and target using side-ratio descriptors
//! 3. Extract point correspondences from matched triangles
//! 4. Estimate homography using RANSAC + Direct Linear Transform (DLT)
//! 5. Apply the homography to warp the target image onto the reference frame

use crate::error::{AstroError, Result};
use crate::image::ImageData;
use crate::star_detection::Star;
use nalgebra::{DMatrix, Matrix3, Vector3};

/// A 3×3 homography matrix for projective transformation.
/// Maps points from the target frame to the reference frame:
///   p_ref = H * p_target  (in homogeneous coordinates)
#[derive(Debug, Clone)]
pub struct Transform {
    pub matrix: Matrix3<f64>,
}

impl Transform {
    /// Identity transform (no change).
    pub fn identity() -> Self {
        Transform {
            matrix: Matrix3::identity(),
        }
    }

    /// Apply the transform to a single point.
    pub fn transform_point(&self, x: f64, y: f64) -> (f64, f64) {
        let p = Vector3::new(x, y, 1.0);
        let tp = self.matrix * p;
        (tp[0] / tp[2], tp[1] / tp[2])
    }

    /// Compute the inverse transform.
    pub fn inverse(&self) -> Result<Transform> {
        self.matrix
            .try_inverse()
            .map(|m| Transform { matrix: m })
            .ok_or_else(|| AstroError::Alignment("Transform matrix is not invertible".into()))
    }
}

/// Parameters for the alignment algorithm.
#[derive(Debug, Clone)]
pub struct AlignmentParams {
    /// Maximum number of brightest stars to use for matching. Default: 50.
    pub max_stars: usize,
    /// Tolerance for triangle side-ratio matching. Default: 0.01.
    pub triangle_tolerance: f64,
    /// Number of RANSAC iterations. Default: 1000.
    pub ransac_iterations: usize,
    /// RANSAC inlier threshold in pixels. Default: 3.0.
    pub ransac_threshold: f64,
    /// Minimum number of inliers for a valid transform. Default: 6.
    pub min_inliers: usize,
}

impl Default for AlignmentParams {
    fn default() -> Self {
        AlignmentParams {
            max_stars: 100,
            triangle_tolerance: 0.05,
            ransac_iterations: 2000,
            ransac_threshold: 5.0,
            min_inliers: 4,
        }
    }
}

/// A triangle descriptor formed by three stars, characterized by its
/// side ratios (invariant to scale, translation, and rotation).
#[derive(Debug, Clone)]
struct TriangleDescriptor {
    /// Indices of the three stars (sorted by side lengths).
    star_indices: [usize; 3],
    /// Ratios of the two shorter sides to the longest side.
    /// Always in [0, 1], sorted: ratio_a <= ratio_b.
    ratio_a: f64,
    ratio_b: f64,
}

/// Compute the alignment transform from a set of reference stars to target stars.
///
/// Uses a three-phase approach:
/// 1. **Translation model** — for tracked telescopes where the shift between
///    frames is small. Computes median (dx, dy) from NN matches. No SVD,
///    numerically bulletproof.
/// 2. **Full homography via RANSAC** — if translation model has too many
///    outliers, estimate a full 8-DOF homography from the NN matches.
/// 3. **Triangle matching (fallback)** — for larger shifts or rotations where
///    NN matching fails entirely.
///
/// Returns a transform that maps target coordinates to reference coordinates.
pub fn compute_alignment(
    ref_stars: &[Star],
    target_stars: &[Star],
    params: &AlignmentParams,
) -> Result<Transform> {
    let n_ref = ref_stars.len().min(params.max_stars);
    let n_target = target_stars.len().min(params.max_stars);

    if n_ref < 3 {
        return Err(AstroError::InsufficientStars { found: n_ref, need: 3 });
    }
    if n_target < 3 {
        return Err(AstroError::InsufficientStars { found: n_target, need: 3 });
    }

    let ref_stars = &ref_stars[..n_ref];
    let target_stars = &target_stars[..n_target];

    // Phase 1: Try nearest-neighbor matching (works for small shifts)
    let search_radius = 150.0; // pixels
    let nn_matches = nearest_neighbor_match(ref_stars, target_stars, search_radius);

    if nn_matches.len() >= 4 {
        log::info!(
            "Nearest-neighbor: {} matches (of {} ref, {} target)",
            nn_matches.len(), n_ref, n_target
        );

        // Phase 1a: Try simple translation model first (most robust for tracked data)
        let translation = estimate_translation(&nn_matches);
        let translation_inliers = count_inliers(&nn_matches, &translation, params.ransac_threshold);

        let inlier_ratio = translation_inliers as f64 / nn_matches.len() as f64;
        log::info!(
            "Translation model: dx={:.2}, dy={:.2}, inliers={}/{} ({:.0}%)",
            translation.matrix[(0, 2)], translation.matrix[(1, 2)],
            translation_inliers, nn_matches.len(), inlier_ratio * 100.0
        );

        if inlier_ratio > 0.5 && translation_inliers >= params.min_inliers {
            return Ok(translation);
        }

        // Phase 1b: Try full homography via RANSAC on NN matches
        match ransac_homography(&nn_matches, params) {
            Ok(t) => return Ok(t),
            Err(e) => {
                log::warn!("NN-RANSAC failed: {}, trying triangle matching", e);
            }
        }
    }

    // Phase 2: Fall back to triangle matching for larger shifts
    log::info!("Falling back to triangle matching...");
    let ref_triangles = build_triangles(ref_stars);
    let target_triangles = build_triangles(target_stars);

    let matches = match_triangles(
        &ref_triangles, &target_triangles,
        ref_stars, target_stars,
        params.triangle_tolerance,
    );

    if matches.len() < 4 {
        return Err(AstroError::Alignment(format!(
            "Only {} point matches found, need at least 4",
            matches.len()
        )));
    }

    ransac_homography(&matches, params)
}

/// Estimate a pure translation transform from point correspondences.
///
/// Uses the median of per-match (dx, dy) shifts, which is robust to outliers.
/// This is the most reliable model for tracked telescope data where frames
/// shift by a few pixels with negligible rotation.
fn estimate_translation(matches: &[((f64, f64), (f64, f64))]) -> Transform {
    let mut dxs: Vec<f64> = matches.iter().map(|((rx, _), (tx, _))| rx - tx).collect();
    let mut dys: Vec<f64> = matches.iter().map(|((_, ry), (_, ty))| ry - ty).collect();

    dxs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    dys.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let dx = dxs[dxs.len() / 2];
    let dy = dys[dys.len() / 2];

    Transform {
        matrix: Matrix3::new(
            1.0, 0.0, dx,
            0.0, 1.0, dy,
            0.0, 0.0, 1.0,
        ),
    }
}

/// Count inliers for a given transform against point correspondences.
fn count_inliers(
    matches: &[((f64, f64), (f64, f64))],
    transform: &Transform,
    threshold: f64,
) -> usize {
    matches.iter().filter(|&&((rx, ry), (tx, ty))| {
        let (px, py) = transform.transform_point(tx, ty);
        let err = ((px - rx).powi(2) + (py - ry).powi(2)).sqrt();
        err < threshold
    }).count()
}

/// Match stars between reference and target using nearest-neighbor within a search radius.
/// For each reference star, finds the closest target star. If the distance is within
/// the radius AND the match is reciprocal (target's nearest ref is also this ref star),
/// it's accepted. This produces very reliable matches for tracked telescope data.
fn nearest_neighbor_match(
    ref_stars: &[Star],
    target_stars: &[Star],
    radius: f64,
) -> Vec<((f64, f64), (f64, f64))> {
    let mut matches = Vec::new();

    for rs in ref_stars {
        // Find nearest target star
        let mut best_dist = f64::INFINITY;
        let mut best_ts: Option<&Star> = None;

        for ts in target_stars {
            let dx = rs.x - ts.x;
            let dy = rs.y - ts.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < best_dist {
                best_dist = dist;
                best_ts = Some(ts);
            }
        }

        if best_dist > radius {
            continue;
        }
        let ts = best_ts.unwrap();

        // Verify reciprocal: is this ref star also the nearest to the target star?
        let mut recip_best_dist = f64::INFINITY;
        let mut recip_best_x = 0.0;
        let mut recip_best_y = 0.0;
        for rs2 in ref_stars {
            let dx = ts.x - rs2.x;
            let dy = ts.y - rs2.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < recip_best_dist {
                recip_best_dist = dist;
                recip_best_x = rs2.x;
                recip_best_y = rs2.y;
            }
        }

        // Check reciprocal (the nearest ref star to our target should be our ref star)
        if (recip_best_x - rs.x).abs() < 0.1 && (recip_best_y - rs.y).abs() < 0.1 {
            matches.push(((rs.x, rs.y), (ts.x, ts.y)));
        }
    }

    matches
}

/// Build triangle descriptors from a list of stars.
/// We limit to triangles formed by the first N stars to keep computation reasonable.
fn build_triangles(stars: &[Star]) -> Vec<TriangleDescriptor> {
    let n = stars.len().min(50); // Limit to avoid combinatorial explosion
    let mut triangles = Vec::new();

    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                let d_ij = star_distance(&stars[i], &stars[j]);
                let d_jk = star_distance(&stars[j], &stars[k]);
                let d_ik = star_distance(&stars[i], &stars[k]);

                // Sort sides: longest is the reference
                let mut sides = [(d_ij, i, j), (d_jk, j, k), (d_ik, i, k)];
                sides.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

                let longest = sides[2].0;
                if longest < 1e-10 {
                    continue; // Degenerate triangle
                }

                let ratio_a = sides[0].0 / longest;
                let ratio_b = sides[1].0 / longest;

                // Find the vertex opposite the longest side
                let opposite = {
                    let all = [i, j, k];
                    *all.iter()
                        .find(|&&v| v != sides[2].1 && v != sides[2].2)
                        .unwrap()
                };
                let star_indices = [sides[2].1, sides[2].2, opposite];

                triangles.push(TriangleDescriptor {
                    star_indices,
                    ratio_a,
                    ratio_b,
                });
            }
        }
    }

    triangles
}

/// Match triangles between reference and target based on side ratios.
/// Returns a list of (ref_star_index, target_star_index) point correspondences.
fn match_triangles(
    ref_tris: &[TriangleDescriptor],
    target_tris: &[TriangleDescriptor],
    ref_stars: &[Star],
    target_stars: &[Star],
    tolerance: f64,
) -> Vec<((f64, f64), (f64, f64))> {
    let mut vote_map: std::collections::HashMap<(usize, usize), usize> =
        std::collections::HashMap::new();

    for rt in ref_tris {
        for tt in target_tris {
            // Check if triangle descriptors match within tolerance
            if (rt.ratio_a - tt.ratio_a).abs() < tolerance
                && (rt.ratio_b - tt.ratio_b).abs() < tolerance
            {
                // These triangles match — vote for the star correspondences
                // Try both orientations of the matched triangle
                for &(ri, ti) in &[
                    (rt.star_indices[0], tt.star_indices[0]),
                    (rt.star_indices[1], tt.star_indices[1]),
                    (rt.star_indices[2], tt.star_indices[2]),
                ] {
                    *vote_map.entry((ri, ti)).or_insert(0) += 1;
                }
            }
        }
    }

    // Keep correspondences that received multiple votes (more robust)
    let mut matches: Vec<((f64, f64), (f64, f64))> = Vec::new();
    let mut used_ref: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut used_target: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // Sort by vote count (descending)
    let mut votes: Vec<_> = vote_map.into_iter().collect();
    votes.sort_by(|a, b| b.1.cmp(&a.1));

    for ((ri, ti), _count) in votes {
        if used_ref.contains(&ri) || used_target.contains(&ti) {
            continue; // Each star can only match once
        }
        used_ref.insert(ri);
        used_target.insert(ti);

        let rs = &ref_stars[ri];
        let ts = &target_stars[ti];
        matches.push(((rs.x, rs.y), (ts.x, ts.y)));
    }

    matches
}

/// Euclidean distance between two stars.
fn star_distance(a: &Star, b: &Star) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

/// RANSAC-based robust homography estimation.
fn ransac_homography(
    matches: &[((f64, f64), (f64, f64))],
    params: &AlignmentParams,
) -> Result<Transform> {

    let n = matches.len();
    if n < 4 {
        return Err(AstroError::Alignment(
            "Need at least 4 point matches for homography".into(),
        ));
    }

    let mut best_inliers = Vec::new();
    let mut best_transform = None;

    // Simple deterministic PRNG based on iteration
    for iter in 0..params.ransac_iterations {
        // Pick 4 random matches using a simple hash-based selection
        let indices = pick_random_4(n, iter);

        let sample: Vec<_> = indices.iter().map(|&i| matches[i]).collect();

        // Estimate homography from 4 points
        let h = match estimate_homography_dlt(&sample) {
            Some(h) => h,
            None => continue,
        };

        // Count inliers
        let inliers: Vec<usize> = (0..n)
            .filter(|&i| {
                let ((rx, ry), (tx, ty)) = matches[i];
                let (px, py) = h.transform_point(tx, ty);
                let err = ((px - rx).powi(2) + (py - ry).powi(2)).sqrt();
                err < params.ransac_threshold
            })
            .collect();

        if inliers.len() > best_inliers.len() {
            best_inliers = inliers;
            best_transform = Some(h);
        }

        // Early termination if we have a very good fit
        if best_inliers.len() > n * 90 / 100 {
            break;
        }
    }

    if best_inliers.len() < params.min_inliers {
        return Err(AstroError::Alignment(format!(
            "RANSAC found only {} inliers, need at least {}",
            best_inliers.len(),
            params.min_inliers
        )));
    }

    log::info!(
        "RANSAC: {} inliers out of {} matches",
        best_inliers.len(),
        n
    );

    // Re-estimate homography using all inliers
    let inlier_matches: Vec<_> = best_inliers.iter().map(|&i| matches[i]).collect();
    let refined = estimate_homography_dlt(&inlier_matches)
        .unwrap_or_else(|| best_transform.unwrap());

    Ok(refined)
}

/// Pick 4 distinct random indices from [0, n) using a simple deterministic approach.
fn pick_random_4(n: usize, seed: usize) -> [usize; 4] {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Simple hash-based pseudo-random selection
    let hash = |v: usize| -> usize {
        let mut h = DefaultHasher::new();
        v.hash(&mut h);
        h.finish() as usize
    };

    let mut indices = [0usize; 4];
    let mut i = 0;
    let mut attempt = seed * 4;
    while i < 4 {
        let idx = hash(attempt) % n;
        attempt += 1;
        if !indices[..i].contains(&idx) {
            indices[i] = idx;
            i += 1;
        }
    }
    indices
}

/// Compute Hartley normalization: translate centroid to origin, scale so
/// average distance from origin is √2. Returns (normalized_points, T_matrix).
fn hartley_normalize(points: &[(f64, f64)]) -> (Vec<(f64, f64)>, Matrix3<f64>) {
    let n = points.len() as f64;
    let cx: f64 = points.iter().map(|p| p.0).sum::<f64>() / n;
    let cy: f64 = points.iter().map(|p| p.1).sum::<f64>() / n;

    let mean_dist: f64 = points.iter()
        .map(|p| ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt())
        .sum::<f64>() / n;

    let scale = if mean_dist > 1e-10 { std::f64::consts::SQRT_2 / mean_dist } else { 1.0 };

    let normalized: Vec<(f64, f64)> = points.iter()
        .map(|p| ((p.0 - cx) * scale, (p.1 - cy) * scale))
        .collect();

    let t = Matrix3::new(
        scale, 0.0,   -cx * scale,
        0.0,   scale, -cy * scale,
        0.0,   0.0,   1.0,
    );

    (normalized, t)
}

/// Estimate a homography from N >= 4 point correspondences using
/// the Direct Linear Transform (DLT) algorithm with Hartley normalization.
///
/// Uses A^T*A eigendecomposition instead of direct SVD of A. This is critical
/// because nalgebra computes a thin SVD: for an 8×9 matrix (4 matches),
/// V^T is only 8×9, missing the null-space row we need. Computing SVD of
/// A^T*A (which is always 9×9) avoids this issue entirely.
fn estimate_homography_dlt(matches: &[((f64, f64), (f64, f64))]) -> Option<Transform> {
    let n = matches.len();
    if n < 4 {
        return None;
    }

    // Separate ref and target points
    let ref_pts: Vec<(f64, f64)> = matches.iter().map(|m| m.0).collect();
    let tgt_pts: Vec<(f64, f64)> = matches.iter().map(|m| m.1).collect();

    // Hartley normalization: crucial for numerical stability
    let (ref_norm, t_ref) = hartley_normalize(&ref_pts);
    let (tgt_norm, t_tgt) = hartley_normalize(&tgt_pts);

    // Build the 2N×9 matrix A using normalized coordinates
    let mut a_data = vec![0.0f64; 2 * n * 9];

    for (i, (rn, tn)) in ref_norm.iter().zip(tgt_norm.iter()).enumerate() {
        let (rx, ry) = *rn;
        let (tx, ty) = *tn;
        let row1 = i * 2;
        let row2 = i * 2 + 1;

        a_data[row1 * 9 + 0] = -tx;
        a_data[row1 * 9 + 1] = -ty;
        a_data[row1 * 9 + 2] = -1.0;
        a_data[row1 * 9 + 6] = rx * tx;
        a_data[row1 * 9 + 7] = rx * ty;
        a_data[row1 * 9 + 8] = rx;

        a_data[row2 * 9 + 3] = -tx;
        a_data[row2 * 9 + 4] = -ty;
        a_data[row2 * 9 + 5] = -1.0;
        a_data[row2 * 9 + 6] = ry * tx;
        a_data[row2 * 9 + 7] = ry * ty;
        a_data[row2 * 9 + 8] = ry;
    }

    let a = DMatrix::from_row_slice(2 * n, 9, &a_data);

    // Compute A^T * A (always 9×9) and find its eigenvector for the smallest eigenvalue.
    // This avoids the thin-SVD problem where nalgebra's SVD of an 8×9 matrix
    // only returns 8 rows of V^T, missing the null-space vector we need.
    let ata = a.transpose() * &a;

    // Symmetric eigendecomposition — eigenvalues in ascending order,
    // so the first eigenvector corresponds to the smallest eigenvalue (null space).
    let eigen = ata.symmetric_eigen();

    // Find index of smallest eigenvalue
    let min_idx = eigen.eigenvalues.imin();
    let h_vec: Vec<f64> = (0..9).map(|i| eigen.eigenvectors[(i, min_idx)]).collect();

    let h_norm = Matrix3::new(
        h_vec[0], h_vec[1], h_vec[2],
        h_vec[3], h_vec[4], h_vec[5],
        h_vec[6], h_vec[7], h_vec[8],
    );

    // De-normalize: H = T_ref^{-1} * H_norm * T_tgt
    let t_ref_inv = t_ref.try_inverse()?;
    let matrix = t_ref_inv * h_norm * t_tgt;

    // Normalize so that h[2][2] = 1
    let scale = matrix[(2, 2)];
    if scale.abs() < 1e-10 {
        return None;
    }
    let matrix = matrix / scale;

    Some(Transform { matrix })
}

/// Apply a homography transform to warp an image.
/// Uses bilinear interpolation for sub-pixel accuracy.
///
/// The output image has the same dimensions as the input.
/// The transform maps target coordinates to reference coordinates,
/// so we apply the inverse to map reference pixels back to target pixels.
pub fn apply_transform(image: &ImageData, transform: &Transform) -> Result<ImageData> {
    let w = image.width;
    let h = image.height;
    let channels = image.channels;

    let inv = transform.inverse()?;

    let mut output = ImageData::new(w, h, channels);

    // For each output pixel, find the corresponding input pixel
    for oy in 0..h {
        for ox in 0..w {
            let (sx, sy) = inv.transform_point(ox as f64, oy as f64);

            // Bilinear interpolation
            for c in 0..channels {
                let val = bilinear_sample(image, sx, sy, c);
                output.set(ox, oy, c, val);
            }
        }
    }

    Ok(output)
}

/// Bilinear interpolation sampling of a single channel.
fn bilinear_sample(image: &ImageData, x: f64, y: f64, channel: usize) -> f32 {
    let w = image.width as f64;
    let h = image.height as f64;

    if x < 0.0 || x >= w - 1.0 || y < 0.0 || y >= h - 1.0 {
        return 0.0; // Out of bounds
    }

    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = x0 + 1;
    let y1 = y0 + 1;

    if x1 >= image.width || y1 >= image.height {
        return image.get(x0, y0, channel);
    }

    let fx = x - x0 as f64;
    let fy = y - y0 as f64;

    let v00 = image.get(x0, y0, channel) as f64;
    let v10 = image.get(x1, y0, channel) as f64;
    let v01 = image.get(x0, y1, channel) as f64;
    let v11 = image.get(x1, y1, channel) as f64;

    let val = v00 * (1.0 - fx) * (1.0 - fy)
        + v10 * fx * (1.0 - fy)
        + v01 * (1.0 - fx) * fy
        + v11 * fx * fy;

    val as f32
}

/// Align multiple images to a reference frame.
/// The first image is used as the reference.
/// Returns the aligned images (reference is unchanged, others are warped).
pub fn align_images(
    images: &[ImageData],
    star_lists: &[Vec<Star>],
    params: &AlignmentParams,
) -> Result<Vec<ImageData>> {
    if images.is_empty() {
        return Err(AstroError::NoFrames {
            operation: "alignment".into(),
        });
    }
    if images.len() != star_lists.len() {
        return Err(AstroError::Alignment(
            "Number of images and star lists must match".into(),
        ));
    }

    log::info!("Aligning {} images to reference frame", images.len());

    let ref_stars = &star_lists[0];
    let mut aligned = Vec::with_capacity(images.len());

    // Reference image stays as-is
    aligned.push(images[0].clone());

    // Align each subsequent image — skip frames that fail alignment
    let mut skipped = 0;
    for i in 1..images.len() {
        log::info!("Aligning image {}/{}", i + 1, images.len());

        match compute_alignment(ref_stars, &star_lists[i], params) {
            Ok(transform) => match apply_transform(&images[i], &transform) {
                Ok(warped) => aligned.push(warped),
                Err(e) => {
                    log::warn!("Skipping image {}: warp failed: {}", i + 1, e);
                    skipped += 1;
                }
            },
            Err(e) => {
                log::warn!("Skipping image {}: alignment failed: {}", i + 1, e);
                skipped += 1;
            }
        }
    }

    if aligned.is_empty() {
        return Err(AstroError::Alignment(
            "All frames failed alignment".into(),
        ));
    }

    if skipped > 0 {
        log::warn!(
            "Alignment complete: {} of {} frames aligned ({} skipped)",
            aligned.len(),
            images.len(),
            skipped
        );
    }

    Ok(aligned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_transform() {
        let t = Transform::identity();
        let (x, y) = t.transform_point(10.0, 20.0);
        assert!((x - 10.0).abs() < 1e-10);
        assert!((y - 20.0).abs() < 1e-10);
    }

    #[test]
    fn test_translation_homography() {
        // Test that DLT can recover a pure translation
        let dx = 5.0;
        let dy = -3.0;
        let matches: Vec<((f64, f64), (f64, f64))> = vec![
            ((10.0 + dx, 20.0 + dy), (10.0, 20.0)),
            ((30.0 + dx, 20.0 + dy), (30.0, 20.0)),
            ((10.0 + dx, 50.0 + dy), (10.0, 50.0)),
            ((30.0 + dx, 50.0 + dy), (30.0, 50.0)),
            ((20.0 + dx, 35.0 + dy), (20.0, 35.0)),
        ];

        let h = estimate_homography_dlt(&matches).expect("Should compute homography");
        let (px, py) = h.transform_point(15.0, 25.0);
        assert!(
            (px - (15.0 + dx)).abs() < 0.1,
            "Expected x={}, got {}",
            15.0 + dx,
            px
        );
        assert!(
            (py - (25.0 + dy)).abs() < 0.1,
            "Expected y={}, got {}",
            25.0 + dy,
            py
        );
    }

    #[test]
    fn test_bilinear_sample_center() {
        let mut img = ImageData::new(10, 10, 1);
        img.set(5, 5, 0, 100.0);
        // Sampling exactly at a pixel should return that value
        let val = bilinear_sample(&img, 5.0, 5.0, 0);
        assert!((val - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_bilinear_sample_between() {
        let mut img = ImageData::new(10, 10, 1);
        img.set(0, 0, 0, 0.0);
        img.set(1, 0, 0, 100.0);
        img.set(0, 1, 0, 0.0);
        img.set(1, 1, 0, 100.0);
        // Midpoint should be ~50
        let val = bilinear_sample(&img, 0.5, 0.5, 0);
        assert!((val - 50.0).abs() < 1e-3);
    }
}
