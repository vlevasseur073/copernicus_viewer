use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use ndarray::{ArrayD, Axis};

use crate::display::stats::{ArrayPreview, ArrayStatistics, build_preview, compute_statistics};
use crate::plot::cf_decode::{apply_cf_decode, parse_cf_encoding};
use crate::plot::flags::{CfFlags, FlagSelection, apply_flag_selection, parse_cf_flags};
use crate::plot::georef::{GeorefInfo, resolve_georef_for_product};
use crate::product::Product;
use crate::zarr::{ZarrNodeKind, ZarrTreeNode};

/// Parameters for loading and plotting a Zarr array subset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlotRequest {
    /// Hierarchy path of the array to plot.
    pub array_path: String,
    /// Fixed indices for dimensions above the trailing 2D slice.
    pub slice_indices: Vec<usize>,
    /// Raw values or a selected CF flag layer.
    pub flag_selection: FlagSelection,
    /// Spatial window size as a percentage of each plotted dimension (1–100).
    pub resolution_percent: u8,
    /// Keep one pixel every N along plotted dimensions (≥ 1).
    pub sample_stride: u32,
}

impl PlotRequest {
    /// Default request for a 2D array at full resolution.
    pub fn new(array_path: impl Into<String>) -> Self {
        Self {
            array_path: array_path.into(),
            slice_indices: Vec::new(),
            flag_selection: FlagSelection::Raw,
            resolution_percent: 100,
            sample_stride: 1,
        }
    }
}

/// Result of loading array data for plotting and inspection.
#[derive(Clone, Debug)]
pub struct PlotLoadResult {
    /// Renderable plot payload (line, heatmap, or message).
    pub plot: PlotData,
    /// Numeric statistics over the loaded subset.
    pub stats: ArrayStatistics,
    /// Tabular preview of the loaded subset.
    pub preview: ArrayPreview,
    /// Coordinate metadata when geo-referencing is available.
    pub georef: Option<GeorefInfo>,
    /// Parsed CF flags when the array defines them.
    pub flags: Option<CfFlags>,
    /// Set when the read subset was clamped by the memory safety ceiling.
    pub resolution_capped: bool,
}

/// Plot payload consumed by the plot workspace.
#[derive(Clone, Debug)]
pub enum PlotData {
    /// 1D line series.
    Line {
        /// Y values.
        y: Vec<f64>,
        /// Plot title / variable label.
        label: String,
        /// Optional coordinate metadata.
        georef: Option<GeorefInfo>,
    },
    /// 2D heatmap raster (downsampled when large).
    Heatmap {
        /// Raster width in pixels.
        width: usize,
        /// Raster height in pixels.
        height: usize,
        /// Normalized pixel values for colouring.
        values: Vec<f32>,
        /// Colour scale minimum.
        vmin: f32,
        /// Colour scale maximum.
        vmax: f32,
        /// Plot title / variable label.
        label: String,
        /// Optional coordinate metadata for axis labels.
        georef: Option<GeorefInfo>,
        /// Source Y dimension range in the original array.
        y_range: std::ops::Range<u64>,
        /// Source X dimension range in the original array.
        x_range: std::ops::Range<u64>,
        /// Full array shape (for subset annotations).
        full_shape: Vec<u64>,
        /// Resolution percentage used for the spatial window.
        resolution_percent: u8,
        /// Stride used when decimating the loaded window.
        sample_stride: u32,
        /// `true` for binary flag plots (two-colour scale).
        binary: bool,
    },
    /// Informational message when plotting is not possible.
    Message(String),
}

/// Callback invoked during async plot loading with `(fraction, message)`.
pub type ProgressCallback = Arc<dyn Fn(f32, &str) + Send + Sync>;

pub(crate) const MAX_PLOT_PIXELS: usize = 4096 * 4096;

/// Load array data for plotting and inspector statistics.
///
/// Reads a 1D slice or 2D subset (with optional downsampling), applies CF flag
/// selection, and resolves geo-referencing from coordinate variables.
pub fn load_plot_data(
    product: &Product,
    tree: &ZarrTreeNode,
    kind: &ZarrNodeKind,
    request: &PlotRequest,
    progress: Option<ProgressCallback>,
) -> Result<PlotLoadResult> {
    let report = |fraction: f32, msg: &str| {
        if let Some(cb) = &progress {
            cb(fraction, msg);
        }
    };

    report(0.05, "Opening array…");

    let ZarrNodeKind::Array {
        shape,
        attributes,
        fill_value,
        ..
    } = kind
    else {
        anyhow::bail!("only arrays can be plotted");
    };

    let flags = parse_cf_flags(attributes, fill_value.as_ref());

    report(0.15, "Building read subset…");
    let (subset_ranges, y_range, x_range, resolution_capped) =
        build_subset(shape, &request.slice_indices, request.resolution_percent)?;

    report(0.35, "Reading array data…");
    let mut values = product
        .read_array_subset_f64(&request.array_path, &subset_ranges)
        .with_context(|| format!("failed to read array at {}", request.array_path))?;

    let encoding = parse_cf_encoding(attributes, fill_value.as_ref());

    if let (Some(flags), FlagSelection::Flag(index)) = (&flags, request.flag_selection) {
        report(0.50, "Applying flag selection…");
        values = apply_flag_selection(&values, flags, index);
    } else {
        report(0.45, "Applying CF decoding…");
        values = apply_cf_decode(&values, &encoding);
    }

    report(0.55, "Decimating…");
    values = squeeze_sliced_dims(values, shape.len().saturating_sub(2));
    values = decimate_array(values, request.sample_stride);

    report(0.65, "Computing statistics…");
    let stats = compute_statistics(&values);
    let preview = build_preview(&values, 8, 8);

    report(0.80, "Resolving geospatial metadata…");
    let georef =
        resolve_georef_for_product(product, tree, &request.array_path, kind, &y_range, &x_range)
            .ok();

    report(0.92, "Preparing plot…");
    let plot = plot_from_values(
        &values,
        shape,
        &request.slice_indices,
        &request.array_path,
        georef.clone(),
        y_range,
        x_range,
        flags.as_ref(),
        request.flag_selection,
        request.resolution_percent,
        request.sample_stride,
    );

    report(1.0, "Done");

    Ok(PlotLoadResult {
        plot,
        stats,
        preview,
        georef,
        flags,
        resolution_capped,
    })
}

/// Build per-dimension read ranges for a plot request.
pub(crate) fn build_subset(
    shape: &[u64],
    slice_indices: &[usize],
    resolution_percent: u8,
) -> Result<(
    Vec<std::ops::Range<u64>>,
    std::ops::Range<u64>,
    std::ops::Range<u64>,
    bool,
)> {
    if shape.is_empty() {
        return Ok((vec![0..1], 0..1, 0..1, false));
    }

    let extra_dims = shape.len().saturating_sub(2);
    if slice_indices.len() != extra_dims {
        anyhow::bail!(
            "expected {} slice indices for shape {:?}, got {}",
            extra_dims,
            shape,
            slice_indices.len()
        );
    }

    let mut subset = Vec::with_capacity(shape.len());
    for (i, &idx) in slice_indices.iter().enumerate() {
        let dim_size = shape[i] as usize;
        if idx >= dim_size {
            anyhow::bail!("slice index {idx} out of range for dimension size {dim_size}");
        }
        subset.push(idx as u64..(idx as u64 + 1));
    }

    let pct = resolution_percent.clamp(1, 100);
    let mut resolution_capped = false;

    let (y_range, x_range) = if shape.len() >= 2 {
        let mut h = spatial_target_dim(shape[shape.len() - 2], pct);
        let mut w = spatial_target_dim(shape[shape.len() - 1], pct);
        let (h2, w2, capped_both) = apply_pixel_budget(h, w);
        resolution_capped |= capped_both;
        h = h2;
        w = w2;
        let y = center_crop_range(shape[shape.len() - 2], h);
        let x = center_crop_range(shape[shape.len() - 1], w);
        subset.push(y.clone());
        subset.push(x.clone());
        (y, x)
    } else if shape.len() == 1 {
        let requested = spatial_target_dim(shape[0], pct);
        let len = requested.clamp(1, MAX_PLOT_PIXELS);
        resolution_capped |= len < requested;
        let r = center_crop_range(shape[0], len);
        subset.push(r.clone());
        (0..1, r)
    } else {
        (0..1, 0..1)
    };

    Ok((subset, y_range, x_range, resolution_capped))
}

fn spatial_target_dim(full: u64, resolution_percent: u8) -> usize {
    let full = full as usize;
    let pct = resolution_percent.clamp(1, 100) as usize;
    (full * pct).div_ceil(100).max(1)
}

fn apply_pixel_budget(height: usize, width: usize) -> (usize, usize, bool) {
    let pixels = height.saturating_mul(width);
    if pixels <= MAX_PLOT_PIXELS {
        return (height, width, false);
    }
    let scale = (MAX_PLOT_PIXELS as f64 / pixels as f64).sqrt();
    let h = (height as f64 * scale).floor().max(1.0) as usize;
    let w = (width as f64 * scale).floor().max(1.0) as usize;
    (h, w, true)
}

fn center_crop_range(size: u64, target: usize) -> std::ops::Range<u64> {
    let size = size as usize;
    if size <= target {
        return 0..size as u64;
    }
    let start = (size - target) / 2;
    (start as u64)..((start + target) as u64)
}

/// Remove leading singleton dimensions produced by fixed-index slices on extra axes.
fn squeeze_sliced_dims(values: ArrayD<f64>, num_extra_dims: usize) -> ArrayD<f64> {
    let mut out = values;
    let mut remaining = num_extra_dims;
    while remaining > 0 && out.ndim() > 2 && out.shape()[0] == 1 {
        out = out.index_axis_move(Axis(0), 0);
        remaining -= 1;
    }
    out
}

pub(crate) fn decimate_array(values: ArrayD<f64>, stride: u32) -> ArrayD<f64> {
    let stride = stride.max(1) as isize;
    if stride == 1 {
        return values;
    }

    match values.ndim() {
        1 => values.slice(ndarray::s![..;stride]).to_owned().into_dyn(),
        2 => values
            .slice(ndarray::s![..;stride, ..;stride])
            .to_owned()
            .into_dyn(),
        _ => values,
    }
}

fn plot_from_values(
    values: &ArrayD<f64>,
    full_shape: &[u64],
    slice_indices: &[usize],
    path: &str,
    georef: Option<GeorefInfo>,
    y_range: std::ops::Range<u64>,
    x_range: std::ops::Range<u64>,
    flags: Option<&CfFlags>,
    flag_selection: FlagSelection,
    resolution_percent: u8,
    sample_stride: u32,
) -> PlotData {
    let label = path.trim_start_matches('/').to_string();
    let shape = values.shape();
    let plotting_flag = matches!(flag_selection, FlagSelection::Flag(_));

    if shape.is_empty() || (shape.len() == 1 && shape[0] == 1) {
        return PlotData::Message("scalar variable — nothing to plot".to_string());
    }

    if shape.len() == 1 {
        let y: Vec<f64> = values.iter().copied().collect();
        let flag_suffix = flag_suffix(flags, flag_selection);
        let stride_suffix = if sample_stride > 1 {
            format!(" @ stride {sample_stride}")
        } else {
            String::new()
        };
        return PlotData::Line {
            y,
            label: format!("{label}{flag_suffix}{stride_suffix}"),
            georef,
        };
    }

    if shape.len() == 2 {
        let height = shape[0];
        let width = shape[1];
        let mut flat = Vec::with_capacity(height * width);
        let mut vmin = f64::INFINITY;
        let mut vmax = f64::NEG_INFINITY;

        for row in 0..height {
            for col in 0..width {
                let v = values[[row, col]];
                if v.is_finite() {
                    vmin = vmin.min(v);
                    vmax = vmax.max(v);
                }
                flat.push(v as f32);
            }
        }

        if !vmin.is_finite() || !vmax.is_finite() {
            return PlotData::Message("array contains no finite values".to_string());
        }

        let binary = plotting_flag;
        if binary {
            vmin = 0.0;
            vmax = 1.0;
        } else if (vmax - vmin).abs() < f32::EPSILON as f64 {
            vmax = vmin + 1.0;
        }

        let mut suffix = if full_shape.len() > 2 {
            format!(" @ slices {slice_indices:?}")
        } else {
            String::new()
        };
        suffix.push_str(&flag_suffix(flags, flag_selection));
        if sample_stride > 1 {
            suffix.push_str(&format!(" @ stride {sample_stride}"));
        }

        return PlotData::Heatmap {
            width,
            height,
            values: flat,
            vmin: vmin as f32,
            vmax: vmax as f32,
            label: format!("{label}{suffix}"),
            georef,
            y_range,
            x_range,
            full_shape: full_shape.to_vec(),
            resolution_percent,
            sample_stride,
            binary,
        };
    }

    PlotData::Message(format!("unexpected plot shape: {:?}", shape))
}

fn flag_suffix(flags: Option<&CfFlags>, flag_selection: FlagSelection) -> String {
    match (flags, flag_selection) {
        (Some(flags), FlagSelection::Flag(index)) => {
            format!(" — {}", flags.flag_label(index))
        }
        _ => String::new(),
    }
}

/// Helper to share progress updates from a background thread.
pub fn shared_progress(tx: std::sync::mpsc::Sender<(f32, String)>) -> ProgressCallback {
    Arc::new(move |fraction, message| {
        let _ = tx.send((fraction, message.to_string()));
    })
}

/// Wrap a mutex-backed progress slot for testing without channels.
pub fn mutex_progress(slot: Arc<Mutex<(f32, String)>>) -> ProgressCallback {
    Arc::new(move |fraction, message| {
        if let Ok(mut s) = slot.lock() {
            *s = (fraction, message.to_string());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_subset_full_resolution_2d() {
        let shape = [100u64, 200];
        let (ranges, y, x, capped) = build_subset(&shape, &[], 100).expect("subset");
        assert!(!capped);
        assert_eq!(y, 0..100);
        assert_eq!(x, 0..200);
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn build_subset_half_resolution_center_crop() {
        let shape = [100u64, 200];
        let (_, y, x, _) = build_subset(&shape, &[], 50).expect("subset");
        assert_eq!(y, 25..75);
        assert_eq!(x, 50..150);
    }

    #[test]
    fn build_subset_hits_pixel_budget() {
        let shape = [10_000u64, 10_000];
        let (_, y, x, capped) = build_subset(&shape, &[], 100).expect("subset");
        assert!(capped);
        let h = (y.end - y.start) as usize;
        let w = (x.end - x.start) as usize;
        assert!(h * w <= MAX_PLOT_PIXELS);
    }

    #[test]
    fn squeeze_sliced_dims_3d() {
        use ndarray::IxDyn;
        let values = ArrayD::from_shape_vec(IxDyn(&[1, 4, 8]), vec![0.0; 32]).unwrap();
        let out = squeeze_sliced_dims(values, 1);
        assert_eq!(out.shape(), &[4, 8]);
    }

    #[test]
    fn squeeze_sliced_dims_4d() {
        use ndarray::IxDyn;
        let values = ArrayD::from_shape_vec(IxDyn(&[1, 1, 4, 8]), vec![0.0; 32]).unwrap();
        let out = squeeze_sliced_dims(values, 2);
        assert_eq!(out.shape(), &[4, 8]);
    }

    #[test]
    fn squeeze_sliced_dims_preserves_1x1_spatial_window() {
        use ndarray::IxDyn;
        let values = ArrayD::from_shape_vec(IxDyn(&[1, 1, 1, 1]), vec![42.0]).unwrap();
        let out = squeeze_sliced_dims(values, 2);
        assert_eq!(out.shape(), &[1, 1]);
        assert_eq!(out[[0, 0]], 42.0);
    }

    #[test]
    fn plot_from_values_accepts_squeezed_3d_slice() {
        use ndarray::IxDyn;
        let values =
            ArrayD::from_shape_vec(IxDyn(&[1, 2, 3]), (0..6).map(|v| v as f64).collect()).unwrap();
        let values = squeeze_sliced_dims(values, 1);
        let plot = plot_from_values(
            &values,
            &[1, 2, 3],
            &[0],
            "/measurements/specific_humidity",
            None,
            0..2,
            0..3,
            None,
            FlagSelection::Raw,
            100,
            1,
        );
        match plot {
            PlotData::Heatmap { width, height, .. } => {
                assert_eq!(width, 3);
                assert_eq!(height, 2);
            }
            other => panic!("expected heatmap, got {other:?}"),
        }
    }

    #[test]
    fn decimate_stride_two_halves_2d() {
        use ndarray::IxDyn;
        let values =
            ArrayD::from_shape_vec(IxDyn(&[4, 4]), (0..16).map(|v| v as f64).collect()).unwrap();
        let out = decimate_array(values, 2);
        assert_eq!(out.shape(), &[2, 2]);
        assert_eq!(out[[0, 0]], 0.0);
        assert_eq!(out[[0, 1]], 2.0);
        assert_eq!(out[[1, 0]], 8.0);
        assert_eq!(out[[1, 1]], 10.0);
    }

    #[test]
    fn decimate_stride_one_is_identity() {
        use ndarray::IxDyn;
        let values = ArrayD::from_shape_vec(IxDyn(&[5]), vec![1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        let out = decimate_array(values.clone(), 1);
        assert_eq!(out, values);
    }

    #[test]
    #[cfg(feature = "safe")]
    fn load_safe_4d_meteorology_array_as_2d_heatmap() {
        use std::path::Path;

        use crate::product::Product;
        use crate::safe::SafeStore;

        let paths = [
            "/home/vincent/Codes/Acri/sentineltoolbox/sentineltoolbox/testing/data/tiny_products/S3A_SL_2_LST____20191227T124111_20191227T124411_20221209T133218_0179_053_109______PS1_D_NR_004.SEN3",
            "/home/vincent/Data/SLSTR/S3A_SL_2_LST____20260622T102053_20260622T102353_20260622T123949_0179_141_008_2160_PS1_O_NR_005.SEN3",
        ];
        let Some(path) = paths.iter().map(Path::new).find(|p| p.is_dir()) else {
            return;
        };

        let store = SafeStore::open(path).expect("open SAFE");
        let product = Product::Safe(store.clone());
        let array_path = "/conditions/meteorology/specific_humidity_tp";
        let node = store
            .tree
            .root
            .find_by_path(array_path)
            .expect("humidity path");
        let kind = node.kind.clone();
        let ZarrNodeKind::Array { shape, .. } = &kind else {
            panic!("expected array node");
        };
        let extra_dims = shape.len().saturating_sub(2);
        let request = PlotRequest {
            array_path: array_path.to_string(),
            slice_indices: vec![0; extra_dims],
            flag_selection: FlagSelection::Raw,
            resolution_percent: 10,
            sample_stride: 1,
        };

        let result = load_plot_data(&product, &store.tree.root, &kind, &request, None)
            .expect("load plot data");

        match result.plot {
            PlotData::Heatmap { height, width, .. } => {
                assert!(height > 0);
                assert!(width > 0);
            }
            PlotData::Message(msg) => panic!("expected heatmap, got message: {msg}"),
            other => panic!("expected heatmap, got {other:?}"),
        }
    }
}
