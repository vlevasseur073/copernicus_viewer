//! Error types for Zarr, L1, and ADF operations. See [`IoError`].
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("S3 credentials error: {0}")]
    S3Credentials(String),

    #[error("S3 client error: {0}")]
    S3Client(String),

    #[error("Zarr store error: {0}")]
    ZarrStore(String),

    #[error("Zarr array error: {0}")]
    ZarrArray(String),

    #[error("Zarr group error: {0}")]
    ZarrGroup(String),

    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
