//! Quick test: detect stars in one frame to check star detection works.
use astro_core::*;
use std::path::Path;

fn main() {
    let path = Path::new("/Users/marcin/Desktop/M33 puszcza notecka/lights/M 33_30s60_Astro_20250901-215717979_30C.fits");
    let dark_path = Path::new("/Users/marcin/Desktop/M33 puszcza notecka/darks/raw_30s_60_0000_20250819-212956160_30C.fits");

    println!("Loading light...");
    let light = fits_io::read_fits(path).unwrap();
    println!("  {}x{}x{}", light.image.width, light.image.height, light.image.channels);
    
    println!("Loading dark...");
    let dark = fits_io::read_fits(dark_path).unwrap();

    println!("Calibrating (dark subtraction)...");
    let cal = calibration::CalibrationFrames {
        master_bias: None,
        master_dark: Some(dark.image),
        master_flat: None,
    };
    let calibrated = calibration::calibrate_light(&light.image, &cal).unwrap();
    println!("  Calibrated: {}x{}x{}", calibrated.width, calibrated.height, calibrated.channels);

    // Check calibrated stats
    let mut vals: Vec<f32> = calibrated.data.iter().copied().collect();
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = vals.len();
    println!("  Stats: min={:.1}, median={:.1}, P99.9%={:.1}, max={:.1}",
        vals[0], vals[n/2], vals[(n as f64 * 0.999) as usize], vals[n-1]);

    println!("Demosaicing RGGB...");
    let demosaiced = bayer::demosaic(&calibrated, bayer::BayerPattern::RGGB).unwrap();
    println!("  Demosaiced: {}x{}x{}", demosaiced.width, demosaiced.height, demosaiced.channels);

    println!("Detecting stars...");
    let params = star_detection::DetectionParams::default();
    println!("  Params: threshold_sigma={}, min_size={}, max_size={}", 
        params.threshold_sigma, params.min_star_size, params.max_star_size);
    
    let stars = star_detection::detect_stars(&demosaiced, &params).unwrap();
    println!("  Detected {} stars", stars.len());
    
    if !stars.is_empty() {
        println!("  Top 10 by flux:");
        for (i, s) in stars.iter().take(10).enumerate() {
            println!("    #{}: pos=({:.1}, {:.1}), flux={:.1}, hfr={:.2}, peak={:.1}, pixels={}",
                i, s.x, s.y, s.flux, s.hfr, s.peak, s.pixel_count);
        }
    }
}
