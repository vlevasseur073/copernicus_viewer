use std::sync::OnceLock;

use serde_json::Value;

use super::render::MapView;

#[derive(Clone)]
pub struct LandRing {
    pub coords: Vec<[f64; 2]>,
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

static LAND_RINGS: OnceLock<Vec<LandRing>> = OnceLock::new();

pub fn land_rings() -> &'static [LandRing] {
    LAND_RINGS.get_or_init(parse_land_geojson)
}

/// Simplify a coastline ring for the current map view (Plate Carrée).
///
/// Large features such as the Afro-Eurasia landmass have thousands of vertices.
/// A fixed global point cap makes continental coastlines blocky while islands
/// stay sharp. Douglas–Peucker in degree space with epsilon tied to pixels/mm
/// keeps detail where the map is zoomed in.
pub fn ring_coords_for_view(
    ring: &LandRing,
    view: MapView,
    pixel_width: usize,
    pixel_height: usize,
) -> Vec<[f64; 2]> {
    if ring.coords.len() <= 3 {
        return ring.coords.clone();
    }

    let lon_eps = (view.lon_span() / pixel_width.max(1) as f64) * 0.6;
    let lat_eps = (view.lat_span() / pixel_height.max(1) as f64) * 0.6;
    let epsilon = lon_eps.min(lat_eps).max(1e-6);

    let mut simplified = douglas_peucker(&ring.coords, epsilon);
    if simplified.len() < 3 {
        simplified = ring.coords.clone();
    }
    simplified
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
    let (min_lon, min_lat, max_lon, max_lat) = ring_bounds(&coords);
    Some(LandRing {
        coords,
        min_lon,
        min_lat,
        max_lon,
        max_lat,
    })
}

fn douglas_peucker(coords: &[[f64; 2]], epsilon: f64) -> Vec<[f64; 2]> {
    if coords.len() <= 2 {
        return coords.to_vec();
    }

    let (start, mut end) = (0, coords.len() - 1);
    while start < end && same_point(coords[start], coords[end]) {
        end -= 1;
    }
    if start >= end {
        return coords.to_vec();
    }

    let mut keep = vec![false; coords.len()];
    keep[start] = true;
    keep[end] = true;
    douglas_peucker_recurse(coords, start, end, epsilon, &mut keep);

    let mut out: Vec<[f64; 2]> = coords
        .iter()
        .zip(keep.iter())
        .filter_map(|(point, &k)| k.then_some(*point))
        .collect();

    if out.len() >= 3 && out.first() != out.last() {
        out.push(out[0]);
    }
    out
}

fn douglas_peucker_recurse(
    coords: &[[f64; 2]],
    start: usize,
    end: usize,
    epsilon: f64,
    keep: &mut [bool],
) {
    if end <= start + 1 {
        return;
    }

    let mut max_dist = 0.0;
    let mut index = start;
    for i in start + 1..end {
        let dist = perpendicular_distance(coords[i], coords[start], coords[end]);
        if dist > max_dist {
            max_dist = dist;
            index = i;
        }
    }

    if max_dist > epsilon {
        keep[index] = true;
        douglas_peucker_recurse(coords, start, index, epsilon, keep);
        douglas_peucker_recurse(coords, index, end, epsilon, keep);
    }
}

fn perpendicular_distance(point: [f64; 2], line_start: [f64; 2], line_end: [f64; 2]) -> f64 {
    let dx = line_end[0] - line_start[0];
    let dy = line_end[1] - line_start[1];
    let len_sq = dx * dx + dy * dy;
    if len_sq < f64::EPSILON {
        let px = point[0] - line_start[0];
        let py = point[1] - line_start[1];
        return (px * px + py * py).sqrt();
    }
    let t = ((point[0] - line_start[0]) * dx + (point[1] - line_start[1]) * dy) / len_sq;
    let t = t.clamp(0.0, 1.0);
    let proj_x = line_start[0] + t * dx;
    let proj_y = line_start[1] + t * dy;
    let px = point[0] - proj_x;
    let py = point[1] - proj_y;
    (px * px + py * py).sqrt()
}

fn same_point(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
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
    }

    #[test]
    fn view_aware_simplify_preserves_regional_coastline_detail() {
        let rings = land_rings();
        let largest = rings
            .iter()
            .max_by_key(|ring| ring.coords.len())
            .expect("land rings");
        let view = MapView {
            min_lon: -5.0,
            min_lat: 35.0,
            max_lon: 25.0,
            max_lat: 55.0,
        };
        let simplified = ring_coords_for_view(largest, view, 480, 280);
        assert!(
            simplified.len() > 80,
            "expected regional coastline detail, got {} points",
            simplified.len()
        );
        assert!(simplified.len() < largest.coords.len());
    }

    #[test]
    fn douglas_peucker_reduces_collinear_points() {
        let coords = vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0], [4.0, 4.0]];
        let out = douglas_peucker(&coords, 0.01);
        assert_eq!(out.len(), 2);
    }
}
