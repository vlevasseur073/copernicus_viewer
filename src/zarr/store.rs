use std::path::Path;
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
    let storage = create_storage(path)?;
    let root = Group::open(storage.clone(), "/").context("failed to open zarr root group")?;
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
