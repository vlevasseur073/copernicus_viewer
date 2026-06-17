use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use egui_plot::{Line, Plot, PlotPoints};

use super::{PlotData, PlotRequest};

pub struct PlotPanel {
    array_path: Option<String>,
    slice_indices: Vec<usize>,
    pending_request: Option<PlotRequest>,
    plot_data: Option<PlotData>,
    texture: Option<TextureHandle>,
    texture_key: Option<String>,
    status: String,
}

impl Default for PlotPanel {
    fn default() -> Self {
        Self {
            array_path: None,
            slice_indices: Vec::new(),
            pending_request: None,
            plot_data: None,
            texture: None,
            texture_key: None,
            status: "Select a 2D variable in the hierarchy to display a heatmap.".to_string(),
        }
    }
}

impl PlotPanel {
    pub fn slice_indices(&self) -> &[usize] {
        &self.slice_indices
    }

    pub fn select_array(&mut self, path: &str, shape: &[u64]) {
        self.array_path = Some(path.to_string());
        self.texture = None;
        self.texture_key = None;
        self.plot_data = None;

        let extra_dims = shape.len().saturating_sub(2);
        self.slice_indices = vec![0; extra_dims];

        if shape.is_empty() {
            self.status = "Scalar variable — no plot available".to_string();
            self.pending_request = None;
            return;
        }

        if shape.len() == 1 {
            self.status = "Loading 1D plot…".to_string();
        } else {
            self.status = "Loading 2D heatmap…".to_string();
        }

        self.pending_request = Some(PlotRequest {
            array_path: path.to_string(),
            slice_indices: self.slice_indices.clone(),
        });
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn take_pending_request(&mut self) -> Option<PlotRequest> {
        self.pending_request.take()
    }

    pub fn set_plot_data(&mut self, data: PlotData) {
        self.texture = None;
        self.texture_key = None;
        self.plot_data = Some(data);
        self.status.clear();
    }

    pub fn set_error(&mut self, message: String) {
        self.plot_data = None;
        self.texture = None;
        self.texture_key = None;
        self.status = message;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Plot");
        ui.separator();

        if let Some(changed) = self.slice_controls(ui) {
            if changed {
                if let Some(path) = self.array_path.clone() {
                    self.pending_request = Some(PlotRequest {
                        array_path: path,
                        slice_indices: self.slice_indices.clone(),
                    });
                    self.status = "Loading…".to_string();
                    self.texture = None;
                    self.texture_key = None;
                }
            }
        }

        if !self.status.is_empty() {
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
                PlotData::Line { y, label } => self.line_plot(ui, plot_height, &label, &y),
                PlotData::Heatmap {
                    width,
                    height,
                    values,
                    vmin,
                    vmax,
                    label,
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
                ),
                PlotData::Message(msg) => {
                    ui.label(msg);
                }
            }
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

    fn line_plot(&self, ui: &mut egui::Ui, height: f32, label: &str, y: &[f64]) {
        ui.label(label);
        if y.is_empty() {
            ui.label("Empty array — nothing to display.");
            return;
        }

        let points: PlotPoints = (0..y.len()).map(|i| [i as f64, y[i]]).collect();

        Plot::new("line_plot")
            .height(height.clamp(120.0, 16_384.0))
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
    ) {
        ui.label(label);
        ui.horizontal(|ui| {
            ui.label(format!("min: {vmin:.4}"));
            ui.separator();
            ui.label(format!("max: {vmax:.4}"));
            ui.separator();
            ui.label(format!("{width} × {height_px} px"));
        });

        let texture_key = format!("{label}_{width}x{height_px}_{vmin}_{vmax}");
        if self.texture.is_none() || self.texture_key.as_deref() != Some(&texture_key) {
            let image = color_image_from_values(width, height_px, values, vmin, vmax);
            let texture = ctx.load_texture(
                format!("heatmap_{texture_key}"),
                image,
                TextureOptions::LINEAR,
            );
            self.texture = Some(texture);
            self.texture_key = Some(texture_key);
        }

        if let Some(texture) = &self.texture {
            let avail_w = ui.available_width().max(1.0);
            let avail_h = height.max(120.0).max(1.0);
            let available = egui::vec2(avail_w, avail_h);

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

            ui.centered_and_justified(|ui| {
                ui.add(
                    egui::Image::from_texture((texture.id(), egui::vec2(w, h)))
                        .fit_to_exact_size(egui::vec2(w, h)),
                );
            });
        }
    }
}

fn color_image_from_values(
    width: usize,
    height: usize,
    values: &[f32],
    vmin: f32,
    vmax: f32,
) -> ColorImage {
    let mut pixels = Vec::with_capacity(width * height);
    let range = (vmax - vmin).max(f32::EPSILON);

    for row in 0..height {
        for col in 0..width {
            let v = values[row * width + col];
            let t = if v.is_finite() {
                ((v - vmin) / range).clamp(0.0, 1.0)
            } else {
                0.0
            };
            pixels.push(viridis_color(t));
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
    Color32::from_rgb(
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8,
    )
}
