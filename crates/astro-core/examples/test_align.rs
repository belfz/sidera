//! Test alignment between two frames.
use astro_core::*;
use std::path::Path;

fn load_and_process(path: &Path, cal: &calibration::CalibrationFrames) -> ImageData {
    let fits = fits_io::read_fits(path).unwrap();
    let calibrated = calibration::calibrate_light(&fits.image, cal).unwrap();
    bayer::demosaic(&calibrated, bayer::BayerPattern::RGGB).unwrap()
}

fn main() {
    let dark_path = Path::new("/Users/marcin/Desktop/M33 puszcza notecka/darks/raw_30s_60_0000_20250819-212956160_30C.fits");
    let dark = fits_io::read_fits(dark_path).unwrap();
    let cal = calibration::CalibrationFrames {
        master_bias: None,
        master_dark: Some(dark.image),
        master_flat: None,
    };

    let lights_dir = "/Users/marcin/Desktop/M33 puszcza notecka/lights";
    let mut paths: Vec<_> = std::fs::read_dir(lights_dir).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "fits"))
        .map(|e| e.path())
        .collect();
    paths.sort();

    let params = star_detection::DetectionParams::default();
    let align_params = alignment::AlignmentParams::default();

    // Load and detect stars for frame 0 (reference)
    println!("Loading frame 0...");
    let frame0 = load_and_process(&paths[0], &cal);
    let stars0 = star_detection::detect_stars(&frame0, &params).unwrap();
    println!("  Frame 0: {} stars", stars0.len());

    // Print top 5 star positions from frame 0
    println!("Frame 0 top 5 stars:");
    for s in stars0.iter().take(5) {
        println!("  ({:.1}, {:.1}) flux={:.0}", s.x, s.y, s.flux);
    }

    // Try aligning frames 1, 10, 50, 100
    for &idx in &[1, 10, 50, 100, 200] {
        if idx >= paths.len() { continue; }
        println!("\nAligning frame {} against frame 0...", idx);
        let frame = load_and_process(&paths[idx], &cal);
        let stars = star_detection::detect_stars(&frame, &params).unwrap();
        println!("  Frame {}: {} stars", idx, stars.len());
        println!("  Top 5 stars:");
        for s in stars.iter().take(5) {
            println!("    ({:.1}, {:.1}) flux={:.0}", s.x, s.y, s.flux);
        }

        match alignment::compute_alignment(&stars0, &stars, &align_params) {
            Ok(t) => {
                let (dx, dy) = t.transform_point(0.0, 0.0);
                println!("  SUCCESS: origin maps to ({:.1}, {:.1})", dx, dy);
            }
            Err(e) => {
                println!("  FAILED: {}", e);
            }
        }
    }
}
