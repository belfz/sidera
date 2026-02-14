//! Pure-Rust FITS file reader and writer.
//!
//! Supports the primary HDU with image data. Handles BITPIX values
//! 8 (u8), 16 (i16), 32 (i32), -32 (f32), and -64 (f64), with
//! BZERO/BSCALE physical value transformation.

use crate::error::{AstroError, Result};
use crate::image::{FitsHeader, FitsValue, FrameMetadata, ImageData};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

/// FITS block size — all headers and data are padded to multiples of this.
const BLOCK_SIZE: usize = 2880;
/// Each header record is exactly 80 bytes.
const RECORD_SIZE: usize = 80;
/// Number of records per block.
const RECORDS_PER_BLOCK: usize = BLOCK_SIZE / RECORD_SIZE; // 36

/// A loaded FITS file with image data and metadata.
#[derive(Debug, Clone)]
pub struct FitsFile {
    pub metadata: FrameMetadata,
    pub image: ImageData,
}

// ─── Reading ────────────────────────────────────────────────────────────────

/// Read a FITS file from disk (headers + pixel data).
pub fn read_fits(path: &Path) -> Result<FitsFile> {
    let file = File::open(path).map_err(|e| AstroError::io(path, e))?;
    let mut reader = BufReader::new(file);

    let headers = read_headers(&mut reader)?;
    let metadata = FrameMetadata::from_headers(headers);

    let image = read_image_data(&mut reader, &metadata)?;

    Ok(FitsFile { metadata, image })
}

/// Read only the headers from a FITS file (no pixel data loaded).
/// Use this for importing files into a list without consuming memory.
pub fn read_fits_headers(path: &Path) -> Result<FrameMetadata> {
    let file = File::open(path).map_err(|e| AstroError::io(path, e))?;
    let mut reader = BufReader::new(file);

    let headers = read_headers(&mut reader)?;
    Ok(FrameMetadata::from_headers(headers))
}

/// Read all header records from the primary HDU.
fn read_headers(reader: &mut BufReader<File>) -> Result<Vec<FitsHeader>> {
    let mut headers = Vec::new();
    let mut buf = [0u8; RECORD_SIZE];

    'outer: loop {
        // Read one full block of 36 records
        for _ in 0..RECORDS_PER_BLOCK {
            reader
                .read_exact(&mut buf)
                .map_err(|e| AstroError::FitsParse(format!("Failed to read header record: {e}")))?;

            let record = std::str::from_utf8(&buf)
                .map_err(|e| AstroError::FitsParse(format!("Non-ASCII header record: {e}")))?;

            let keyword = record[..8].trim().to_string();

            if keyword == "END" {
                break 'outer;
            }

            if keyword.is_empty() || keyword == "COMMENT" || keyword == "HISTORY" {
                // Skip blank, COMMENT, and HISTORY records
                continue;
            }

            // Check for value indicator "= "
            if record.len() >= 10 && &record[8..10] == "= " {
                let value_comment = &record[10..];
                let (value, comment) = parse_value_comment(value_comment);
                headers.push(FitsHeader {
                    keyword,
                    value,
                    comment,
                });
            }
        }
    }

    Ok(headers)
}

/// Parse the value and optional comment from a FITS header record
/// (the part after "= ").
fn parse_value_comment(raw: &str) -> (FitsValue, Option<String>) {
    let raw = raw.trim();

    // String value: starts with '
    if raw.starts_with('\'') {
        // Find the closing quote
        if let Some(end) = raw[1..].find('\'') {
            let string_val = raw[1..=end].trim().to_string();
            // Look for comment after the closing quote
            let rest = &raw[end + 2..];
            let comment = rest
                .find('/')
                .map(|i| rest[i + 1..].trim().to_string())
                .filter(|s| !s.is_empty());
            return (FitsValue::String(string_val), comment);
        }
        // Malformed string, take everything
        return (FitsValue::String(raw.trim_matches('\'').trim().to_string()), None);
    }

    // Split on '/' for comment
    let (val_str, comment) = if let Some(slash_pos) = find_comment_slash(raw) {
        let c = raw[slash_pos + 1..].trim().to_string();
        let c = if c.is_empty() { None } else { Some(c) };
        (raw[..slash_pos].trim(), c)
    } else {
        (raw.trim(), None)
    };

    // Logical value: T or F
    if val_str == "T" {
        return (FitsValue::Logical(true), comment);
    }
    if val_str == "F" {
        return (FitsValue::Logical(false), comment);
    }

    // Try integer
    if let Ok(i) = val_str.parse::<i64>() {
        return (FitsValue::Integer(i), comment);
    }

    // Try float
    // FITS uses 'D' or 'E' for exponent
    let float_str = val_str.replace('D', "E").replace('d', "e");
    if let Ok(f) = float_str.parse::<f64>() {
        return (FitsValue::Float(f), comment);
    }

    // Fallback: treat as string
    if val_str.is_empty() {
        (FitsValue::None, comment)
    } else {
        (FitsValue::String(val_str.to_string()), comment)
    }
}

/// Find the position of the comment separator '/' that is NOT inside a string.
fn find_comment_slash(s: &str) -> Option<usize> {
    let mut in_string = false;
    for (i, ch) in s.char_indices() {
        if ch == '\'' {
            in_string = !in_string;
        }
        if ch == '/' && !in_string {
            return Some(i);
        }
    }
    None
}

/// Read image data from the primary HDU based on header metadata.
fn read_image_data(reader: &mut BufReader<File>, meta: &FrameMetadata) -> Result<ImageData> {
    let naxis = meta
        .header_map
        .get("NAXIS")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as usize;

    if naxis == 0 {
        return Err(AstroError::FitsParse("NAXIS is 0 — no image data".into()));
    }

    let naxis1 = meta
        .header_map
        .get("NAXIS1")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AstroError::FitsHeader("Missing NAXIS1".into()))? as usize;

    let naxis2 = meta
        .header_map
        .get("NAXIS2")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AstroError::FitsHeader("Missing NAXIS2".into()))? as usize;

    let naxis3 = if naxis >= 3 {
        meta.header_map
            .get("NAXIS3")
            .and_then(|v| v.as_i64())
            .unwrap_or(1) as usize
    } else {
        1
    };

    let bitpix = meta.bitpix;

    let bzero = meta
        .header_map
        .get("BZERO")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let bscale = meta
        .header_map
        .get("BSCALE")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);

    let width = naxis1;
    let height = naxis2;
    let channels = naxis3;
    let total_pixels = width * height * channels;

    // Read raw data based on BITPIX
    let data = match bitpix {
        8 => read_data_u8(reader, total_pixels, bzero, bscale)?,
        16 => read_data_i16(reader, total_pixels, bzero, bscale)?,
        32 => read_data_i32(reader, total_pixels, bzero, bscale)?,
        -32 => read_data_f32(reader, total_pixels, bzero, bscale)?,
        -64 => read_data_f64(reader, total_pixels, bzero, bscale)?,
        _ => return Err(AstroError::UnsupportedBitpix(bitpix)),
    };

    // FITS stores data in [channel][row][col] order (plane-interleaved).
    // We need to convert to pixel-interleaved [row][col][channel].
    let data = if channels > 1 {
        plane_to_pixel_interleaved(&data, width, height, channels)
    } else {
        data
    };

    ImageData::from_data(width, height, channels, data)
}

fn read_data_u8(
    reader: &mut BufReader<File>,
    count: usize,
    bzero: f64,
    bscale: f64,
) -> Result<Vec<f32>> {
    let mut buf = vec![0u8; count];
    reader
        .read_exact(&mut buf)
        .map_err(|e| AstroError::FitsParse(format!("Failed to read image data: {e}")))?;
    skip_padding(reader, count)?;
    Ok(buf
        .iter()
        .map(|&v| (bzero + bscale * v as f64) as f32)
        .collect())
}

fn read_data_i16(
    reader: &mut BufReader<File>,
    count: usize,
    bzero: f64,
    bscale: f64,
) -> Result<Vec<f32>> {
    let mut data = Vec::with_capacity(count);
    for _ in 0..count {
        let v = reader
            .read_i16::<BigEndian>()
            .map_err(|e| AstroError::FitsParse(format!("Failed to read i16 data: {e}")))?;
        data.push((bzero + bscale * v as f64) as f32);
    }
    skip_padding(reader, count * 2)?;
    Ok(data)
}

fn read_data_i32(
    reader: &mut BufReader<File>,
    count: usize,
    bzero: f64,
    bscale: f64,
) -> Result<Vec<f32>> {
    let mut data = Vec::with_capacity(count);
    for _ in 0..count {
        let v = reader
            .read_i32::<BigEndian>()
            .map_err(|e| AstroError::FitsParse(format!("Failed to read i32 data: {e}")))?;
        data.push((bzero + bscale * v as f64) as f32);
    }
    skip_padding(reader, count * 4)?;
    Ok(data)
}

fn read_data_f32(
    reader: &mut BufReader<File>,
    count: usize,
    bzero: f64,
    bscale: f64,
) -> Result<Vec<f32>> {
    let mut data = Vec::with_capacity(count);
    for _ in 0..count {
        let v = reader
            .read_f32::<BigEndian>()
            .map_err(|e| AstroError::FitsParse(format!("Failed to read f32 data: {e}")))?;
        data.push((bzero + bscale * v as f64) as f32);
    }
    skip_padding(reader, count * 4)?;
    Ok(data)
}

fn read_data_f64(
    reader: &mut BufReader<File>,
    count: usize,
    bzero: f64,
    bscale: f64,
) -> Result<Vec<f32>> {
    let mut data = Vec::with_capacity(count);
    for _ in 0..count {
        let v = reader
            .read_f64::<BigEndian>()
            .map_err(|e| AstroError::FitsParse(format!("Failed to read f64 data: {e}")))?;
        data.push((bzero + bscale * v) as f32);
    }
    skip_padding(reader, count * 8)?;
    Ok(data)
}

/// Skip padding bytes to align to the next BLOCK_SIZE boundary.
fn skip_padding(reader: &mut BufReader<File>, bytes_read: usize) -> Result<()> {
    let remainder = bytes_read % BLOCK_SIZE;
    if remainder != 0 {
        let pad = BLOCK_SIZE - remainder;
        let mut skip = vec![0u8; pad];
        reader
            .read_exact(&mut skip)
            .map_err(|e| AstroError::FitsParse(format!("Failed to skip padding: {e}")))?;
    }
    Ok(())
}

/// Convert plane-interleaved [channel][row][col] to pixel-interleaved [row][col][channel].
fn plane_to_pixel_interleaved(
    data: &[f32],
    width: usize,
    height: usize,
    channels: usize,
) -> Vec<f32> {
    let plane_size = width * height;
    let mut result = vec![0.0f32; width * height * channels];
    for c in 0..channels {
        for i in 0..plane_size {
            result[i * channels + c] = data[c * plane_size + i];
        }
    }
    result
}

// ─── Writing ────────────────────────────────────────────────────────────────

/// Write an image to a FITS file (32-bit float, primary HDU only).
pub fn write_fits(
    path: &Path,
    image: &ImageData,
    extra_headers: &HashMap<String, FitsValue>,
) -> Result<()> {
    let file = File::create(path).map_err(|e| AstroError::io(path, e))?;
    let mut writer = BufWriter::new(file);

    write_headers(&mut writer, image, extra_headers)?;
    write_image_data(&mut writer, image)?;

    writer.flush().map_err(|e| AstroError::io(path, e))?;
    Ok(())
}

/// Write the primary HDU header.
fn write_headers(
    writer: &mut BufWriter<File>,
    image: &ImageData,
    extra: &HashMap<String, FitsValue>,
) -> Result<()> {
    let mut records: Vec<String> = Vec::new();

    // Mandatory keywords
    records.push(format_header_logical("SIMPLE", true, Some("FITS standard")));
    records.push(format_header_integer("BITPIX", -32, Some("32-bit float")));

    if image.channels == 1 {
        records.push(format_header_integer("NAXIS", 2, Some("Number of axes")));
    } else {
        records.push(format_header_integer("NAXIS", 3, Some("Number of axes")));
    }

    records.push(format_header_integer(
        "NAXIS1",
        image.width as i64,
        Some("Width"),
    ));
    records.push(format_header_integer(
        "NAXIS2",
        image.height as i64,
        Some("Height"),
    ));

    if image.channels > 1 {
        records.push(format_header_integer(
            "NAXIS3",
            image.channels as i64,
            Some("Channels"),
        ));
    }

    // BZERO and BSCALE (identity for float data)
    records.push(format_header_float("BZERO", 0.0, Some("Zero offset")));
    records.push(format_header_float("BSCALE", 1.0, Some("Scale factor")));

    // Extra headers
    for (keyword, value) in extra {
        let rec = match value {
            FitsValue::Integer(v) => format_header_integer(keyword, *v, None),
            FitsValue::Float(v) => format_header_float(keyword, *v, None),
            FitsValue::String(v) => format_header_string(keyword, v, None),
            FitsValue::Logical(v) => format_header_logical(keyword, *v, None),
            FitsValue::None => continue,
        };
        records.push(rec);
    }

    // END record
    records.push(format!("{:<80}", "END"));

    // Write records, padded to block boundary
    let mut bytes_written = 0;
    for rec in &records {
        writer
            .write_all(rec.as_bytes())
            .map_err(|e| AstroError::Encoding(format!("Failed to write header: {e}")))?;
        bytes_written += RECORD_SIZE;
    }

    // Pad to block boundary
    let remainder = bytes_written % BLOCK_SIZE;
    if remainder != 0 {
        let pad = BLOCK_SIZE - remainder;
        writer
            .write_all(&vec![b' '; pad])
            .map_err(|e| AstroError::Encoding(format!("Failed to write header padding: {e}")))?;
    }

    Ok(())
}

/// Write image data as 32-bit big-endian floats.
fn write_image_data(writer: &mut BufWriter<File>, image: &ImageData) -> Result<()> {
    // Convert pixel-interleaved to plane-interleaved if multi-channel
    let data = if image.channels > 1 {
        pixel_to_plane_interleaved(&image.data, image.width, image.height, image.channels)
    } else {
        image.data.clone()
    };

    let mut bytes_written = 0;
    for &value in &data {
        writer
            .write_f32::<BigEndian>(value)
            .map_err(|e| AstroError::Encoding(format!("Failed to write pixel data: {e}")))?;
        bytes_written += 4;
    }

    // Pad to block boundary
    let remainder = bytes_written % BLOCK_SIZE;
    if remainder != 0 {
        let pad = BLOCK_SIZE - remainder;
        writer
            .write_all(&vec![0u8; pad])
            .map_err(|e| AstroError::Encoding(format!("Failed to write data padding: {e}")))?;
    }

    Ok(())
}

/// Convert pixel-interleaved [row][col][channel] to plane-interleaved [channel][row][col].
fn pixel_to_plane_interleaved(
    data: &[f32],
    width: usize,
    height: usize,
    channels: usize,
) -> Vec<f32> {
    let plane_size = width * height;
    let mut result = vec![0.0f32; width * height * channels];
    for c in 0..channels {
        for i in 0..plane_size {
            result[c * plane_size + i] = data[i * channels + c];
        }
    }
    result
}

// ─── Header formatting helpers ──────────────────────────────────────────────

fn format_header_integer(keyword: &str, value: i64, comment: Option<&str>) -> String {
    let val_str = format!("{:>20}", value);
    format_record(keyword, &val_str, comment)
}

fn format_header_float(keyword: &str, value: f64, comment: Option<&str>) -> String {
    let val_str = format!("{:>20.10E}", value);
    format_record(keyword, &val_str, comment)
}

fn format_header_string(keyword: &str, value: &str, comment: Option<&str>) -> String {
    // FITS strings are enclosed in single quotes, padded to at least 8 chars
    let padded = format!("{:<8}", value);
    let val_str = format!("'{}'", padded);
    // Left-align the string value with padding
    let val_str = format!("{:<20}", val_str);
    format_record(keyword, &val_str, comment)
}

fn format_header_logical(keyword: &str, value: bool, comment: Option<&str>) -> String {
    let val_str = format!("{:>20}", if value { "T" } else { "F" });
    format_record(keyword, &val_str, comment)
}

fn format_record(keyword: &str, value: &str, comment: Option<&str>) -> String {
    let kw = format!("{:<8}", keyword);
    let record = if let Some(cmt) = comment {
        format!("{}= {} / {}", kw, value, cmt)
    } else {
        format!("{}= {}", kw, value)
    };
    format!("{:<80}", &record[..record.len().min(80)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_value_integer() {
        let (val, comment) = parse_value_comment("                  42 / The answer");
        assert!(matches!(val, FitsValue::Integer(42)));
        assert_eq!(comment, Some("The answer".to_string()));
    }

    #[test]
    fn test_parse_value_float() {
        let (val, _) = parse_value_comment("  1.23456789E+02");
        match val {
            FitsValue::Float(f) => assert!((f - 123.456789).abs() < 1e-3),
            _ => panic!("Expected float"),
        }
    }

    #[test]
    fn test_parse_value_string() {
        let (val, _) = parse_value_comment("'RGGB    '");
        assert!(matches!(val, FitsValue::String(s) if s == "RGGB"));
    }

    #[test]
    fn test_parse_value_logical() {
        let (val, _) = parse_value_comment("                   T");
        assert!(matches!(val, FitsValue::Logical(true)));
    }

    #[test]
    fn test_plane_to_pixel_roundtrip() {
        let plane = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
        let pixel = plane_to_pixel_interleaved(&plane, 2, 2, 3);
        let back = pixel_to_plane_interleaved(&pixel, 2, 2, 3);
        assert_eq!(plane, back);
    }
}
