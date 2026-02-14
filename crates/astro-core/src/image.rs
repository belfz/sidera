use crate::error::{AstroError, Result};
use std::collections::HashMap;

/// The type of frame, as indicated by the IMAGETYP FITS header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameType {
    Light,
    Dark,
    Flat,
    Bias,
    Unknown,
}

impl FrameType {
    /// Parse from a FITS IMAGETYP header value string.
    pub fn from_header(value: &str) -> Self {
        let v = value.trim().to_lowercase();
        if v.contains("light") {
            FrameType::Light
        } else if v.contains("dark") {
            FrameType::Dark
        } else if v.contains("flat") {
            FrameType::Flat
        } else if v.contains("bias") || v.contains("offset") {
            FrameType::Bias
        } else {
            FrameType::Unknown
        }
    }
}

/// A single value from a FITS header record.
#[derive(Debug, Clone)]
pub enum FitsValue {
    Integer(i64),
    Float(f64),
    String(String),
    Logical(bool),
    None,
}

impl FitsValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            FitsValue::Integer(v) => Some(*v),
            FitsValue::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            FitsValue::Float(v) => Some(*v),
            FitsValue::Integer(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            FitsValue::String(v) => Some(v.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            FitsValue::Logical(v) => Some(*v),
            _ => None,
        }
    }
}

/// A FITS header record (keyword = value / comment).
#[derive(Debug, Clone)]
pub struct FitsHeader {
    pub keyword: String,
    pub value: FitsValue,
    pub comment: Option<String>,
}

/// Core image data container. All pixel values are stored as f32 in
/// row-major order. For multi-channel images, data is stored as
/// [R0, G0, B0, R1, G1, B1, ...] (pixel-interleaved).
#[derive(Debug, Clone)]
pub struct ImageData {
    /// Image width in pixels.
    pub width: usize,
    /// Image height in pixels.
    pub height: usize,
    /// Number of channels (1 = mono, 3 = RGB).
    pub channels: usize,
    /// Pixel data in row-major, channel-interleaved order.
    /// Length = width * height * channels.
    pub data: Vec<f32>,
}

impl ImageData {
    /// Create a new image filled with zeros.
    pub fn new(width: usize, height: usize, channels: usize) -> Self {
        ImageData {
            width,
            height,
            channels,
            data: vec![0.0; width * height * channels],
        }
    }

    /// Create an image from existing data.
    pub fn from_data(width: usize, height: usize, channels: usize, data: Vec<f32>) -> Result<Self> {
        let expected = width * height * channels;
        if data.len() != expected {
            return Err(AstroError::InvalidDimensions {
                expected: format!("{}x{}x{} = {} pixels", width, height, channels, expected),
                got: format!("{} pixels", data.len()),
            });
        }
        Ok(ImageData {
            width,
            height,
            channels,
            data,
        })
    }

    /// Total number of pixels (width * height).
    pub fn pixel_count(&self) -> usize {
        self.width * self.height
    }

    /// Get the value at (x, y, channel).
    #[inline]
    pub fn get(&self, x: usize, y: usize, c: usize) -> f32 {
        self.data[(y * self.width + x) * self.channels + c]
    }

    /// Set the value at (x, y, channel).
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, c: usize, value: f32) {
        self.data[(y * self.width + x) * self.channels + c] = value;
    }

    /// Get the index into the data array for (x, y, channel).
    #[inline]
    pub fn index(&self, x: usize, y: usize, c: usize) -> usize {
        (y * self.width + x) * self.channels + c
    }

    /// Check if dimensions match another image.
    pub fn same_dimensions(&self, other: &ImageData) -> bool {
        self.width == other.width && self.height == other.height && self.channels == other.channels
    }

    /// Convert a mono (1-channel) image to 3-channel RGB by duplicating.
    pub fn to_rgb(&self) -> Result<ImageData> {
        if self.channels == 3 {
            return Ok(self.clone());
        }
        if self.channels != 1 {
            return Err(AstroError::InvalidDimensions {
                expected: "1 or 3 channels".into(),
                got: format!("{} channels", self.channels),
            });
        }
        let mut rgb = ImageData::new(self.width, self.height, 3);
        for i in 0..self.pixel_count() {
            let v = self.data[i];
            rgb.data[i * 3] = v;
            rgb.data[i * 3 + 1] = v;
            rgb.data[i * 3 + 2] = v;
        }
        Ok(rgb)
    }

    /// Extract a single channel as a mono image.
    pub fn extract_channel(&self, channel: usize) -> Result<ImageData> {
        if channel >= self.channels {
            return Err(AstroError::InvalidDimensions {
                expected: format!("channel < {}", self.channels),
                got: format!("channel {}", channel),
            });
        }
        let mut mono = ImageData::new(self.width, self.height, 1);
        for i in 0..self.pixel_count() {
            mono.data[i] = self.data[i * self.channels + channel];
        }
        Ok(mono)
    }

    /// Compute the luminance (0.2126R + 0.7152G + 0.0722B) as a mono image.
    /// For mono input, returns a clone.
    pub fn to_luminance(&self) -> Result<ImageData> {
        if self.channels == 1 {
            return Ok(self.clone());
        }
        if self.channels != 3 {
            return Err(AstroError::InvalidDimensions {
                expected: "1 or 3 channels".into(),
                got: format!("{} channels", self.channels),
            });
        }
        let mut lum = ImageData::new(self.width, self.height, 1);
        for i in 0..self.pixel_count() {
            let r = self.data[i * 3];
            let g = self.data[i * 3 + 1];
            let b = self.data[i * 3 + 2];
            lum.data[i] = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        }
        Ok(lum)
    }

    /// Clamp all values to [min, max].
    pub fn clamp(&mut self, min: f32, max: f32) {
        for v in &mut self.data {
            *v = v.clamp(min, max);
        }
    }

    /// Compute min and max values across all channels.
    pub fn min_max(&self) -> (f32, f32) {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for &v in &self.data {
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
        }
        (min, max)
    }

    /// Normalize the image to [0, 1] range based on current min/max.
    pub fn normalize(&mut self) {
        let (min, max) = self.min_max();
        let range = max - min;
        if range <= 0.0 {
            return;
        }
        for v in &mut self.data {
            *v = (*v - min) / range;
        }
    }
}

/// Metadata extracted from FITS headers relevant to astrophotography.
#[derive(Debug, Clone)]
pub struct FrameMetadata {
    pub frame_type: FrameType,
    pub exposure_time: Option<f64>,
    pub temperature: Option<f64>,
    pub gain: Option<f64>,
    pub filter: Option<String>,
    pub bayer_pattern: Option<String>,
    pub binning_x: Option<u32>,
    pub binning_y: Option<u32>,
    pub bitpix: i64,
    /// All raw FITS headers.
    pub headers: Vec<FitsHeader>,
    /// Quick lookup by keyword.
    pub header_map: HashMap<String, FitsValue>,
}

impl FrameMetadata {
    /// Build metadata from a list of FITS headers.
    pub fn from_headers(headers: Vec<FitsHeader>) -> Self {
        let mut header_map = HashMap::new();
        for h in &headers {
            header_map.insert(h.keyword.clone(), h.value.clone());
        }

        let frame_type = header_map
            .get("IMAGETYP")
            .and_then(|v| v.as_str())
            .map(FrameType::from_header)
            .unwrap_or(FrameType::Unknown);

        let exposure_time = header_map
            .get("EXPTIME")
            .or_else(|| header_map.get("EXPOSURE"))
            .and_then(|v| v.as_f64());

        let temperature = header_map
            .get("CCD-TEMP")
            .or_else(|| header_map.get("SET-TEMP"))
            .and_then(|v| v.as_f64());

        let gain = header_map.get("GAIN").and_then(|v| v.as_f64());

        let filter = header_map
            .get("FILTER")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let bayer_pattern = header_map
            .get("BAYERPAT")
            .or_else(|| header_map.get("COLORTYP"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let binning_x = header_map
            .get("XBINNING")
            .and_then(|v| v.as_i64())
            .map(|v| v as u32);

        let binning_y = header_map
            .get("YBINNING")
            .and_then(|v| v.as_i64())
            .map(|v| v as u32);

        let bitpix = header_map
            .get("BITPIX")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        FrameMetadata {
            frame_type,
            exposure_time,
            temperature,
            gain,
            filter,
            bayer_pattern,
            binning_x,
            binning_y,
            bitpix,
            headers,
            header_map,
        }
    }
}
