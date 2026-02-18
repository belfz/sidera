//! astro-cli: JSON-over-stdio server for Sidera.
//!
//! Runs as a long-lived child process of the Electron app. Reads JSON
//! commands from stdin (one per line) and writes JSON responses to stdout.
//! Log output (env_logger) goes to stderr and is forwarded to the UI.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use astro_core::{
    bayer::BayerPattern,
    fits_io,
    histogram::{self, StretchParams},
    output::{self, OutputFormat},
    stacking::StackMethod,
    ImageData, Pipeline, PipelineStage, ProgressCallback,
};

// ─── Image store ─────────────────────────────────────────────────────────────

struct ImageStore {
    images: HashMap<String, ImageData>,
    next_id: u64,
}

impl ImageStore {
    fn new() -> Self {
        Self {
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

    fn get(&self, id: &str) -> Option<&ImageData> {
        self.images.get(id)
    }

    fn remove(&mut self, id: &str) {
        self.images.remove(id);
    }

    fn clear(&mut self) {
        self.images.clear();
    }
}

// ─── Protocol helpers ────────────────────────────────────────────────────────

fn send(msg: &Value) {
    let line = serde_json::to_string(msg).unwrap();
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{}", line).ok();
    handle.flush().ok();
}

fn send_ok(id: &str, data: Value) {
    send(&json!({"id": id, "ok": true, "data": data}));
}

fn send_error(id: &str, error: &str) {
    send(&json!({"id": id, "ok": false, "error": error}));
}

fn send_progress(id: &str, stage: &str, percent: f32) {
    send(&json!({"id": id, "progress": {"stage": stage, "percent": percent}}));
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    env_logger::Builder::from_default_env()
        .target(env_logger::Target::Stderr)
        .filter_level(log::LevelFilter::Info)
        .init();

    eprintln!("[astro-cli] Server starting");

    let temp_dir = std::env::temp_dir().join("sidera");
    std::fs::create_dir_all(&temp_dir).ok();

    let mut store = ImageStore::new();
    let stdin = io::stdin();

    for line_result in stdin.lock().lines() {
        let line = match line_result {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => l,
            Err(_) => break,
        };

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[astro-cli] Invalid JSON: {}", e);
                continue;
            }
        };

        let id = req["id"].as_str().unwrap_or("0").to_string();
        let cmd = req["cmd"].as_str().unwrap_or("");

        match cmd {
            "info" => cmd_info(&id, &req),
            "load" => cmd_load(&mut store, &id, &req),
            "release" => cmd_release(&mut store, &id, &req),
            "releaseAll" => cmd_release_all(&mut store, &id),
            "imageInfo" => cmd_image_info(&store, &id, &req),
            "preview" => cmd_preview(&store, &id, &req),
            "histogram" => cmd_histogram(&store, &id, &req),
            "autoStretch" => cmd_auto_stretch(&store, &id, &req),
            "pipeline" => cmd_pipeline(&mut store, &id, &req),
            "save" => cmd_save(&store, &id, &req),
            _ => send_error(&id, &format!("Unknown command: {}", cmd)),
        }
    }

    eprintln!("[astro-cli] Server shutting down");
}

// ─── Command handlers ────────────────────────────────────────────────────────

/// Read FITS headers only (no pixel data). Returns metadata for the file list.
fn cmd_info(id: &str, req: &Value) {
    let path_str = req["path"].as_str().unwrap_or("");
    let path = Path::new(path_str);

    match fits_io::read_fits_headers(path) {
        Ok(meta) => {
            let naxis1 = meta
                .header_map
                .get("NAXIS1")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let naxis2 = meta
                .header_map
                .get("NAXIS2")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let naxis = meta
                .header_map
                .get("NAXIS")
                .and_then(|v| v.as_i64())
                .unwrap_or(2);
            let naxis3 = meta
                .header_map
                .get("NAXIS3")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            let channels = if naxis >= 3 { naxis3 } else { 1 };

            // Deterministic ID from path hash
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            path_str.hash(&mut h);
            let file_id = format!("file_{:x}", h.finish());

            send_ok(
                id,
                json!({
                    "id": file_id,
                    "path": path_str,
                    "width": naxis1,
                    "height": naxis2,
                    "channels": channels,
                    "bitpix": meta.bitpix,
                    "frameType": format!("{:?}", meta.frame_type),
                    "exposureTime": meta.exposure_time,
                    "temperature": meta.temperature,
                    "gain": meta.gain,
                    "filterName": meta.filter,
                    "bayerPattern": meta.bayer_pattern,
                }),
            );
        }
        Err(e) => send_error(id, &e.to_string()),
    }
}

/// Load a full FITS file (headers + pixel data) into the image store for preview.
fn cmd_load(store: &mut ImageStore, id: &str, req: &Value) {
    let path_str = req["path"].as_str().unwrap_or("");
    let path = Path::new(path_str);

    match fits_io::read_fits(path) {
        Ok(fits) => {
            let image_id = store.insert(fits.image);
            send_ok(id, json!({"imageId": image_id}));
        }
        Err(e) => send_error(id, &e.to_string()),
    }
}

/// Release an image from the store.
fn cmd_release(store: &mut ImageStore, id: &str, req: &Value) {
    let image_id = req["imageId"].as_str().unwrap_or("");
    store.remove(image_id);
    send_ok(id, json!({}));
}

/// Release all images from the store.
fn cmd_release_all(store: &mut ImageStore, id: &str) {
    store.clear();
    send_ok(id, json!({}));
}

/// Get image dimensions.
fn cmd_image_info(store: &ImageStore, id: &str, req: &Value) {
    let image_id = req["imageId"].as_str().unwrap_or("");
    match store.get(image_id) {
        Some(img) => send_ok(
            id,
            json!({
                "width": img.width,
                "height": img.height,
                "channels": img.channels,
            }),
        ),
        None => send_error(id, &format!("Image not found: {}", image_id)),
    }
}

/// Generate RGBA preview data and write to a temp file.
fn cmd_preview(store: &ImageStore, id: &str, req: &Value) {
    let image_id = req["imageId"].as_str().unwrap_or("");
    let output_path = req["outputPath"].as_str().unwrap_or("");
    let stretch = parse_stretch(&req["stretch"]);

    match store.get(image_id) {
        Some(img) => {
            let rgba = histogram::to_rgba_preview(img, stretch.as_ref());
            match std::fs::write(output_path, &rgba) {
                Ok(_) => send_ok(
                    id,
                    json!({
                        "path": output_path,
                        "width": img.width,
                        "height": img.height,
                    }),
                ),
                Err(e) => send_error(id, &format!("Failed to write preview: {}", e)),
            }
        }
        None => send_error(id, &format!("Image not found: {}", image_id)),
    }
}

/// Compute histogram for a channel.
fn cmd_histogram(store: &ImageStore, id: &str, req: &Value) {
    let image_id = req["imageId"].as_str().unwrap_or("");
    let channel = req["channel"].as_u64().unwrap_or(0) as usize;
    let bins = req["bins"].as_u64().unwrap_or(256) as usize;

    match store.get(image_id) {
        Some(img) => {
            let hist = histogram::compute_channel_histogram(img, channel, bins);
            send_ok(
                id,
                json!({
                    "bins": hist.bins,
                    "min": hist.min,
                    "max": hist.max,
                    "channel": channel,
                }),
            );
        }
        None => send_error(id, &format!("Image not found: {}", image_id)),
    }
}

/// Compute auto-stretch parameters.
fn cmd_auto_stretch(store: &ImageStore, id: &str, req: &Value) {
    let image_id = req["imageId"].as_str().unwrap_or("");
    match store.get(image_id) {
        Some(img) => {
            let params = histogram::auto_stretch(img);
            send_ok(
                id,
                json!({
                    "shadows": params.shadows,
                    "midtones": params.midtones,
                    "highlights": params.highlights,
                }),
            );
        }
        None => send_error(id, &format!("Image not found: {}", image_id)),
    }
}

/// Run the full stacking pipeline with progress updates.
fn cmd_pipeline(store: &mut ImageStore, id: &str, req: &Value) {
    let light_paths = parse_path_array(&req["lightPaths"]);
    let dark_paths = parse_path_array(&req["darkPaths"]);
    let flat_paths = parse_path_array(&req["flatPaths"]);
    let bias_paths = parse_path_array(&req["biasPaths"]);

    let bayer_str = req["bayerPattern"].as_str();
    let bayer = bayer_str.and_then(BayerPattern::from_str);
    let skip_demosaic = bayer_str == Some("none");

    let stack_method = parse_stack_method(
        req["stackMethod"].as_str().unwrap_or("sigma_clip_mean"),
        req["kappa"].as_f64().unwrap_or(3.0),
        req["iterations"].as_u64().unwrap_or(5) as usize,
    );

    log::info!(
        "Pipeline: {} lights, {} darks, {} flats, {} biases",
        light_paths.len(),
        dark_paths.len(),
        flat_paths.len(),
        bias_paths.len()
    );

    let pipeline = Pipeline {
        light_paths,
        dark_paths,
        flat_paths,
        bias_paths,
        bayer_pattern: bayer,
        skip_demosaic,
        stack_method,
        ..Pipeline::default()
    };

    // Progress callback writes JSON to stdout inline during pipeline execution
    let id_for_cb = id.to_string();
    let progress_cb: ProgressCallback = Box::new(move |stage: PipelineStage, pct: f32| {
        send_progress(&id_for_cb, &stage.to_string(), pct);
    });

    match pipeline.run(Some(progress_cb)) {
        Ok(result) => {
            let image_id = store.insert(result);
            send_ok(id, json!({"imageId": image_id}));
        }
        Err(e) => send_error(id, &e.to_string()),
    }
}

/// Save an image to disk in a specified format.
fn cmd_save(store: &ImageStore, id: &str, req: &Value) {
    let image_id = req["imageId"].as_str().unwrap_or("");
    let output_path = req["outputPath"].as_str().unwrap_or("");
    let format_str = req["format"].as_str().unwrap_or("fits");
    let stretch = parse_stretch(&req["stretch"]);

    match store.get(image_id) {
        Some(img) => {
            let fmt = match format_str {
                "fits" => OutputFormat::Fits,
                "tiff" | "tif" => OutputFormat::Tiff,
                "png" => OutputFormat::Png,
                _ => {
                    send_error(id, &format!("Unknown format: {}", format_str));
                    return;
                }
            };
            match output::save_image(Path::new(output_path), img, fmt, stretch.as_ref()) {
                Ok(_) => send_ok(id, json!({})),
                Err(e) => send_error(id, &e.to_string()),
            }
        }
        None => send_error(id, &format!("Image not found: {}", image_id)),
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn parse_stretch(v: &Value) -> Option<StretchParams> {
    if v.is_null() || !v.is_object() {
        return None;
    }
    Some(StretchParams {
        shadows: v["shadows"].as_f64().unwrap_or(0.0) as f32,
        midtones: v["midtones"].as_f64().unwrap_or(0.25) as f32,
        highlights: v["highlights"].as_f64().unwrap_or(1.0) as f32,
    })
}

fn parse_path_array(v: &Value) -> Vec<PathBuf> {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(PathBuf::from))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_stack_method(method: &str, kappa: f64, iterations: usize) -> StackMethod {
    match method {
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
