//! Quick diagnostic: read a FITS file and print headers + pixel statistics.
//! Usage: cargo run --example inspect_fits -- /path/to/file.fits

use astro_core::fits_io;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <fits_file>", args[0]);
        std::process::exit(1);
    }

    let path = Path::new(&args[1]);
    println!("=== Inspecting: {} ===\n", path.display());

    let fits = match fits_io::read_fits(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to read FITS: {}", e);
            std::process::exit(1);
        }
    };

    // Print all headers
    println!("--- FITS Headers ---");
    for h in &fits.metadata.headers {
        let val_str = match &h.value {
            astro_core::FitsValue::Integer(v) => format!("{}", v),
            astro_core::FitsValue::Float(v) => format!("{}", v),
            astro_core::FitsValue::String(v) => format!("'{}'", v),
            astro_core::FitsValue::Logical(v) => format!("{}", v),
            astro_core::FitsValue::None => "None".to_string(),
        };
        let cmt = h.comment.as_deref().unwrap_or("");
        println!("  {:<16} = {:<30} / {}", h.keyword, val_str, cmt);
    }

    // Print image info
    let img = &fits.image;
    println!("\n--- Image Data ---");
    println!("  Dimensions: {} x {} x {} channels", img.width, img.height, img.channels);
    println!("  Total values: {}", img.data.len());
    println!("  BITPIX: {}", fits.metadata.bitpix);
    println!("  Frame type: {:?}", fits.metadata.frame_type);
    println!("  Bayer pattern: {:?}", fits.metadata.bayer_pattern);
    println!("  Exposure: {:?}", fits.metadata.exposure_time);

    // Pixel statistics
    let data = &img.data;
    let mut sorted: Vec<f32> = data.iter().copied().filter(|v| v.is_finite()).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap().into());
    let n = sorted.len();

    if n == 0 {
        println!("  NO VALID PIXEL DATA!");
        return;
    }

    let min = sorted[0];
    let max = sorted[n - 1];
    let median = sorted[n / 2];
    let p01 = sorted[(n as f64 * 0.001) as usize];
    let p05 = sorted[(n as f64 * 0.005) as usize];
    let p25 = sorted[n / 4];
    let p75 = sorted[3 * n / 4];
    let p95 = sorted[(n as f64 * 0.995) as usize];
    let p99 = sorted[(n as f64 * 0.999) as usize];
    let mean: f64 = data.iter().map(|&v| v as f64).sum::<f64>() / n as f64;

    // Count negatives
    let neg_count = sorted.iter().take_while(|&&v| v < 0.0).count();

    println!("\n--- Pixel Statistics ---");
    println!("  Min:        {:.4}", min);
    println!("  P0.1%:      {:.4}", p01);
    println!("  P0.5%:      {:.4}", p05);
    println!("  P25%:       {:.4}", p25);
    println!("  Median:     {:.4}", median);
    println!("  Mean:       {:.4}", mean);
    println!("  P75%:       {:.4}", p75);
    println!("  P99.5%:     {:.4}", p95);
    println!("  P99.9%:     {:.4}", p99);
    println!("  Max:        {:.4}", max);
    println!("  Negatives:  {} ({:.2}%)", neg_count, 100.0 * neg_count as f64 / n as f64);

    // Per-channel stats if multi-channel
    if img.channels > 1 {
        println!("\n--- Per-Channel Statistics ---");
        for c in 0..img.channels {
            let mut ch_vals: Vec<f32> = (0..img.pixel_count())
                .map(|i| img.data[i * img.channels + c])
                .filter(|v| v.is_finite())
                .collect();
            ch_vals.sort_by(|a, b| a.partial_cmp(b).unwrap().into());
            let cn = ch_vals.len();
            if cn > 0 {
                println!(
                    "  Ch{}: min={:.2}, median={:.2}, mean={:.2}, max={:.2}",
                    c,
                    ch_vals[0],
                    ch_vals[cn / 2],
                    ch_vals.iter().map(|&v| v as f64).sum::<f64>() / cn as f64,
                    ch_vals[cn - 1]
                );
            }
        }
    }
}
