use serde_json::{Map, Value};

use crate::plot::flags::parse_cf_flags;
use crate::zarr::{ZarrNodeKind, ZarrStore, ZarrTreeNode};

use super::options::CompareOptions;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StructureStatus {
    Passed,
    /// Non-fatal difference (e.g. chunk layout) — data is still compared on the reference grid.
    Warning,
    Failed,
    MissingInNew,
}

#[derive(Clone, Debug)]
pub struct StructureIssue {
    pub path: String,
    pub field: String,
    pub status: StructureStatus,
    pub detail: String,
}

#[derive(Clone, Debug, Default)]
pub struct StructureReport {
    pub issues: Vec<StructureIssue>,
    pub skip_data_comparison: bool,
}

impl StructureReport {
    pub fn passed_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.status == StructureStatus::Passed)
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| {
                matches!(
                    i.status,
                    StructureStatus::Failed | StructureStatus::MissingInNew
                )
            })
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.status == StructureStatus::Warning)
            .count()
    }
}

/// Compare hierarchy metadata between two products (no array payload I/O).
pub fn compare_structure(
    left: &ZarrStore,
    right: &ZarrStore,
    options: &CompareOptions,
) -> StructureReport {
    let mut report = StructureReport::default();

    left.tree.root.visit_nodes(&mut |node| {
        let Some(other) = right.tree.root.find_by_path(&node.path) else {
            report.issues.push(StructureIssue {
                path: node.path.clone(),
                field: "node".to_string(),
                status: StructureStatus::MissingInNew,
                detail: "exists in reference but not in new product".to_string(),
            });
            return;
        };

        compare_node_pair(node, other, options, &mut report);
    });

    report
}

fn compare_node_pair(
    left: &ZarrTreeNode,
    right: &ZarrTreeNode,
    options: &CompareOptions,
    report: &mut StructureReport,
) {
    match (&left.kind, &right.kind) {
        (ZarrNodeKind::Group { attributes: a }, ZarrNodeKind::Group { attributes: b }) => {
            compare_field(
                &left.path,
                "attrs",
                attrs_equal(a, b),
                report,
                "group attributes differ",
            );
            compare_child_names(left, right, report);
        }
        (
            ZarrNodeKind::Array {
                shape: ls,
                chunks: lc,
                dtype: lt,
                dimension_names: ld,
                attributes: la,
                fill_value: lf,
            },
            ZarrNodeKind::Array {
                shape: rs,
                chunks: rc,
                dtype: rt,
                dimension_names: rd,
                attributes: ra,
                fill_value: rf,
            },
        ) => {
            let shape_ok = ls == rs;
            compare_field(
                &left.path,
                "shape",
                shape_ok,
                report,
                format!("reference {ls:?} vs new {rs:?}"),
            );
            if !shape_ok {
                report.skip_data_comparison = true;
            }

            compare_field(
                &left.path,
                "dtype",
                lt == rt,
                report,
                format!("reference {lt} vs new {rt}"),
            );
            compare_field(
                &left.path,
                "dimension_names",
                ld == rd,
                report,
                format!("reference {ld:?} vs new {rd:?}"),
            );
            if options.chunks {
                if lc == rc {
                    compare_field(&left.path, "chunks", true, report, "identical");
                } else if shape_ok {
                    report.issues.push(StructureIssue {
                        path: left.path.clone(),
                        field: "chunks".to_string(),
                        status: StructureStatus::Warning,
                        detail: format!(
                            "reference {lc:?} vs new {rc:?} — data compared on reference chunk grid"
                        ),
                    });
                } else {
                    compare_field(
                        &left.path,
                        "chunks",
                        false,
                        report,
                        format!("reference {lc:?} vs new {rc:?}"),
                    );
                }
            }
            compare_field(
                &left.path,
                "attrs",
                attrs_equal(la, ra),
                report,
                "array attributes differ",
            );
            compare_field(
                &left.path,
                "fill_value",
                lf == rf,
                report,
                format!("reference {lf:?} vs new {rf:?}"),
            );
        }
        _ => {
            report.issues.push(StructureIssue {
                path: left.path.clone(),
                field: "kind".to_string(),
                status: StructureStatus::Failed,
                detail: "node kind mismatch (group vs array)".to_string(),
            });
            report.skip_data_comparison = true;
        }
    }
}

fn compare_child_names(left: &ZarrTreeNode, right: &ZarrTreeNode, report: &mut StructureReport) {
    let mut left_names: Vec<_> = left.children.iter().map(|c| c.name.as_str()).collect();
    let mut right_names: Vec<_> = right.children.iter().map(|c| c.name.as_str()).collect();
    left_names.sort_unstable();
    right_names.sort_unstable();
    compare_field(
        &left.path,
        "children",
        left_names == right_names,
        report,
        format!("reference {left_names:?} vs new {right_names:?}"),
    );
}

fn compare_field(
    path: &str,
    field: &str,
    ok: bool,
    report: &mut StructureReport,
    detail: impl Into<String>,
) {
    report.issues.push(StructureIssue {
        path: path.to_string(),
        field: field.to_string(),
        status: if ok {
            StructureStatus::Passed
        } else {
            StructureStatus::Failed
        },
        detail: if ok {
            "identical".to_string()
        } else {
            detail.into()
        },
    });
}

fn attrs_equal(a: &Map<String, Value>, b: &Map<String, Value>) -> bool {
    a == b
}

pub fn is_coordinate_variable(node: &ZarrTreeNode) -> bool {
    let ZarrNodeKind::Array { attributes, .. } = &node.kind else {
        return false;
    };

    if node.path.contains("/coordinates/") || node.path.contains("/conditions/geometry/") {
        return true;
    }

    if attribute_str(attributes, "standard_name").is_some_and(|name| {
        matches!(
            name.as_str(),
            "time" | "latitude" | "longitude" | "projection" | "grid_mapping"
        )
    }) {
        return true;
    }

    if attribute_str(attributes, "axis")
        .is_some_and(|axis| matches!(axis.as_str(), "T" | "X" | "Y" | "Z"))
    {
        return true;
    }

    let name = node.name.as_str();
    if node.path.contains("/geometry/")
        && matches!(
            name,
            "x" | "y" | "latitude" | "longitude" | "lon" | "lat" | "columns" | "rows"
        )
    {
        return true;
    }

    false
}

pub fn is_data_variable(node: &ZarrTreeNode) -> bool {
    let ZarrNodeKind::Array {
        attributes, dtype, ..
    } = &node.kind
    else {
        return false;
    };
    if node.is_empty_array() {
        return false;
    }
    if is_coordinate_variable(node) {
        return false;
    }
    if parse_cf_flags(attributes).is_some() {
        return false;
    }
    if is_unsupported_dtype(dtype) {
        return false;
    }
    let name = node.name.as_str();
    if name.ends_with("spatial_ref") || name.ends_with("band") {
        return false;
    }
    true
}

fn is_unsupported_dtype(dtype: &str) -> bool {
    let normalized = dtype.to_ascii_lowercase();
    normalized.contains("datetime") || normalized.contains("m8[")
}

fn attribute_str(attributes: &Map<String, Value>, key: &str) -> Option<String> {
    attributes
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

pub fn is_flag_variable(node: &ZarrTreeNode) -> bool {
    let ZarrNodeKind::Array { attributes, .. } = &node.kind else {
        return false;
    };
    if node.is_empty_array() {
        return false;
    }
    parse_cf_flags(attributes).is_some()
}

pub fn collect_data_variables(root: &ZarrTreeNode) -> Vec<String> {
    let mut paths = Vec::new();
    root.visit_nodes(&mut |node| {
        if is_data_variable(node) {
            paths.push(node.path.clone());
        }
    });
    paths.sort();
    paths
}

pub fn collect_flag_variables(root: &ZarrTreeNode) -> Vec<String> {
    let mut paths = Vec::new();
    root.visit_nodes(&mut |node| {
        if is_flag_variable(node) {
            paths.push(node.path.clone());
        }
    });
    paths.sort();
    paths
}
