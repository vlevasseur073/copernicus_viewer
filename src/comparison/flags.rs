use anyhow::Result;
use ndarray::ArrayD;

use crate::plot::flags::{parse_cf_flags, CfFlagMode, CfFlags};
use crate::zarr::{ZarrNodeKind, ZarrStore};

use super::array_io::for_each_aligned_chunk;
use super::options::CompareOptions;

#[derive(Clone, Debug)]
pub struct FlagBitComparison {
    pub meaning: String,
    pub equal_percentage: f64,
    pub different_percentage: f64,
    pub passed: bool,
}

#[derive(Clone, Debug)]
pub struct FlagVariableComparison {
    pub path: String,
    pub bits: Vec<FlagBitComparison>,
    pub score: f64,
    pub chunks_compared: usize,
}

#[derive(Clone, Debug, Default)]
pub struct FlagReport {
    pub variables: Vec<FlagVariableComparison>,
}

impl FlagReport {
    pub fn passed_count(&self) -> usize {
        self.variables
            .iter()
            .flat_map(|v| v.bits.iter())
            .filter(|b| b.passed)
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.variables
            .iter()
            .flat_map(|v| v.bits.iter())
            .filter(|b| !b.passed)
            .count()
    }
}

pub fn compare_flag_variables(
    left: &ZarrStore,
    right: &ZarrStore,
    paths: &[String],
    options: &CompareOptions,
) -> FlagReport {
    let mut report = FlagReport::default();
    let eps = 100.0 * options.threshold_nb_outliers;

    for path in paths {
        let Ok(comparison) = compare_one_flag_variable(left, right, path, eps) else {
            continue;
        };
        if let Some(comparison) = comparison {
            report.variables.push(comparison);
        }
    }

    report
}

fn compare_one_flag_variable(
    left: &ZarrStore,
    right: &ZarrStore,
    path: &str,
    eps: f64,
) -> Result<Option<FlagVariableComparison>> {
    let Some(left_node) = left.tree.root.find_by_path(path) else {
        return Ok(None);
    };
    let Some(right_node) = right.tree.root.find_by_path(path) else {
        return Ok(None);
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
        return Ok(None);
    }

    let Some(flags) = parse_cf_flags(attributes) else {
        return Ok(None);
    };

    let mut accumulator = BitwiseAccumulator::new(flags.codes.len());
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
            accumulator.ingest(reference, new, &flags);
            chunks_compared += 1;
            Ok(())
        },
    )?;

    let bits = accumulator.finish(&flags, eps);
    let score = flag_variable_score(&bits);
    Ok(Some(FlagVariableComparison {
        path: path.to_string(),
        bits,
        score,
        chunks_compared,
    }))
}

struct BitwiseAccumulator {
    total: u64,
    equal: Vec<u64>,
}

impl BitwiseAccumulator {
    fn new(flag_count: usize) -> Self {
        Self {
            total: 0,
            equal: vec![0; flag_count],
        }
    }

    fn ingest(&mut self, reference: &ArrayD<f64>, new: &ArrayD<f64>, flags: &CfFlags) {
        self.total += reference.len() as u64;
        for (&r, &n) in reference.iter().zip(new.iter()) {
            if !r.is_finite() || !n.is_finite() {
                continue;
            }
            for (idx, &mask) in flags.codes.iter().enumerate() {
                let rb = bit_value(r, mask, &flags.mode);
                let nb = bit_value(n, mask, &flags.mode);
                if rb == nb {
                    self.equal[idx] += 1;
                }
            }
        }
    }

    fn finish(self, flags: &CfFlags, eps: f64) -> Vec<FlagBitComparison> {
        if self.total == 0 {
            return Vec::new();
        }

        flags
            .codes
            .iter()
            .enumerate()
            .map(|(idx, _)| {
                let equal = self.equal[idx];
                let equal_percentage = equal as f64 / self.total as f64 * 100.0;
                let different_percentage = 100.0 - equal_percentage;
                let meaning = flags
                    .meanings
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| "?".to_string());
                FlagBitComparison {
                    meaning,
                    equal_percentage,
                    different_percentage,
                    passed: different_percentage <= eps,
                }
            })
            .collect()
    }
}

fn bit_value(value: f64, mask: u64, mode: &CfFlagMode) -> u64 {
    let bits = if value >= 0.0 {
        value as u64
    } else {
        (value as i64) as u64
    };
    match mode {
        CfFlagMode::Values => {
            if (value - mask as f64).abs() <= 1e-9 {
                1
            } else {
                0
            }
        }
        CfFlagMode::Masks => ((bits & mask) != 0) as u64,
    }
}

fn flag_variable_score(bits: &[FlagBitComparison]) -> f64 {
    if bits.is_empty() {
        return 0.0;
    }
    bits.iter().map(|b| b.equal_percentage).sum::<f64>() / bits.len() as f64
}

pub fn global_flag_score(report: &FlagReport) -> Option<f64> {
    if report.variables.is_empty() {
        return None;
    }
    let mut scores: Vec<f64> = report.variables.iter().map(|v| v.score).collect();
    scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(scores[scores.len() / 2])
}
