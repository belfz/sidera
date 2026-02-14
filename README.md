# astro-viber

A modern astrophotography image stacking tool built with Rust and Electron.

## Architecture

- **`crates/astro-core`** — Pure Rust image processing library: FITS I/O, calibration, star detection, alignment, stacking, and output.
- **`crates/napi-bridge`** — napi-rs bindings exposing the Rust core to Node.js / Electron.
- **`electron/`** — Electron + React (TypeScript) frontend with dark-themed UI.

## Features

- **FITS file support** — Custom pure-Rust FITS reader/writer (no C dependencies)
- **Full calibration pipeline** — Master bias, dark, and flat frame creation with automatic light frame calibration
- **Bayer demosaicing** — Support for RGGB, BGGR, GRBG, GBRG patterns with auto-detection from FITS headers
- **Star detection** — Threshold-based detection with Gaussian centroiding and HFR measurement
- **Alignment** — Triangle pattern matching with full homography estimation (RANSAC) and Lanczos resampling
- **Stacking** — Mean, median, sigma-clipped mean, and sigma-clipped median integration
- **Interactive preview** — Real-time histogram with adjustable midtone transfer function stretch
- **Multiple output formats** — FITS (32-bit float), TIFF (16-bit), PNG (8-bit with stretch)

## Prerequisites

- [Rust](https://rustup.rs/) (stable, 1.75+)
- [Node.js](https://nodejs.org/) (20+)
- npm (comes with Node.js)

## Development

### Build the Rust core

```bash
cargo build --release
```

### Run tests

```bash
cargo test
```

### Set up the Electron app

```bash
cd electron
npm install
npm run dev
```

### Build the native addon

```bash
cd crates/napi-bridge
npm install
npm run build
```

## Calibration Pipeline

The tool follows the standard astrophotography calibration workflow:

1. **Master Bias** = median stack of bias frames
2. **Master Dark** = median stack of dark frames − Master Bias
3. **Master Flat** = median stack of flat frames − Master Bias, normalized to mean = 1.0
4. **Calibrated Light** = (Raw Light − Master Bias − Master Dark) / Master Flat

## License

MIT
