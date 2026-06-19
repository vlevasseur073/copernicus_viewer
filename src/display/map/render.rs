use egui::epaint::PathShape;
use egui::{Color32, Pos2, Rect, Shape, Stroke, TextureHandle, TextureOptions, Ui, Vec2};

use crate::display::footprint::ProductFootprint;

use super::raster::rasterize_basemap;

const FOOTPRINT_STROKE: Color32 = Color32::from_rgb(255, 170, 40);
pub(crate) const OCEAN: Color32 = Color32::from_rgb(18, 44, 78);
pub(crate) const LAND: Color32 = Color32::from_rgb(196, 196, 176);

/// Footprint occupies roughly one quarter of the view; context is continent-scale.
const VIEW_MULTIPLIER: f64 = 4.0;
const MIN_CONTINENT_LON: f64 = 45.0;
const MIN_CONTINENT_LAT: f64 = 35.0;
const MAX_REGIONAL_LON: f64 = 110.0;
const MAX_REGIONAL_LAT: f64 = 75.0;
/// Fixed raster size for the cached basemap; UI scales the texture.
const BASEMAP_WIDTH: u32 = 480;
const BASEMAP_HEIGHT: u32 = 280;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MapView {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl MapView {
    pub fn lon_span(&self) -> f64 {
        round_degrees((self.max_lon - self.min_lon).max(f64::EPSILON))
    }

    pub fn lat_span(&self) -> f64 {
        round_degrees((self.max_lat - self.min_lat).max(f64::EPSILON))
    }

    fn is_global(&self) -> bool {
        self.lon_span() >= 300.0 || self.lat_span() >= 150.0
    }

    fn cache_key(&self) -> String {
        format!(
            "basemap_{:.2}_{:.2}_{:.2}_{:.2}",
            self.min_lon, self.min_lat, self.max_lon, self.max_lat
        )
    }
}

fn round_degrees(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn footprint_fill_color() -> Color32 {
    Color32::from_rgba_unmultiplied(255, 140, 0, 90)
}

/// Draw an adaptive Plate Carrée coverage map for `footprint` in the inspector.
pub fn render_footprint_map(ui: &mut Ui, footprint: &ProductFootprint) {
    let view = compute_map_view(footprint);

    ui.label(egui::RichText::new("Coverage map").strong());
    ui.label(
        egui::RichText::new(if view.is_global() {
            "Product tile on a global Plate Carrée basemap"
        } else {
            "Continent-scale Plate Carrée view centred on the product footprint"
        })
        .small()
        .weak(),
    );

    let width = ui.available_width().max(280.0);
    let aspect = (view.lon_span() / view.lat_span()) as f32;
    let height = if aspect > 1.4 {
        (width / aspect).clamp(150.0, 260.0)
    } else {
        (width * 0.58).clamp(150.0, 260.0)
    };
    let desired = Vec2::new(width, height);
    let (response, painter) = ui.allocate_painter(desired, egui::Sense::hover());
    let rect = response.rect;

    let pixel_w = BASEMAP_WIDTH;
    let pixel_h = BASEMAP_HEIGHT;
    let texture = basemap_texture(ui, view, pixel_w, pixel_h);

    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, Color32::from_gray(70)),
        egui::StrokeKind::Inside,
    );
    painter.image(
        texture.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    draw_footprint(&painter, rect, footprint, view);
    draw_map_labels(&painter, rect, view);

    ui.add_space(4.0);
    if let Some(id) = &footprint.product_id {
        ui.label(format!("Product: {id}"));
    }
    ui.monospace(footprint.summary());
    if !view.is_global() {
        ui.label(
            egui::RichText::new(format!(
                "View {:.1}° × {:.1}°",
                view.lon_span(),
                view.lat_span()
            ))
            .small()
            .weak(),
        );
    }
}

fn basemap_texture(ui: &Ui, view: MapView, width: u32, height: u32) -> TextureHandle {
    let cache_id = egui::Id::new(("coverage_basemap", view.cache_key()));
    if let Some(texture) = ui.ctx().data(|d| d.get_temp::<TextureHandle>(cache_id)) {
        return texture;
    }

    let image = rasterize_basemap(view, width as usize, height as usize);
    let texture = ui
        .ctx()
        .load_texture(view.cache_key(), image, TextureOptions::LINEAR);
    ui.ctx()
        .data_mut(|d| d.insert_temp(cache_id, texture.clone()));
    texture
}

fn centered_view(center_lon: f64, center_lat: f64, lon_span: f64, lat_span: f64) -> MapView {
    let mut min_lon = center_lon - lon_span / 2.0;
    let mut max_lon = min_lon + lon_span;
    let mut min_lat = center_lat - lat_span / 2.0;
    let mut max_lat = min_lat + lat_span;

    if min_lon < -180.0 {
        min_lon = -180.0;
        max_lon = min_lon + lon_span;
    }
    if max_lon > 180.0 {
        max_lon = 180.0;
        min_lon = max_lon - lon_span;
    }
    if min_lat < -80.0 {
        min_lat = -80.0;
        max_lat = min_lat + lat_span;
    }
    if max_lat > 80.0 {
        max_lat = 80.0;
        min_lat = max_lat - lat_span;
    }

    MapView {
        min_lon: min_lon.clamp(-180.0, 180.0),
        min_lat: min_lat.clamp(-80.0, 80.0),
        max_lon: max_lon.clamp(-180.0, 180.0),
        max_lat: max_lat.clamp(-80.0, 80.0),
    }
}

fn compute_map_view(footprint: &ProductFootprint) -> MapView {
    let (min_lon, min_lat, max_lon, max_lat) = footprint_bounds(footprint);
    let lon_span = (max_lon - min_lon).max(0.05);
    let lat_span = (max_lat - min_lat).max(0.05);

    if lon_span >= 120.0 || lat_span >= 90.0 {
        return MapView {
            min_lon: -180.0,
            min_lat: -60.0,
            max_lon: 180.0,
            max_lat: 85.0,
        };
    }

    let center_lon = (min_lon + max_lon) * 0.5;
    let center_lat = (min_lat + max_lat) * 0.5;

    let view_lon_span = (lon_span * VIEW_MULTIPLIER).clamp(MIN_CONTINENT_LON, MAX_REGIONAL_LON);
    let view_lat_span = (lat_span * VIEW_MULTIPLIER).clamp(MIN_CONTINENT_LAT, MAX_REGIONAL_LAT);

    centered_view(center_lon, center_lat, view_lon_span, view_lat_span)
}

fn footprint_bounds(footprint: &ProductFootprint) -> (f64, f64, f64, f64) {
    if let Some(polygon) = &footprint.polygon {
        let mut min_lon = f64::INFINITY;
        let mut min_lat = f64::INFINITY;
        let mut max_lon = f64::NEG_INFINITY;
        let mut max_lat = f64::NEG_INFINITY;
        for [lon, lat] in polygon {
            min_lon = min_lon.min(*lon);
            min_lat = min_lat.min(*lat);
            max_lon = max_lon.max(*lon);
            max_lat = max_lat.max(*lat);
        }
        if min_lon.is_finite() {
            return (min_lon, min_lat, max_lon, max_lat);
        }
    }

    (
        footprint.west(),
        footprint.south(),
        footprint.east(),
        footprint.north(),
    )
}

pub(crate) fn graticule_step(span: f64) -> f64 {
    const STEPS: [f64; 8] = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 45.0];
    for &step in &STEPS {
        if span / step <= 8.0 {
            return step;
        }
    }
    45.0
}

fn draw_footprint(
    painter: &egui::Painter,
    rect: Rect,
    footprint: &ProductFootprint,
    view: MapView,
) {
    if let Some(polygon) = &footprint.polygon {
        let decimated = decimate_polygon(polygon, 120);
        let points = project_ring(&decimated, rect, view);
        if points.len() >= 3 {
            painter.add(Shape::Path(PathShape {
                points,
                closed: true,
                fill: footprint_fill_color(),
                stroke: Stroke::new(2.0, FOOTPRINT_STROKE).into(),
            }));
            return;
        }
    }

    let corners = [
        project(footprint.west(), footprint.south(), rect, view),
        project(footprint.east(), footprint.south(), rect, view),
        project(footprint.east(), footprint.north(), rect, view),
        project(footprint.west(), footprint.north(), rect, view),
    ];
    painter.add(Shape::convex_polygon(
        corners.to_vec(),
        footprint_fill_color(),
        Stroke::new(2.0, FOOTPRINT_STROKE),
    ));
}

fn decimate_polygon(polygon: &[[f64; 2]], max_points: usize) -> Vec<[f64; 2]> {
    if polygon.len() <= max_points {
        return polygon.to_vec();
    }
    let step = (polygon.len() as f64 / max_points as f64).ceil() as usize;
    polygon.iter().step_by(step.max(1)).copied().collect()
}

fn draw_map_labels(painter: &egui::Painter, rect: Rect, view: MapView) {
    let label_color = Color32::from_rgba_unmultiplied(220, 220, 220, 180);
    let font = egui::FontId::proportional(10.0);

    painter.text(
        Pos2::new(rect.left() + 4.0, rect.top() + 2.0),
        egui::Align2::LEFT_TOP,
        format_lat(view.max_lat),
        font.clone(),
        label_color,
    );
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.bottom() - 2.0),
        egui::Align2::LEFT_BOTTOM,
        format_lat(view.min_lat),
        font.clone(),
        label_color,
    );
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        format_lon(view.min_lon),
        font.clone(),
        label_color,
    );
    painter.text(
        Pos2::new(rect.right() - 4.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format_lon(view.max_lon),
        font,
        label_color,
    );
}

fn format_lon(lon: f64) -> String {
    if lon >= 0.0 {
        format!("{lon:.0}°E")
    } else {
        format!("{:.0}°W", lon.abs())
    }
}

fn format_lat(lat: f64) -> String {
    if lat >= 0.0 {
        format!("{lat:.0}°N")
    } else {
        format!("{:.0}°S", lat.abs())
    }
}

fn project_ring(coords: &[[f64; 2]], rect: Rect, view: MapView) -> Vec<Pos2> {
    coords
        .iter()
        .map(|&[lon, lat]| project(lon, lat, rect, view))
        .collect()
}

pub(crate) fn project(lon: f64, lat: f64, rect: Rect, view: MapView) -> Pos2 {
    let x = rect.left() + ((lon - view.min_lon) / view.lon_span()) as f32 * rect.width();
    let y = rect.top() + ((view.max_lat - lat) / view.lat_span()) as f32 * rect.height();
    Pos2::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::footprint::ProductFootprint;

    fn sample_footprint(w: f64, s: f64, e: f64, n: f64) -> ProductFootprint {
        ProductFootprint {
            bbox: [w, s, e, n],
            crs: Some("EPSG:4326".to_string()),
            polygon: None,
            product_id: Some("test".to_string()),
        }
    }

    #[test]
    fn regional_tile_uses_continent_scale_view() {
        let footprint = sample_footprint(-91.8195, 28.6875, -72.8128, 41.9988);
        let view = compute_map_view(&footprint);
        assert!(view.lon_span() >= MIN_CONTINENT_LON);
        assert!(view.lat_span() >= MIN_CONTINENT_LAT);
        assert!(view.lon_span() >= (footprint.east() - footprint.west()) * 3.0);
    }

    #[test]
    fn global_tile_keeps_world_view() {
        let footprint = sample_footprint(-180.0, -60.0, 180.0, 80.0);
        let view = compute_map_view(&footprint);
        assert!(view.is_global());
    }

    #[test]
    fn sample_europe_tile_is_continent_scale() {
        let footprint = sample_footprint(-5.0, 45.0, 1.4, 48.2);
        let view = compute_map_view(&footprint);
        assert!(view.lon_span() >= MIN_CONTINENT_LON);
        assert!(view.lat_span() >= MIN_CONTINENT_LAT);
    }
}
