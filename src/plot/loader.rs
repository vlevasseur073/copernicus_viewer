use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use ndarray::ArrayD;

use crate::display::stats::{ArrayPreview, ArrayStatistics, build_preview, compute_statistics};
use crate::plot::cf_decode::{apply_cf_decode, parse_cf_encoding};
use crate::plot::flags::{CfFlags, FlagSelection, apply_flag_selection, parse_cf_flags};
use crate::plot::georef::{GeorefInfo, resolve_georef_for_product};
use crate::product::Product;
use crate::zarr::{ZarrNodeKind, ZarrTreeNode};

/// Parameters for loading and plotting a Zarr array subset.
#[derive(Clone, Debug)]
pub struct PlotRequest {
    /// Hierarchy path of the array to plot.
    pub array_path: String,
    /// Fixed indices for dimensions above the trailing 2D slice.
    pub slice_indices: Vec<usize>,
    /// Raw values or a selected CF flag layer.
    pub flag_selection: FlagSelection,
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
}

/// Plot payload consumed by [`crate::plot::PlotPanel`].
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
        /// `true` for binary flag plots (two-colour scale).
        binary: bool,
    },
    /// Informational message when plotting is not possible.
    Message(String),
}

/// Callback invoked during async plot loading with `(fraction, message)`.
pub type ProgressCallback = Arc<dyn Fn(f32, &str) + Send + Sync>;

const MAX_PLOT_PIXELS: usize = 4096 * 4096;

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
    let (subset_ranges, y_range, x_range) = build_subset(shape, &request.slice_indices)?;

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
    );

    report(1.0, "Done");

    Ok(PlotLoadResult {
        plot,
        stats,
        preview,
        georef,
        flags,
    })
}

fn build_subset(
    shape: &[u64],
    slice_indices: &[usize],
) -> Result<(
    Vec<std::ops::Range<u64>>,
    std::ops::Range<u64>,
    std::ops::Range<u64>,
)> {
    if shape.is_empty() {
        return Ok((vec![0..1], 0..1, 0..1));
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

    let (y_range, x_range) = if shape.len() >= 2 {
        let h = capped_range(shape[shape.len() - 2], max_plot_dim());
        let w = capped_range(shape[shape.len() - 1], max_plot_dim());
        subset.push(h.clone());
        subset.push(w.clone());
        (h, w)
    } else if shape.len() == 1 {
        let r = capped_range(shape[0], max_plot_dim());
        subset.push(r.clone());
        (0..1, r)
    } else {
        (0..1, 0..1)
    };

    Ok((subset, y_range, x_range))
}

fn max_plot_dim() -> usize {
    (MAX_PLOT_PIXELS as f64).sqrt() as usize
}

fn capped_range(size: u64, max: usize) -> std::ops::Range<u64> {
    let size = size as usize;
    if size <= max {
        return 0..size as u64;
    }
    let start = (size - max) / 2;
    (start as u64)..((start + max) as u64)
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
        return PlotData::Line {
            y,
            label: format!("{label}{flag_suffix}"),
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
            format!(" @ slices {:?}", slice_indices)
        } else {
            String::new()
        };
        suffix.push_str(&flag_suffix(flags, flag_selection));

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
