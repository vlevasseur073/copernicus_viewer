use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use egui_plot::{Line, Plot, PlotPoints};
use serde_json::Map;

use super::flags::{CfFlags, FlagSelection, parse_cf_flags};
use super::georef::{GeorefInfo, axis_label, extent_description};
use super::{PlotData, PlotLoadResult, PlotRequest};

const COLOR_BAR_WIDTH_PX: f32 = 24.0;
const COLOR_BAR_GUTTER: f32 = 72.0;
const COLOR_BAR_TEXTURE_HEIGHT: usize = 256;

/// Plot panel state: slice controls, async loading, heatmap texture, and flag selector.
pub struct PlotPanel {
    array_path: Option<String>,
    slice_indices: Vec<usize>,
    flag_selection: FlagSelection,
    flags: Option<CfFlags>,
    pending_request: Option<PlotRequest>,
    plot_data: Option<PlotData>,
    texture: Option<TextureHandle>,
    texture_key: Option<String>,
    color_bar_texture: Option<TextureHandle>,
    status: String,
    load_progress: Option<(f32, String)>,
}

impl Default for PlotPanel {
    fn default() -> Self {
        Self {
            array_path: None,
            slice_indices: Vec::new(),
            flag_selection: FlagSelection::Raw,
            flags: None,
            pending_request: None,
            plot_data: None,
            texture: None,
            texture_key: None,
            color_bar_texture: None,
            status: "Select a 2D variable in the hierarchy to display a heatmap.".to_string(),
            load_progress: None,
        }
    }
}

impl PlotPanel {
    /// Current fixed indices for dimensions above the plotted 2D slice.
    pub fn slice_indices(&self) -> &[usize] {
        &self.slice_indices
    }

    /// Currently selected CF flag view (`Raw` or a flag index).
    pub fn flag_selection(&self) -> FlagSelection {
        self.flag_selection
    }

    /// Select an array for plotting and queue an initial load request.
    pub fn select_array(
        &mut self,
        path: &str,
        shape: &[u64],
        attributes: &Map<String, serde_json::Value>,
    ) {
        self.array_path = Some(path.to_string());
        self.texture = None;
        self.texture_key = None;
        self.color_bar_texture = None;
        self.plot_data = None;
        self.load_progress = Some((0.0, "Starting…".to_string()));
        self.flags = parse_cf_flags(attributes);
        self.flag_selection = default_flag_selection(self.flags.as_ref());

        let extra_dims = shape.len().saturating_sub(2);
        self.slice_indices = vec![0; extra_dims];

        if shape.is_empty() {
            self.status = "Scalar variable — no plot available".to_string();
            self.pending_request = None;
            self.load_progress = None;
            return;
        }

        if shape.len() == 1 {
            self.status = "Loading 1D plot…".to_string();
        } else if self.flags.is_some() {
            self.status = "Loading flag plot…".to_string();
        } else {
            self.status = "Loading 2D heatmap…".to_string();
        }

        self.pending_request = Some(self.build_request(path));
    }

    /// Reset the panel to its default empty state.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Take the pending load request (called by the app background loader).
    pub fn take_pending_request(&mut self) -> Option<PlotRequest> {
        self.pending_request.take()
    }

    /// Update the progress bar during async loading.
    pub fn set_load_progress(&mut self, fraction: f32, message: &str) {
        self.load_progress = Some((fraction.clamp(0.0, 1.0), message.to_string()));
    }

    /// Apply a completed load result (plot data and optional flags).
    pub fn set_load_result(&mut self, result: PlotLoadResult) {
        if result.flags.is_some() {
            self.flags = result.flags;
        }
        self.set_plot_data(result.plot);
    }

    /// Set plot data directly (clears loading state).
    pub fn set_plot_data(&mut self, data: PlotData) {
        self.texture = None;
        self.texture_key = None;
        self.color_bar_texture = None;
        self.plot_data = Some(data);
        self.status.clear();
        self.load_progress = None;
    }

    /// Show a load error and clear any partial plot.
    pub fn set_error(&mut self, message: String) {
        self.plot_data = None;
        self.texture = None;
        self.texture_key = None;
        self.color_bar_texture = None;
        self.status = message;
        self.load_progress = None;
    }

    /// Returns `true` when plot data is ready and loading has finished.
    pub fn is_plot_ready(&self) -> bool {
        self.plot_data.is_some() && self.load_progress.is_none()
    }

    /// Render the plot panel (controls, progress bar, line plot or heatmap).
    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Plot");
        ui.separator();

        let mut request_reload = false;

        if let Some(changed) = self.slice_controls(ui)
            && changed
        {
            request_reload = true;
        }

        if let Some(changed) = self.flag_controls(ui)
            && changed
        {
            request_reload = true;
        }

        if request_reload && let Some(path) = self.array_path.clone() {
            self.pending_request = Some(self.build_request(&path));
            self.status = "Loading…".to_string();
            self.texture = None;
            self.texture_key = None;
            self.color_bar_texture = None;
            self.load_progress = Some((0.0, "Starting…".to_string()));
        }

        if let Some((fraction, message)) = &self.load_progress {
            ui.add(
                egui::ProgressBar::new(*fraction)
                    .text(message)
                    .animate(true),
            );
            ui.separator();
        } else if !self.status.is_empty() {
            ui.label(egui::RichText::new(&self.status).weak());
            ui.separator();
        }

        let plot_height = ui.available_height();
        if plot_height < 1.0 {
            ui.label("Resize the window to display the plot.");
            return;
        }
        if let Some(data) = self.plot_data.clone() {
            match data {
                PlotData::Line { y, label, georef } => {
                    self.line_plot(ui, plot_height, &label, &y, georef.as_ref())
                }
                PlotData::Heatmap {
                    width,
                    height,
                    values,
                    vmin,
                    vmax,
                    label,
                    georef,
                    y_range,
                    x_range,
                    binary,
                } => self.heatmap_plot(
                    ui,
                    ctx,
                    plot_height,
                    &label,
                    width,
                    height,
                    &values,
                    vmin,
                    vmax,
                    georef.as_ref(),
                    &y_range,
                    &x_range,
                    binary,
                ),
                PlotData::Message(msg) => {
                    ui.label(msg);
                }
            }
        }
    }

    fn build_request(&self, path: &str) -> PlotRequest {
        PlotRequest {
            array_path: path.to_string(),
            slice_indices: self.slice_indices.clone(),
            flag_selection: self.flag_selection,
        }
    }

    fn slice_controls(&mut self, ui: &mut egui::Ui) -> Option<bool> {
        if self.slice_indices.is_empty() {
            return None;
        }

        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Extra-dim slices:");
            let slice_len = self.slice_indices.len();
            for (i, idx) in self.slice_indices.iter_mut().enumerate() {
                if ui.add(egui::DragValue::new(idx).speed(1)).changed() {
                    changed = true;
                }
                if i + 1 < slice_len {
                    ui.label(",");
                }
            }
        });

        Some(changed)
    }

    fn flag_controls(&mut self, ui: &mut egui::Ui) -> Option<bool> {
        let flags = self.flags.clone()?;
        let mut changed = false;

        ui.horizontal_wrapped(|ui| {
            ui.label("Flag view:");
            let selected = match self.flag_selection {
                FlagSelection::Raw => "Raw values".to_string(),
                FlagSelection::Flag(index) => flags.flag_label(index),
            };

            egui::ComboBox::from_id_salt("flag_view")
                .selected_text(selected)
                .width(280.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(
                            &mut self.flag_selection,
                            FlagSelection::Raw,
                            "Raw values",
                        )
                        .clicked()
                    {
                        changed = true;
                    }
                    for index in 0..flags.meanings.len() {
                        let label = flags.flag_label(index);
                        if ui
                            .selectable_value(
                                &mut self.flag_selection,
                                FlagSelection::Flag(index),
                                label,
                            )
                            .clicked()
                        {
                            changed = true;
                        }
                    }
                });
        });

        if flags.uses_bitmasks() {
            ui.label(
                egui::RichText::new(
                    "Bitmask flags are non-exclusive — select a bit to plot where it is set.",
                )
                .small()
                .weak(),
            );
        }

        Some(changed)
    }

    fn line_plot(
        &self,
        ui: &mut egui::Ui,
        height: f32,
        label: &str,
        y: &[f64],
        georef: Option<&GeorefInfo>,
    ) {
        ui.label(label);
        if let Some(info) = georef {
            let extent = extent_description(info);
            if !extent.is_empty() {
                ui.label(egui::RichText::new(extent).small().weak());
            }
        }
        if y.is_empty() {
            ui.label("Empty array — nothing to display.");
            return;
        }

        let points: PlotPoints = (0..y.len())
            .map(|i| {
                let x = georef
                    .and_then(|g| g.x_coords.as_ref())
                    .and_then(|c| c.get(i))
                    .copied()
                    .unwrap_or(i as f64);
                [x, y[i]]
            })
            .collect();

        let x_label = georef.map(|g| g.x_name.as_str()).unwrap_or("index");
        let y_label = label;

        Plot::new("line_plot")
            .height(height.clamp(120.0, 16_384.0))
            .x_axis_label(x_label)
            .y_axis_label(y_label)
            .allow_zoom(true)
            .allow_drag(true)
            .show(ui, |plot_ui| {
                plot_ui.line(Line::new(label.to_string(), points));
            });
    }

    fn heatmap_plot(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        height: f32,
        label: &str,
        width: usize,
        height_px: usize,
        values: &[f32],
        vmin: f32,
        vmax: f32,
        georef: Option<&GeorefInfo>,
        y_range: &std::ops::Range<u64>,
        x_range: &std::ops::Range<u64>,
        binary: bool,
    ) {
        ui.label(label);
        ui.label(format!("{width} × {height_px} px"));

        if let Some(info) = georef {
            let extent = extent_description(info);
            if !extent.is_empty() {
                ui.label(egui::RichText::new(extent).small().weak());
            }
            ui.horizontal(|ui| {
                let y0 = y_range.start as usize;
                let y1 = y_range.end.saturating_sub(1) as usize;
                let x0 = x_range.start as usize;
                let x1 = x_range.end.saturating_sub(1) as usize;
                ui.label(format!(
                    "Y ({}) {} … {}",
                    info.y_name,
                    axis_label(info, 'y', 0, y0 as f64),
                    axis_label(info, 'y', height_px.saturating_sub(1), y1 as f64),
                ));
                ui.separator();
                ui.label(format!(
                    "X ({}) {} … {}",
                    info.x_name,
                    axis_label(info, 'x', 0, x0 as f64),
                    axis_label(info, 'x', width.saturating_sub(1), x1 as f64),
                ));
            });
        }

        let texture_key = format!("{label}_{width}x{height_px}_{vmin}_{vmax}_{binary}");
        if self.texture.is_none() || self.texture_key.as_deref() != Some(&texture_key) {
            let image = color_image_from_values(width, height_px, values, vmin, vmax, binary);
            let texture = ctx.load_texture(
                format!("heatmap_{texture_key}"),
                image,
                TextureOptions::LINEAR,
            );
            let color_bar = ctx.load_texture(
                format!("color_bar_{texture_key}"),
                color_bar_image(binary),
                TextureOptions::LINEAR,
            );
            self.texture = Some(texture);
            self.color_bar_texture = Some(color_bar);
            self.texture_key = Some(texture_key);
        }

        if let (Some(texture), Some(color_bar_texture)) = (&self.texture, &self.color_bar_texture) {
            let avail_w = ui.available_width().max(1.0);
            let avail_h = height.max(120.0).max(1.0);
            let plot_avail_w = (avail_w - COLOR_BAR_GUTTER).max(1.0);
            let available = egui::vec2(plot_avail_w, avail_h);

            if width == 0 || height_px == 0 {
                ui.label("Empty array — nothing to display.");
                return;
            }

            let aspect = (width as f32 / height_px as f32).max(f32::EPSILON);
            let (w, h) = if available.x / available.y > aspect {
                (available.y * aspect, available.y)
            } else {
                (available.x, available.x / aspect)
            };
            let w = w.clamp(1.0, 16_384.0);
            let h = h.clamp(1.0, 16_384.0);

            let (top_label, bottom_label) = color_bar_labels(vmin, vmax, binary);
            let bar_size = egui::vec2(COLOR_BAR_WIDTH_PX, h);

            ui.centered_and_justified(|ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::from_texture((texture.id(), egui::vec2(w, h)))
                            .fit_to_exact_size(egui::vec2(w, h)),
                    );
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(top_label).small().weak());
                        ui.add(
                            egui::Image::from_texture((color_bar_texture.id(), bar_size))
                                .fit_to_exact_size(bar_size),
                        );
                        ui.label(RichText::new(bottom_label).small().weak());
                    });
                });
            });
        }
    }
}

fn default_flag_selection(flags: Option<&CfFlags>) -> FlagSelection {
    if flags.is_some_and(|flags| flags.uses_bitmasks()) {
        FlagSelection::Flag(0)
    } else {
        FlagSelection::Raw
    }
}

fn color_bar_labels(vmin: f32, vmax: f32, binary: bool) -> (String, String) {
    if binary {
        ("1".to_string(), "0".to_string())
    } else {
        (format!("{vmax:.4}"), format!("{vmin:.4}"))
    }
}

fn binary_off_color() -> Color32 {
    Color32::from_gray(30)
}

fn binary_on_color() -> Color32 {
    Color32::from_rgb(255, 204, 0)
}

fn nan_color() -> Color32 {
    Color32::from_gray(40)
}

fn color_bar_image(binary: bool) -> ColorImage {
    let width = COLOR_BAR_WIDTH_PX as usize;
    let height = COLOR_BAR_TEXTURE_HEIGHT;
    let mut pixels = Vec::with_capacity(width * height);

    for row in 0..height {
        let color = if binary {
            if row < height / 2 {
                binary_on_color()
            } else {
                binary_off_color()
            }
        } else {
            let t = if height <= 1 {
                1.0
            } else {
                1.0 - row as f32 / (height - 1) as f32
            };
            viridis_color(t)
        };
        pixels.extend(std::iter::repeat_n(color, width));
    }

    ColorImage {
        size: [width, height],
        pixels,
        ..Default::default()
    }
}

fn color_image_from_values(
    width: usize,
    height: usize,
    values: &[f32],
    vmin: f32,
    vmax: f32,
    binary: bool,
) -> ColorImage {
    let mut pixels = Vec::with_capacity(width * height);
    let range = (vmax - vmin).max(f32::EPSILON);

    for row in 0..height {
        for col in 0..width {
            let v = values[row * width + col];
            let color = if !v.is_finite() {
                nan_color()
            } else if binary {
                if v >= 0.5 {
                    binary_on_color()
                } else {
                    binary_off_color()
                }
            } else {
                let t = ((v - vmin) / range).clamp(0.0, 1.0);
                viridis_color(t)
            };
            pixels.push(color);
        }
    }

    ColorImage {
        size: [width, height],
        pixels,
        ..Default::default()
    }
}

fn viridis_color(t: f32) -> Color32 {
    let r = (0.267 + t * (0.993 - 0.267)).clamp(0.0, 1.0);
    let g = (0.004 + t * (0.906 - 0.004)).clamp(0.0, 1.0);
    let b = (0.329 + t * (0.143 - 0.329)).clamp(0.0, 1.0);
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}
