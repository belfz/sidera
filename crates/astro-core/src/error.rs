use std::path::PathBuf;

/// All errors that can occur in astro-core.
#[derive(Debug, thiserror::Error)]
pub enum AstroError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("FITS parse error: {0}")]
    FitsParse(String),

    #[error("Invalid FITS header: {0}")]
    FitsHeader(String),

    #[error("Unsupported BITPIX value: {0}")]
    UnsupportedBitpix(i64),

    #[error("Image dimension mismatch: {0}")]
    DimensionMismatch(String),

    #[error("Invalid image dimensions: expected {expected}, got {got}")]
    InvalidDimensions { expected: String, got: String },

    #[error("Calibration error: {0}")]
    Calibration(String),

    #[error("No frames provided for {operation}")]
    NoFrames { operation: String },

    #[error("Star detection error: {0}")]
    StarDetection(String),

    #[error("Alignment error: {0}")]
    Alignment(String),

    #[error("Not enough stars for alignment: found {found}, need {need}")]
    InsufficientStars { found: usize, need: usize },

    #[error("Stacking error: {0}")]
    Stacking(String),

    #[error("Image encoding error: {0}")]
    Encoding(String),
}

pub type Result<T> = std::result::Result<T, AstroError>;

impl AstroError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        AstroError::Io {
            path: path.into(),
            source,
        }
    }
}
