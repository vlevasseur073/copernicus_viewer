//! Product comparison logic (ported from sentineltoolbox `compare_product_datatrees`).

use crate::zarr::ZarrStore;

use super::data::{
    DataReport, compare_variable_data, format_variable_detail, global_relative_score,
};
use super::flags::{FlagReport, compare_flag_variables, global_flag_score as median_flag_score};
use super::options::CompareOptions;
use super::product_label;
use super::structure::{
    StructureReport, StructureStatus, collect_data_variables, collect_flag_variables,
    compare_structure,
};

pub use super::options::CompareOptions as ComparisonOptions;

/// Outcome of comparing two open EOPF Zarr products.
#[derive(Clone, Debug)]
pub struct ComparisonResult {
    /// Display label for the reference product (final path segment).
    pub reference_label: String,
    /// Display label for the new product.
    pub new_label: String,
    /// Pre-formatted summary report (non-verbose).
    pub summary: String,
    /// Overall pass when structure, data, and flags all succeed.
    pub success: bool,
    /// Whether both products have the same hierarchy paths.
    pub isomorphic: bool,
    /// `true` when fatal structure differences prevented data comparison.
    pub skip_data: bool,
    /// Structure and metadata comparison details.
    pub structure: StructureReport,
    /// Measurement variable data comparison details.
    pub data: DataReport,
    /// CF flag variable comparison details.
    pub flags: FlagReport,
    /// Median relative score across variables (relative mode only).
    pub global_score: Option<f64>,
    /// Median equal-percentage score across flag variables.
    pub global_flag_score: Option<f64>,
    /// Outlier ratio threshold used for this run.
    pub threshold_nb_outliers: f64,
    /// Coverage difference threshold used for this run.
    pub threshold_coverage: f64,
}

impl ComparisonResult {
    /// Re-format the report; set `verbose` to list every variable and flag bit.
    pub fn formatted_summary(&self, verbose: bool) -> String {
        format_summary(self, verbose)
    }
}

/// Compare two loaded EOPF Zarr products using default sentineltoolbox thresholds.
pub fn compare_products(left: &ZarrStore, right: &ZarrStore) -> ComparisonResult {
    compare_products_with_options(left, right, &CompareOptions::default())
}

/// Compare two loaded products with explicit options.
pub fn compare_products_with_options(
    left: &ZarrStore,
    right: &ZarrStore,
    options: &CompareOptions,
) -> ComparisonResult {
    compare_product_trees(left, right, options)
}

/// Rust port of `compare_product_datatrees` (structure, data, flags).
fn compare_product_trees(
    left: &ZarrStore,
    right: &ZarrStore,
    options: &CompareOptions,
) -> ComparisonResult {
    let left_label = product_label(left);
    let right_label = product_label(right);

    let isomorphic = left.tree.root.is_isomorphic_to(&right.tree.root);
    if !isomorphic {
        let result = ComparisonResult {
            reference_label: left_label,
            new_label: right_label,
            summary: String::new(),
            success: false,
            isomorphic: false,
            skip_data: true,
            structure: StructureReport::default(),
            data: DataReport::default(),
            flags: FlagReport::default(),
            global_score: None,
            global_flag_score: None,
            threshold_nb_outliers: options.threshold_nb_outliers,
            threshold_coverage: options.threshold_coverage,
        };
        return ComparisonResult {
            summary: result.formatted_summary(false),
            ..result
        };
    }

    let mut skip_data = false;
    let structure = if options.structure {
        let report = compare_structure(left, right, options);
        skip_data = report.skip_data_comparison;
        report
    } else {
        StructureReport::default()
    };

    let data_paths = collect_data_variables(&left.tree.root);
    let flag_paths = collect_flag_variables(&left.tree.root);

    let data = if options.data && !skip_data {
        compare_variable_data(left, right, &data_paths, options)
    } else {
        DataReport::default()
    };

    let flags = if options.flags && !skip_data {
        compare_flag_variables(left, right, &flag_paths, options)
    } else {
        FlagReport::default()
    };

    let global_score = if options.relative {
        global_relative_score(&data)
    } else {
        None
    };
    let global_flag_score = median_flag_score(&flags);

    let success = isomorphic
        && structure.failed_count() == 0
        && data.failed_count() == 0
        && flags.failed_count() == 0
        && !skip_data;

    let mut result = ComparisonResult {
        reference_label: left_label,
        new_label: right_label,
        summary: String::new(),
        success,
        isomorphic,
        skip_data,
        structure,
        data,
        flags,
        global_score,
        global_flag_score,
        threshold_nb_outliers: options.threshold_nb_outliers,
        threshold_coverage: options.threshold_coverage,
    };
    result.summary = result.formatted_summary(false);
    result
}

fn format_summary(result: &ComparisonResult, verbose: bool) -> String {
    let left = &result.reference_label;
    let right = &result.new_label;
    let structure = &result.structure;
    let data = &result.data;
    let flags = &result.flags;

    let mut lines = vec![
        format!("Compare “{right}” to reference “{left}”"),
        format!(
            "Overall: {}",
            if result.success { "PASSED" } else { "FAILED" }
        ),
    ];

    if !result.isomorphic {
        lines.push("Products are not isomorphic.".to_string());
        return lines.join("\n");
    }

    if result.skip_data {
        lines.push(
            "Fatal structure differences — variable data comparison was skipped.".to_string(),
        );
    }

    lines.push(format!(
        "Structure: {} passed, {} warnings, {} failed",
        structure.passed_count(),
        structure.warning_count(),
        structure.failed_count()
    ));
    for issue in structure
        .issues
        .iter()
        .filter(|i| i.status == StructureStatus::Warning)
        .take(8)
    {
        lines.push(format!(
            "  ⚠ [{}][{}] {}",
            issue.path, issue.field, issue.detail
        ));
    }
    for issue in structure
        .issues
        .iter()
        .filter(|i| {
            matches!(
                i.status,
                StructureStatus::Failed | StructureStatus::MissingInNew
            )
        })
        .take(12)
    {
        lines.push(format!(
            "  ✗ [{}][{}] {}",
            issue.path, issue.field, issue.detail
        ));
    }
    if structure.failed_count() > 12 {
        lines.push(format!(
            "  … and {} more structure issues",
            structure.failed_count() - 12
        ));
    }

    lines.push(format!(
        "Variables: {} passed, {} failed, {} skipped",
        data.passed_count(),
        data.failed_count(),
        data.skipped_count()
    ));
    for skipped in data.skipped.iter().take(8) {
        lines.push(format!("  ⊘ {} — {}", skipped.path, skipped.reason));
    }
    if data.skipped_count() > 8 {
        lines.push(format!(
            "  … and {} more skipped variables",
            data.skipped_count() - 8
        ));
    }
    let failed_vars: Vec<_> = data.variables.iter().filter(|v| !v.passed).collect();
    let passed_vars: Vec<_> = data.variables.iter().filter(|v| v.passed).collect();
    let failed_limit = if verbose {
        failed_vars.len()
    } else {
        failed_vars.len().min(12)
    };
    let passed_limit = if verbose {
        passed_vars.len()
    } else {
        passed_vars.len().min(6)
    };

    for var in failed_vars.iter().take(failed_limit) {
        lines.push(format!(
            "  ✗ {}",
            format_variable_detail(var, result.threshold_nb_outliers, result.threshold_coverage)
        ));
    }
    if failed_vars.len() > failed_limit {
        lines.push(format!(
            "  … and {} more failed variables",
            failed_vars.len() - failed_limit
        ));
    }

    if !passed_vars.is_empty() && !verbose && failed_vars.is_empty() {
        lines.push("  (passed variables omitted — use --verbose to list them)".to_string());
    } else if !passed_vars.is_empty() && !verbose && !failed_vars.is_empty() {
        lines.push("  Passed:".to_string());
    }

    for var in passed_vars.iter().take(passed_limit) {
        if verbose {
            lines.push(format!(
                "  ✓ {}",
                format_variable_detail(
                    var,
                    result.threshold_nb_outliers,
                    result.threshold_coverage
                )
            ));
        } else {
            lines.push(format!("  ✓ {}", var.path));
        }
    }
    if passed_vars.len() > passed_limit {
        lines.push(format!(
            "  … and {} more passed variables (use --verbose to list all)",
            passed_vars.len() - passed_limit
        ));
    }

    lines.push(format!(
        "Flags: {} bits passed, {} bits failed",
        flags.passed_count(),
        flags.failed_count()
    ));
    for var in flags.variables.iter() {
        for bit in var.bits.iter().filter(|b| !b.passed) {
            lines.push(format!(
                "  ✗ {} — {} different {:.2}%",
                var.path, bit.meaning, bit.different_percentage
            ));
        }
    }
    if verbose {
        for var in flags.variables.iter() {
            for bit in var.bits.iter().filter(|b| b.passed) {
                lines.push(format!(
                    "  ✓ {} — {} equal {:.2}%",
                    var.path, bit.meaning, bit.equal_percentage
                ));
            }
        }
    }

    if let Some(score) = result.global_score {
        lines.push(format!("Global relative score: {score:.6}%"));
    }
    if let Some(score) = result.global_flag_score {
        lines.push(format!("Global flag score (median): {score:.6}%"));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zarr::open_store;
    use std::path::Path;

    #[test]
    fn identical_sample_products_pass() {
        let path = Path::new("sample_data/S03OLCEFR_sample.zarr");
        if !path.exists() {
            return;
        }

        let left = open_store(path.to_str().unwrap()).expect("open left");
        let right = open_store(path.to_str().unwrap()).expect("open right");
        let result = compare_products(&left, &right);
        assert!(result.isomorphic);
        assert!(result.success, "{}", result.summary);
    }

    #[test]
    fn slstr_products_compare_data_variables() {
        let ref_path = Path::new(
            "/tmp/s3slstr_data_verification/reference/fixed/S03SLSLST_20230514T074253_0180_A377_SBBE.zarr",
        );
        let new_path = Path::new(
            "/tmp/s3slstr_data_verification/output_dpr/S03SLSLST_20230514T074253_0180_A377_S000.zarr",
        );
        if !ref_path.exists() || !new_path.exists() {
            return;
        }

        let left = open_store(ref_path.to_str().unwrap()).expect("open reference");
        let right = open_store(new_path.to_str().unwrap()).expect("open new");
        let result = compare_products(&left, &right);

        assert!(result.isomorphic, "{}", result.summary);
        assert!(
            !result.skip_data,
            "data comparison should not be skipped: {}",
            result.summary
        );
        assert!(
            !result.data.variables.is_empty(),
            "expected compared variables: {}",
            result.summary
        );
    }
}
