//! Metadata display, statistics, STAC attributes, and footprint mapping for EOPF products.
//!
//! The inspector combines xarray-style text representations with optional coverage maps
//! and numeric summaries for selected array variables.

pub mod footprint;
pub mod inspector;
pub mod map;
pub mod repr;
pub mod stac;
pub mod stats;

pub use footprint::{ProductFootprint, parse_product_footprint};
pub use inspector::{InspectorView, render_inspector};
pub use map::render_footprint_map;
pub use repr::format_node_repr;
pub use stac::{
    AttributeNode, build_attribute_tree, merge_flat_attributes, parse_root_attributes,
    render_attribute_tree,
};
pub use stats::{
    ArrayPreview, ArrayStatistics, build_preview, compute_statistics, format_preview_table,
    format_statistics,
};
