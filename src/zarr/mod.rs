//! Zarr store access for EOPF products on local paths and AWS S3.
//!
//! Use [`open_store`] to open a product from a filesystem path or `s3://` URI.
//! Opening reads hierarchy metadata only (`.zgroup`, `.zarray`, `.zattrs`); array
//! chunk data is loaded on demand by [`crate::plot::load_plot_data`] or
//! [`crate::comparison`].

pub mod creds;
pub mod error;
pub mod location;
pub mod runtime;
pub mod store;
pub mod tree;

pub use location::{
    format_s3_uri, parent_prefix, parse_product_location, resolve_zarr_product_path,
    s3_config_path, ProductLocation,
};
pub use store::{open_store, ZarrStore};
pub use tree::{ZarrNodeKind, ZarrTreeNode};
