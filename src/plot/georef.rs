use std::ops::Range;

use anyhow::{Context, Result};
use ndarray::ArrayD;
use serde_json::Map;
use zarrs::array::Array;
use zarrs::array::ArraySubset;
use zarrs::storage::ReadableListableStorage;

use crate::zarr::{ZarrNodeKind, ZarrTreeNode};

#[derive(Clone, Debug, Default)]
pub struct GeorefInfo {
    pub crs: Option<String>,
    pub x_name: String,
    pub y_name: String,
    pub x_coords: Option<Vec<f64>>,
    pub y_coords: Option<Vec<f64>>,
    pub x_unit: Option<String>,
    pub y_unit: Option<String>,
}

const X_ALIASES: &[&str] = &["x", "lon", "longitude", "easting"];
const Y_ALIASES: &[&str] = &["y", "lat", "latitude", "northing"];

pub fn resolve_georef(
    storage: &ReadableListableStorage,
    tree: &ZarrTreeNode,
    array_path: &str,
    kind: &ZarrNodeKind,
    y_range: &Range<u64>,
    x_range: &Range<u64>,
) -> Result<GeorefInfo> {
    let ZarrNodeKind::Array {
        dimension_names,
        attributes,
        ..
    } = kind
    else {
        anyhow::bail!("not an array");
    };

    let y_name = dimension_names
        .get(dimension_names.len().saturating_sub(2))
        .filter(|n| *n != "_")
        .cloned()
        .unwrap_or_else(|| "y".to_string());
    let x_name = dimension_names
        .last()
        .filter(|n| *n != "_")
        .cloned()
        .unwrap_or_else(|| "x".to_string());

    let crs = extract_crs(attributes);
    let parent = parent_path(array_path);

    let y_coords = resolve_coord(storage, tree, &parent, &y_name, Y_ALIASES, y_range)?;
    let x_coords = resolve_coord(storage, tree, &parent, &x_name, X_ALIASES, x_range)?;

    let y_unit = y_coords
        .as_ref()
        .and_then(|(_, attrs)| attrs.get("units").and_then(|v| v.as_str()).map(str::to_string));
    let x_unit = x_coords
        .as_ref()
        .and_then(|(_, attrs)| attrs.get("units").and_then(|v| v.as_str()).map(str::to_string));

    Ok(GeorefInfo {
        crs,
        x_name: x_name.clone(),
        y_name: y_name.clone(),
        x_coords: x_coords.map(|(v, _)| v),
        y_coords: y_coords.map(|(v, _)| v),
        x_unit,
        y_unit,
    })
}

fn extract_crs(attributes: &Map<String, serde_json::Value>) -> Option<String> {
    for key in ["crs", "_CRS", "spatial_ref", "grid_mapping", "epsg"] {
        if let Some(v) = attributes.get(key) {
            return Some(match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            });
        }
    }
    attributes
        .get("grid_mapping_name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn parent_path(array_path: &str) -> String {
    let trimmed = array_path.trim_end_matches('/');
    match trimmed.rfind('/') {
        None | Some(0) => "/".to_string(),
        Some(idx) => trimmed[..idx].to_string(),
    }
}

fn resolve_coord(
    storage: &ReadableListableStorage,
    tree: &ZarrTreeNode,
    parent: &str,
    name: &str,
    aliases: &[&str],
    range: &Range<u64>,
) -> Result<Option<(Vec<f64>, Map<String, serde_json::Value>)>> {
    if let Some(found) = read_coord_array(storage, tree, parent, name, range)? {
        return Ok(Some(found));
    }
    try_alias_coord(storage, tree, parent, aliases, range)
}

fn try_alias_coord(
    storage: &ReadableListableStorage,
    tree: &ZarrTreeNode,
    parent: &str,
    aliases: &[&str],
    range: &Range<u64>,
) -> Result<Option<(Vec<f64>, Map<String, serde_json::Value>)>> {
    for alias in aliases {
        if let Ok(Some(found)) = read_coord_array(storage, tree, parent, alias, range) {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn read_coord_array(
    storage: &ReadableListableStorage,
    tree: &ZarrTreeNode,
    parent: &str,
    name: &str,
    range: &Range<u64>,
) -> Result<Option<(Vec<f64>, Map<String, serde_json::Value>)>> {
    let path = if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    };

    let Some(node) = tree.find_by_path(&path) else {
        return Ok(None);
    };

    let ZarrNodeKind::Array { dtype, attributes, shape, .. } = &node.kind else {
        return Ok(None);
    };

    if shape.len() != 1 {
        return Ok(None);
    }

    let start = range.start.min(shape[0]);
    let end = range.end.min(shape[0]);
    if start >= end {
        return Ok(None);
    }

    let array = Array::open(storage.clone(), &path)
        .with_context(|| format!("failed to open coordinate array {path}"))?;
    let subset = ArraySubset::new_with_ranges(&[start..end]);
    let values = read_as_f64(&array, &subset, dtype)?;

    Ok(Some((values, attributes.clone())))
}

fn read_as_f64(
    array: &Array<dyn zarrs::storage::ReadableListableStorageTraits>,
    subset: &ArraySubset,
    dtype_hint: &str,
) -> Result<Vec<f64>> {
    let normalized = dtype_hint.to_ascii_lowercase();
    macro_rules! read_as {
        ($t:ty) => {{
            let arr: ArrayD<$t> = array.retrieve_array_subset::<ArrayD<$t>>(subset)?;
            return Ok(arr.iter().map(|&v| v as f64).collect());
        }};
    }

    if normalized.contains("float64") || normalized.contains("<f8") {
        read_as!(f64);
    }
    if normalized.contains("float32") || normalized.contains("<f4") {
        read_as!(f32);
    }
    if normalized.contains("int") || normalized.contains("uint") {
        read_as!(i64);
    }
    read_as!(f64);
}

pub fn axis_label(info: &GeorefInfo, axis: char, index: usize, fallback: f64) -> String {
    let coords = match axis {
        'x' => info.x_coords.as_ref(),
        'y' => info.y_coords.as_ref(),
        _ => None,
    };
    if let Some(c) = coords.and_then(|v| v.get(index)) {
        if c.fract() == 0.0 && c.abs() < 1e8 {
            return format!("{c:.0}");
        }
        return format!("{c:.3}");
    }
    format!("{fallback:.0}")
}

pub fn extent_description(info: &GeorefInfo) -> String {
    let mut parts = Vec::new();
    if let Some(crs) = &info.crs {
        parts.push(format!("CRS: {crs}"));
    }
    if let (Some(x), Some(y)) = (&info.x_coords, &info.y_coords) {
        if let (Some(x0), Some(x1)) = (x.first(), x.last()) {
            parts.push(format!("{}: {x0:.4} … {x1:.4}", info.x_name));
        }
        if let (Some(y0), Some(y1)) = (y.first(), y.last()) {
            parts.push(format!("{}: {y0:.4} … {y1:.4}", info.y_name));
        }
    }
    parts.join("  |  ")
}
