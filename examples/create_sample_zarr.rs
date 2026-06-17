//! Generate a minimal EOPF-style Zarr product for local testing.
//!
//! ```bash
//! cargo run --example create_sample_zarr
//! cargo run -- sample_data/S03OLCEFR_sample.zarr
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use zarrs::array::data_type::float32;
use zarrs::array::ArrayBuilder;
use zarrs::filesystem::FilesystemStore;
use zarrs::group::GroupBuilder;
use zarrs::storage::ReadableWritableListableStorage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from("sample_data/S03OLCEFR_sample.zarr");
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    std::fs::create_dir_all(out.parent().unwrap())?;

    let store: ReadableWritableListableStorage =
        Arc::new(FilesystemStore::new(out.to_str().unwrap())?);

    GroupBuilder::new()
        .attributes(serde_json::json!({
            "stac_version": "1.1.0",
            "id": "S03OLCEFR_sample",
            "properties:product:type": "OLCEFR",
            "properties:description": "Sample EOPF-style product for Copernicus Viewer"
        }).as_object().unwrap().clone())
        .build(store.clone(), "/")?
        .store_metadata()?;

    GroupBuilder::new()
        .build(store.clone(), "/measurements")?
        .store_metadata()?;

    GroupBuilder::new()
        .build(store.clone(), "/measurements/image")?
        .store_metadata()?;

    let radiance = ArrayBuilder::new(vec![64, 128], vec![32, 32], float32(), 0.0f32)
        .dimension_names(["y", "x"].into())
        .attributes(serde_json::json!({
            "long_name": "Top of atmosphere radiance",
            "units": "mW m-2 sr-1 nm-1"
        }).as_object().unwrap().clone())
        .build(store.clone(), "/measurements/image/oa01_radiance")?;
    radiance.store_metadata()?;

    let mut data = vec![0.0f32; 64 * 128];
    for y in 0..64 {
        for x in 0..128 {
            data[y * 128 + x] = ((x as f32 / 16.0).sin() * (y as f32 / 12.0).cos() * 50.0 + 60.0);
        }
    }
    radiance.store_array_subset(&[0..64, 0..128], &data)?;

    GroupBuilder::new()
        .build(store.clone(), "/conditions")?
        .store_metadata()?;
    GroupBuilder::new()
        .build(store.clone(), "/conditions/geometry")?
        .store_metadata()?;

    let sza = ArrayBuilder::new(vec![128], vec![128], float32(), 0.0f32)
        .dimension_names(["x"].into())
        .attributes(serde_json::json!({
            "long_name": "Solar zenith angle",
            "units": "degrees"
        }).as_object().unwrap().clone())
        .build(store.clone(), "/conditions/geometry/sza")?;
    sza.store_metadata()?;
    let line: Vec<f32> = (0..128)
        .map(|x| 20.0 + 15.0 * (x as f32 / 20.0).sin())
        .collect();
    sza.store_array_subset(&[0..128], &line)?;

    println!("Created sample product at {}", out.display());
    Ok(())
}
