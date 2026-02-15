# astro-viber

A modern astrophotography image stacking tool built with Rust and Electron.

## Architecture

- **`crates/astro-core`** — Pure Rust image processing library: FITS I/O, calibration, star detection, alignment, stacking, and output.
- **`crates/astro-cli`** — CLI server binary that runs as a child process of the Electron app, communicating via JSON over stdio. Keeps processing isolated from the UI — crashes in the engine don't take down the app.
- **`electron/`** — Electron + React (TypeScript) frontend with dark-themed UI, including real-time log output from the processing engine.

## Features

- **FITS file support** — Custom pure-Rust FITS reader/writer (no C dependencies)
- **Full calibration pipeline** — Master bias, dark, and flat frame creation with automatic light frame calibration
- **Bayer demosaicing** — Support for RGGB, BGGR, GRBG, GBRG patterns with auto-detection from FITS headers
- **Star detection** — Threshold-based detection with Gaussian centroiding and HFR measurement
- **Alignment** — Triangle pattern matching + nearest-neighbor with full homography estimation (RANSAC)
- **Stacking** — Mean, median, sigma-clipped mean, and sigma-clipped median integration
- **Interactive preview** — Real-time histogram with adjustable midtone transfer function stretch
- **Output log** — Live processing engine output in a scrollable panel
- **Multiple output formats** — FITS (32-bit float), TIFF (16-bit), PNG (8-bit with stretch)

## Prerequisites

- [Rust](https://rustup.rs/) (stable, 1.75+)
- [Node.js](https://nodejs.org/) (20+)
- npm (comes with Node.js)

## Development

### Build the Rust CLI

```bash
cargo build --release -p astro-cli
```

### Run tests

```bash
cargo test
```

### Set up and run the Electron app

```bash
cd electron
npm install
npm run dev
```

The `dev` script automatically builds the CLI binary before launching.

## Calibration Pipeline

The tool follows the standard astrophotography calibration workflow:

1. **Master Bias** = median stack of bias frames
2. **Master Dark** = median stack of dark frames − Master Bias
3. **Master Flat** = median stack of flat frames − Master Bias, normalized to mean = 1.0
4. **Calibrated Light** = (Raw Light − Master Bias − Master Dark) / Master Flat

## License

MIT
