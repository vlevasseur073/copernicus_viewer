use copernicus_viewer::zarr::open_store;
use copernicus_viewer::plot::{load_plot_data, PlotRequest};

#[test]
fn opens_sample_product_and_loads_plot() {
    let path = std::path::Path::new("sample_data/S03OLCEFR_sample.zarr");
    if !path.exists() {
        eprintln!("sample data missing — run: cargo run --example create_sample_zarr");
        return;
    }

    let store = open_store(path).expect("open store");
    let node = store
        .tree
        .root
        .find_by_path("/measurements/image/oa01_radiance")
        .expect("find array");

    let request = PlotRequest {
        array_path: "/measurements/image/oa01_radiance".to_string(),
        slice_indices: vec![],
    };
    let plot = load_plot_data(&store.storage, &node.kind, &request).expect("load plot");
    assert!(matches!(plot, copernicus_viewer::plot::PlotData::Heatmap { .. }));
}
