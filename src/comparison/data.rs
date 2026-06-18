use anyhow::Result;
use ndarray::ArrayD;
use serde_json::{Map, Value};

use crate::zarr::{ZarrNodeKind, ZarrStore};

use super::array_io::for_each_aligned_chunk;
use super::options::CompareOptions;

#[derive(Clone, Debug)]
pub struct VariableComparison {
    pub path: String,
    pub passed: bool,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub std: f64,
    pub median: f64,
    pub mse: f64,
    pub psnr: f64,
    pub outlier_count: u64,
    pub valid_pixels: u64,
    pub total_pixels: u64,
    pub coverage_diff: i64,
    pub outlier_ratio: f64,
    pub coverage_diff_ratio: f64,
    pub relative_mode: bool,
    pub threshold: f64,
    pub chunks_compared: usize,
}

#[derive(Clone, Debug)]
pub struct SkippedVariable {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct DataReport {
    pub variables: Vec<VariableComparison>,
    pub skipped: Vec<SkippedVariable>,
}

impl DataReport {
    pub fn passed_count(&self) -> usize {
        self.variables.iter().filter(|v| v.passed).count()
    }

    pub fn failed_count(&self) -> usize {
        self.variables.iter().filter(|v| !v.passed).count()
    }

    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }
}

pub fn compare_variable_data(
    left: &ZarrStore,
    right: &ZarrStore,
    paths: &[String],
    options: &CompareOptions,
) -> DataReport {
    let mut report = DataReport::default();

    for path in paths {
        match compare_one_variable(left, right, path, options) {
            Ok(Some(comparison)) => report.variables.push(comparison),
            Ok(None) => {}
            Err(reason) => report.skipped.push(SkippedVariable {
                path: path.clone(),
                reason,
            }),
        }
    }

    report
}

fn compare_one_variable(
    left: &ZarrStore,
    right: &ZarrStore,
    path: &str,
    options: &CompareOptions,
) -> Result<Option<VariableComparison>, String> {
    let Some(left_node) = left.tree.root.find_by_path(path) else {
        return Ok(None);
    };
    let Some(right_node) = right.tree.root.find_by_path(path) else {
        return Err("missing in new product".to_string());
    };

    let ZarrNodeKind::Array {
        shape: ref_shape,
        chunks: ref_chunks,
        dtype: ref_dtype,
        attributes,
        ..
    } = &left_node.kind
    else {
        return Ok(None);
    };

    let ZarrNodeKind::Array {
        shape: new_shape,
        dtype: new_dtype,
        ..
    } = &right_node.kind
    else {
        return Ok(None);
    };

    if ref_shape != new_shape {
        return Err(format!(
            "shape mismatch: reference {ref_shape:?} vs new {new_shape:?}"
        ));
    }

    let relative_mode = is_relative_mode(attributes, options);
    let threshold = variable_threshold(attributes, relative_mode, options);

    let mut accumulator = StatsAccumulator::default();
    let mut chunks_compared = 0usize;

    for_each_aligned_chunk(
        &left.storage,
        &right.storage,
        path,
        ref_shape,
        ref_chunks,
        ref_dtype,
        new_dtype,
        |reference, new| {
            accumulator.ingest(reference, new, relative_mode);
            chunks_compared += 1;
            Ok(())
        },
    )
    .map_err(|err| err.to_string())?;

    let stats = accumulator.finish(
        threshold,
        options.threshold_nb_outliers,
        options.threshold_coverage,
    );

    Ok(Some(VariableComparison {
        path: path.to_string(),
        passed: stats.passed,
        min: stats.min,
        max: stats.max,
        mean: stats.mean,
        std: stats.std,
        median: stats.median,
        mse: stats.mse,
        psnr: stats.psnr,
        outlier_count: stats.outlier_count,
        valid_pixels: stats.valid_pixels,
        total_pixels: stats.total,
        coverage_diff: stats.coverage_diff,
        outlier_ratio: stats.outlier_ratio,
        coverage_diff_ratio: stats.coverage_diff_ratio,
        relative_mode,
        threshold,
        chunks_compared,
    }))
}

#[derive(Default)]
struct StatsAccumulator {
    err_values: Vec<f64>,
    ref_valid: u64,
    new_valid: u64,
    total: u64,
    mse_sum: f64,
    mse_count: u64,
    new_min: f64,
    new_max: f64,
}

impl StatsAccumulator {
    fn ingest(&mut self, reference: &ArrayD<f64>, new: &ArrayD<f64>, relative_mode: bool) {
        self.total += reference.len() as u64;
        for (&r, &n) in reference.iter().zip(new.iter()) {
            if r.is_finite() {
                self.ref_valid += 1;
            }
            if n.is_finite() {
                self.new_valid += 1;
                if self.mse_count == 0 {
                    self.new_min = n;
                    self.new_max = n;
                } else {
                    self.new_min = self.new_min.min(n);
                    self.new_max = self.new_max.max(n);
                }
            }
            if !r.is_finite() || !n.is_finite() {
                continue;
            }
            let diff = n - r;
            self.mse_sum += diff * diff;
            self.mse_count += 1;
            let err = if relative_mode {
                if r.abs() > f64::EPSILON {
                    diff / r
                } else {
                    diff
                }
            } else {
                diff
            };
            self.err_values.push(err);
        }
    }

    fn finish(
        self,
        threshold: f64,
        threshold_nb_outliers: f64,
        threshold_coverage: f64,
    ) -> StatsOutcome {
        compute_stats_from_errors(
            &self.err_values,
            self.ref_valid,
            self.new_valid,
            self.total,
            self.mse_sum,
            self.mse_count,
            self.new_min,
            self.new_max,
            threshold,
            threshold_nb_outliers,
            threshold_coverage,
        )
    }
}

struct StatsOutcome {
    passed: bool,
    min: f64,
    max: f64,
    mean: f64,
    std: f64,
    median: f64,
    mse: f64,
    psnr: f64,
    outlier_count: u64,
    valid_pixels: u64,
    total: u64,
    coverage_diff: i64,
    outlier_ratio: f64,
    coverage_diff_ratio: f64,
}

fn compute_psnr(mse: f64, sq_max_value: f64) -> f64 {
    if mse <= 0.0 || !mse.is_finite() {
        return f64::INFINITY;
    }
    20.0 * (sq_max_value / mse).log10()
}

fn compute_stats_from_errors(
    err_values: &[f64],
    ref_valid: u64,
    new_valid: u64,
    total: u64,
    mse_sum: f64,
    mse_count: u64,
    new_min: f64,
    new_max: f64,
    threshold: f64,
    threshold_nb_outliers: f64,
    threshold_coverage: f64,
) -> StatsOutcome {
    let coverage_diff = new_valid as i64 - ref_valid as i64;
    let coverage_diff_ratio = if total == 0 {
        0.0
    } else {
        coverage_diff as f64 / total as f64
    };
    let mse = if mse_count == 0 {
        f64::NAN
    } else {
        mse_sum / mse_count as f64
    };
    let sq_max_value = if mse_count == 0 {
        0.0
    } else {
        (new_max - new_min).powi(2)
    };
    let psnr = compute_psnr(mse, sq_max_value);

    if err_values.is_empty() {
        return StatsOutcome {
            passed: coverage_diff_ratio.abs() <= threshold_coverage,
            min: f64::NAN,
            max: f64::NAN,
            mean: f64::NAN,
            std: f64::NAN,
            median: f64::NAN,
            mse,
            psnr,
            outlier_count: 0,
            valid_pixels: 0,
            total,
            coverage_diff,
            outlier_ratio: 0.0,
            coverage_diff_ratio,
        };
    }

    let mut sorted = err_values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    let mean = err_values.iter().sum::<f64>() / err_values.len() as f64;
    let variance = err_values
        .iter()
        .map(|v| {
            let d = v - mean;
            d * d
        })
        .sum::<f64>()
        / err_values.len() as f64;
    let std = variance.sqrt();
    let median = sorted[sorted.len() / 2];
    let outlier_count = err_values.iter().filter(|&&v| v.abs() > threshold).count() as u64;
    let valid_pixels = err_values.len() as u64;
    let outlier_ratio = if valid_pixels == 0 {
        0.0
    } else {
        outlier_count as f64 / valid_pixels as f64
    };

    let passed =
        outlier_ratio <= threshold_nb_outliers && coverage_diff_ratio.abs() <= threshold_coverage;

    StatsOutcome {
        passed,
        min,
        max,
        mean,
        std,
        median,
        mse,
        psnr,
        outlier_count,
        valid_pixels,
        total,
        coverage_diff,
        outlier_ratio,
        coverage_diff_ratio,
    }
}

fn is_relative_mode(attributes: &Map<String, Value>, options: &CompareOptions) -> bool {
    if options.relative {
        return true;
    }
    if options.absolute {
        return false;
    }
    !attributes.contains_key("scale_factor")
}

fn variable_threshold(
    attributes: &Map<String, Value>,
    relative_mode: bool,
    options: &CompareOptions,
) -> f64 {
    if relative_mode {
        return options.threshold;
    }
    if let Some(scale) = attributes.get("scale_factor").and_then(json_number) {
        return scale * options.threshold_packed;
    }
    options.threshold
}

fn json_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn global_relative_score(report: &DataReport) -> Option<f64> {
    if report.variables.is_empty() {
        return None;
    }
    let mut scores: Vec<f64> = report
        .variables
        .iter()
        .filter(|v| v.relative_mode && v.mean.is_finite() && v.median.is_finite())
        .map(|v| ((v.mean + v.median) * 0.5).abs())
        .collect();
    if scores.is_empty() {
        return None;
    }
    scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = scores[scores.len() / 2];
    Some(100.0 - (median * 100.0).abs())
}

/// Format variable statistics like sentineltoolbox `_get_failed_formatted_string_vars`.
pub fn format_variable_detail(
    var: &VariableComparison,
    threshold_nb_outliers: f64,
    threshold_coverage: f64,
) -> String {
    let outlier_pct = if var.valid_pixels == 0 {
        0.0
    } else {
        var.outlier_count as f64 / var.valid_pixels as f64 * 100.0
    };
    let coverage_pct = if var.total_pixels == 0 {
        0.0
    } else {
        var.coverage_diff as f64 / var.total_pixels as f64 * 100.0
    };

    let base = if var.relative_mode {
        format!(
            "{}: min={:8.4}% max={:8.4}% mean={:8.4}% stdev={:8.4}% median={:8.4}% \
             mse={:9.6} psnr={:9.6}dB -- eps={:.6}%",
            var.path,
            var.min * 100.0,
            var.max * 100.0,
            var.mean * 100.0,
            var.std * 100.0,
            var.median * 100.0,
            var.mse,
            var.psnr,
            var.threshold * 100.0,
        )
    } else {
        format!(
            "{}: min={:9.6} max={:9.6} mean={:9.6} stdev={:9.6} median={:9.6} \
             mse={:9.6} psnr={:9.6}dB -- eps={:.6}",
            var.path,
            var.min,
            var.max,
            var.mean,
            var.std,
            var.median,
            var.mse,
            var.psnr,
            var.threshold,
        )
    };

    format!(
        "{base} outliers={} (={outlier_pct:.3}% allowed={:.3}%) \
         coverage={coverage_pct:+.3}% (allowed={:.3}%)",
        var.outlier_count,
        threshold_nb_outliers * 100.0,
        threshold_coverage * 100.0,
    )
}
