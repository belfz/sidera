# Sidera

A modern astrophotography image stacking tool built from scratch in Rust with an Electron UI. No C/C++ dependencies for the core processing — the entire pipeline (FITS I/O, calibration, star detection, alignment, stacking, stretching) is pure Rust.

> *sidera* — Latin for "stars"

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Electron (React + TypeScript)                          │
│  ┌────────────┐  ┌──────────────┐  ┌──────────────────┐ │
│  │ FilePanel  │  │ ImagePreview │  │ HistogramPanel   │ │
│  │            │  │   (canvas)   │  │ + STF stretch    │ │
│  │ Lights     │  │              │  │                  │ │
│  │ Darks      │  ├──────────────┤  ├──────────────────┤ │
│  │ Flats      │  │  LogPanel    │  │ StackingPanel    │ │
│  │ Biases     │  │  (live logs) │  │ method / params  │ │
│  └────────────┘  └──────────────┘  └──────────────────┘ │
│         │              ▲                   │            │
│         └──── JSON over stdio ─────────────┘            │
│                        │                                │
└────────────────────────┼────────────────────────────────┘
                         │
              ┌──────────┴──────────┐
              │   astro-cli (Rust)  │
              │   child process     │
              │                     │
              │  ┌───────────────┐  │
              │  │  astro-core   │  │
              │  │  (pure Rust)  │  │
              │  └───────────────┘  │
              └─────────────────────┘
```

The processing engine runs as a **separate child process** (`astro-cli`), communicating with the Electron UI via newline-delimited JSON over stdin/stdout. Log output goes to stderr and is forwarded to the UI's output panel in real time.

This design means:
- **Crashes in the engine don't kill the UI** — the engine auto-restarts
- **No shared memory** — the UI stays responsive even during heavy processing
- **No native Node.js addon** — no NAPI, no `node-gyp`, no platform-specific build headaches

### Crates

| Crate | Description |
|-------|-------------|
| `astro-core` | Pure Rust processing library. FITS I/O, calibration, Bayer demosaicing, star detection, alignment (homography), stacking, histogram/stretch, and output (FITS/TIFF/PNG). Zero C dependencies. |
| `astro-cli` | JSON-over-stdio server binary. Wraps `astro-core` with an image store and a request/response protocol. Launched by Electron as a child process. |

---

## How the Pipeline Works

The pipeline uses a **memory-efficient two-pass approach** that can process hundreds of frames without loading them all into memory.

### 1. Calibration

Standard astrophotography calibration, creating master frames via median stacking:

| Step | Formula |
|------|---------|
| Master Bias | median(bias₁, bias₂, ...) |
| Master Dark | median(dark₁, dark₂, ...) − Master Bias |
| Master Flat | median(flat₁, flat₂, ...) − Master Bias, normalized to mean = 1.0 |
| Calibrated Light | (Light − Master Bias − Master Dark) / Master Flat |

If calibration frames have a different channel count (e.g., mono darks with color lights), the pipeline automatically broadcasts or converts.

### 2. Bayer Demosaicing

For one-shot color (OSC) cameras, the raw Bayer-patterned data is converted to RGB using bilinear interpolation. Supported patterns: RGGB, BGGR, GRBG, GBRG. The pattern is auto-detected from FITS headers (`BAYERPAT` / `COLORTYP`) or can be set manually.

### 3. Registration (Pass 1)

Each light frame is loaded one at a time, calibrated, demosaiced, and then stars are detected. Only the star lists are kept in memory — the frame itself is dropped.

**Star detection** uses threshold-based blob detection:
1. Estimate the sky background using sigma-clipped median statistics on a 64px grid
2. Bilinearly interpolate the grid to full resolution for a smooth background model
3. Threshold: mark pixels above background + 5σ
4. Connected component labeling (8-connected flood fill) to find star blobs
5. Filter by size (5–500 pixels) and reject blobs near the image edge
6. Refine positions via flux-weighted centroid (center of mass)
7. Compute Half Flux Radius (HFR) for quality metrics
8. Sort by flux (brightest first)

**Alignment** uses a three-phase strategy:

1. **Nearest-neighbor matching** — For tracked telescope data with small inter-frame shifts. Each reference star is matched to the closest target star within a 150px radius, with reciprocal verification (the match must be mutual). This produces very reliable correspondences.

2. **Translation model** — If NN matching succeeds (≥4 matches), first try fitting a pure translation (median dx, dy). This is numerically bulletproof and sufficient for well-tracked data.

3. **Full homography (RANSAC + DLT)** — If the translation model has too many outliers, estimate a full 8-DOF projective transform using RANSAC with the Direct Linear Transform algorithm. Uses Hartley normalization for numerical stability and A^T A eigendecomposition to avoid thin-SVD issues.

4. **Triangle pattern matching (fallback)** — For large shifts or rotations where NN matching fails. Forms triangles from the brightest stars, matches them by side-ratio descriptors (scale/rotation invariant), then builds point correspondences via voting and estimates a homography.

The reference frame is chosen automatically as the frame with the most detected stars.

### 4. Stacking (Pass 2)

Each frame is loaded a second time, calibrated, demosaiced, warped with the stored transform, and accumulated into a running sum. Memory usage stays at roughly 2 frames regardless of the total frame count.

Before accumulation, each frame's background level is equalized to the reference frame to prevent brightness variations from degrading the stack.

Available stacking methods:

| Method | Description |
|--------|-------------|
| Mean | Simple average. Best SNR but sensitive to outliers. |
| Median | Robust to outliers (satellite trails, cosmic rays) but slightly lower SNR. |
| Sigma-Clipped Mean | Iteratively rejects pixels > κσ from the mean, then averages survivors. Best balance. |
| Sigma-Clipped Median | Rejects outliers then takes the median of survivors. Most robust. |

### 5. Post-Processing

After stacking, four automatic corrections are applied:

1. **Background gradient extraction** — Models and subtracts the smooth sky background (light pollution, vignetting) using a 16-point grid with sigma-clipped sampling and bilinear interpolation.

2. **Background neutralization** — Equalizes the per-channel sky background levels to remove color casts from OSC cameras (automatic white balance).

3. **Border cropping** — Removes 8% from each edge to eliminate alignment artifacts (zero-fill borders from frame warping).

4. **Noise reduction** — Selective 3×3 median filter applied only to background pixels (below sky + 5σ). Stars and bright features are left untouched.

### 6. Visualization

The result is displayed with a **PixInsight-style Screen Transfer Function (STF)** auto-stretch:

1. Robust normalization using luminance-based median/MAD statistics (avoids mixing RGB offsets)
2. Shadow clipping at median − 2.8σ (noise floor)
3. Midtone transfer function mapping the sky background to ~10% brightness
4. This aggressively reveals faint extended objects (galaxies, nebulae) that are invisible in the linear data

---

## Features

- **FITS file support** — Custom pure-Rust reader/writer handling BITPIX 8, 16, 32, −32, −64 with BZERO/BSCALE
- **Full calibration pipeline** — Master bias, dark, flat creation with automatic light calibration
- **Bayer demosaicing** — Bilinear interpolation for RGGB, BGGR, GRBG, GBRG with auto-detection
- **Star detection** — Sigma-clipped background estimation, flood-fill blob detection, flux-weighted centroids, HFR
- **Alignment** — Nearest-neighbor + translation, RANSAC homography (DLT with Hartley normalization), triangle pattern matching fallback
- **Stacking** — Mean, median, sigma-clipped mean, sigma-clipped median with parallel per-pixel computation (rayon)
- **Interactive preview** — Real-time histogram with adjustable shadow/midtone/highlight stretch
- **Output log** — Live processing engine output in a scrollable panel
- **Multiple output formats** — FITS (32-bit float), TIFF (16-bit), PNG (8-bit with stretch)

---

## Prerequisites

- [Rust](https://rustup.rs/) (stable, 1.75+)
- [Node.js](https://nodejs.org/) (20+)
- npm (comes with Node.js)

## Development

### Quick start

```bash
# Build the Rust CLI engine
cargo build --release -p astro-cli

# Set up and run the Electron app
cd electron
npm install
npm run dev
```

The `npm run dev` script automatically builds the CLI binary before launching Electron.

### Run tests

```bash
cargo test
```

### Project structure

```
sidera/
├── Cargo.toml                  # Workspace root
├── crates/
│   ├── astro-core/             # Pure Rust processing library
│   │   └── src/
│   │       ├── lib.rs          # Pipeline orchestration
│   │       ├── fits_io.rs      # FITS reader/writer
│   │       ├── calibration.rs  # Master frame creation & light calibration
│   │       ├── bayer.rs        # Bayer demosaicing
│   │       ├── star_detection.rs # Star detection & centroiding
│   │       ├── alignment.rs    # Homography estimation & image warping
│   │       ├── stacking.rs     # Image integration methods
│   │       ├── histogram.rs    # Histogram computation & MTF stretch
│   │       ├── gradient.rs     # Background gradient extraction
│   │       ├── output.rs       # FITS/TIFF/PNG export
│   │       ├── image.rs        # Core image data structures
│   │       └── error.rs        # Error types
│   └── astro-cli/              # JSON-over-stdio server binary
│       └── src/main.rs
└── electron/                   # Electron + React frontend
    ├── src/
    │   ├── main/               # Electron main process (spawns CLI)
    │   └── renderer/           # React UI components
    └── package.json
```

---

## Building a Standalone Executable

### 1. Build the Rust CLI binary

```bash
cargo build --release -p astro-cli
```

The binary is at `target/release/astro-cli` (or `target/release/astro-cli.exe` on Windows).

This is a fully self-contained binary with no external dependencies. You can run the entire processing pipeline from the command line by piping JSON commands to it:

```bash
echo '{"id":"1","cmd":"info","path":"/path/to/frame.fits"}' | ./target/release/astro-cli
```

### 2. Build the Electron app for distribution

```bash
cd electron

# Build the frontend and main process
npm run build

# Package with electron-builder
npx electron-builder --mac        # macOS (.dmg)
npx electron-builder --linux      # Linux (.AppImage, .deb)
npx electron-builder --win        # Windows (.exe, .nsis)
```

You'll need to include the `astro-cli` binary in the packaged app. Add this to `electron/package.json` under the `"build"` key:

```json
{
  "build": {
    "appId": "com.sidera.app",
    "productName": "sidera",
    "extraResources": [
      {
        "from": "../target/release/astro-cli",
        "to": "astro-cli"
      }
    ],
    "mac": {
      "category": "public.app-category.photography"
    }
  }
}
```

### Cross-compilation

The CLI binary must be compiled for the target platform. For cross-compilation:

```bash
# macOS ARM → macOS Intel
rustup target add x86_64-apple-darwin
cargo build --release -p astro-cli --target x86_64-apple-darwin

# macOS → Linux
rustup target add x86_64-unknown-linux-gnu
cargo build --release -p astro-cli --target x86_64-unknown-linux-gnu
```

---

## Known Limitations & Future Improvements

### Alignment
- **No sub-pixel star matching** — NN matching uses pixel-level positions; sub-pixel refinement via cross-correlation would improve alignment of well-sampled data
- **No optical distortion modeling** — the homography model assumes a projective (pinhole) camera; wide-field images with barrel/pincushion distortion would benefit from a polynomial distortion model
- **O(n²) nearest-neighbor** — could be accelerated with a k-d tree for large star counts

### Stacking
- **Mean-only incremental stacking** — the two-pass pipeline uses running mean for memory efficiency; true per-pixel median/sigma-clip across all frames would require either loading all frames or an approximation algorithm (e.g., binned histogram approach)
- **No weighting** — frames could be weighted by FWHM, background level, or number of detected stars to prioritize the best-quality subframes
- **No drizzle** — sub-pixel dithering patterns could be exploited with drizzle integration to recover resolution beyond the sensor's native sampling

### Image Processing
- **Basic demosaicing** — bilinear interpolation is the simplest method; higher-quality algorithms (VNG, AHD, AMAZE) would reduce color artifacts
- **No deconvolution** — Richardson-Lucy or Wiener deconvolution could sharpen the stacked result
- **No star removal / separation** — separating stars from the nebula/galaxy signal would enable independent processing of each layer
- **Gradient extraction is global** — a more sophisticated approach would use automatic star masking and iterative local fitting (like Siril's GraXpert integration)

### UI
- **No batch processing / session save** — the UI doesn't persist the file list or settings between sessions
- **No live stacking preview** — would be useful to see the stack build up in real time
- **Stretch adjustments have latency** — each slider change requires a round-trip to the CLI process; applying the stretch in WebGL on the GPU would make it instant

---

## License

MIT
