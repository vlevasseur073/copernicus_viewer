//! Zarr store access for EOPF products on local paths and AWS S3.
//!
//! Use [`open_store`] to open a product from a filesystem path or `s3://` URI.
//! Opening reads hierarchy metadata only (`.zgroup`, `.zarray`, `.zattrs`); array
//! chunk data is loaded on demand by [`crate::plot::load_plot_data`] or
//! [`crate::comparison`].

pub mod creds;
pub mod download;
pub mod error;
pub mod location;
pub mod runtime;
pub mod store;
pub mod tree;

pub use download::{
    DownloadProgress, DownloadProgressCallback, download_s3_product, is_s3_product,
    parse_s3_location,
};
pub use location::{
    ProductLocation, format_s3_uri, parent_prefix, parse_product_location,
    resolve_zarr_product_path, s3_config_path,
};
pub use store::{ZarrStore, open_store};
pub use tree::{ZarrNodeKind, ZarrTreeNode};
