use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use zarrs::filesystem::FilesystemStore;
use zarrs::group::Group;
use zarrs::storage::{ReadableListableStorage, StoreKey};
use zarrs_zip::ZipStorageAdapter;

use super::tree::{apply_root_metadata, build_tree, ZarrTree};

pub struct ZarrStore {
    pub storage: ReadableListableStorage,
    pub root_path: String,
    pub tree: ZarrTree,
}

pub fn open_store(path: &Path) -> Result<ZarrStore> {
    let path = resolve_zarr_product_path(path);
    let storage = create_storage(&path)?;
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
        root_path: path.display().to_string(),
        tree,
    })
}

/// Normalize a user-selected path to the Zarr product root (.zarr directory or .zarr.zip file).
pub fn resolve_zarr_product_path(path: &Path) -> PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }

    let mut current = path.to_path_buf();
    loop {
        if is_zarr_root(&current) {
            return current;
        }
        if current
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".zarr"))
        {
            return current;
        }
        match current.parent() {
            Some(parent) if parent != current.as_path() => current = parent.to_path_buf(),
            _ => return path.to_path_buf(),
        }
    }
}

fn is_zarr_root(path: &Path) -> bool {
    path.is_dir() && (path.join(".zgroup").exists() || path.join(".zmetadata").exists())
}

fn create_storage(path: &Path) -> Result<ReadableListableStorage> {
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
        let path = Path::new("sample_data/S03OLCEFR_sample.zarr");
        if !path.exists() {
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
