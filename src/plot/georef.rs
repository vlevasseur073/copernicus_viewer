use std::ops::Range;

use anyhow::{Context, Result};
use ndarray::ArrayD;
use serde_json::Map;
use zarrs::array::Array;
use zarrs::array::ArraySubset;
use zarrs::storage::ReadableListableStorage;

use crate::product::Product;
#[cfg(feature = "safe")]
use crate::safe::SafeStore;
use crate::zarr::{ZarrNodeKind, ZarrTreeNode};

/// Coordinate metadata and axis values for geo-referenced plots.
#[derive(Clone, Debug, Default)]
pub struct GeorefInfo {
    /// CRS identifier from array or coordinate attributes.
    pub crs: Option<String>,
    /// Name of the X / longitude dimension.
    pub x_name: String,
    /// Name of the Y / latitude dimension.
    pub y_name: String,
    /// Coordinate values along X for the plotted subset.
    pub x_coords: Option<Vec<f64>>,
    /// Coordinate values along Y for the plotted subset.
    pub y_coords: Option<Vec<f64>>,
    /// Units for the X coordinate variable.
    pub x_unit: Option<String>,
    /// Units for the Y coordinate variable.
    pub y_unit: Option<String>,
}

const X_ALIASES: &[&str] = &["x", "lon", "longitude", "easting"];
const Y_ALIASES: &[&str] = &["y", "lat", "latitude", "northing"];

/// Resolve coordinate arrays and CRS for a plotted array subset.
pub fn resolve_georef(
    storage: &ReadableListableStorage,
    tree: &ZarrTreeNode,
    array_path: &str,
    kind: &ZarrNodeKind,
    y_range: &Range<u64>,
    x_range: &Range<u64>,
) -> Result<GeorefInfo> {
    resolve_georef_inner(Some(storage), tree, array_path, kind, y_range, x_range)
}

/// Resolve georeferencing for any product backend (Zarr or SAFE).
pub fn resolve_georef_for_product(
    product: &Product,
    tree: &ZarrTreeNode,
    array_path: &str,
    kind: &ZarrNodeKind,
    y_range: &Range<u64>,
    x_range: &Range<u64>,
) -> Result<GeorefInfo> {
    #[cfg(feature = "safe")]
    {
        let mut info = resolve_georef_inner(
            product.zarr_storage(),
            tree,
            array_path,
            kind,
            y_range,
            x_range,
        )?;
        if let Some(safe) = product.as_safe() {
            supplement_safe_georef(safe, tree, array_path, y_range, x_range, &mut info)?;
        }
        Ok(info)
    }
    #[cfg(not(feature = "safe"))]
    {
        resolve_georef_inner(
            product.zarr_storage(),
            tree,
            array_path,
            kind,
            y_range,
            x_range,
        )
    }
}

fn resolve_georef_inner(
    storage: Option<&ReadableListableStorage>,
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

    let y_unit = y_coords.as_ref().and_then(|(_, attrs)| {
        attrs
            .get("units")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });
    let x_unit = x_coords.as_ref().and_then(|(_, attrs)| {
        attrs
            .get("units")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });

    Ok(GeorefInfo {
        crs: crs.or_else(|| Some("EPSG:4326".to_string())),
        x_name: if x_coords.is_some() {
            "longitude".to_string()
        } else {
            x_name.clone()
        },
        y_name: if y_coords.is_some() {
            "latitude".to_string()
        } else {
            y_name.clone()
        },
        x_coords: x_coords.map(|(v, _)| v),
        y_coords: y_coords.map(|(v, _)| v),
        x_unit,
        y_unit,
    })
}

#[cfg(feature = "safe")]
fn supplement_safe_georef(
    safe: &SafeStore,
    tree: &ZarrTreeNode,
    array_path: &str,
    y_range: &Range<u64>,
    x_range: &Range<u64>,
    info: &mut GeorefInfo,
) -> Result<()> {
    if info.y_coords.is_some() && info.x_coords.is_some() {
        return Ok(());
    }
    let Some((lat_path, lon_path)) = safe.georef_coord_paths(array_path) else {
        return Ok(());
    };
    if info.y_coords.is_none()
        && let Some((values, attrs)) =
            read_safe_coord(safe, tree, &lat_path, y_range, CoordSlice::Rows)?
    {
        info.y_coords = Some(values);
        info.y_name = "latitude".to_string();
        info.y_unit = attrs
            .get("units")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    if info.x_coords.is_none()
        && let Some((values, attrs)) =
            read_safe_coord(safe, tree, &lon_path, x_range, CoordSlice::Columns)?
    {
        info.x_coords = Some(values);
        info.x_name = "longitude".to_string();
        info.x_unit = attrs
            .get("units")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    Ok(())
}

#[cfg(feature = "safe")]
fn read_safe_coord(
    safe: &SafeStore,
    tree: &ZarrTreeNode,
    path: &str,
    range: &Range<u64>,
    slice: CoordSlice,
) -> Result<Option<(Vec<f64>, Map<String, serde_json::Value>)>> {
    let Some(node) = tree.find_by_path(path) else {
        return Ok(None);
    };
    let ZarrNodeKind::Array {
        shape, attributes, ..
    } = &node.kind
    else {
        return Ok(None);
    };

    if shape.len() == 1 {
        let start = range.start.min(shape[0]);
        let end = range.end.min(shape[0]);
        if start >= end {
            return Ok(None);
        }
        let values = safe
            .read_array_subset_f64(path, &[start..end])?
            .iter()
            .copied()
            .collect();
        return Ok(Some((values, attributes.clone())));
    }

    if shape.len() != 2 {
        return Ok(None);
    }

    match slice {
        CoordSlice::Rows => {
            let row_start = range.start.min(shape[0]);
            let row_end = range.end.min(shape[0]);
            if row_start >= row_end {
                return Ok(None);
            }
            let col_mid = shape[1] / 2;
            let mut values = Vec::with_capacity((row_end - row_start) as usize);
            for row in row_start..row_end {
                let point =
                    safe.read_array_subset_f64(path, &[row..row + 1, col_mid..col_mid + 1])?;
                values.push(point[[0, 0]]);
            }
            Ok(Some((values, attributes.clone())))
        }
        CoordSlice::Columns => {
            let col_start = range.start.min(shape[1]);
            let col_end = range.end.min(shape[1]);
            if col_start >= col_end {
                return Ok(None);
            }
            let row_mid = shape[0] / 2;
            let mut values = Vec::with_capacity((col_end - col_start) as usize);
            for col in col_start..col_end {
                let point =
                    safe.read_array_subset_f64(path, &[row_mid..row_mid + 1, col..col + 1])?;
                values.push(point[[0, 0]]);
            }
            Ok(Some((values, attributes.clone())))
        }
    }
}

#[cfg(feature = "safe")]
#[derive(Clone, Copy)]
enum CoordSlice {
    Rows,
    Columns,
}

fn resolve_coord(
    storage: Option<&ReadableListableStorage>,
    tree: &ZarrTreeNode,
    parent: &str,
    name: &str,
    aliases: &[&str],
    range: &Range<u64>,
) -> Result<Option<(Vec<f64>, Map<String, serde_json::Value>)>> {
    let Some(storage) = storage else {
        return Ok(None);
    };
    if let Some(found) = read_coord_array(storage, tree, parent, name, range)? {
        return Ok(Some(found));
    }
    try_alias_coord(storage, tree, parent, aliases, range)
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

    let ZarrNodeKind::Array {
        dtype,
        attributes,
        shape,
        ..
    } = &node.kind
    else {
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

/// Format an axis tick label from coordinate values or a fallback index.
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

/// One-line summary of CRS and coordinate extents for plot captions.
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
