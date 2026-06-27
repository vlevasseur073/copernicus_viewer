use std::path::Path;

use anyhow::{Result, bail};

use super::Product;
use crate::zarr::{open_store, resolve_zarr_product_path};

#[cfg(not(feature = "safe"))]
const SAFE_DISABLED_MSG: &str =
    "Sentinel-3 SAFE support was disabled at compile time (rebuild with --features safe)";

#[cfg(not(feature = "safe"))]
fn looks_like_safe_dir(path: &Path) -> bool {
    path.is_dir()
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".SEN3"))
        && path.join("xfdumanifest.xml").is_file()
}

/// Open an EOPF Zarr product or a Sentinel-3 SAFE directory.
pub fn open_product(input: &str) -> Result<Product> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("product path cannot be empty");
    }

    if trimmed.starts_with("s3://") {
        return open_store(trimmed).map(Product::Zarr);
    }

    let path = Path::new(trimmed);

    #[cfg(feature = "safe")]
    {
        use crate::safe::{SafeStore, is_safe_product_dir};

        if is_safe_product_dir(path) {
            return SafeStore::open(path).map(Product::Safe);
        }

        let resolved = resolve_zarr_product_path(path);
        if is_safe_product_dir(&resolved) {
            return SafeStore::open(&resolved).map(Product::Safe);
        }
    }

    #[cfg(not(feature = "safe"))]
    {
        if looks_like_safe_dir(path) {
            bail!(SAFE_DISABLED_MSG);
        }
        let resolved = resolve_zarr_product_path(path);
        if looks_like_safe_dir(&resolved) {
            bail!(SAFE_DISABLED_MSG);
        }
    }

    open_store(trimmed).map(Product::Zarr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_sample_zarr() {
        let path = Path::new("sample_data/S03OLCEFR_sample.zarr");
        if !path.exists() {
            return;
        }
        let product = open_product(path.to_str().unwrap()).expect("open");
        assert!(matches!(product, Product::Zarr(_)));
    }

    #[cfg(feature = "safe")]
    #[test]
    fn dispatches_sl_lst_safe_when_present() {
        let path = Path::new(
            "/home/vincent/Data/SLSTR/S3A_SL_2_LST____20260622T102053_20260622T102353_20260622T123949_0179_141_008_2160_PS1_O_NR_005.SEN3",
        );
        if !path.exists() {
            return;
        }
        let product = open_product(path.to_str().unwrap()).expect("open safe");
        assert!(matches!(product, Product::Safe(_)));
        let lst = product
            .tree()
            .root
            .find_by_path("/measurements/lst")
            .expect("lst path");
        assert!(lst.is_array());
    }
}
