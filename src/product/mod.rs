//! Unified access to EOPF Zarr and Sentinel-3 SAFE products.

mod open;

use std::sync::Arc;

use anyhow::Result;
use ndarray::ArrayD;
use zarrs::storage::ReadableListableStorage;

pub use open::open_product;

#[cfg(feature = "safe")]
use crate::safe::SafeStore;
use crate::zarr::{ZarrStore, tree::ZarrTree};

/// An opened Copernicus product (EOPF Zarr or Sentinel-3 SAFE).
#[derive(Clone)]
pub enum Product {
    /// EOPF Zarr store on disk, zip, or S3.
    Zarr(ZarrStore),
    /// Sentinel-3 SAFE directory mapped to EOPF-like hierarchy.
    #[cfg(feature = "safe")]
    Safe(SafeStore),
}

impl Product {
    /// Canonical product path or URI.
    pub fn root_path(&self) -> &str {
        match self {
            Self::Zarr(store) => &store.root_path,
            #[cfg(feature = "safe")]
            Self::Safe(store) => &store.root_path,
        }
    }

    /// Pre-built hierarchy tree shared by both backends.
    pub fn tree(&self) -> &ZarrTree {
        match self {
            Self::Zarr(store) => &store.tree,
            #[cfg(feature = "safe")]
            Self::Safe(store) => &store.tree,
        }
    }

    /// Zarr storage when this product is backed by zarrs (plot/comparison fast path).
    pub fn zarr_storage(&self) -> Option<&ReadableListableStorage> {
        match self {
            Self::Zarr(store) => Some(&store.storage),
            #[cfg(feature = "safe")]
            Self::Safe(_) => None,
        }
    }

    /// Underlying SAFE store when applicable.
    #[cfg(feature = "safe")]
    pub fn as_safe(&self) -> Option<&SafeStore> {
        match self {
            Self::Safe(store) => Some(store),
            Self::Zarr(_) => None,
        }
    }

    /// Read a numeric array subset as `f64` values (backend-agnostic).
    pub fn read_array_subset_f64(
        &self,
        path: &str,
        ranges: &[std::ops::Range<u64>],
    ) -> Result<ArrayD<f64>> {
        match self {
            Self::Zarr(store) => store.read_array_subset_f64(path, ranges),
            #[cfg(feature = "safe")]
            Self::Safe(store) => store.read_array_subset_f64(path, ranges),
        }
    }
}

impl std::fmt::Debug for Product {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Product")
            .field("root_path", &self.root_path())
            .field(
                "format",
                &match self {
                    Self::Zarr(_) => "zarr",
                    #[cfg(feature = "safe")]
                    Self::Safe(_) => "safe",
                },
            )
            .finish()
    }
}

/// Shared pointer type used by the GUI and comparison tool.
pub type ProductHandle = Arc<Product>;
