use serde_json::{Map, Value};

use crate::display::stac::merge_flat_attributes;

const STAC_ITEM_KEYS: &[&str] = &["stac_discovery", "stac", "item", "stac_item"];

/// Geographic footprint extracted from root STAC / EOPF metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ProductFootprint {
    /// Bounding box as `[west, south, east, north]`.
    pub bbox: [f64; 4],
    /// Coordinate reference system identifier, when declared.
    pub crs: Option<String>,
    /// Outer ring lon/lat vertices when a polygon geometry is available.
    pub polygon: Option<Vec<[f64; 2]>>,
    /// STAC item or product identifier.
    pub product_id: Option<String>,
}

impl ProductFootprint {
    /// Western boundary (minimum longitude or easting).
    pub fn west(&self) -> f64 {
        self.bbox[0]
    }

    /// Southern boundary (minimum latitude or northing).
    pub fn south(&self) -> f64 {
        self.bbox[1]
    }

    /// Eastern boundary (maximum longitude or easting).
    pub fn east(&self) -> f64 {
        self.bbox[2]
    }

    /// Northern boundary (maximum latitude or northing).
    pub fn north(&self) -> f64 {
        self.bbox[3]
    }

    /// One-line summary of bbox and CRS for display.
    pub fn summary(&self) -> String {
        let crs = self.crs.as_deref().unwrap_or("EPSG:4326");
        format!(
            "bbox [{:.3}, {:.3}, {:.3}, {:.3}] ({crs})",
            self.west(),
            self.south(),
            self.east(),
            self.north()
        )
    }
}

/// Parse a product footprint from root `.zattrs` STAC metadata.
pub fn parse_product_footprint(attrs: &Map<String, Value>) -> Option<ProductFootprint> {
    if attrs.is_empty() {
        return None;
    }

    let merged = merge_flat_attributes(attrs);
    let fallback_id = attrs.get("id").and_then(|v| v.as_str()).map(str::to_string);

    for item in stac_item_candidates(&merged, attrs) {
        if let Some(footprint) = footprint_from_stac_item(item, fallback_id.clone()) {
            return Some(footprint);
        }
    }

    None
}

fn stac_item_candidates<'a>(
    merged: &'a Map<String, Value>,
    attrs: &'a Map<String, Value>,
) -> Vec<&'a Map<String, Value>> {
    let mut candidates: Vec<&Map<String, Value>> = Vec::new();
    let mut push_unique = |map: &'a Map<String, Value>| {
        if has_footprint_fields(map) && !candidates.iter().any(|c| std::ptr::eq(*c, map)) {
            candidates.push(map);
        }
    };

    push_unique(merged);
    if !std::ptr::eq(merged, attrs) {
        push_unique(attrs);
    }

    for key in STAC_ITEM_KEYS {
        if let Some(Value::Object(item)) = merged.get(*key) {
            push_unique(item);
        }
        if let Some(Value::Object(item)) = attrs.get(*key) {
            push_unique(item);
        }
    }

    candidates
}

fn has_footprint_fields(map: &Map<String, Value>) -> bool {
    map.contains_key("bbox")
        || map.contains_key("geometry")
        || map
            .get("extent")
            .and_then(|extent| extent.get("spatial"))
            .and_then(|spatial| spatial.get("bbox"))
            .is_some()
}

fn footprint_from_stac_item(
    item: &Map<String, Value>,
    fallback_id: Option<String>,
) -> Option<ProductFootprint> {
    let product_id = item
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or(fallback_id);

    let polygon = item
        .get("geometry")
        .and_then(parse_geometry)
        .and_then(|(_, polygon)| polygon);

    if let Some(bbox) = item.get("bbox").and_then(parse_bbox_value) {
        return Some(ProductFootprint {
            bbox: normalize_bbox(bbox),
            crs: Some("EPSG:4326".to_string()),
            polygon,
            product_id,
        });
    }

    if let Some(value) = item
        .get("extent")
        .and_then(|extent| extent.get("spatial"))
        .and_then(|spatial| spatial.get("bbox"))
        && let Some(bbox) = parse_bbox_value(value)
    {
        let crs = item
            .get("extent")
            .and_then(|extent| extent.get("spatial"))
            .and_then(|spatial| spatial.get("crs"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        return Some(ProductFootprint {
            bbox: normalize_bbox(bbox),
            crs,
            polygon,
            product_id,
        });
    }

    if let Some(geometry) = item.get("geometry")
        && let Some((bbox, geometry_polygon)) = parse_geometry(geometry)
    {
        return Some(ProductFootprint {
            bbox: normalize_bbox(bbox),
            crs: Some("EPSG:4326".to_string()),
            polygon: geometry_polygon,
            product_id,
        });
    }

    None
}

fn normalize_bbox([lon_a, lat_a, lon_b, lat_b]: [f64; 4]) -> [f64; 4] {
    [
        lon_a.min(lon_b),
        lat_a.min(lat_b),
        lon_a.max(lon_b),
        lat_a.max(lat_b),
    ]
}

fn parse_bbox_value(value: &Value) -> Option<[f64; 4]> {
    match value {
        Value::Array(values) if values.is_empty() => None,
        Value::Array(values) if values[0].is_number() => parse_bbox_components(values),
        Value::Array(values) => values.first().and_then(|inner| match inner {
            Value::Array(inner_values) => parse_bbox_components(inner_values),
            _ => None,
        }),
        _ => None,
    }
}

fn parse_bbox_components(values: &[Value]) -> Option<[f64; 4]> {
    if values.len() < 4 {
        return None;
    }
    Some([
        json_number(&values[0])?,
        json_number(&values[1])?,
        json_number(&values[2])?,
        json_number(&values[3])?,
    ])
}

fn parse_geometry(value: &Value) -> Option<([f64; 4], Option<Vec<[f64; 2]>>)> {
    let obj = value.as_object()?;
    let coordinates = obj.get("coordinates")?;
    let geometry_type = obj
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| infer_geometry_type(coordinates));

    match geometry_type {
        "Polygon" => {
            let ring = coordinates.as_array()?.first()?.as_array()?;
            let polygon = parse_ring(ring)?;
            Some((bbox_from_ring(&polygon), Some(polygon)))
        }
        "MultiPolygon" => {
            let mut best: Option<Vec<[f64; 2]>> = None;
            for polygon in coordinates.as_array()? {
                let ring = polygon.as_array()?.first()?.as_array()?;
                let parsed = parse_ring(ring)?;
                if best
                    .as_ref()
                    .is_none_or(|existing| parsed.len() > existing.len())
                {
                    best = Some(parsed);
                }
            }
            best.map(|polygon| (bbox_from_ring(&polygon), Some(polygon)))
        }
        _ => None,
    }
}

fn infer_geometry_type(coordinates: &Value) -> &'static str {
    match coordinates {
        Value::Array(values) => match values.first() {
            Some(Value::Array(inner)) => match inner.first() {
                Some(Value::Array(_)) => "Polygon",
                Some(Value::Number(_)) => "Polygon",
                _ => "Polygon",
            },
            _ => "Polygon",
        },
        _ => "Polygon",
    }
}

fn parse_ring(values: &[Value]) -> Option<Vec<[f64; 2]>> {
    let mut ring = Vec::with_capacity(values.len());
    for point in values {
        let coords = point.as_array()?;
        if coords.len() < 2 {
            continue;
        }
        ring.push([json_number(&coords[0])?, json_number(&coords[1])?]);
    }
    if ring.len() < 3 { None } else { Some(ring) }
}

fn bbox_from_ring(ring: &[[f64; 2]]) -> [f64; 4] {
    let mut min_lon = f64::INFINITY;
    let mut min_lat = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    let mut max_lat = f64::NEG_INFINITY;

    for [lon, lat] in ring {
        min_lon = min_lon.min(*lon);
        min_lat = min_lat.min(*lat);
        max_lon = max_lon.max(*lon);
        max_lat = max_lat.max(*lat);
    }

    [min_lon, min_lat, max_lon, max_lat]
}

fn json_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_extent_spatial_bbox() {
        let attrs = json!({
            "id": "tile-1",
            "extent": {
                "spatial": {
                    "bbox": [-5.0, 45.0, 1.4, 48.2],
                    "crs": "EPSG:4326"
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let footprint = parse_product_footprint(&attrs).expect("footprint");
        assert_eq!(footprint.bbox, [-5.0, 45.0, 1.4, 48.2]);
        assert_eq!(footprint.crs.as_deref(), Some("EPSG:4326"));
    }

    #[test]
    fn parses_colon_separated_extent_bbox() {
        let attrs = json!({
            "extent:spatial:bbox": [-10.0, 40.0, 0.0, 50.0],
            "extent:spatial:crs": "EPSG:4326"
        })
        .as_object()
        .unwrap()
        .clone();

        let footprint = parse_product_footprint(&attrs).expect("footprint");
        assert_eq!(footprint.bbox, [-10.0, 40.0, 0.0, 50.0]);
    }

    #[test]
    fn parses_stac_geometry_polygon() {
        let attrs = json!({
            "geometry": {
                "type": "Polygon",
                "coordinates": [[
                    [-5.0, 45.0],
                    [1.4, 45.0],
                    [1.4, 48.2],
                    [-5.0, 48.2],
                    [-5.0, 45.0]
                ]]
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let footprint = parse_product_footprint(&attrs).expect("footprint");
        assert_eq!(footprint.bbox, [-5.0, 45.0, 1.4, 48.2]);
        assert_eq!(footprint.polygon.as_ref().map(|p| p.len()), Some(5));
    }

    #[test]
    fn parses_nested_stac_discovery_item() {
        let attrs = json!({
            "other_metadata": {"foo": "bar"},
            "stac_discovery": {
                "id": "S03SLSRBT_sample",
                "bbox": [-72.8128, 28.6875, -91.8195, 41.9988],
                "geometry": {
                    "coordinates": [[
                        [-90.1351, 41.9988],
                        [-91.8195, 31.5053],
                        [-72.8128, 28.6875],
                        [-90.1351, 41.9988]
                    ]],
                    "type": "Polygon"
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let footprint = parse_product_footprint(&attrs).expect("footprint");
        assert_eq!(footprint.bbox, [-91.8195, 28.6875, -72.8128, 41.9988]);
        assert_eq!(footprint.product_id.as_deref(), Some("S03SLSRBT_sample"));
        assert!(footprint.polygon.is_some());
    }

    #[test]
    fn parses_real_product_footprint_if_present() {
        let path = std::path::Path::new("/tmp/S03SLSRBT_20230509T154335_0180_B168_SB28.zarr");
        if !path.exists() {
            return;
        }

        let store = crate::zarr::open_store(path.to_str().unwrap()).expect("open store");
        let crate::zarr::ZarrNodeKind::Group { attributes, .. } = &store.tree.root.kind else {
            panic!("root group");
        };

        let footprint = parse_product_footprint(attributes).expect("footprint");
        assert_eq!(
            footprint.product_id.as_deref(),
            Some("S03SLSRBT_20230509T154335_0180_B168_S34C")
        );
        assert!(footprint.west() < -70.0);
        assert!(footprint.east() > -95.0);
        assert!(footprint.polygon.as_ref().is_some_and(|p| p.len() > 10));
    }
}
