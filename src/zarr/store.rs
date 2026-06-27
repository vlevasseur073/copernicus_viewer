use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use ndarray::ArrayD;
use zarrs::array::Array;
use zarrs::array::ArraySubset;
use zarrs::filesystem::FilesystemStore;
use zarrs::group::Group;
use zarrs::storage::storage_adapter::async_to_sync::AsyncToSyncStorageAdapter;
use zarrs::storage::{ReadableListableStorage, StoreKey};
use zarrs_object_store::AsyncObjectStore;
use zarrs_zip::ZipStorageAdapter;

use super::creds::S3Config;
use super::location::{ProductLocation, parse_product_location, resolve_product_location};
use super::runtime::TokioBlockOn;
use super::tree::{ZarrTree, apply_root_metadata, build_tree};

pub use super::location::resolve_zarr_product_path;

/// An opened EOPF Zarr product with readable storage and a pre-built hierarchy tree.
#[derive(Clone)]
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

impl ZarrStore {
    /// Read a Zarr array subset as `f64` values.
    pub fn read_array_subset_f64(&self, path: &str, ranges: &[Range<u64>]) -> Result<ArrayD<f64>> {
        let node = self
            .tree
            .root
            .find_by_path(path)
            .with_context(|| format!("unknown array path {path}"))?;
        let dtype = match &node.kind {
            super::tree::ZarrNodeKind::Array { dtype, .. } => dtype.clone(),
            _ => anyhow::bail!("{path} is not an array"),
        };

        let array = Array::open(self.storage.clone(), path)
            .with_context(|| format!("failed to open array at {path}"))?;
        let subset = ArraySubset::new_with_ranges(ranges);
        read_zarr_subset_as_f64(&array, &subset, &dtype)
    }
}

fn read_zarr_subset_as_f64(
    array: &Array<dyn zarrs::storage::ReadableListableStorageTraits>,
    subset: &ArraySubset,
    dtype_hint: &str,
) -> Result<ArrayD<f64>> {
    let normalized = dtype_hint.to_ascii_lowercase();

    macro_rules! read_as {
        ($t:ty) => {{
            let arr: ArrayD<$t> = array
                .retrieve_array_subset::<ArrayD<$t>>(subset)
                .with_context(|| {
                    format!(
                        "failed to read array subset as {}",
                        std::any::type_name::<$t>()
                    )
                })?;
            return Ok(arr.mapv(|v| v as f64));
        }};
    }

    if normalized.contains("float64") || normalized.contains("<f8") || normalized.contains("|f8") {
        read_as!(f64);
    }
    if normalized.contains("float32") || normalized.contains("<f4") || normalized.contains("|f4") {
        read_as!(f32);
    }
    if normalized.contains("uint64") || normalized.contains("<u8") || normalized.contains("|u8") {
        read_as!(u64);
    }
    if normalized.contains("uint32") || normalized.contains("<u4") || normalized.contains("|u4") {
        read_as!(u32);
    }
    if normalized.contains("uint16") || normalized.contains("<u2") || normalized.contains("|u2") {
        read_as!(u16);
    }
    if normalized.contains("uint8") || normalized.contains("<u1") || normalized.contains("|u1") {
        read_as!(u8);
    }
    if normalized.contains("int64") || normalized.contains("<i8") || normalized.contains("|i8") {
        read_as!(i64);
    }
    if normalized.contains("int32") || normalized.contains("<i4") || normalized.contains("|i4") {
        read_as!(i32);
    }
    if normalized.contains("int16") || normalized.contains("<i2") || normalized.contains("|i2") {
        read_as!(i16);
    }
    if normalized.contains("int8") || normalized.contains("<i1") || normalized.contains("|i1") {
        read_as!(i8);
    }

    anyhow::bail!("unsupported data type for array read: {dtype_hint}")
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
    let config =
        S3Config::resolve(bucket, config_path.as_deref()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let prefixed = config
        .build_prefixed_s3_client(bucket, prefix)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let async_store = Arc::new(AsyncObjectStore::new(prefixed));
    let sync_store = AsyncToSyncStorageAdapter::new(async_store, TokioBlockOn::shared());
    Ok(Arc::new(sync_store))
}

fn s3_config_path() -> Option<PathBuf> {
    for var in ["COPERNICUS_VIEWER_S3_CONFIG", "S3_CONFIG"] {
        if let Ok(path) = std::env::var(var)
            && !path.is_empty()
        {
            return Some(PathBuf::from(path));
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
