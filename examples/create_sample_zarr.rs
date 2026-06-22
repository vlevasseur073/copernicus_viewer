//! Generate a minimal EOPF-style Zarr product for local testing.
#![allow(clippy::single_range_in_vec_init)]
//!
//! ```bash
//! cargo run --example create_sample_zarr
//! cargo run -- sample_data/S03OLCEFR_sample.zarr
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use zarrs::array::ArrayBuilder;
use zarrs::array::data_type::{float32, uint8};
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
        .attributes(
            serde_json::json!({
                "stac_version": "1.1.0",
                "type": "Feature",
                "id": "S03OLCEFR_sample",
                "properties:product:type": "OLCEFR",
                "properties:description": "Sample EOPF-style product for Copernicus Viewer",
                "properties:datetime": "2024-06-01T12:00:00Z",
                "properties:platform": "Sentinel-3",
                "extent": {
                    "spatial": {
                        "bbox": [-5.0, 45.0, 1.4, 48.2],
                        "crs": "EPSG:4326"
                    }
                },
                "links": [
                    {"rel": "self", "href": "https://example.test/items/S03OLCEFR_sample"},
                    {"rel": "collection", "href": "https://example.test/collections/olcefr"}
                ]
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .build(store.clone(), "/")?
        .store_metadata()?;

    GroupBuilder::new()
        .build(store.clone(), "/measurements")?
        .store_metadata()?;

    GroupBuilder::new()
        .attributes(
            serde_json::json!({
                "crs": "EPSG:4326"
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .build(store.clone(), "/measurements/image")?
        .store_metadata()?;

    let y = ArrayBuilder::new(vec![64], vec![64], float32(), 0.0f32)
        .dimension_names(["y"].into())
        .attributes(
            serde_json::json!({
                "long_name": "latitude",
                "units": "degrees_north",
                "standard_name": "latitude"
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .build(store.clone(), "/measurements/image/y")?;
    y.store_metadata()?;
    let y_coords: Vec<f32> = (0..64).map(|i| 45.0 + i as f32 * 0.05).collect();
    y.store_array_subset(&[0..64], &y_coords)?;

    let x = ArrayBuilder::new(vec![128], vec![128], float32(), 0.0f32)
        .dimension_names(["x"].into())
        .attributes(
            serde_json::json!({
                "long_name": "longitude",
                "units": "degrees_east",
                "standard_name": "longitude"
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .build(store.clone(), "/measurements/image/x")?;
    x.store_metadata()?;
    let x_coords: Vec<f32> = (0..128).map(|i| -5.0 + i as f32 * 0.05).collect();
    x.store_array_subset(&[0..128], &x_coords)?;

    let radiance = ArrayBuilder::new(vec![64, 128], vec![32, 32], float32(), 0.0f32)
        .dimension_names(["y", "x"].into())
        .attributes(
            serde_json::json!({
                "long_name": "Top of atmosphere radiance",
                "units": "mW m-2 sr-1 nm-1",
                "crs": "EPSG:4326"
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .build(store.clone(), "/measurements/image/oa01_radiance")?;
    radiance.store_metadata()?;

    let mut data = vec![0.0f32; 64 * 128];
    for row in 0..64 {
        for col in 0..128 {
            data[row * 128 + col] =
                (col as f32 / 16.0).sin() * (row as f32 / 12.0).cos() * 50.0 + 60.0;
        }
    }
    radiance.store_array_subset(&[0..64, 0..128], &data)?;

    let qa_flags = ArrayBuilder::new(vec![64, 128], vec![32, 32], uint8(), 255u8)
        .dimension_names(["y", "x"].into())
        .attributes(
            serde_json::json!({
                "long_name": "Quality flags",
                "flag_meanings": "good saturation cloud shadow",
                "flag_masks": "1 2 4 8",
                "_FillValue": 255,
                "comment": "CF bitmask variable — select individual bits in the plot panel"
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .build(store.clone(), "/measurements/image/qa_flags")?;
    qa_flags.store_metadata()?;

    let mut flag_data = vec![255u8; 64 * 128];
    for row in 0..64 {
        for col in 0..128 {
            let mut value = 1u8; // good
            if col % 17 == 0 {
                value |= 4; // cloud
            }
            if row % 23 == 0 {
                value |= 8; // shadow
            }
            if (row + col) % 41 == 0 {
                value |= 2; // saturation
            }
            flag_data[row * 128 + col] = value;
        }
    }
    qa_flags.store_array_subset(&[0..64, 0..128], &flag_data)?;

    GroupBuilder::new()
        .build(store.clone(), "/conditions")?
        .store_metadata()?;
    GroupBuilder::new()
        .build(store.clone(), "/conditions/geometry")?
        .store_metadata()?;

    let sza = ArrayBuilder::new(vec![128], vec![128], float32(), 0.0f32)
        .dimension_names(["x"].into())
        .attributes(
            serde_json::json!({
                "long_name": "Solar zenith angle",
                "units": "degrees"
            })
            .as_object()
            .unwrap()
            .clone(),
        )
        .build(store.clone(), "/conditions/geometry/sza")?;
    sza.store_metadata()?;
    let line: Vec<f32> = (0..128)
        .map(|x| 20.0 + 15.0 * (x as f32 / 20.0).sin())
        .collect();
    sza.store_array_subset(&[0..128], &line)?;

    println!("Created sample product at {}", out.display());
    Ok(())
}
