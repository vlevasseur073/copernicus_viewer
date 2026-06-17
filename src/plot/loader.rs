use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use ndarray::ArrayD;
use zarrs::array::Array;
use zarrs::array::ArraySubset;
use zarrs::storage::ReadableListableStorage;

use crate::display::stats::{build_preview, compute_statistics, ArrayPreview, ArrayStatistics};
use crate::plot::flags::{apply_flag_selection, parse_cf_flags, CfFlags, FlagSelection};
use crate::plot::georef::{resolve_georef, GeorefInfo};
use crate::zarr::{ZarrNodeKind, ZarrTreeNode};

#[derive(Clone, Debug)]
pub struct PlotRequest {
    pub array_path: String,
    pub slice_indices: Vec<usize>,
    pub flag_selection: FlagSelection,
}

#[derive(Clone, Debug)]
pub struct PlotLoadResult {
    pub plot: PlotData,
    pub stats: ArrayStatistics,
    pub preview: ArrayPreview,
    pub georef: Option<GeorefInfo>,
    pub flags: Option<CfFlags>,
}

#[derive(Clone, Debug)]
pub enum PlotData {
    Line {
        y: Vec<f64>,
        label: String,
        georef: Option<GeorefInfo>,
    },
    Heatmap {
        width: usize,
        height: usize,
        values: Vec<f32>,
        vmin: f32,
        vmax: f32,
        label: String,
        georef: Option<GeorefInfo>,
        y_range: std::ops::Range<u64>,
        x_range: std::ops::Range<u64>,
        binary: bool,
    },
    Message(String),
}

pub type ProgressCallback = Arc<dyn Fn(f32, &str) + Send + Sync>;

const MAX_PLOT_PIXELS: usize = 512 * 512;

pub fn load_plot_data(
    storage: &ReadableListableStorage,
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

    let ZarrNodeKind::Array { shape, dtype, attributes, .. } = kind else {
        anyhow::bail!("only arrays can be plotted");
    };

    let flags = parse_cf_flags(attributes);

    let array = Array::open(storage.clone(), &request.array_path)
        .with_context(|| format!("failed to open array at {}", request.array_path))?;

    report(0.15, "Building read subset…");
    let (subset_ranges, y_range, x_range) =
        build_subset(shape, &request.slice_indices)?;
    let array_subset = ArraySubset::new_with_ranges(&subset_ranges);

    report(0.35, "Reading array data…");
    let mut values = read_as_f64_array(&array, &array_subset, dtype)?;

    if let (Some(flags), FlagSelection::Flag(index)) = (&flags, request.flag_selection) {
        report(0.50, "Applying flag selection…");
        values = apply_flag_selection(&values, flags, index);
    }

    report(0.65, "Computing statistics…");
    let stats = compute_statistics(&values);
    let preview = build_preview(&values, 8, 8);

    report(0.80, "Resolving geospatial metadata…");
    let georef = resolve_georef(
        storage,
        tree,
        &request.array_path,
        kind,
        &y_range,
        &x_range,
    )
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
) -> Result<(Vec<std::ops::Range<u64>>, std::ops::Range<u64>, std::ops::Range<u64>)> {
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

fn read_as_f64_array(
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

    anyhow::bail!("unsupported data type for plotting: {dtype_hint}")
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
pub fn shared_progress(
    tx: std::sync::mpsc::Sender<(f32, String)>,
) -> ProgressCallback {
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
