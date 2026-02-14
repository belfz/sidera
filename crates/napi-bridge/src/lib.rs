//! napi-rs bridge: exposes astro-core functions to Node.js / Electron.
//!
//! **Memory-efficient design:**
//! - `load_fits_file` reads only FITS headers (no pixel data) for the file list.
//! - `run_pipeline` takes file paths and uses the two-pass streaming pipeline.
//! - The image store only holds images that are actively needed (previews, results).
//! - `catch_unwind` prevents Rust panics from crashing the Electron process.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use astro_core::bayer::BayerPattern;
use astro_core::histogram::{self, StretchParams};
use astro_core::stacking::StackMethod;
use astro_core::{alignment, fits_io, output, star_detection, ImageData};

// ─── Global image store ─────────────────────────────────────────────────────

/// In-memory store for processed images only (results, previews).
/// Raw frames are NOT stored here — the pipeline reads them from disk.
struct ImageStore {
    images: HashMap<String, ImageData>,
    next_id: u64,
}

impl ImageStore {
    fn new() -> Self {
        ImageStore {
            images: HashMap::new(),
            next_id: 0,
        }
    }

    fn insert(&mut self, image: ImageData) -> String {
        let id = format!("img_{}", self.next_id);
        self.next_id += 1;
        self.images.insert(id.clone(), image);
        id
    }
}

static STORE: std::sync::OnceLock<Arc<Mutex<ImageStore>>> = std::sync::OnceLock::new();

fn get_store() -> Arc<Mutex<ImageStore>> {
    STORE
        .get_or_init(|| Arc::new(Mutex::new(ImageStore::new())))
        .clone()
}

// ─── JS-facing types ────────────────────────────────────────────────────────

#[napi(object)]
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub id: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub channels: u32,
    pub bitpix: i32,
    pub frame_type: String,
    pub exposure_time: Option<f64>,
    pub temperature: Option<f64>,
    pub gain: Option<f64>,
    pub filter_name: Option<String>,
    pub bayer_pattern: Option<String>,
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct DetectedStar {
    pub x: f64,
    pub y: f64,
    pub flux: f64,
    pub hfr: f64,
    pub peak: f64,
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct HistogramData {
    pub bins: Vec<u32>,
    pub min: f64,
    pub max: f64,
    pub channel: u32,
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct StretchConfig {
    pub shadows: f64,
    pub midtones: f64,
    pub highlights: f64,
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct StackingConfig {
    pub method: String, // "mean", "median", "sigma_clip_mean", "sigma_clip_median"
    pub kappa: Option<f64>,
    pub iterations: Option<u32>,
}

// ─── Initialization ─────────────────────────────────────────────────────────

#[napi]
pub fn init_logger() {
    let _ = env_logger::try_init();
}

// ─── File operations ────────────────────────────────────────────────────────

/// Load only the FITS headers for a file (no pixel data loaded into memory).
/// Returns metadata for display in the file list.
#[napi]
pub async fn load_fits_file(path: String) -> Result<FileInfo> {
    let path_clone = path.clone();
    let path_buf = PathBuf::from(&path);

    let metadata = tokio_rayon_spawn(move || fits_io::read_fits_headers(&path_buf))
        .await
        .map_err(|e| napi::Error::from_reason(format!("Failed to load FITS headers: {e}")))?;

    // Extract dimensions from the headers
    let naxis1 = metadata.header_map.get("NAXIS1")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as u32;
    let naxis2 = metadata.header_map.get("NAXIS2")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as u32;
    let naxis3 = metadata.header_map.get("NAXIS3")
        .and_then(|v| v.as_i64())
        .unwrap_or(1) as u32;
    let naxis = metadata.header_map.get("NAXIS")
        .and_then(|v| v.as_i64())
        .unwrap_or(2);
    let channels = if naxis >= 3 { naxis3 } else { 1 };

    // Generate a unique ID based on the path (no image stored)
    let id = format!("file_{:x}", {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        path_clone.hash(&mut h);
        h.finish()
    });

    Ok(FileInfo {
        id,
        path: path_clone,
        width: naxis1,
        height: naxis2,
        channels,
        bitpix: metadata.bitpix as i32,
        frame_type: format!("{:?}", metadata.frame_type),
        exposure_time: metadata.exposure_time,
        temperature: metadata.temperature,
        gain: metadata.gain,
        filter_name: metadata.filter.clone(),
        bayer_pattern: metadata.bayer_pattern.clone(),
    })
}

/// Remove an image from the store to free memory.
#[napi]
pub fn release_image(id: String) -> Result<()> {
    let store = get_store();
    let mut store = store.lock().unwrap();
    store.images.remove(&id);
    Ok(())
}

/// Release all images from the store.
#[napi]
pub fn release_all_images() -> Result<()> {
    let store = get_store();
    let mut store = store.lock().unwrap();
    store.images.clear();
    Ok(())
}

// ─── Preview & histogram ────────────────────────────────────────────────────

/// Get basic info (width, height, channels) for any image in the store.
#[napi(object)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub channels: u32,
}

#[napi]
pub fn get_image_info(image_id: String) -> Result<ImageInfo> {
    let store = get_store();
    let store = store.lock().unwrap();
    let image = store
        .images
        .get(&image_id)
        .ok_or_else(|| napi::Error::from_reason(format!("Image not found: {image_id}")))?;

    Ok(ImageInfo {
        width: image.width as u32,
        height: image.height as u32,
        channels: image.channels as u32,
    })
}

/// Get a histogram for a specific channel of a stored image.
#[napi]
pub fn get_histogram(image_id: String, channel: u32, bins: u32) -> Result<HistogramData> {
    let store = get_store();
    let store = store.lock().unwrap();
    let image = store
        .images
        .get(&image_id)
        .ok_or_else(|| napi::Error::from_reason(format!("Image not found: {image_id}")))?;

    let hist = histogram::compute_channel_histogram(image, channel as usize, bins as usize);

    Ok(HistogramData {
        bins: hist.bins,
        min: hist.min as f64,
        max: hist.max as f64,
        channel,
    })
}

/// Get auto-stretch parameters for a stored image.
#[napi]
pub fn get_auto_stretch(image_id: String) -> Result<StretchConfig> {
    let store = get_store();
    let store = store.lock().unwrap();
    let image = store
        .images
        .get(&image_id)
        .ok_or_else(|| napi::Error::from_reason(format!("Image not found: {image_id}")))?;

    let params = histogram::auto_stretch(image);

    Ok(StretchConfig {
        shadows: params.shadows as f64,
        midtones: params.midtones as f64,
        highlights: params.highlights as f64,
    })
}

/// Get RGBA preview data for an image (for rendering on HTML Canvas).
/// Returns a Buffer of width * height * 4 bytes (RGBA).
#[napi]
pub fn get_preview(
    image_id: String,
    stretch: Option<StretchConfig>,
) -> Result<Buffer> {
    let store = get_store();
    let store = store.lock().unwrap();
    let image = store
        .images
        .get(&image_id)
        .ok_or_else(|| napi::Error::from_reason(format!("Image not found: {image_id}")))?;

    let stretch_params = stretch.map(|s| StretchParams {
        shadows: s.shadows as f32,
        midtones: s.midtones as f32,
        highlights: s.highlights as f32,
    });

    let rgba = histogram::to_rgba_preview(image, stretch_params.as_ref());

    Ok(Buffer::from(rgba))
}

/// Load a single FITS file fully (headers + pixel data) into the store for preview.
/// Use this when the user clicks on a file to preview it.
#[napi]
pub async fn load_preview(path: String) -> Result<String> {
    let path_buf = PathBuf::from(&path);

    let fits = tokio_rayon_spawn(move || fits_io::read_fits(&path_buf))
        .await
        .map_err(|e| napi::Error::from_reason(format!("Failed to load FITS: {e}")))?;

    let store = get_store();
    let mut store = store.lock().unwrap();
    let id = store.insert(fits.image);
    Ok(id)
}

// ─── Output ─────────────────────────────────────────────────────────────────

/// Save an image to disk in the specified format.
#[napi]
pub async fn save_image(
    image_id: String,
    path: String,
    format: String,
    stretch: Option<StretchConfig>,
) -> Result<()> {
    let store = get_store();
    let image = {
        let store = store.lock().unwrap();
        store
            .images
            .get(&image_id)
            .cloned()
            .ok_or_else(|| napi::Error::from_reason(format!("Image not found: {image_id}")))?
    };

    let fmt = match format.to_lowercase().as_str() {
        "fits" => output::OutputFormat::Fits,
        "tiff" | "tif" => output::OutputFormat::Tiff,
        "png" => output::OutputFormat::Png,
        _ => return Err(napi::Error::from_reason(format!("Unknown format: {format}"))),
    };

    let stretch_params = stretch.map(|s| StretchParams {
        shadows: s.shadows as f32,
        midtones: s.midtones as f32,
        highlights: s.highlights as f32,
    });

    let path_buf = PathBuf::from(path);
    tokio_rayon_spawn(move || {
        output::save_image(&path_buf, &image, fmt, stretch_params.as_ref())
    })
    .await
    .map_err(|e| napi::Error::from_reason(format!("Save failed: {e}")))?;

    Ok(())
}

// ─── Full pipeline ──────────────────────────────────────────────────────────

/// Run the complete pipeline. Takes file paths (not store IDs) and returns
/// the ID of the stacked result stored in the image store.
///
/// Memory-efficient: the pipeline reads frames from disk one at a time.
/// Only the final result is kept in memory.
#[napi]
pub async fn run_pipeline(
    light_paths: Vec<String>,
    dark_paths: Vec<String>,
    flat_paths: Vec<String>,
    bias_paths: Vec<String>,
    bayer_pattern: Option<String>,
    stacking_config: StackingConfig,
) -> Result<String> {
    let pipeline = astro_core::Pipeline {
        light_paths: light_paths.into_iter().map(PathBuf::from).collect(),
        dark_paths: dark_paths.into_iter().map(PathBuf::from).collect(),
        flat_paths: flat_paths.into_iter().map(PathBuf::from).collect(),
        bias_paths: bias_paths.into_iter().map(PathBuf::from).collect(),
        bayer_pattern: bayer_pattern.as_deref().and_then(|s| BayerPattern::from_str(s)),
        // If user explicitly chose "none" (mono), skip auto-detection too
        skip_demosaic: bayer_pattern.as_deref() == Some("none"),
        stack_method: parse_stack_method(&stacking_config),
        alignment_params: alignment::AlignmentParams::default(),
        detection_params: star_detection::DetectionParams::default(),
    };

    let result = tokio_rayon_spawn(move || pipeline.run(None))
        .await
        .map_err(|e| napi::Error::from_reason(format!("Pipeline failed: {e}")))?;

    let store = get_store();
    let mut store = store.lock().unwrap();
    let id = store.insert(result);
    Ok(id)
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn parse_stack_method(config: &StackingConfig) -> StackMethod {
    let kappa = config.kappa.unwrap_or(3.0);
    let iterations = config.iterations.unwrap_or(5) as usize;

    match config.method.to_lowercase().as_str() {
        "mean" => StackMethod::Mean,
        "median" => StackMethod::Median,
        "sigma_clip_mean" | "sigma_clipped_mean" => StackMethod::SigmaClippedMean {
            kappa,
            iterations,
        },
        "sigma_clip_median" | "sigma_clipped_median" => StackMethod::SigmaClippedMedian {
            kappa,
            iterations,
        },
        _ => StackMethod::SigmaClippedMean {
            kappa,
            iterations,
        },
    }
}

/// Run a CPU-intensive closure on a rayon thread pool, returning a future.
/// This prevents blocking the Node.js event loop.
///
/// Uses `catch_unwind` to convert Rust panics into errors instead of
/// crashing the Electron process (e.g., from out-of-memory allocation failures).
async fn tokio_rayon_spawn<F, T>(f: F) -> T
where
    F: FnOnce() -> T + Send + std::panic::UnwindSafe + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    rayon::spawn(move || {
        let result = std::panic::catch_unwind(f);
        let _ = tx.send(result);
    });
    // Poll in a non-blocking way
    loop {
        match rx.try_recv() {
            Ok(Ok(result)) => return result,
            Ok(Err(panic_info)) => {
                // Convert the panic into a descriptive error message
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    format!("Internal error (Rust panic): {}", s)
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    format!("Internal error (Rust panic): {}", s)
                } else {
                    "Internal error (Rust panic): unknown cause — possible out of memory"
                        .to_string()
                };
                // We can't return an error from this generic fn, so log and panic
                // with a clear message. The napi #[napi] wrapper will catch this.
                panic!("{}", msg);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Yield to the event loop
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("Processing task crashed unexpectedly — possible out of memory");
            }
        }
    }
}
