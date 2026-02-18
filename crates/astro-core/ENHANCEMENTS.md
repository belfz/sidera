# astro-core: Image Quality Enhancements

This document captures the full history of image quality improvements made to
the stacking pipeline, the reasoning behind each change, what remains to be
fixed, and ideas for future work. It is intended as context for future agentic
sessions — read this first before making changes to the pipeline.

---

## Table of Contents

1. [Starting Point — What Was Wrong](#starting-point)
2. [Completed Fixes](#completed-fixes)
   - [Color Cast: Purple/Magenta](#fix-magenta)
   - [Color Cast: Green](#fix-green)
   - [Faint Galaxy / Dark Image](#fix-faint)
   - [Edge Banding from Stacking Borders](#fix-banding)
   - [Calibration Safety Net](#fix-calibration)
3. [Current Pipeline Order](#pipeline-order)
4. [Known Remaining Issues](#remaining-issues)
5. [Ideas for Further Improvement](#future-ideas)

---

<a id="starting-point"></a>
## 1. Starting Point — What Was Wrong

The initial stacking pipeline produced a washed-out, grey image where the
target (M33, Triangulum Galaxy) was completely invisible. Subsequent
iterations introduced a series of color cast problems (purple, then green,
then magenta) and the galaxy remained stubbornly faint. The root causes
spanned nearly every stage of the pipeline:

- **Flat normalization** was global (single mean), not Bayer-aware. For a
  color-filter-array sensor (RGGB), this created per-channel gain errors that
  propagated as a persistent color shift.
- **Background neutralization** used multiplicative scaling, which distorted
  signal colors — a bright star's R/G/B ratios would change depending on the
  background level.
- **Normalization bounds** (the `[low, high]` range mapping raw ADU to [0,1])
  were either min/max (crushed by hot pixels) or percentile-based but too
  wide (P99.99% ≈ 700+ ADU from bright stars), compressing the galaxy signal
  into ~2% of the dynamic range.
- **Auto-stretch statistics** were contaminated by zero-fill pixels from
  alignment borders and gradient extraction artifacts, dragging the median to
  near-zero and producing a linear (m=0.5) stretch — effectively no stretch.
- **No chromatic noise reduction** — residual Bayer demosaic artifacts and
  per-channel noise produced a magenta cast that survived neutralization.
- **Fixed 8% crop** was either too aggressive (cutting into the galaxy) or
  not targeted enough (leaving color-banded edges that skewed gradient
  extraction and neutralization).

The test data: 400 light frames (30s, ISO-equivalent auto) from a DwarfLab
Dwarf 3 smart telescope, color CMOS sensor, Bayer RGGB pattern, FITS format
with 16-bit unsigned data (BITPIX=16, BZERO=32768).

---

<a id="completed-fixes"></a>
## 2. Completed Fixes

<a id="fix-magenta"></a>
### 2.1 Color Cast: Purple/Magenta

**Symptoms:** Stacked result had a strong purple/magenta tint across the
entire image, most visible in the sky background.

**Root causes:**
1. Flat normalization computed a single global mean and divided all channels
   by it. For RGGB Bayer data where G has 2× the photosites, this
   systematically under-corrected R and B.
2. Background neutralization used multiplicative correction
   (`pixel *= ref/channel_bg`), which amplified channel differences in
   bright areas, distorting star colors.
3. After fixing the above, a residual magenta cast remained from demosaic
   artifacts and per-channel noise in the Bayer data.

**Fixes applied:**
- **Bayer-aware flat normalization** (`calibration.rs`): The master flat is
  now normalized per Bayer channel — R, G (both G1 and G2), and B each get
  their own mean, so division by the flat corrects each filter's response
  independently.
- **Additive background neutralization** (`lib.rs: neutralize_background`):
  Replaced multiplicative scaling with additive correction. For each channel,
  compute the sigma-clipped median of the background, then shift
  (add/subtract a constant) so all three channels have the same background
  level. Uses green channel as reference (best SNR, standard in Siril/PI).
  Additive correction preserves the R-G-B *differences* within a star or
  galaxy — only the pedestal changes.
- **SCNR "Average Neutral"** (`lib.rs: scnr_average_neutral`): For each
  pixel where both R > G *and* B > G (i.e., the pixel is magenta), reduce R
  and B toward the average of their original value and G. Pixels that are
  genuinely red (R > G but B ≤ G) or blue (B > G but R ≤ G) are untouched.
  This is gentler than full SCNR (which clamps to G outright) and targets
  only the chromatic noise, not real object color.

**Key code locations:**
- `calibration.rs` — `normalize_flat()`, `normalize_flat_bayer()`
- `lib.rs` — `neutralize_background()`, `scnr_average_neutral()`

---

<a id="fix-green"></a>
### 2.2 Color Cast: Green

**Symptoms:** After fixing the magenta cast, the image turned green. The
green cast was subtle but clearly visible in the sky background.

**Root cause:** A redundant per-channel background equalization was added
inside `histogram.rs: robust_normalize` that conflicted with the additive
neutralization in the pipeline. The normalize step was shifting each channel
to align backgrounds *again*, but using different statistics (luminance-
weighted bounds), which favored green (71.5% luminance weight).

**Fix applied:**
- **Reverted `robust_normalize`** in `histogram.rs` to use a single
  luminance-derived `(low, high)` range for all channels. Color balance is
  *not* adjusted during normalization — that's the pipeline's job
  (neutralization + SCNR). The stretch faithfully displays whatever color
  balance the data has.
- **Per-channel background equalization during stacking** (`lib.rs`): Instead
  of equalizing in the stretch, we now compute per-channel background
  deltas during the stacking loop itself. Each frame's per-channel background
  is estimated and an additive offset is applied to match the reference
  frame's background *before* accumulation. This prevents inter-frame
  background variations from creating color gradients in the stack.

**Key code locations:**
- `histogram.rs` — `robust_normalize_bounds()`, `robust_normalize()`
- `lib.rs` — stacking loop (the `bg_deltas` / `estimate_frame_background_per_channel` section)

---

<a id="fix-faint"></a>
### 2.3 Faint Galaxy / Dark Image

**Symptoms:** The galaxy was barely visible — the sky background and the
galaxy arms had almost the same brightness after stretching. The auto-stretch
was computing `midtones ≈ 0.5` (linear stretch, i.e., no stretch at all).

**Root causes (three independent issues, all had to be fixed together):**

1. **Zero-pixel contamination of auto-stretch statistics.** Alignment borders
   (zero-fill from homography warping) and gradient extraction artifacts
   (negative values clamped to 0) were included in the luminance data used to
   compute median/MAD. With ~10-20% of pixels at zero, the median was dragged
   to a very low value, and the normalized median fell below 0.001, triggering
   the fallback `midtones = 0.5` (linear, no stretch).

2. **Off-by-one boundary condition.** The midtone computation condition was
   `if normalized_median > 0.001` but the lower clamp was `0.001`, so
   values of exactly 0.001 fell through to the `else` branch returning 0.5.
   Changed to `>= 0.001`.

3. **Normalization range too wide.** The `high` bound in
   `robust_normalize_bounds` was computed as `median + 100σ` (later
   `P99.8%`), which for stacked data could be 700+ ADU. This compressed the
   faint galaxy signal (typically 2-10× above sky noise) into ~2% of the
   [0, 1] range. The MTF stretch had almost no room to separate the galaxy
   from the background.

**Fixes applied:**
- **Zero-pixel filtering in auto-stretch** (`histogram.rs: auto_stretch`):
  Luminance values ≤ 0.005 are filtered out before computing
  median/MAD. This removes alignment border artifacts and gradient extraction
  edge effects from the statistics.
- **Boundary condition fix**: Changed `> 0.001` to `>= 0.001`.
- **Tighter normalization bound** (`histogram.rs: robust_normalize_bounds`):
  Changed `high` from `median + 100σ` / percentile-based to
  `median + 15σ`. Rationale: galaxies are typically 2-10σ above the sky
  noise. 15σ captures all extended object signal while clipping bright star
  cores to white. This puts the sky background at ~15% of the [0, 1] range
  (instead of ~2%), giving the MTF stretch much more room to separate the
  galaxy from the background. Stars clip to white, but that's acceptable —
  star cores are point sources with no recoverable detail anyway.

**Key code locations:**
- `histogram.rs` — `robust_normalize_bounds()`, `auto_stretch()` (the
  `zero_thresh` filtering and `>= 0.001` condition)

---

<a id="fix-banding"></a>
### 2.4 Edge Banding from Stacking Borders

**Symptoms:** Colored bands (green and magenta) at the edges of the stacked
image, particularly on the left side. Caused by regions where only a few
frames overlapped after alignment (dithered frames have different offsets).

**Root cause:** The fixed 8% crop was not data-driven — it either cut too
little (leaving low-coverage edges) or too much (wasting valid data). The
low-coverage edges had wildly different per-channel backgrounds because only
2-3 frames contributed, and those frames hadn't been equalized well enough.
These bad edges then *contaminated* gradient extraction and neutralization,
skewing their statistics.

**Fixes applied:**
- **Coverage-based smart crop** (`lib.rs: smart_crop_by_coverage`): Uses the
  per-pixel stacking count array to find the bounding box where at least 80%
  of the maximum frame count contributed. An additional 8-pixel margin is
  trimmed for safety. Falls back to 5% crop if coverage analysis fails.
- **Pipeline reordering**: Crop now happens *first* in post-processing
  (before gradient extraction and neutralization), so those steps operate on
  clean data where every pixel has full stacking coverage. Previously the crop
  was step 3 (after gradient and neutralization), meaning those algorithms
  were contaminated by edge artifacts.

**Key code locations:**
- `lib.rs` — `smart_crop_by_coverage()`, pipeline post-processing order

**Note:** Some banding may still persist deeper inside the frame, beyond the
crop boundary. See [Remaining Issues](#remaining-issues).

---

<a id="fix-calibration"></a>
### 2.5 Calibration Safety Net

**Symptom:** User accidentally loaded flat frames as biases. The master flat
became zero after bias subtraction, causing division by zero during light
frame calibration → 0 stars detected → pipeline failure.

**Fix:** Added a safety check in `calibration.rs: create_master_flat`. After
building the master flat, if its mean is near-zero (< 1.0 ADU), it logs a
warning and rebuilds the flat *without* bias subtraction. This prevents a
hard pipeline failure from incorrect calibration files.

---

<a id="pipeline-order"></a>
## 3. Current Pipeline Order

After all fixes, the post-processing pipeline runs in this order:

```
Stacking (with per-channel background equalization per frame)
    ↓
1. Smart crop by coverage (80% threshold + 8px margin)
    ↓
2. Background gradient extraction (16×N grid, sigma-clipped median, bilinear interpolation)
    ↓
3. Background neutralization (additive, green-channel reference)
    ↓
4. SCNR "Average Neutral" (magenta removal, R and B toward avg with G)
    ↓
5. Noise reduction (selective 3×3 median filter)
    ↓
Stretch (robust normalization + MTF auto-stretch)
```

The order matters:
- Crop first removes bad edges so they don't poison gradient/neutralization.
- Gradient extraction before neutralization ensures the background is smooth
  before we try to equalize channel levels.
- Neutralization before SCNR so that SCNR only has to handle residual
  chromatic noise, not a systematic channel offset.
- Noise reduction last (before stretch) so that it operates on linear data
  where noise statistics are well-defined.

---

<a id="remaining-issues"></a>
## 4. Known Remaining Issues

### 4.1 Banding Artifacts (Green/Magenta Vertical Bands)

**Status:** Partially mitigated by the smart crop but still present in some
stacks, particularly visible on the left side of the image.

**Likely causes:**
1. **Per-frame spatial gradients.** The current per-frame background
   equalization applies a single per-channel offset (scalar delta) to each
   frame. If a frame has a *spatial* gradient (e.g., left side brighter than
   right due to light pollution or vignetting), the scalar offset only
   corrects the average — the spatial variation remains. When frames with
   different gradient directions/magnitudes are stacked, the uncorrected
   gradients create bands.
2. **Dithering-induced coverage variation.** Even inside the 80% coverage
   boundary, different columns/rows may have slightly different frame counts.
   If the frames that contribute to a given column have a systematic color
   bias (because they came from the same part of the imaging session where
   conditions were changing), that column gets a different background color.
3. **Post-stack gradient extraction limitations.** The 16-cell grid is coarse
   and models a smooth gradient. Sharp transitions between bands are
   essentially invisible to it — the banding frequency is higher than the
   grid can resolve.

**Approaches to fix (in order of expected impact):**

1. **Per-frame 2D gradient subtraction.** Before accumulation, fit a low-order
   2D polynomial (or tilted plane) to each frame's background and subtract it.
   This removes the spatial component of the gradient that the scalar offset
   misses. Implementation: fit a first-order surface `bg(x,y) = a + bx + cy`
   using sigma-clipped background samples (same approach as
   `gradient::extract_gradient` but applied per frame, and only a plane fit,
   not a full grid). This is how Siril's "Background Extraction" works
   when applied to individual subs.

2. **Finer post-stack gradient grid.** Increase `GRID_SIZE` from 16 to 32 or
   64. This lets the bilinear interpolation capture higher-frequency
   background variations. Risk: too fine a grid might subtract real signal
   (galaxy arms can be faint and extend over large areas).

3. **Winsorized stacking** or **linear-fit rejection.** Instead of sigma-
   clipped mean, use a stacking algorithm that is more robust to per-frame
   gradient residuals. Linear-fit clipping (as in DeepSkyStacker) fits a
   line to each pixel's value across frames and rejects outliers from the
   fit, which naturally handles frames with different gradients.

### 4.2 Star Color Accuracy

Stars appear white/cyan in the current pipeline because the 15σ
normalization bound clips bright star cores. This is acceptable for galaxy
imaging (the priority) but could be improved for star fields where color is
scientifically interesting. A dual-range approach (one normalization for
extended objects, one for stars) could address this.

---

<a id="future-ideas"></a>
## 5. Ideas for Further Improvement

### 5.1 Per-Frame 2D Gradient Modeling (High Priority)

As described in 4.1. This is the single highest-impact improvement remaining.
By removing spatial gradients *before* stacking rather than *after*, the
accumulation itself becomes cleaner and the post-stack gradient extraction has
much less work to do.

**Implementation sketch:**
```
For each calibrated, demosaiced, aligned frame:
  1. Sample background in an 8×8 grid (sigma-clipped median, reject stars)
  2. Fit a first-order surface: bg(x,y) = a + bx + cy (least-squares)
  3. Subtract the surface (keeping a small pedestal for positivity)
  4. Then apply the scalar per-channel offset for inter-frame equalization
  5. Accumulate into the stack
```

### 5.2 Adaptive Sigma-Clipping Based on Frame Count

Currently using a fixed kappa (sigma threshold) of 3.0 for sigma-clipped
stacking. With 400 frames, a tighter kappa (2.5 or even 2.0) would reject
more outliers (satellites, cosmic rays, hot pixels) without losing signal.
With fewer frames (< 30), a wider kappa (3.5-4.0) is needed to avoid
rejecting valid data. The kappa could be automatically adjusted based on the
frame count.

### 5.3 Linked/Unlinked Channel Stretch

Currently the auto-stretch uses a single luminance-derived midtone parameter
for all channels ("linked stretch"). An "unlinked" mode that computes
separate stretch parameters per channel could help in situations where the
color balance is slightly off — each channel would be optimally stretched
independently, which sometimes reveals more color in extended objects.
PixInsight offers both modes; linked is the default, unlinked is sometimes
useful for narrowband or heavily color-cast data.

### 5.4 Color Saturation Enhancement

After stretching, deep-sky objects often look desaturated because the MTF
stretch compresses the color differences along with the brightness. A post-
stretch saturation boost (in HSL/HSV space) could make galaxy arms and nebula
regions more vivid. This should be applied selectively — background pixels
should *not* have their saturation boosted (that would amplify color noise).

### 5.5 Drizzle Integration

For small-sensor telescopes like the Dwarf 3, drizzle (sub-pixel
registration and integration at 2× resolution) can significantly improve
resolution. Requires sub-pixel alignment accuracy (we already have
homography-based alignment, so the transforms are available). The stacking
loop would need to be modified to place each frame's pixels on a 2× grid
with fractional offsets.

### 5.6 Better Demosaicing

Currently using bilinear interpolation for Bayer demosaicing, which is the
simplest and fastest method but produces the most chromatic artifacts (color
fringing at star edges, moiré patterns). Better algorithms:
- **VNG (Variable Number of Gradients)**: Better edge-awareness, standard in
  dcraw/libraw.
- **AHD (Adaptive Homogeneity-Directed)**: Best quality in dcraw, slower but
  produces fewer artifacts.
- **Superpixel mode**: Instead of interpolating, bin the 2×2 Bayer quad into
  a single RGB pixel. Half the resolution but zero demosaic artifacts. Good
  for small telescopes where resolution is already limited by optics.

### 5.7 Local Normalization Before Stacking

Instead of normalizing each frame globally (single per-channel offset),
compute a local normalization map for each frame. Divide the frame into tiles,
compute per-tile background and scale, and normalize each tile independently.
This handles both additive (sky background) and multiplicative (transparency
variations, thin clouds) differences between frames. Used by PixInsight's
`LocalNormalization` process.

### 5.8 Photometric Color Calibration (PCC)

Replace the current empirical background neutralization with photometric
color calibration: identify stars in the image, match them to a catalog
(Gaia, APASS, etc.), and compute per-channel scaling factors that make
measured star magnitudes match catalog values. This gives *physically correct*
color balance rather than just neutral backgrounds. Requires plate-solving
or at minimum a rough WCS (sky coordinate) solution.

### 5.9 Wavelet-Based Sharpening

After stacking and stretching, apply wavelet decomposition to separate the
image into frequency layers. Boost the high-frequency layers (fine detail)
while leaving the low-frequency layers (smooth background) untouched. This
is equivalent to PixInsight's `MultiscaleLinearTransform` and can dramatically
sharpen galaxy structure without amplifying background noise.

### 5.10 HDR Composition

For images with both very bright (star cores, galaxy nucleus) and very faint
(outer spiral arms) features, a single stretch cannot show both well. An HDR
approach would compute multiple stretches at different intensity ranges and
blend them, similar to PixInsight's `HDRMultiscaleTransform`. This would let
star cores retain color while still revealing faint nebulosity.

---

## Appendix: Diagnostic Logging

The pipeline emits per-channel background statistics at every post-processing
stage. When diagnosing color issues, the most useful log lines are:

```
Stacked result per-channel backgrounds: [R, G, B]
Post-gradient per-channel backgrounds: [R, G, B]
Post-neutralize per-channel backgrounds: [R, G, B]  ← these should be ~equal
Neutralize (additive): R=X, G=Y, B=Z → ref=G=Y     ← shows the offsets applied
SCNR: adjusted N/M pixels (P%)                       ← >20% suggests strong cast
Robust normalize (luminance): median=X, sigma=Y, low=L, high=H
Auto-stretch stats: median=X, MAD=Y, MAD_sigma=Z
Auto-stretch result: shadows=S, midtones=M, highlights=H, normalized_median=NM
```

If the stacked R/G/B backgrounds are wildly different *before* neutralization,
the problem is upstream (calibration, flat correction, or stacking
equalization). If they diverge *after* neutralization, the neutralization
itself is failing (e.g., the sigma-clipping is rejecting too many pixels in
one channel).

The "Save Logs" button in the UI exports the full untruncated log history to
a text file for offline analysis.
