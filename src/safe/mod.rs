//! Sentinel-3 SAFE product support mapped to EOPF-like hierarchy.

mod io;
mod manifest;
mod mapping;
mod product_type;
mod store;

pub use store::{SafeStore, is_safe_product_dir};
