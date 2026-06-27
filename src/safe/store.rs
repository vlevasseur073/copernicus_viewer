use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ndarray::ArrayD;

use super::io::{probe_variable, read_subset_f64};
use super::manifest::parse_manifest_attributes;
use super::mapping::{ArraySource, build_tree, load_mapping};
use super::product_type::detect_product_type;
use crate::zarr::tree::ZarrTree;

/// An opened Sentinel-3 SAFE product with EOPF-like hierarchy metadata.
#[derive(Clone, Debug)]
pub struct SafeStore {
    /// Absolute path to the `.SEN3` directory.
    pub root_path: String,
    /// EOPF product type code.
    pub product_type: String,
    /// Hierarchy built from sentineltoolbox mapping JSON.
    pub tree: ZarrTree,
    /// Tree path → NetCDF source.
    pub arrays: HashMap<String, ArraySource>,
    /// Geolocation coord tree paths (`latitude_in` → `/coords/latitude_in`).
    pub coord_paths: HashMap<String, String>,
}

/// Returns `true` when `path` is a Sentinel-3 SAFE product directory.
pub fn is_safe_product_dir(path: &Path) -> bool {
    path.is_dir()
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".SEN3"))
        && path.join("xfdumanifest.xml").is_file()
}

impl SafeStore {
    /// Open a `.SEN3` directory and build hierarchy metadata from mapping JSON.
    pub fn open(path: &Path) -> Result<Self> {
        if !is_safe_product_dir(path) {
            bail!(
                "expected a Sentinel-3 SAFE directory (*.SEN3 with xfdumanifest.xml): {}",
                path.display()
            );
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("invalid SAFE path")?;
        let product_type = detect_product_type(name)?;
        let mapping = load_mapping(&product_type)?;

        let chunk_sizes: Vec<(String, u64)> = mapping
            .chunk_sizes
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        let root_dir = path.to_path_buf();
        let probe = |source: &ArraySource| {
            let nc_path = root_dir.join(&source.nc_path);
            probe_variable(&nc_path, &source.var_name, &chunk_sizes)
        };

        let manifest_path = path.join("xfdumanifest.xml");
        let root_attributes = parse_manifest_attributes(&manifest_path).unwrap_or_default();

        let tree = build_tree(&mapping, probe, root_attributes)?;

        let arrays = mapping
            .arrays
            .into_iter()
            .map(|(k, mut v)| {
                v.nc_path = root_dir.join(&v.nc_path);
                (k, v)
            })
            .collect();

        Ok(Self {
            root_path: path.display().to_string(),
            product_type,
            tree,
            arrays,
            coord_paths: mapping.coord_paths,
        })
    }

    /// Read array data at a hierarchy path for the given dimension ranges.
    pub fn read_array_subset_f64(&self, path: &str, ranges: &[Range<u64>]) -> Result<ArrayD<f64>> {
        let normalized = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        let source = self
            .arrays
            .get(&normalized)
            .with_context(|| format!("unknown SAFE array path {normalized}"))?;
        read_subset_f64(&source.nc_path, &source.var_name, ranges)
    }

    /// Resolve geolocation coordinate tree paths for a 2D measurement array.
    pub fn georef_coord_paths(&self, array_path: &str) -> Option<(String, String)> {
        let suffix = array_path.trim_end_matches('/').rsplit('/').next()?;

        let lat_keys = [
            format!("latitude_{suffix}"),
            "latitude_in".to_string(),
            "latitude".to_string(),
        ];
        let lon_keys = [
            format!("longitude_{suffix}"),
            "longitude_in".to_string(),
            "longitude".to_string(),
        ];

        let lat = lat_keys
            .iter()
            .find_map(|k| self.coord_paths.get(k))
            .cloned()?;
        let lon = lon_keys
            .iter()
            .find_map(|k| self.coord_paths.get(k))
            .cloned()?;
        Some((lat, lon))
    }
}
