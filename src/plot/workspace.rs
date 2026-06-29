use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use egui_plot::{Line, Plot, PlotPoints};
use serde_json::Map;

use super::flags::{CfFlags, FlagSelection, parse_cf_flags};
use super::georef::{GeorefInfo, axis_label, extent_description};
use super::{PlotData, PlotLoadResult, PlotRequest};

const COLOR_BAR_WIDTH_PX: f32 = 24.0;
const COLOR_BAR_GUTTER: f32 = 72.0;
const COLOR_BAR_TEXTURE_HEIGHT: usize = 256;
/// Maximum open plot tabs; each retains a heatmap texture and value buffer.
const MAX_PLOT_SLOTS: usize = 8;
const MIN_SPLIT_PANE_WIDTH: f32 = 200.0;
const MIN_SPLIT_PANE_HEIGHT: f32 = 150.0;
const MAX_SAMPLE_STRIDE: u32 = 64;

/// Stable identifier for a plot tab (survives slot reordering on close).
pub type PlotSlotId = u64;

/// Dedup key: one tab per product + array path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlotSlotKey {
    pub store_index: usize,
    pub array_path: String,
}

/// Layout for displaying multiple plots at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlotLayout {
    #[default]
    Tabs,
    Horizontal,
    Vertical,
    Grid,
}

impl PlotLayout {
    fn label(self) -> &'static str {
        match self {
            Self::Tabs => "Tabs",
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::Grid => "Grid",
        }
    }

    fn capacity(self) -> usize {
        match self {
            Self::Tabs => 1,
            Self::Horizontal | Self::Vertical => 2,
            Self::Grid => 4,
        }
    }
}

struct HeatmapMeta {
    y_range: std::ops::Range<u64>,
    x_range: std::ops::Range<u64>,
    full_shape: Vec<u64>,
    resolution_percent: u8,
    sample_stride: u32,
}

/// State for a single plot tab.
pub struct PlotSlot {
    id: PlotSlotId,
    key: PlotSlotKey,
    product_label: String,
    full_shape: Vec<u64>,
    slice_indices: Vec<usize>,
    flag_selection: FlagSelection,
    resolution_percent: u8,
    sample_stride: u32,
    flags: Option<CfFlags>,
    pending_request: Option<PlotRequest>,
    plot_data: Option<PlotData>,
    texture: Option<TextureHandle>,
    texture_key: Option<String>,
    color_bar_texture: Option<TextureHandle>,
    status: String,
    load_progress: Option<(f32, String)>,
    resolution_capped: bool,
    visible_in_split: bool,
}

impl PlotSlot {
    fn new(
        id: PlotSlotId,
        key: PlotSlotKey,
        product_label: String,
        path: &str,
        shape: &[u64],
        attributes: &Map<String, serde_json::Value>,
        zarr_fill_value: Option<&serde_json::Value>,
    ) -> Self {
        let extra_dims = shape.len().saturating_sub(2);
        let flags = parse_cf_flags(attributes, zarr_fill_value);
        let flag_selection = default_flag_selection(flags.as_ref());
        let mut slot = Self {
            id,
            key,
            product_label,
            full_shape: shape.to_vec(),
            slice_indices: vec![0; extra_dims],
            flag_selection,
            resolution_percent: 100,
            sample_stride: 1,
            flags,
            pending_request: None,
            plot_data: None,
            texture: None,
            texture_key: None,
            color_bar_texture: None,
            status: String::new(),
            load_progress: None,
            resolution_capped: false,
            visible_in_split: true,
        };

        if shape.is_empty() {
            slot.status = "Scalar variable — no plot available".to_string();
        } else if shape.len() == 1 {
            slot.status = "Loading 1D plot…".to_string();
            slot.queue_reload(path);
        } else if slot.flags.is_some() {
            slot.status = "Loading flag plot…".to_string();
            slot.queue_reload(path);
        } else {
            slot.status = "Loading 2D heatmap…".to_string();
            slot.queue_reload(path);
        }

        slot
    }

    fn tab_label(&self) -> String {
        let var = self
            .key
            .array_path
            .trim_start_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("plot");
        format!("{} / {var}", self.product_label)
    }

    fn tab_hover(&self) -> String {
        format!("{} — {}", self.product_label, self.key.array_path)
    }

    fn is_loading(&self) -> bool {
        self.load_progress.is_some()
    }

    fn has_error(&self) -> bool {
        !self.status.is_empty() && self.plot_data.is_none() && !self.is_loading()
    }

    fn build_request(&self, path: &str) -> PlotRequest {
        PlotRequest {
            array_path: path.to_string(),
            slice_indices: self.slice_indices.clone(),
            flag_selection: self.flag_selection,
            resolution_percent: self.resolution_percent,
            sample_stride: self.sample_stride,
        }
    }

    fn queue_reload(&mut self, path: &str) {
        self.pending_request = Some(self.build_request(path));
        self.texture = None;
        self.texture_key = None;
        self.color_bar_texture = None;
        self.plot_data = None;
        self.load_progress = Some((0.0, "Starting…".to_string()));
        if !self.status.starts_with("Scalar") {
            self.status = "Loading…".to_string();
        }
    }

    fn take_pending_request(&mut self) -> Option<PlotRequest> {
        self.pending_request.take()
    }

    fn set_load_progress(&mut self, fraction: f32, message: &str) {
        self.load_progress = Some((fraction.clamp(0.0, 1.0), message.to_string()));
    }

    fn set_load_result(&mut self, result: PlotLoadResult) {
        if result.flags.is_some() {
            self.flags = result.flags;
        }
        self.resolution_capped = result.resolution_capped;
        self.set_plot_data(result.plot);
    }

    fn set_plot_data(&mut self, data: PlotData) {
        self.texture = None;
        self.texture_key = None;
        self.color_bar_texture = None;
        self.plot_data = Some(data);
        self.status.clear();
        self.load_progress = None;
    }

    fn set_error(&mut self, message: String) {
        self.plot_data = None;
        self.texture = None;
        self.texture_key = None;
        self.color_bar_texture = None;
        self.status = message;
        self.load_progress = None;
    }

    fn is_plot_ready(&self) -> bool {
        self.plot_data.is_some() && self.load_progress.is_none()
    }

    fn plottable(&self) -> bool {
        !self.full_shape.is_empty()
    }

    fn controls_ui(&mut self, ui: &mut egui::Ui, ui_id: egui::Id) -> bool {
        let mut request_reload = false;

        if let Some(changed) = self.slice_controls(ui)
            && changed
        {
            request_reload = true;
        }
        if let Some(changed) = self.flag_controls(ui, ui_id)
            && changed
        {
            request_reload = true;
        }
        if let Some(changed) = self.resolution_controls(ui)
            && changed
        {
            request_reload = true;
        }
        if let Some(changed) = self.stride_controls(ui)
            && changed
        {
            request_reload = true;
        }

        if request_reload {
            let path = self.key.array_path.clone();
            self.queue_reload(&path);
        }

        request_reload
    }

    fn status_ui(&self, ui: &mut egui::Ui) {
        if let Some((fraction, message)) = &self.load_progress {
            ui.add(
                egui::ProgressBar::new(*fraction)
                    .text(message)
                    .animate(true),
            );
            ui.separator();
        } else if self.resolution_capped {
            ui.label(
                RichText::new("Resolution capped by memory limit — reduce resolution or stride.")
                    .small()
                    .weak(),
            );
            ui.separator();
        } else if !self.status.is_empty() {
            ui.label(RichText::new(&self.status).weak());
            ui.separator();
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

    fn flag_controls(&mut self, ui: &mut egui::Ui, ui_id: egui::Id) -> Option<bool> {
        let flags = self.flags.clone()?;
        let mut changed = false;

        ui.horizontal_wrapped(|ui| {
            ui.label("Flag view:");
            let selected = match self.flag_selection {
                FlagSelection::Raw => "Raw values".to_string(),
                FlagSelection::Flag(index) => flags.flag_label(index),
            };

            egui::ComboBox::from_id_salt(ui_id.with("flag_view"))
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
                RichText::new(
                    "Bitmask flags are non-exclusive — select a bit to plot where it is set.",
                )
                .small()
                .weak(),
            );
        }

        Some(changed)
    }

    fn resolution_controls(&mut self, ui: &mut egui::Ui) -> Option<bool> {
        if !self.plottable() {
            return None;
        }

        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Resolution:");
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.resolution_percent, 1..=100)
                        .suffix("%")
                        .logarithmic(false),
                )
                .changed();
        });

        Some(changed)
    }

    fn stride_controls(&mut self, ui: &mut egui::Ui) -> Option<bool> {
        if !self.plottable() {
            return None;
        }

        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Stride:");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut self.sample_stride)
                        .range(1..=MAX_SAMPLE_STRIDE)
                        .speed(0.1),
                )
                .changed();
            ui.label(RichText::new("1 pixel every N").small().weak());
        });

        Some(changed)
    }

    fn plot_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, height: f32, compact: bool) {
        if height < 1.0 {
            ui.label("Resize the window to display the plot.");
            return;
        }

        let Some(data) = self.plot_data.clone() else {
            return;
        };

        match data {
            PlotData::Line { y, label, georef } => {
                self.line_plot(ui, height, &label, &y, georef.as_ref(), compact);
            }
            PlotData::Heatmap {
                width,
                height: height_px,
                values,
                vmin,
                vmax,
                label,
                georef,
                y_range,
                x_range,
                full_shape,
                resolution_percent,
                sample_stride,
                binary,
            } => {
                let meta = HeatmapMeta {
                    y_range,
                    x_range,
                    full_shape,
                    resolution_percent,
                    sample_stride,
                };
                self.heatmap_plot(
                    ui,
                    ctx,
                    height,
                    &label,
                    width,
                    height_px,
                    &values,
                    vmin,
                    vmax,
                    georef.as_ref(),
                    &meta,
                    binary,
                    compact,
                );
            }
            PlotData::Message(msg) => {
                ui.label(msg);
            }
        }
    }

    fn line_plot(
        &self,
        ui: &mut egui::Ui,
        height: f32,
        label: &str,
        y: &[f64],
        georef: Option<&GeorefInfo>,
        compact: bool,
    ) {
        if !compact {
            ui.label(label);
            if let Some(info) = georef {
                let extent = extent_description(info);
                if !extent.is_empty() {
                    ui.label(RichText::new(extent).small().weak());
                }
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
        let plot_id = egui::Id::new(("line_plot", self.id));

        Plot::new(plot_id)
            .height(height.clamp(120.0, 16_384.0))
            .x_axis_label(if compact { "" } else { x_label })
            .y_axis_label(if compact { "" } else { label })
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
        meta: &HeatmapMeta,
        binary: bool,
        compact: bool,
    ) {
        if !compact {
            ui.label(label);
            ui.label(heatmap_size_label(width, height_px, meta));

            if let Some(info) = georef {
                let extent = extent_description(info);
                if !extent.is_empty() {
                    ui.label(RichText::new(extent).small().weak());
                }
                ui.horizontal(|ui| {
                    let y0 = meta.y_range.start as usize;
                    let y1 = meta.y_range.end.saturating_sub(1) as usize;
                    let x0 = meta.x_range.start as usize;
                    let x1 = meta.x_range.end.saturating_sub(1) as usize;
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
        }

        let texture_key = format!(
            "slot{}_{label}_{width}x{height_px}_{vmin}_{vmax}_{binary}",
            self.id
        );
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
            let plot_avail_w = if compact {
                avail_w
            } else {
                (avail_w - COLOR_BAR_GUTTER).max(1.0)
            };
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

            if compact {
                ui.centered_and_justified(|ui| {
                    ui.add(
                        egui::Image::from_texture((texture.id(), egui::vec2(w, h)))
                            .fit_to_exact_size(egui::vec2(w, h)),
                    );
                });
            } else {
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
}

/// Multi-tab plot workspace with optional split layouts.
pub struct PlotWorkspace {
    slots: Vec<PlotSlot>,
    active_slot: Option<PlotSlotId>,
    layout: PlotLayout,
    next_id: PlotSlotId,
}

impl Default for PlotWorkspace {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            active_slot: None,
            layout: PlotLayout::Tabs,
            next_id: 1,
        }
    }
}

impl PlotWorkspace {
    /// Open or focus a plot tab for the given array. Returns `(slot_id, is_new)`.
    pub fn open_or_focus(
        &mut self,
        store_index: usize,
        path: &str,
        shape: &[u64],
        attributes: &Map<String, serde_json::Value>,
        zarr_fill_value: Option<&serde_json::Value>,
        product_label: &str,
    ) -> (PlotSlotId, bool) {
        let key = PlotSlotKey {
            store_index,
            array_path: path.to_string(),
        };

        if let Some(existing) = self.slots.iter().find(|s| s.key == key) {
            let id = existing.id;
            self.active_slot = Some(id);
            return (id, false);
        }

        self.evict_if_needed();

        let id = self.next_id;
        self.next_id += 1;
        let slot = PlotSlot::new(
            id,
            key,
            product_label.to_string(),
            path,
            shape,
            attributes,
            zarr_fill_value,
        );
        self.slots.push(slot);
        self.active_slot = Some(id);
        self.ensure_split_visibility();
        (id, true)
    }

    /// Remove all plot tabs.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Returns `true` when the active tab's plot is ready.
    pub fn is_plot_ready(&self) -> bool {
        self.active_slot().is_some_and(|slot| slot.is_plot_ready())
    }

    /// Returns `true` when the tab for `store_index` + `path` has finished loading.
    pub fn is_slot_ready(&self, store_index: usize, path: &str) -> bool {
        self.slots.iter().any(|slot| {
            slot.key.store_index == store_index
                && slot.key.array_path == path
                && slot.is_plot_ready()
        })
    }

    /// Set resolution on a tab and queue a reload.
    pub fn set_slot_resolution_percent(
        &mut self,
        store_index: usize,
        path: &str,
        percent: u8,
    ) -> bool {
        let path = path.to_string();
        let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| s.key.store_index == store_index && s.key.array_path == path)
        else {
            return false;
        };
        slot.resolution_percent = percent.clamp(1, 100);
        slot.queue_reload(&path);
        self.active_slot = Some(slot.id);
        true
    }

    /// Switch plot layout mode (tabs, horizontal, vertical, grid).
    pub fn set_layout(&mut self, layout: PlotLayout) {
        self.layout = layout;
        self.ensure_split_visibility();
    }

    /// Active tab slice indices (for building fallback requests).
    pub fn active_slice_indices(&self) -> &[usize] {
        self.active_slot()
            .map(|s| s.slice_indices.as_slice())
            .unwrap_or(&[])
    }

    /// Active tab flag selection.
    pub fn active_flag_selection(&self) -> FlagSelection {
        self.active_slot()
            .map(|s| s.flag_selection)
            .unwrap_or(FlagSelection::Raw)
    }

    /// Collect pending load requests from all slots.
    pub fn take_pending_requests(&mut self) -> Vec<(PlotSlotId, PlotRequest)> {
        let mut pending = Vec::new();
        for slot in &mut self.slots {
            if let Some(request) = slot.take_pending_request() {
                pending.push((slot.id, request));
            }
        }
        pending
    }

    pub fn set_load_progress(&mut self, slot_id: PlotSlotId, fraction: f32, message: &str) {
        if let Some(slot) = self.slot_mut(slot_id) {
            slot.set_load_progress(fraction, message);
        }
    }

    pub fn set_load_result(&mut self, slot_id: PlotSlotId, result: PlotLoadResult) {
        if let Some(slot) = self.slot_mut(slot_id) {
            slot.set_load_result(result);
        }
    }

    pub fn set_error(&mut self, slot_id: PlotSlotId, message: String) {
        if let Some(slot) = self.slot_mut(slot_id) {
            slot.set_error(message);
        }
    }

    pub fn store_index_for_slot(&self, slot_id: PlotSlotId) -> Option<usize> {
        self.slot(slot_id).map(|s| s.key.store_index)
    }

    pub fn slot_matches_selection(
        &self,
        slot_id: PlotSlotId,
        store_index: usize,
        path: &str,
    ) -> bool {
        self.slot(slot_id)
            .is_some_and(|s| s.key.store_index == store_index && s.key.array_path == path)
    }

    /// Reindex and remove slots after a product was closed.
    pub fn on_product_closed(&mut self, closed_index: usize, selection_was_closed: bool) {
        self.slots
            .retain(|slot| slot.key.store_index != closed_index);
        for slot in &mut self.slots {
            if slot.key.store_index > closed_index {
                slot.key.store_index -= 1;
            }
        }

        if selection_was_closed {
            self.active_slot = self.slots.last().map(|s| s.id);
        } else if self.active_slot.is_some_and(|id| self.slot(id).is_none()) {
            self.active_slot = self.slots.first().map(|s| s.id);
        }

        if self.slots.is_empty() {
            self.active_slot = None;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.heading("Plot");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.layout_picker(ui);
            });
        });
        ui.separator();

        if self.slots.is_empty() {
            ui.label(
                RichText::new("Select a 2D variable in the hierarchy to display a heatmap.").weak(),
            );
            return;
        }

        self.tab_bar_ui(ui);

        if let Some(active_id) = self.active_slot
            && let Some(slot) = self.slot_mut(active_id)
        {
            slot.controls_ui(ui, egui::Id::new(("plot_controls", active_id)));
            slot.status_ui(ui);
        }

        let plot_height = ui.available_height();
        if plot_height < 1.0 {
            ui.label("Resize the window to display the plot.");
            return;
        }

        match self.layout {
            PlotLayout::Tabs => {
                if let Some(active_id) = self.active_slot
                    && let Some(slot) = self.slot_mut(active_id)
                {
                    slot.plot_ui(ui, ctx, plot_height, false);
                }
            }
            _ => self.split_layout_ui(ui, ctx, plot_height),
        }
    }

    fn layout_picker(&mut self, ui: &mut egui::Ui) {
        ui.label("Layout:");
        for layout in [
            PlotLayout::Tabs,
            PlotLayout::Horizontal,
            PlotLayout::Vertical,
            PlotLayout::Grid,
        ] {
            if ui
                .selectable_label(self.layout == layout, layout.label())
                .clicked()
            {
                self.layout = layout;
                self.ensure_split_visibility();
            }
        }
    }

    fn tab_bar_ui(&mut self, ui: &mut egui::Ui) {
        let mut to_close: Option<PlotSlotId> = None;
        let mut to_activate: Option<PlotSlotId> = None;
        let mut visibility_changes: Vec<(PlotSlotId, bool)> = Vec::new();
        let mut close_all = false;

        ui.horizontal(|ui| {
            egui::ScrollArea::horizontal()
                .id_salt("plot_tab_bar")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for slot in &self.slots {
                            let id = slot.id;
                            let active = self.active_slot == Some(id);
                            let label = if slot.is_loading() {
                                format!("⏳ {}", slot.tab_label())
                            } else if slot.has_error() {
                                format!("⚠ {}", slot.tab_label())
                            } else {
                                slot.tab_label()
                            };

                            let response = ui
                                .selectable_label(active, label)
                                .on_hover_text(slot.tab_hover());

                            if response.clicked() {
                                to_activate = Some(id);
                            }

                            if self.layout != PlotLayout::Tabs {
                                let mut visible = slot.visible_in_split;
                                if ui.checkbox(&mut visible, "show").changed() {
                                    visibility_changes.push((id, visible));
                                }
                            }
                        }
                    });
                });

            if ui.small_button("Close all").clicked() {
                close_all = true;
            }
        });

        ui.horizontal(|ui| {
            if let Some(active_id) = self.active_slot {
                if ui.small_button("Close tab").clicked() {
                    to_close = Some(active_id);
                }
                if ui.small_button("Close others").clicked() {
                    self.slots.retain(|s| s.id == active_id);
                    self.ensure_split_visibility();
                }
            }
        });

        if close_all {
            self.clear();
            return;
        }

        if let Some(id) = to_activate {
            self.active_slot = Some(id);
        }
        for (id, visible) in visibility_changes {
            if let Some(slot) = self.slot_mut(id) {
                slot.visible_in_split = visible;
            }
        }
        if let Some(id) = to_close {
            self.close_slot(id);
        }

        ui.separator();
    }

    fn split_layout_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, total_height: f32) {
        let visible: Vec<PlotSlotId> = self
            .slots
            .iter()
            .filter(|s| s.visible_in_split)
            .map(|s| s.id)
            .collect();

        let capacity = self.layout.capacity();
        let mut panes: Vec<PlotSlotId> = visible;
        if panes.is_empty()
            && let Some(id) = self.active_slot
        {
            panes.push(id);
        }
        panes.truncate(capacity);

        if panes.is_empty() {
            ui.label("No plots visible — enable tabs in the tab bar.");
            return;
        }

        let min_ok = match self.layout {
            PlotLayout::Horizontal => {
                ui.available_width() >= MIN_SPLIT_PANE_WIDTH * panes.len() as f32
            }
            PlotLayout::Vertical => total_height >= MIN_SPLIT_PANE_HEIGHT * panes.len() as f32,
            PlotLayout::Grid => {
                ui.available_width() >= MIN_SPLIT_PANE_WIDTH * 2.0
                    && total_height >= MIN_SPLIT_PANE_HEIGHT * 2.0
            }
            PlotLayout::Tabs => true,
        };

        if !min_ok {
            ui.label(
                RichText::new("Not enough space for split layout — switch to Tabs or resize.")
                    .weak(),
            );
            if let Some(id) = panes.first()
                && let Some(slot) = self.slot_mut(*id)
            {
                slot.plot_ui(ui, ctx, total_height, true);
            }
            return;
        }

        match self.layout {
            PlotLayout::Horizontal => {
                ui.horizontal(|ui| {
                    let w = ui.available_width() / panes.len() as f32;
                    for id in panes {
                        ui.allocate_ui_with_layout(
                            egui::vec2(w, total_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                if let Some(slot) = self.slot_mut(id) {
                                    ui.label(RichText::new(slot.tab_label()).small());
                                    ui.separator();
                                    slot.plot_ui(ui, ctx, ui.available_height(), true);
                                }
                            },
                        );
                    }
                });
            }
            PlotLayout::Vertical => {
                let h = total_height / panes.len() as f32;
                for id in panes {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            if let Some(slot) = self.slot_mut(id) {
                                ui.label(RichText::new(slot.tab_label()).small());
                                ui.separator();
                                slot.plot_ui(ui, ctx, ui.available_height(), true);
                            }
                        },
                    );
                }
            }
            PlotLayout::Grid => {
                egui::Grid::new("plot_grid")
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        let cell_h = (total_height / 2.0).max(MIN_SPLIT_PANE_HEIGHT);
                        let cell_w = (ui.available_width() / 2.0).max(MIN_SPLIT_PANE_WIDTH);
                        for id in panes {
                            ui.allocate_ui_with_layout(
                                egui::vec2(cell_w, cell_h),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    if let Some(slot) = self.slot_mut(id) {
                                        ui.label(RichText::new(slot.tab_label()).small());
                                        ui.separator();
                                        slot.plot_ui(ui, ctx, ui.available_height(), true);
                                    }
                                },
                            );
                        }
                    });
            }
            PlotLayout::Tabs => {}
        }
    }

    fn close_slot(&mut self, slot_id: PlotSlotId) {
        self.slots.retain(|s| s.id != slot_id);
        if self.active_slot == Some(slot_id) {
            self.active_slot = self.slots.last().map(|s| s.id);
        }
        self.ensure_split_visibility();
    }

    fn evict_if_needed(&mut self) {
        while self.slots.len() >= MAX_PLOT_SLOTS {
            let evict = self
                .slots
                .iter()
                .find(|s| Some(s.id) != self.active_slot)
                .map(|s| s.id)
                .or_else(|| self.slots.first().map(|s| s.id));
            if let Some(id) = evict {
                self.close_slot(id);
            } else {
                break;
            }
        }
    }

    fn ensure_split_visibility(&mut self) {
        let capacity = self.layout.capacity();
        let mut visible_count = self.slots.iter().filter(|s| s.visible_in_split).count();
        if visible_count == 0 {
            for slot in self.slots.iter_mut().rev().take(capacity) {
                slot.visible_in_split = true;
            }
            visible_count = self.slots.iter().filter(|s| s.visible_in_split).count();
        }
        if visible_count > capacity {
            let active = self.active_slot;
            let mut kept = 0;
            for slot in self.slots.iter_mut() {
                if slot.visible_in_split {
                    if Some(slot.id) == active || kept < capacity {
                        kept += 1;
                    } else {
                        slot.visible_in_split = false;
                    }
                }
            }
        }
    }

    fn slot(&self, id: PlotSlotId) -> Option<&PlotSlot> {
        self.slots.iter().find(|s| s.id == id)
    }

    fn slot_mut(&mut self, id: PlotSlotId) -> Option<&mut PlotSlot> {
        self.slots.iter_mut().find(|s| s.id == id)
    }

    fn active_slot(&self) -> Option<&PlotSlot> {
        self.active_slot.and_then(|id| self.slot(id))
    }
}

fn heatmap_size_label(width: usize, height: usize, meta: &HeatmapMeta) -> String {
    let window_h = (meta.y_range.end - meta.y_range.start) as usize;
    let window_w = (meta.x_range.end - meta.x_range.start) as usize;
    let mut label = format!("{width} × {height} px");
    if meta.full_shape.len() >= 2 {
        let full_h = meta.full_shape[meta.full_shape.len() - 2];
        let full_w = meta.full_shape[meta.full_shape.len() - 1];
        if meta.resolution_percent < 100 || window_h < full_h as usize || window_w < full_w as usize
        {
            label.push_str(&format!(
                " ({}% window {window_h} × {window_w} of {full_h} × {full_w})",
                meta.resolution_percent
            ));
        }
    }
    if meta.sample_stride > 1 {
        label.push_str(&format!(", stride {}", meta.sample_stride));
    }
    label
}

fn default_flag_selection(flags: Option<&CfFlags>) -> FlagSelection {
    if flags.is_some_and(CfFlags::uses_bitmasks) {
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

/// Backward-compatible alias for the plot workspace.
pub type PlotPanel = PlotWorkspace;

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_attrs() -> Map<String, serde_json::Value> {
        Map::new()
    }

    #[test]
    fn open_or_focus_dedupes() {
        let mut ws = PlotWorkspace::default();
        let shape = [64u64, 128];
        let (id1, new1) = ws.open_or_focus(0, "/a", &shape, &empty_attrs(), None, "P1");
        assert!(new1);
        assert_eq!(ws.take_pending_requests().len(), 1);
        let (id2, new2) = ws.open_or_focus(0, "/a", &shape, &empty_attrs(), None, "P1");
        assert!(!new2);
        assert_eq!(id1, id2);
        assert!(ws.take_pending_requests().is_empty());
    }

    #[test]
    fn open_different_paths_creates_tabs() {
        let mut ws = PlotWorkspace::default();
        let shape = [64u64, 128];
        ws.open_or_focus(0, "/a", &shape, &empty_attrs(), None, "P1");
        ws.open_or_focus(0, "/b", &shape, &empty_attrs(), None, "P1");
        assert!(ws.take_pending_requests().len() <= 2);
        let (id_a, _) = ws.open_or_focus(0, "/a", &shape, &empty_attrs(), None, "P1");
        let (id_b, _) = ws.open_or_focus(0, "/b", &shape, &empty_attrs(), None, "P1");
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn queue_reload_includes_resolution_and_stride() {
        let mut ws = PlotWorkspace::default();
        let shape = [64u64, 128];
        ws.open_or_focus(0, "/measurements/lst", &shape, &empty_attrs(), None, "P1");
        ws.take_pending_requests();

        ws.slots[0].resolution_percent = 50;
        ws.slots[0].sample_stride = 4;
        ws.slots[0].queue_reload("/measurements/lst");

        let pending = ws.take_pending_requests();
        assert_eq!(pending.len(), 1);
        let (_, request) = &pending[0];
        assert_eq!(request.resolution_percent, 50);
        assert_eq!(request.sample_stride, 4);
    }

    #[test]
    fn product_close_reindexes() {
        let mut ws = PlotWorkspace::default();
        let shape = [64u64, 128];
        let (id, _) = ws.open_or_focus(1, "/a", &shape, &empty_attrs(), None, "P2");
        ws.on_product_closed(0, false);
        assert_eq!(ws.store_index_for_slot(id), Some(0));
    }
}
