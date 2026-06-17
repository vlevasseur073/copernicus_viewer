pub mod footprint;
pub mod inspector;
pub mod map;
pub mod repr;
pub mod stac;
pub mod stats;

pub use footprint::{parse_product_footprint, ProductFootprint};
pub use inspector::{render_inspector, InspectorView};
pub use map::render_footprint_map;
pub use repr::format_node_repr;
pub use stac::{
    build_attribute_tree, merge_flat_attributes, parse_root_attributes, render_attribute_tree,
    AttributeNode,
};
pub use stats::{
    build_preview, compute_statistics, format_preview_table, format_statistics, ArrayPreview,
    ArrayStatistics,
};
