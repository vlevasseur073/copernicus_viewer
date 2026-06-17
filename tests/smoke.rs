use copernicus_viewer::display::{parse_root_attributes, InspectorView, AttributeNode};
use copernicus_viewer::plot::{load_plot_data, parse_cf_flags, FlagSelection, PlotData, PlotRequest};
use copernicus_viewer::zarr::{open_store, ZarrNodeKind};

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
        flag_selection: FlagSelection::Raw,
    };
    let loaded = load_plot_data(&store.storage, &store.tree.root, &node.kind, &request, None)
        .expect("load plot");
    assert!(matches!(loaded.plot, PlotData::Heatmap { .. }));
    assert!(loaded.stats.finite_count > 0);
    assert!(!loaded.preview.rows.is_empty());
    assert!(loaded.georef.is_some());
    let georef = loaded.georef.unwrap();
    assert!(georef.x_coords.is_some());
    assert!(georef.y_coords.is_some());
}

#[test]
fn parses_root_attribute_tree() {
    let path = std::path::Path::new("sample_data/S03OLCEFR_sample.zarr");
    if !path.exists() {
        return;
    }

    let store = open_store(path).expect("open store");
    let root = &store.tree.root;

    let tree = parse_root_attributes(root, None).expect("root attrs");
    assert!(tree.iter().any(|node| matches!(
        node,
        AttributeNode::Scalar { name, .. } if name == "stac_version"
    )));
    assert!(tree.iter().any(|node| matches!(
        node,
        AttributeNode::Group { name, .. } if name == "properties"
    )));

    let view = InspectorView::from_node(root, "sample");
    assert!(view.root_attributes.is_some());
    assert!(view.footprint.is_some());
}

#[test]
fn parses_product_footprint_from_sample() {
    let path = std::path::Path::new("sample_data/S03OLCEFR_sample.zarr");
    if !path.exists() {
        return;
    }

    let store = open_store(path).expect("open store");
    let root = &store.tree.root;
    let ZarrNodeKind::Group { attributes, .. } = &root.kind else {
        panic!("root group");
    };

    let footprint = copernicus_viewer::display::parse_product_footprint(attributes).expect("footprint");
    assert!(footprint.west() < footprint.east());
    assert!(footprint.south() < footprint.north());
}

#[test]
fn loads_bitmask_flag_plot() {
    let path = std::path::Path::new("sample_data/S03OLCEFR_sample.zarr");
    if !path.exists() {
        return;
    }

    let store = open_store(path).expect("open store");
    let node = store
        .tree
        .root
        .find_by_path("/measurements/image/qa_flags")
        .expect("find qa_flags");

    let ZarrNodeKind::Array { attributes, .. } = &node.kind else {
        panic!("qa_flags should be an array");
    };
    let flags = parse_cf_flags(attributes).expect("cf flags");
    assert!(flags.uses_bitmasks());
    assert_eq!(flags.meanings.len(), 4);

    let request = PlotRequest {
        array_path: "/measurements/image/qa_flags".to_string(),
        slice_indices: vec![],
        flag_selection: FlagSelection::Flag(2), // cloud bit
    };
    let loaded = load_plot_data(&store.storage, &store.tree.root, &node.kind, &request, None)
        .expect("load flag plot");

    let PlotData::Heatmap { binary, values, .. } = loaded.plot else {
        panic!("expected binary heatmap");
    };
    assert!(binary);
    assert!(values.iter().any(|v| (*v - 1.0).abs() < f32::EPSILON));
    assert!(values.iter().any(|v| (*v - 0.0).abs() < f32::EPSILON));
}
