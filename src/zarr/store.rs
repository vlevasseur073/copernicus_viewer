use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use zarrs::filesystem::FilesystemStore;
use zarrs::group::Group;
use zarrs::storage::storage_adapter::async_to_sync::AsyncToSyncStorageAdapter;
use zarrs::storage::{ReadableListableStorage, StoreKey};
use zarrs_object_store::AsyncObjectStore;
use zarrs_zip::ZipStorageAdapter;

use super::creds::S3Config;
use super::location::{parse_product_location, resolve_product_location, ProductLocation};
use super::runtime::TokioBlockOn;
use super::tree::{apply_root_metadata, build_tree, ZarrTree};

pub use super::location::resolve_zarr_product_path;

/// An opened EOPF Zarr product with readable storage and a pre-built hierarchy tree.
///
/// Array chunk data is not loaded at open time; use [`crate::plot::load_plot_data`] or
/// comparison helpers when pixel values are needed.
pub struct ZarrStore {
    /// Backing zarrs storage (filesystem, zip archive, or S3 via async adapter).
    pub storage: ReadableListableStorage,
    /// Canonical product identifier (absolute local path or `s3://` URI).
    pub root_path: String,
    /// Hierarchy built from group/array metadata at open time.
    pub tree: ZarrTree,
}

/// Open an EOPF Zarr product from a local path or `s3://` URI.
///
/// Accepts `.zarr` directories, `.zarr.zip` archives (local only), and nested paths
/// that are resolved to the product root automatically.
pub fn open_store(input: &str) -> Result<ZarrStore> {
    let loc = parse_product_location(input)?;
    let resolved = resolve_product_location(loc)?;
    let storage = create_storage(&resolved.location)?;
    let root = Group::open(storage.clone(), "/").context("failed to open zarr root group")?;
    // Traverses the hierarchy and reads group/array *metadata* (.zgroup, .zarray, .zattrs).
    // Array chunk data is not read here; that happens in `plot::load_plot_data` on demand.
    let nodes = root
        .traverse()
        .context("failed to traverse zarr hierarchy")?;
    let mut tree = build_tree(&nodes);
    apply_root_metadata(&mut tree, root.metadata());

    Ok(ZarrStore {
        storage,
        root_path: resolved.canonical_id,
        tree,
    })
}

fn create_storage(loc: &ProductLocation) -> Result<ReadableListableStorage> {
    match loc {
        ProductLocation::Local(path) => create_local_storage(path),
        ProductLocation::S3 { bucket, prefix } => create_s3_storage(bucket, prefix),
    }
}

fn create_local_storage(path: &Path) -> Result<ReadableListableStorage> {
    let path_str = path
        .to_str()
        .with_context(|| format!("invalid path: {}", path.display()))?;

    if path.is_dir() {
        let store = FilesystemStore::new(path_str).context("failed to open zarr directory")?;
        return Ok(Arc::new(store));
    }

    if path.extension().and_then(|e| e.to_str()) == Some("zip") {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent_store = FilesystemStore::new(parent.to_str().unwrap())
            .context("failed to open parent directory for zip store")?;
        let zip_key = StoreKey::new(path.file_name().unwrap().to_str().unwrap())
            .context("invalid zip file name")?;
        let zip_store = ZipStorageAdapter::new(Arc::new(parent_store), zip_key)
            .context("failed to open zarr zip archive")?;
        return Ok(Arc::new(zip_store));
    }

    anyhow::bail!(
        "expected a .zarr directory or .zarr.zip file, got: {}",
        path.display()
    )
}

fn create_s3_storage(bucket: &str, prefix: &str) -> Result<ReadableListableStorage> {
    let config_path = s3_config_path();
    let config = S3Config::resolve(bucket, config_path.as_deref())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let prefixed = config
        .build_prefixed_s3_client(bucket, prefix)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let async_store = Arc::new(AsyncObjectStore::new(prefixed));
    let sync_store = AsyncToSyncStorageAdapter::new(async_store, TokioBlockOn::shared());
    Ok(Arc::new(sync_store))
}

fn s3_config_path() -> Option<PathBuf> {
    for var in ["COPERNICUS_VIEWER_S3_CONFIG", "S3_CONFIG"] {
        if let Ok(path) = std::env::var(var) {
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_nested_path_to_zarr_root() {
        let root = PathBuf::from("/data/product.zarr");
        assert_eq!(
            resolve_zarr_product_path(&root.join("measurements/image")),
            root
        );
    }

    #[test]
    fn keeps_zip_path_unchanged() {
        let zip = PathBuf::from("/data/product.zarr.zip");
        assert_eq!(resolve_zarr_product_path(&zip), zip);
    }

    #[test]
    fn open_store_builds_tree_without_reading_array_chunks() {
        let path = "sample_data/S03OLCEFR_sample.zarr";
        if !Path::new(path).exists() {
            return;
        }

        let store = open_store(path).expect("open store");
        let array = store
            .tree
            .root
            .find_by_path("/measurements/image/oa01_radiance")
            .expect("array node in tree");
        assert!(array.is_array());
        // Chunk I/O is confined to plot::load_plot_data; opening only materializes metadata.
    }
}
