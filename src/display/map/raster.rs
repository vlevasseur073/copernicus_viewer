use egui::{Color32, ColorImage, Pos2, Rect};

use super::land::{land_rings, ring_coords_for_view};
use super::render::{LAND, MapView, OCEAN, graticule_step, project};

pub fn rasterize_basemap(view: MapView, width: usize, height: usize) -> ColorImage {
    let mut pixels = vec![OCEAN; width * height];
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(width as f32, height as f32));

    draw_graticule_pixels(&mut pixels, width, height, rect, view);
    draw_land_pixels(&mut pixels, width, height, rect, view);

    ColorImage {
        size: [width, height],
        pixels,
        ..Default::default()
    }
}

fn draw_graticule_pixels(
    pixels: &mut [Color32],
    width: usize,
    height: usize,
    rect: Rect,
    view: MapView,
) {
    let lon_step = graticule_step(view.lon_span());
    let lat_step = graticule_step(view.lat_span());
    let minor = Color32::from_rgba_unmultiplied(255, 255, 255, 35);
    let major = Color32::from_rgba_unmultiplied(255, 255, 255, 70);

    let mut lon = (view.min_lon / lon_step).floor() * lon_step;
    while lon <= view.max_lon + f64::EPSILON {
        let is_major = lon.abs() < f64::EPSILON || (lon.abs() % 90.0).abs() < f64::EPSILON;
        let color = if is_major { major } else { minor };
        let x = project(lon, view.min_lat, rect, view).x.round() as i32;
        if (0..width as i32).contains(&x) {
            draw_v_line(pixels, width, height, x, color);
        }
        lon += lon_step;
    }

    let mut lat = (view.min_lat / lat_step).floor() * lat_step;
    while lat <= view.max_lat + f64::EPSILON {
        let color = if lat.abs() < f64::EPSILON {
            major
        } else {
            minor
        };
        let y = project(view.min_lon, lat, rect, view).y.round() as i32;
        if (0..height as i32).contains(&y) {
            draw_h_line(pixels, width, height, y, color);
        }
        lat += lat_step;
    }
}

fn draw_land_pixels(
    pixels: &mut [Color32],
    width: usize,
    height: usize,
    rect: Rect,
    view: MapView,
) {
    for ring in land_rings() {
        if !ring_intersects_view(ring, view) {
            continue;
        }
        let simplified = ring_coords_for_view(ring, view, width, height);
        let points: Vec<Pos2> = simplified
            .iter()
            .map(|&[lon, lat]| project(lon, lat, rect, view))
            .collect();
        fill_polygon(pixels, width, height, &points, LAND);
    }
}

fn ring_intersects_view(ring: &super::land::LandRing, view: MapView) -> bool {
    ring.max_lon >= view.min_lon
        && ring.min_lon <= view.max_lon
        && ring.max_lat >= view.min_lat
        && ring.min_lat <= view.max_lat
}

fn draw_v_line(pixels: &mut [Color32], width: usize, height: usize, x: i32, color: Color32) {
    for y in 0..height {
        pixels[y * width + x as usize] = color;
    }
}

fn draw_h_line(pixels: &mut [Color32], width: usize, _height: usize, y: i32, color: Color32) {
    let row = &mut pixels[y as usize * width..(y as usize + 1) * width];
    row.fill(color);
}

fn fill_polygon(
    pixels: &mut [Color32],
    width: usize,
    height: usize,
    points: &[Pos2],
    fill: Color32,
) {
    if points.len() < 3 {
        return;
    }

    let min_y = points
        .iter()
        .map(|p| p.y.floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, height as i32 - 1);
    let max_y = points
        .iter()
        .map(|p| p.y.ceil() as i32)
        .max()
        .unwrap_or(0)
        .clamp(0, height as i32 - 1);

    for y in min_y..=max_y {
        let scanline = y as f32 + 0.5;
        let mut intersections = Vec::new();
        for i in 0..points.len() {
            let p0 = points[i];
            let p1 = points[(i + 1) % points.len()];
            if (p0.y <= scanline && p1.y > scanline) || (p1.y <= scanline && p0.y > scanline) {
                let t = (scanline - p0.y) / (p1.y - p0.y);
                intersections.push(p0.x + t * (p1.x - p0.x));
            }
        }
        intersections.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in intersections.chunks(2) {
            if pair.len() == 2 {
                let x0 = pair[0].floor().max(0.0) as usize;
                let x1 = pair[1].ceil().min(width as f32 - 1.0) as usize;
                if x0 <= x1 {
                    pixels[y as usize * width + x0..=y as usize * width + x1].fill(fill);
                }
            }
        }
    }
}
