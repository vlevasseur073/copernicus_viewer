use std::sync::OnceLock;

use serde_json::Value;

#[derive(Clone)]
pub struct LandRing {
    pub coords: Vec<[f64; 2]>,
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

static LAND_RINGS: OnceLock<Vec<LandRing>> = OnceLock::new();

const MAX_RING_POINTS: usize = 96;

pub fn land_rings() -> &'static [LandRing] {
    LAND_RINGS.get_or_init(parse_land_geojson)
}

fn parse_land_geojson() -> Vec<LandRing> {
    let raw = include_str!("../../../assets/ne_110m_land.geojson");
    let value: Value = serde_json::from_str(raw).expect("valid land geojson");
    let features = value
        .get("features")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut rings = Vec::new();
    for feature in features {
        let Some(geometry) = feature.get("geometry") else {
            continue;
        };
        rings.extend(parse_geometry_rings(geometry));
    }
    rings
}

fn parse_geometry_rings(geometry: &Value) -> Vec<LandRing> {
    let geometry_type = geometry
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let coordinates = geometry.get("coordinates");

    match (geometry_type, coordinates) {
        ("Polygon", Some(Value::Array(rings))) => {
            rings.first().and_then(build_ring).into_iter().collect()
        }
        ("MultiPolygon", Some(Value::Array(polygons))) => polygons
            .iter()
            .filter_map(|polygon| polygon.as_array()?.first())
            .filter_map(build_ring)
            .collect(),
        _ => Vec::new(),
    }
}

fn build_ring(value: &Value) -> Option<LandRing> {
    let points = value.as_array()?;
    let mut coords = Vec::with_capacity(points.len());
    for point in points {
        let pair = point.as_array()?;
        if pair.len() < 2 {
            continue;
        }
        let lon = json_number(&pair[0])?;
        let lat = json_number(&pair[1])?;
        coords.push([lon, lat]);
    }
    if coords.len() < 3 {
        return None;
    }
    simplify_ring(&mut coords);
    let (min_lon, min_lat, max_lon, max_lat) = ring_bounds(&coords);
    Some(LandRing {
        coords,
        min_lon,
        min_lat,
        max_lon,
        max_lat,
    })
}

fn simplify_ring(coords: &mut Vec<[f64; 2]>) {
    if coords.len() <= MAX_RING_POINTS {
        return;
    }
    let step = (coords.len() as f64 / MAX_RING_POINTS as f64).ceil() as usize;
    let simplified: Vec<[f64; 2]> = coords.iter().step_by(step.max(1)).copied().collect();
    *coords = simplified;
    if coords.len() >= 3 && coords.first() != coords.last() {
        let first = coords[0];
        coords.push(first);
    }
}

fn ring_bounds(coords: &[[f64; 2]]) -> (f64, f64, f64, f64) {
    let mut min_lon = f64::INFINITY;
    let mut min_lat = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;
    let mut max_lat = f64::NEG_INFINITY;
    for [lon, lat] in coords {
        min_lon = min_lon.min(*lon);
        min_lat = min_lat.min(*lat);
        max_lon = max_lon.max(*lon);
        max_lat = max_lat.max(*lat);
    }
    (min_lon, min_lat, max_lon, max_lat)
}

fn json_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_land_rings_from_embedded_geojson() {
        let rings = land_rings();
        assert!(rings.len() > 50);
        assert!(rings[0].coords.len() <= MAX_RING_POINTS + 1);
    }
}
