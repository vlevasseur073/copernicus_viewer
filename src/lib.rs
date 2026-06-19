//! Lightweight viewer and inspector for [EOPF](https://cpm.pages.eopf.copernicus.eu/eopf-cpm/main/PSFD/index.html)
//! Zarr products from the Copernicus ecosystem.
//!
//! The library opens EOPF Zarr stores on the local filesystem or AWS S3, builds a
//! hierarchy tree from consolidated metadata, and provides helpers for metadata
//! display, geo-referenced plotting, and reference-vs-new product comparison.
//!
//! # Modules
//!
//! - [`zarr`] — store I/O, S3 credentials, product location resolution, hierarchy tree
//! - [`display`] — xarray-style representations, STAC attributes, statistics, footprint map
//! - [`plot`] — async array loading, heatmaps, CF flag variables, geo-referencing
//! - [`comparison`] — structure, variable data, and flag comparison between two products
//!
//! # Examples
//!
//! Open a product and inspect its hierarchy:
//!
//! ```no_run
//! use copernicus_viewer::zarr::open_store;
//!
//! let store = open_store("/path/to/product.zarr").expect("open product");
//! for path in store.tree.root.collect_paths() {
//!     println!("{path}");
//! }
//! ```
//!
//! Compare a reference and a reprocessed product (same logic as the GUI tool):
//!
//! ```no_run
//! use copernicus_viewer::comparison::{compare_products, ComparisonOptions};
//! use copernicus_viewer::zarr::open_store;
//!
//! let reference = open_store("/path/to/reference.zarr").unwrap();
//! let new = open_store("/path/to/new.zarr").unwrap();
//! let result = compare_products(&reference, &new);
//! println!("{}", result.formatted_summary(false));
//! ```
//!
//! See also the [`compare_products`](https://github.com/vlevasseur073/copernicus_viewer/blob/main/examples/compare_products.rs)
//! and [`create_sample_zarr`](https://github.com/vlevasseur073/copernicus_viewer/blob/main/examples/create_sample_zarr.rs)
//! examples in the repository.

pub mod comparison;
pub mod display;
pub mod plot;
pub mod zarr;
