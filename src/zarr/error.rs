//! Error types for Zarr store and S3 I/O.

use thiserror::Error;

/// I/O and configuration errors when opening stores or reading arrays.
#[derive(Debug, Error)]
pub enum IoError {
    /// A required local path does not exist.
    #[error("file not found: {0}")]
    FileNotFound(String),

    /// S3 credentials could not be resolved or parsed.
    #[error("S3 credentials error: {0}")]
    S3Credentials(String),

    /// Failed to construct or use an S3 client.
    #[error("S3 client error: {0}")]
    S3Client(String),

    /// Zarr store-level read or traversal failure.
    #[error("Zarr store error: {0}")]
    ZarrStore(String),

    /// Zarr array open or chunk read failure.
    #[error("Zarr array error: {0}")]
    ZarrArray(String),

    /// Zarr group open or metadata failure.
    #[error("Zarr group error: {0}")]
    ZarrGroup(String),

    /// Unsupported Zarr layout, dtype, or storage backend.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    /// Underlying OS I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
