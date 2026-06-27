use std::path::Path;

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde_json::{Map, Value};

/// Extract STAC-like root attributes from `xfdumanifest.xml`.
pub fn parse_manifest_attributes(manifest_path: &Path) -> Result<Map<String, Value>> {
    let content = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;

    let mut attrs = Map::new();
    attrs.insert("type".to_string(), Value::String("Feature".to_string()));
    attrs.insert(
        "stac_version".to_string(),
        Value::String("1.0.0".to_string()),
    );

    if let Some(id) = manifest_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
    {
        attrs.insert("id".to_string(), Value::String(id.to_string()));
    }

    if let Some(start) = extract_tag_text(&content, "sentinel-safe:startTime") {
        attrs.insert(
            "properties:datetime".to_string(),
            Value::String(start.clone()),
        );
        attrs.insert(
            "properties:start_datetime".to_string(),
            Value::String(start),
        );
    }
    if let Some(stop) = extract_tag_text(&content, "sentinel-safe:stopTime") {
        attrs.insert("properties:end_datetime".to_string(), Value::String(stop));
    }

    if let Some(platform) = extract_tag_text(&content, "sentinel-safe:familyName") {
        attrs.insert("properties:platform".to_string(), Value::String(platform));
    }
    if let Some(instrument) = extract_nested_text(
        &content,
        "sentinel-safe:instrument",
        "sentinel-safe:familyName",
    ) {
        attrs.insert(
            "properties:instruments".to_string(),
            Value::Array(vec![Value::String(instrument)]),
        );
    }

    if let Some(product_type) = infer_product_type_from_manifest(&content) {
        attrs.insert(
            "properties:product:type".to_string(),
            Value::String(product_type),
        );
    }

    if let Some(bbox) = extract_footprint_bbox(&content) {
        let extent = serde_json::json!({
            "spatial": {
                "bbox": [bbox],
                "crs": "EPSG:4326"
            }
        });
        attrs.insert("extent".to_string(), extent);
    }

    if let Some(ring) = extract_footprint_ring_from_content(&content) {
        let coordinates: Vec<Value> = ring
            .iter()
            .map(|&[lon, lat]| Value::Array(vec![Value::from(lon), Value::from(lat)]))
            .collect();
        attrs.insert(
            "geometry".to_string(),
            serde_json::json!({
                "type": "Polygon",
                "coordinates": [coordinates]
            }),
        );
    }

    Ok(attrs)
}

fn infer_product_type_from_manifest(content: &str) -> Option<String> {
    if content.contains("SLSTR Level 2 Land") || content.contains("SL_2_LST") {
        return Some("SLSLST".to_string());
    }
    if content.contains("SLSTR Level 1") {
        return Some("SLSRBT".to_string());
    }
    if content.contains("OLCI Level 1") {
        return Some("OLCEFR".to_string());
    }
    None
}

fn extract_tag_text(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = content.find(&open)? + open.len();
    let end = content[start..].find(&close)? + start;
    let text = content[start..end].trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn extract_nested_text(content: &str, outer: &str, inner: &str) -> Option<String> {
    let outer_open = format!("<{outer}");
    let start = content.find(&outer_open)?;
    let slice = &content[start..];
    extract_tag_text(slice, inner)
}

fn extract_footprint_bbox(content: &str) -> Option<[f64; 4]> {
    let coords = extract_gml_pos_list(content)?;
    if coords.len() < 8 {
        return None;
    }

    let mut lons = Vec::new();
    let mut lats = Vec::new();
    for pair in coords.chunks(2) {
        if pair.len() == 2 {
            // Sentinel SAFE gml:posList is latitude then longitude (EPSG:4326).
            lats.push(pair[0]);
            lons.push(pair[1]);
        }
    }
    if lons.is_empty() {
        return None;
    }

    let west = lons.iter().copied().fold(f64::INFINITY, f64::min);
    let east = lons.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let south = lats.iter().copied().fold(f64::INFINITY, f64::min);
    let north = lats.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some([west, south, east, north])
}

fn extract_gml_pos_list(content: &str) -> Option<Vec<f64>> {
    let marker = "<gml:posList>";
    let end_marker = "</gml:posList>";
    let start = content.find(marker)? + marker.len();
    let end = content[start..].find(end_marker)? + start;
    let text = content[start..end].trim();
    let values: Vec<f64> = text
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

/// Best-effort footprint polygon ring from manifest GML coordinates.
#[allow(dead_code)]
pub fn extract_footprint_ring(manifest_path: &Path) -> Option<Vec<[f64; 2]>> {
    let content = std::fs::read_to_string(manifest_path).ok()?;
    extract_footprint_ring_from_content(&content)
}

fn extract_footprint_ring_from_content(content: &str) -> Option<Vec<[f64; 2]>> {
    let coords = extract_gml_pos_list(content)?;
    let mut ring = Vec::new();
    for pair in coords.chunks(2) {
        if pair.len() == 2 {
            // Store as [longitude, latitude] for STAC / map rendering.
            ring.push([pair[1], pair[0]]);
        }
    }
    if ring.len() >= 3 { Some(ring) } else { None }
}

/// Validate manifest is readable XML (smoke check).
#[allow(dead_code)]
pub fn validate_manifest(manifest_path: &Path) -> Result<()> {
    let content = std::fs::read(manifest_path)?;
    let mut reader = Reader::from_reader(content.as_slice());
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err(err) => {
                return Err(err).with_context(|| format!("parse {}", manifest_path.display()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_POS_LIST: &str =
        "41.9852 -10.1884 41.9335 -9.57268 41.8729 -8.96633 41.8179 -8.35919";

    fn sample_manifest_xml() -> String {
        format!("<root><gml:posList>{SAMPLE_POS_LIST}</gml:posList></root>")
    }

    #[test]
    fn footprint_bbox_uses_latitude_longitude_order() {
        let bbox = extract_footprint_bbox(&sample_manifest_xml()).expect("bbox");
        assert!(
            bbox[0] < 0.0,
            "west longitude should be negative, got {}",
            bbox[0]
        );
        assert!(
            bbox[1] > 35.0,
            "south latitude should be ~41N, got {}",
            bbox[1]
        );
        assert!(
            bbox[2] < 0.0,
            "east longitude should be negative, got {}",
            bbox[2]
        );
        assert!(
            bbox[3] > 35.0,
            "north latitude should be ~42N, got {}",
            bbox[3]
        );
        assert!(bbox[0] < bbox[2], "west < east");
        assert!(bbox[1] < bbox[3], "south < north");
    }

    #[test]
    fn footprint_ring_stores_longitude_first() {
        let ring = extract_footprint_ring_from_content(&sample_manifest_xml()).expect("ring");
        assert_eq!(ring[0], [-10.1884, 41.9852]);
    }

    #[test]
    fn parse_manifest_attributes_includes_geometry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = dir.path().join("xfdumanifest.xml");
        std::fs::write(&manifest, sample_manifest_xml()).expect("write manifest");

        let attrs = parse_manifest_attributes(&manifest).expect("parse");
        assert!(attrs.get("geometry").is_some());
        assert!(attrs.get("extent").is_some());
    }

    #[test]
    fn sl_lst_manifest_footprint_if_present() {
        let manifest = std::path::Path::new(
            "/home/vincent/Data/SLSTR/S3A_SL_2_LST____20260622T102053_20260622T102353_20260622T123949_0179_141_008_2160_PS1_O_NR_005.SEN3/xfdumanifest.xml",
        );
        if !manifest.exists() {
            return;
        }
        let attrs = parse_manifest_attributes(manifest).expect("parse");
        let footprint = crate::display::parse_product_footprint(&attrs).expect("footprint");
        assert!(footprint.south() > 35.0 && footprint.north() > 35.0);
        assert!(footprint.west() < footprint.east());
        assert!(footprint.south() < footprint.north());
        assert!(footprint.west() < 0.0);
        assert!(footprint.polygon.is_some());
    }
}
