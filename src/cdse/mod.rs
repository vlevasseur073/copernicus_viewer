//! CDSE catalogue search and product download via [`copernicus_explorer`].
//!
//! The GUI lives in [`CdseDownloadTool`]; the application main loop runs search
//! and download workers on a shared Tokio runtime and auto-opens successful
//! downloads with the existing product open path.

mod extract;

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{Duration, Utc};
use copernicus_explorer::{
    BoundingBox, DownloadProgressEvent, Geometry, Point, Product, Satellite, SearchQuery,
};
use eframe::egui;

pub use extract::{CdsePreparedProduct, prepare_downloaded_product};

/// Snapshot of a catalogue hit for display and download.
#[derive(Clone, Debug)]
pub struct CdseProductRow {
    pub id: String,
    pub name: String,
    pub acquisition_date: String,
    pub cloud_cover: Option<f64>,
    pub online: bool,
}

impl From<&Product> for CdseProductRow {
    fn from(product: &Product) -> Self {
        Self {
            id: product.id.clone(),
            name: product.name.clone(),
            acquisition_date: product.acquisition_date.clone(),
            cloud_cover: product.cloud_cover,
            online: product.online,
        }
    }
}

/// Parameters for a background catalogue search.
#[derive(Clone, Debug)]
pub struct CdseSearchRequest {
    pub satellite: Satellite,
    pub product: String,
    pub start_date: String,
    pub end_date: String,
    pub tile: String,
    pub cloud_cover: f64,
    pub point: String,
    pub bbox: String,
    pub geojson: String,
    pub max_results: u32,
}

impl CdseSearchRequest {
    /// Build and execute a [`SearchQuery`] (async).
    pub async fn execute(self) -> Result<Vec<Product>, String> {
        build_search_query(&self)?
            .execute()
            .await
            .map_err(|err| err.to_string())
    }
}

/// Parameters for a background product download.
#[derive(Clone, Debug)]
pub struct CdseDownloadRequest {
    pub product_id: String,
    pub product_name: String,
    pub dest_dir: PathBuf,
    /// When both are non-empty, use explicit credentials; otherwise env vars.
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug)]
pub enum CdseDownloadUiStatus {
    Downloading {
        label: String,
        downloaded: u64,
        total: Option<u64>,
    },
    Extracting,
    Completed {
        path: String,
    },
    Failed {
        message: String,
    },
}

const SATELLITES: [Satellite; 5] = [
    Satellite::Sentinel1,
    Satellite::Sentinel2,
    Satellite::Sentinel3,
    Satellite::Sentinel5P,
    Satellite::Sentinel6,
];

/// CDSE search / download workspace window.
pub struct CdseDownloadTool {
    open: bool,
    satellite: Satellite,
    previous_satellite: Satellite,
    product: String,
    start_date: String,
    end_date: String,
    tile: String,
    cloud_cover: f64,
    point: String,
    bbox: String,
    geojson: String,
    max_results: u32,
    download_dir: String,
    username: String,
    password: String,
    products: Vec<CdseProductRow>,
    selected: HashMap<String, bool>,
    downloads: HashMap<String, CdseDownloadUiStatus>,
    status: String,
    searching: bool,
    pending_search: bool,
    pending_downloads: Vec<CdseDownloadRequest>,
    pending_dest_folder_pick: bool,
    pending_geojson_pick: bool,
    paths_to_open: Vec<PathBuf>,
}

impl Default for CdseDownloadTool {
    fn default() -> Self {
        let end_date = Utc::now().date_naive();
        let start_date = end_date - Duration::days(7);
        let satellite = Satellite::Sentinel2;
        Self {
            open: false,
            satellite,
            previous_satellite: satellite,
            product: satellite.known_products()[0].to_string(),
            start_date: start_date.format("%Y-%m-%d").to_string(),
            end_date: end_date.format("%Y-%m-%d").to_string(),
            tile: String::new(),
            cloud_cover: 30.0,
            point: String::new(),
            bbox: String::new(),
            geojson: String::new(),
            max_results: 10,
            download_dir: String::from("."),
            username: String::new(),
            password: String::new(),
            products: Vec::new(),
            selected: HashMap::new(),
            downloads: HashMap::new(),
            status: String::new(),
            searching: false,
            pending_search: false,
            pending_downloads: Vec::new(),
            pending_dest_folder_pick: false,
            pending_geojson_pick: false,
            paths_to_open: Vec::new(),
        }
    }
}

impl CdseDownloadTool {
    pub fn show(&mut self) {
        self.open = true;
    }

    pub fn take_pending_search(&mut self) -> Option<CdseSearchRequest> {
        if !self.pending_search {
            return None;
        }
        self.pending_search = false;
        Some(self.search_request())
    }

    pub fn take_pending_downloads(&mut self) -> Vec<CdseDownloadRequest> {
        std::mem::take(&mut self.pending_downloads)
    }

    pub fn take_pending_dest_folder_pick(&mut self) -> bool {
        let pending = self.pending_dest_folder_pick;
        self.pending_dest_folder_pick = false;
        pending
    }

    pub fn take_pending_geojson_pick(&mut self) -> bool {
        let pending = self.pending_geojson_pick;
        self.pending_geojson_pick = false;
        pending
    }

    pub fn set_download_dir(&mut self, path: PathBuf) {
        self.download_dir = path.display().to_string();
    }

    pub fn set_geojson_path(&mut self, path: PathBuf) {
        self.geojson = path.display().to_string();
    }

    pub fn start_searching(&mut self) {
        self.searching = true;
        self.status = "Searching CDSE catalogue…".to_string();
    }

    pub fn apply_search_result(&mut self, result: Result<Vec<Product>, String>) {
        self.searching = false;
        match result {
            Ok(products) => {
                self.products = products.iter().map(CdseProductRow::from).collect();
                self.selected.clear();
                for row in &self.products {
                    self.selected.insert(row.id.clone(), false);
                }
                self.status = format!("Found {} product(s).", self.products.len());
            }
            Err(err) => {
                self.products.clear();
                self.selected.clear();
                self.status = format!("Search failed: {err}");
            }
        }
    }

    pub fn apply_download_progress(&mut self, product_id: &str, event: DownloadProgressEvent) {
        match event {
            DownloadProgressEvent::Started { label, total } => {
                self.downloads.insert(
                    product_id.to_string(),
                    CdseDownloadUiStatus::Downloading {
                        label,
                        downloaded: 0,
                        total,
                    },
                );
            }
            DownloadProgressEvent::Progress { downloaded } => {
                match self.downloads.get(product_id).cloned() {
                    Some(CdseDownloadUiStatus::Downloading { label, total, .. }) => {
                        self.downloads.insert(
                            product_id.to_string(),
                            CdseDownloadUiStatus::Downloading {
                                label,
                                downloaded,
                                total,
                            },
                        );
                    }
                    _ => {
                        self.downloads.insert(
                            product_id.to_string(),
                            CdseDownloadUiStatus::Downloading {
                                label: product_id.to_string(),
                                downloaded,
                                total: None,
                            },
                        );
                    }
                }
            }
            DownloadProgressEvent::Completed { .. } => {
                self.downloads
                    .insert(product_id.to_string(), CdseDownloadUiStatus::Extracting);
                self.status = "Download complete; extracting product…".to_string();
            }
            DownloadProgressEvent::Failed { message } => {
                self.downloads.insert(
                    product_id.to_string(),
                    CdseDownloadUiStatus::Failed {
                        message: message.clone(),
                    },
                );
                self.status = format!("Download failed: {message}");
            }
        }
    }

    pub fn apply_download_result(
        &mut self,
        product_id: &str,
        result: Result<CdsePreparedProduct, String>,
    ) -> Option<tempfile::TempDir> {
        match result {
            Ok(prepared) => {
                let status_path = match &prepared.zip_path {
                    Some(zip) => format!(
                        "{} (extracted from {})",
                        prepared.open_path.display(),
                        zip.display()
                    ),
                    None => prepared.open_path.display().to_string(),
                };
                self.downloads.insert(
                    product_id.to_string(),
                    CdseDownloadUiStatus::Completed { path: status_path },
                );
                self.paths_to_open.push(prepared.open_path);
                self.status = "Download complete; opening product…".to_string();
                prepared.extract_guard
            }
            Err(message) => {
                self.downloads.insert(
                    product_id.to_string(),
                    CdseDownloadUiStatus::Failed {
                        message: message.clone(),
                    },
                );
                self.status = format!("Download failed: {message}");
                None
            }
        }
    }

    pub fn take_paths_to_open(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.paths_to_open)
    }

    pub fn note_open_failed(&mut self, path: &str, err: &str) {
        self.status = format!("Prepared product at {path}; open failed: {err}.");
    }

    fn search_request(&self) -> CdseSearchRequest {
        CdseSearchRequest {
            satellite: self.satellite,
            product: self.product.clone(),
            start_date: self.start_date.clone(),
            end_date: self.end_date.clone(),
            tile: self.tile.clone(),
            cloud_cover: self.cloud_cover,
            point: self.point.clone(),
            bbox: self.bbox.clone(),
            geojson: self.geojson.clone(),
            max_results: self.max_results,
        }
    }

    fn download_request_for(&self, product: &CdseProductRow) -> Option<CdseDownloadRequest> {
        let dest = PathBuf::from(self.download_dir.trim());
        if self.download_dir.trim().is_empty() {
            return None;
        }
        Some(CdseDownloadRequest {
            product_id: product.id.clone(),
            product_name: product.name.clone(),
            dest_dir: dest,
            username: self.username.clone(),
            password: self.password.clone(),
        })
    }

    fn sync_product_with_satellite(&mut self) {
        if self.satellite != self.previous_satellite {
            self.previous_satellite = self.satellite;
            self.product = self.satellite.known_products()[0].to_string();
        } else if !self.satellite.is_valid_product(&self.product) {
            self.product = self.satellite.known_products()[0].to_string();
        }
    }

    fn queue_download(&mut self, product: &CdseProductRow) {
        if matches!(
            self.downloads.get(&product.id),
            Some(CdseDownloadUiStatus::Downloading { .. } | CdseDownloadUiStatus::Extracting)
        ) {
            return;
        }
        let Some(request) = self.download_request_for(product) else {
            self.status = "Choose a download folder first.".to_string();
            return;
        };
        self.downloads.insert(
            product.id.clone(),
            CdseDownloadUiStatus::Downloading {
                label: product.name.clone(),
                downloaded: 0,
                total: None,
            },
        );
        self.pending_downloads.push(request);
        self.status = format!("Downloading {}…", product.name);
    }

    /// Render the CDSE download window.
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }

        let viewport = ctx.input(|input| {
            input
                .viewport()
                .inner_rect
                .unwrap_or_else(|| input.content_rect())
        });
        let max_size = egui::vec2(
            (viewport.width() - 48.0).clamp(480.0, 900.0),
            (viewport.height() - 48.0).clamp(400.0, viewport.height()),
        );
        let default_size = egui::vec2(720.0_f32.min(max_size.x), 640.0_f32.min(max_size.y));

        let mut keep_open = self.open;
        egui::Window::new("CDSE download")
            .collapsible(true)
            .resizable(true)
            .default_size(default_size)
            .min_width(480.0)
            .max_size(max_size)
            .open(&mut keep_open)
            .show(ctx, |ui| {
                // Cap content width so widgets that fill the row cannot grow the window.
                ui.set_max_width(ui.available_width().min(max_size.x - 24.0));
                self.form_ui(ui);
                ui.add_space(8.0);
                ui.separator();
                self.results_ui(ui);
                if !self.status.is_empty() {
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(egui::RichText::new(&self.status).weak())
                            .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                }
            });
        self.open = keep_open;

        if self.downloads.values().any(|s| {
            matches!(
                s,
                CdseDownloadUiStatus::Downloading { .. } | CdseDownloadUiStatus::Extracting
            )
        }) {
            ctx.request_repaint();
        }
    }

    fn form_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Search CDSE catalogue");
        ui.add_space(4.0);

        let field_width = (ui.available_width() - 160.0).clamp(200.0, 520.0);

        egui::Grid::new("cdse_search_grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Satellite");
                egui::ComboBox::from_id_salt("cdse_satellite")
                    .selected_text(self.satellite.to_string())
                    .width(field_width)
                    .show_ui(ui, |ui| {
                        for sat in SATELLITES {
                            ui.selectable_value(&mut self.satellite, sat, sat.to_string());
                        }
                    });
                ui.end_row();

                self.sync_product_with_satellite();

                ui.label("Product type");
                egui::ComboBox::from_id_salt("cdse_product")
                    .selected_text(if self.product.is_empty() {
                        "Select…".to_string()
                    } else {
                        self.product.clone()
                    })
                    .width(field_width)
                    .show_ui(ui, |ui| {
                        for product in self.satellite.known_products() {
                            ui.selectable_value(&mut self.product, product.to_string(), *product);
                        }
                    });
                ui.end_row();

                ui.label("Start date");
                ui.add(egui::TextEdit::singleline(&mut self.start_date).desired_width(field_width));
                ui.end_row();

                ui.label("End date");
                ui.add(egui::TextEdit::singleline(&mut self.end_date).desired_width(field_width));
                ui.end_row();

                ui.label("Tile");
                ui.add(egui::TextEdit::singleline(&mut self.tile).desired_width(field_width));
                ui.end_row();

                ui.label("Max cloud cover (%)");
                ui.add(
                    egui::DragValue::new(&mut self.cloud_cover)
                        .speed(0.5)
                        .range(0.0..=100.0),
                );
                ui.end_row();

                ui.label("Point (lat,lon)");
                ui.add(egui::TextEdit::singleline(&mut self.point).desired_width(field_width));
                ui.end_row();

                ui.label("BBox (tlat,llon,blat,rlon)");
                ui.add(egui::TextEdit::singleline(&mut self.bbox).desired_width(field_width));
                ui.end_row();

                ui.label("GeoJSON file");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.geojson)
                            .desired_width((field_width - 72.0).max(120.0)),
                    );
                    if ui.button("Browse…").clicked() {
                        self.pending_geojson_pick = true;
                    }
                });
                ui.end_row();

                ui.label("Max results");
                ui.add(
                    egui::DragValue::new(&mut self.max_results)
                        .speed(1.0)
                        .range(1..=100),
                );
                ui.end_row();

                ui.label("Download folder");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.download_dir)
                            .desired_width((field_width - 72.0).max(120.0)),
                    );
                    if ui.button("Browse…").clicked() {
                        self.pending_dest_folder_pick = true;
                    }
                });
                ui.end_row();
            });

        ui.add_space(8.0);
        egui::CollapsingHeader::new("CDSE credentials (optional)")
            .default_open(false)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Leave empty to use COPERNICUS_USER / COPERNICUS_PASS from the environment.",
                        )
                        .weak()
                        .small(),
                    )
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
                egui::Grid::new("cdse_auth_grid")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Username");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.username)
                                .desired_width(field_width),
                        );
                        ui.end_row();
                        ui.label("Password");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.password)
                                .password(true)
                                .desired_width(field_width),
                        );
                        ui.end_row();
                    });
            });

        ui.add_space(8.0);
        ui.add_enabled_ui(!self.searching, |ui| {
            if ui.button("Search").clicked() {
                self.products.clear();
                self.selected.clear();
                self.pending_search = true;
            }
        });
        if self.searching {
            ui.label(egui::RichText::new("Searching…").weak());
        }
    }

    fn results_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Results");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let any_selected = self.selected.values().any(|v| *v);
                ui.add_enabled_ui(any_selected && !self.download_dir.trim().is_empty(), |ui| {
                    if ui.button("Download selected").clicked() {
                        let selected: Vec<CdseProductRow> = self
                            .products
                            .iter()
                            .filter(|p| self.selected.get(&p.id).copied().unwrap_or(false))
                            .cloned()
                            .collect();
                        for product in &selected {
                            self.queue_download(product);
                        }
                    }
                });
            });
        });

        if self.products.is_empty() {
            ui.label(egui::RichText::new("No products yet. Run a search.").weak());
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(280.0)
            .show(ui, |ui| {
                let products = self.products.clone();
                for product in &products {
                    ui.group(|ui| {
                        ui.set_max_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let selected = self.selected.entry(product.id.clone()).or_insert(false);
                            ui.checkbox(selected, "");
                            ui.vertical(|ui| {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(&product.name).strong())
                                        .wrap_mode(egui::TextWrapMode::Wrap),
                                );
                                let cloud = match product.cloud_cover {
                                    Some(c) => format!("{c:.1}%"),
                                    None => "N/A".to_string(),
                                };
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(format!(
                                            "Acquired: {}  ·  Cloud: {}  ·  Online: {}",
                                            product.acquisition_date, cloud, product.online
                                        ))
                                        .small()
                                        .weak(),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Wrap),
                                );
                            });
                        });

                        if let Some(state) = self.downloads.get(&product.id).cloned() {
                            match state {
                                CdseDownloadUiStatus::Downloading {
                                    downloaded, total, ..
                                } => {
                                    let (fraction, text) = progress_display(downloaded, total);
                                    ui.add(
                                        egui::ProgressBar::new(fraction)
                                            .text(text)
                                            .animate(total.is_none_or(|t| t == 0)),
                                    );
                                }
                                CdseDownloadUiStatus::Extracting => {
                                    ui.add(
                                        egui::ProgressBar::new(1.0)
                                            .text("Extracting…")
                                            .animate(true),
                                    );
                                }
                                CdseDownloadUiStatus::Completed { path } => {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!("Saved to {path}"))
                                                .color(egui::Color32::from_rgb(80, 180, 80)),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Wrap),
                                    );
                                }
                                CdseDownloadUiStatus::Failed { message } => {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(message)
                                                .color(egui::Color32::LIGHT_RED),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Wrap),
                                    );
                                }
                            }
                        } else {
                            ui.add_enabled_ui(!self.download_dir.trim().is_empty(), |ui| {
                                if ui.button("Download").clicked() {
                                    self.queue_download(product);
                                }
                            });
                        }
                    });
                    ui.add_space(4.0);
                }
            });
    }
}

fn progress_display(downloaded: u64, total: Option<u64>) -> (f32, String) {
    if let Some(total) = total.filter(|t| *t > 0) {
        let fraction = (downloaded as f32 / total as f32).clamp(0.0, 1.0);
        let text = format!(
            "{} / {} ({:.0}%)",
            format_bytes(downloaded),
            format_bytes(total),
            fraction * 100.0
        );
        (fraction, text)
    } else {
        (0.0, format!("{} downloaded", format_bytes(downloaded)))
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn build_search_query(request: &CdseSearchRequest) -> Result<SearchQuery, String> {
    let mut query = SearchQuery::new(request.satellite);
    if !request.product.trim().is_empty() {
        query = query.product(request.product.trim());
    }

    let start = request.start_date.trim();
    let end = request.end_date.trim();
    if !start.is_empty() || !end.is_empty() {
        if start.is_empty() || end.is_empty() {
            return Err("Provide both start and end dates (YYYY-MM-DD).".to_string());
        }
        let start_dt = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d")
            .map_err(|err| format!("invalid start date: {err}"))?
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "invalid start date time".to_string())?
            .and_utc();
        let end_dt = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d")
            .map_err(|err| format!("invalid end date: {err}"))?
            .and_hms_opt(23, 59, 59)
            .ok_or_else(|| "invalid end date time".to_string())?
            .and_utc();
        query = query.dates(start_dt, end_dt);
    }

    if !request.tile.trim().is_empty() {
        query = query.tile(request.tile.trim());
    }
    if request.cloud_cover > 0.0 {
        query = query.max_cloud_cover(request.cloud_cover);
    }

    let point = request.point.trim();
    let bbox = request.bbox.trim();
    let geojson = request.geojson.trim();
    let geometry_count = [!point.is_empty(), !bbox.is_empty(), !geojson.is_empty()]
        .into_iter()
        .filter(|v| *v)
        .count();
    if geometry_count > 1 {
        return Err("Use only one of point, bounding box, or GeoJSON.".to_string());
    }
    if !point.is_empty() {
        let parts = parse_f64_list(point, 2, "point (lat,lon)")?;
        query = query.geometry(Geometry::Point(Point::new(parts[0], parts[1])));
    } else if !bbox.is_empty() {
        let parts = parse_f64_list(bbox, 4, "bbox (tlat,llon,blat,rlon)")?;
        query = query.geometry(Geometry::BoundingBox(BoundingBox::new(
            (parts[0], parts[1]),
            (parts[2], parts[3]),
        )));
    } else if !geojson.is_empty() {
        let geometry = Geometry::from_geojson_file(std::path::Path::new(geojson))
            .map_err(|err| err.to_string())?;
        query = query.geometry(geometry);
    }

    Ok(query.max_results(request.max_results.max(1)))
}

fn parse_f64_list(input: &str, expected: usize, label: &str) -> Result<Vec<f64>, String> {
    let parts: Result<Vec<f64>, _> = input.split(',').map(|s| s.trim().parse::<f64>()).collect();
    let parts = parts.map_err(|err| format!("invalid {label}: {err}"))?;
    if parts.len() != expected {
        return Err(format!(
            "invalid {label}: expected {expected} numbers, got {}",
            parts.len()
        ));
    }
    Ok(parts)
}
