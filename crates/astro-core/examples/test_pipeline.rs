//! Quick end-to-end test: run the pipeline on a few real frames and save the result.
//! Usage: cargo run --release --example test_pipeline

use astro_core::*;
use std::path::PathBuf;

fn main() {
    env_logger::init();

    let light_dir = "/Users/marcin/Desktop/M33 puszcza notecka/lights";
    let dark_dir = "/Users/marcin/Desktop/M33 puszcza notecka/darks";

    // Collect light and dark paths
    let mut light_paths: Vec<PathBuf> = std::fs::read_dir(light_dir)
        .expect("Cannot read lights dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "fits"))
        .map(|e| e.path())
        .collect();
    light_paths.sort();

    let mut dark_paths: Vec<PathBuf> = std::fs::read_dir(dark_dir)
        .expect("Cannot read darks dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "fits"))
        .map(|e| e.path())
        .collect();
    dark_paths.sort();

    // Use ALL available frames — the two-pass pipeline handles memory efficiently
    // light_paths: all 400, dark_paths: all 20

    println!("Testing pipeline with {} lights, {} darks", light_paths.len(), dark_paths.len());

    // Use X frames for a test
    light_paths.truncate(400);

    let pipeline = Pipeline {
        light_paths,
        dark_paths,
        flat_paths: vec![],
        bias_paths: vec![],
        bayer_pattern: None, // Auto-detect from FITS headers
        skip_demosaic: false,
        stack_method: stacking::StackMethod::Mean,
        alignment_params: alignment::AlignmentParams::default(),
        detection_params: star_detection::DetectionParams::default(),
    };

    let progress_cb: ProgressCallback = Box::new(|stage, pct| {
        println!("  [{:.0}%] {}", pct * 100.0, stage);
    });

    match pipeline.run(Some(progress_cb)) {
        Ok(result) => {
            println!("\nPipeline succeeded!");
            println!("Result: {}x{}x{}", result.width, result.height, result.channels);

            // Print pixel stats
            let mut vals: Vec<f32> = result.data.iter().copied().collect();
            vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let n = vals.len();
            println!("  Min:    {:.2}", vals[0]);
            println!("  P1%:    {:.2}", vals[n / 100]);
            println!("  Median: {:.2}", vals[n / 2]);
            println!("  P99%:   {:.2}", vals[99 * n / 100]);
            println!("  Max:    {:.2}", vals[n - 1]);

            // Debug: check the normalization bounds
            {
                // Compute luminance
                let lum: Vec<f32> = (0..result.pixel_count())
                    .map(|i| {
                        let r = result.data[i * result.channels];
                        let g = result.data[i * result.channels + 1];
                        let b = result.data[i * result.channels + 2];
                        0.2126 * r + 0.7152 * g + 0.0722 * b
                    })
                    .filter(|v| *v > 1.0)
                    .collect();
                let mut sorted = lum.clone();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let n = sorted.len();
                println!("\nLuminance stats ({} valid pixels):", n);
                println!("  Min:    {:.2}", sorted[0]);
                println!("  P0.1%:  {:.2}", sorted[(n as f64 * 0.001) as usize]);
                println!("  Median: {:.2}", sorted[n/2]);
                println!("  P99.9%: {:.2}", sorted[(n as f64 * 0.999) as usize]);
                println!("  Max:    {:.2}", sorted[n-1]);

                let med = sorted[n/2];
                let mut devs: Vec<f32> = sorted.iter().map(|&v| (v - med).abs()).collect();
                devs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let mad = devs[n/2];
                let sigma = 1.4826 * mad;
                println!("  MAD:    {:.4}", mad);
                println!("  Sigma:  {:.4}", sigma);
                println!("  Low (med-2.8σ): {:.2}", med - 2.8 * sigma);
            }

            // Compute auto-stretch
            let stretch = histogram::auto_stretch(&result);
            println!("\nAuto-stretch: shadows={:.4}, midtones={:.4}, highlights={:.4}",
                     stretch.shadows, stretch.midtones, stretch.highlights);

            // Save as PNG with auto-stretch
            let out_path = std::path::Path::new("/Users/marcin/Desktop/M33 puszcza notecka/results/astro_test_result.png");
            match output::save_image(out_path, &result, output::OutputFormat::Tiff, Some(&stretch)) {
                Ok(_) => println!("\nSaved to {}", out_path.display()),
                Err(e) => println!("\nFailed to save: {}", e),
            }
        }
        Err(e) => {
            eprintln!("\nPipeline FAILED: {}", e);
            std::process::exit(1);
        }
    }
}
