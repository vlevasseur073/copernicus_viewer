//! Product comparison logic.

use crate::zarr::ZarrStore;

use super::product_label;

/// Outcome of comparing two open products (placeholder until real diff/plot logic exists).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComparisonResult {
    pub summary: String,
}

/// Compare two loaded EOPF Zarr products.
///
/// Implementation pending — callers should pass fully opened [`ZarrStore`] handles.
pub fn compare_products(left: &ZarrStore, right: &ZarrStore) -> ComparisonResult {
    ComparisonResult {
        summary: format!(
            "Comparison of “{}” vs “{}” is not implemented yet.",
            product_label(left),
            product_label(right),
        ),
    }
}
