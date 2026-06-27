//! Side-by-side comparison of two EOPF Zarr products (structure, variable data, CF flags).
//!
//! The core API is [`compare_products`] / [`compare_products_with_options`]. The GUI
//! wraps these in [`ComparisonTool`]; the `compare_products` binary example exposes
//! the same logic on the command line.

mod array_io;
mod compare;
mod data;
mod flags;
mod options;
mod structure;

use std::path::PathBuf;

use eframe::egui;

use crate::product::{Product, ProductHandle};

pub use compare::{
    ComparisonOptions, ComparisonResult, compare_products, compare_products_with_options,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ErrorMode {
    Relative,
    Absolute,
    Auto,
}

impl ErrorMode {
    fn label(self) -> &'static str {
        match self {
            Self::Relative => "Relative (all variables)",
            Self::Absolute => "Absolute (all variables)",
            Self::Auto => "Auto (from scale_factor)",
        }
    }
}

/// Comparison workspace: pick two open products and run [`compare_products`].
#[derive(Default)]
pub struct ComparisonTool {
    open: bool,
    left_index: usize,
    right_index: usize,
    options: ComparisonOptions,
    verbose: bool,
    result: Option<ComparisonResult>,
    /// Whether a compare run has been requested by the UI (not yet started by the
    /// background worker).
    pending_run: bool,
    /// True while a background compare thread is running.
    running: bool,
}

pub(crate) fn product_label(store: &Product) -> String {
    PathBuf::from(store.root_path())
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| store.root_path().to_string())
}

impl ComparisonTool {
    /// Open the comparison window (requires at least two products for a meaningful run).
    pub fn show(&mut self, store_count: usize) {
        self.open = true;
        self.clamp_indices(store_count);
    }

    /// Open the comparison window and immediately compare two open products by index.
    pub fn open_and_compare(&mut self, left: usize, right: usize, stores: &[ProductHandle]) {
        // Open the comparison window and request a background run. The actual
        // work is performed by the application main loop which will detect the
        // pending request and spawn a worker thread.
        self.open = true;
        self.left_index = left;
        self.right_index = right;
        self.clamp_indices(stores.len());
        if left < stores.len() && right < stores.len() && left != right {
            self.result = None;
            self.pending_run = true;
        }
    }

    /// Returns `true` after a comparison has been run in this session.
    pub fn has_result(&self) -> bool {
        self.result.is_some()
    }

    /// Render the comparison window and controls.
    pub fn ui(&mut self, ctx: &egui::Context, stores: &[ProductHandle]) {
        if !self.open {
            return;
        }

        self.clamp_indices(stores.len());

        let mut keep_open = self.open;
        egui::Window::new("Comparison")
            .collapsible(true)
            .resizable(true)
            .default_width(640.0)
            .default_height(560.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut keep_open)
            .show(ctx, |ui| {
                ui.heading("Comparison");
                ui.separator();
                ui.label("Select reference (A) and new product (B):");

                if stores.len() < 2 {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Open at least two products to use this tool.").weak(),
                    );
                    return;
                }

                ui.add_space(8.0);
                let mut left_index = self.left_index;
                let mut right_index = self.right_index;
                Self::product_selector(ui, stores, "Product A (reference)", &mut left_index);
                ui.add_space(4.0);
                Self::product_selector(ui, stores, "Product B (new)", &mut right_index);
                self.left_index = left_index;
                self.right_index = right_index;

                let same_product = self.left_index == self.right_index;
                if same_product {
                    ui.add_space(8.0);
                    ui.colored_label(
                        egui::Color32::LIGHT_YELLOW,
                        "Choose two different products.",
                    );
                }

                ui.add_space(8.0);
                Self::options_ui(ui, &mut self.options, &mut self.verbose);

                ui.add_space(12.0);
                ui.add_enabled_ui(!same_product && !self.running, |ui| {
                    if ui.button("Compare").clicked() {
                        // Request a background comparison; do not block the UI.
                        self.result = None;
                        self.pending_run = true;
                    }
                });
                if self.running {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("Comparing…").weak());
                }

                if let Some(result) = &self.result {
                    ui.add_space(12.0);
                    ui.separator();
                    let color = if result.success {
                        egui::Color32::LIGHT_GREEN
                    } else {
                        egui::Color32::LIGHT_RED
                    };
                    ui.colored_label(color, if result.success { "PASSED" } else { "FAILED" });
                    let mut report = result.formatted_summary(self.verbose);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut report)
                                    .desired_width(f32::INFINITY)
                                    .interactive(false),
                            );
                        });
                }
            });
        self.open = keep_open;
    }

    fn product_selector(
        ui: &mut egui::Ui,
        stores: &[ProductHandle],
        label: &str,
        index: &mut usize,
    ) {
        let selected = product_label(&stores[*index]);
        egui::ComboBox::from_id_salt(label)
            .selected_text(selected)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (i, store) in stores.iter().enumerate() {
                    ui.selectable_value(index, i, product_label(store));
                }
            });
    }

    fn options_ui(ui: &mut egui::Ui, options: &mut ComparisonOptions, verbose: &mut bool) {
        egui::CollapsingHeader::new("Options")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Compare");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut options.structure, "Structure / metadata");
                    ui.checkbox(&mut options.data, "Variable data");
                    ui.checkbox(&mut options.flags, "Flags / masks");
                });
                ui.add_enabled_ui(options.structure, |ui| {
                    ui.checkbox(&mut options.chunks, "Chunk layout");
                });

                ui.add_space(4.0);
                ui.label("Error mode");
                Self::error_mode_selector(ui, options);

                ui.add_space(4.0);
                ui.label("Thresholds");
                egui::Grid::new("comparison_thresholds")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Data threshold");
                        ui.add(
                            egui::DragValue::new(&mut options.threshold)
                                .speed(0.000001)
                                .range(0.0..=f64::MAX),
                        );
                        ui.end_row();

                        ui.label("Packed threshold factor");
                        ui.add(
                            egui::DragValue::new(&mut options.threshold_packed)
                                .speed(0.01)
                                .range(0.0..=f64::MAX),
                        );
                        ui.end_row();

                        ui.label("Max outlier ratio");
                        ui.add(
                            egui::DragValue::new(&mut options.threshold_nb_outliers)
                                .speed(0.001)
                                .range(0.0..=1.0)
                                .fixed_decimals(4),
                        );
                        ui.end_row();

                        ui.label("Max coverage difference");
                        ui.add(
                            egui::DragValue::new(&mut options.threshold_coverage)
                                .speed(0.001)
                                .range(0.0..=1.0)
                                .fixed_decimals(4),
                        );
                        ui.end_row();
                    });

                ui.add_space(4.0);
                ui.checkbox(verbose, "Verbose report (list all variables and flag bits)");
            });
    }

    fn error_mode_selector(ui: &mut egui::Ui, options: &mut ComparisonOptions) {
        let mode = if options.relative {
            ErrorMode::Relative
        } else if options.absolute {
            ErrorMode::Absolute
        } else {
            ErrorMode::Auto
        };
        let mut selected = mode;
        egui::ComboBox::from_id_salt("comparison_error_mode")
            .selected_text(selected.label())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut selected,
                    ErrorMode::Relative,
                    ErrorMode::Relative.label(),
                );
                ui.selectable_value(
                    &mut selected,
                    ErrorMode::Absolute,
                    ErrorMode::Absolute.label(),
                );
                ui.selectable_value(&mut selected, ErrorMode::Auto, ErrorMode::Auto.label());
            });
        match selected {
            ErrorMode::Relative => {
                options.relative = true;
                options.absolute = false;
            }
            ErrorMode::Absolute => {
                options.relative = false;
                options.absolute = true;
            }
            ErrorMode::Auto => {
                options.relative = false;
                options.absolute = false;
            }
        }
    }

    fn clamp_indices(&mut self, store_count: usize) {
        if store_count == 0 {
            self.left_index = 0;
            self.right_index = 0;
            self.result = None;
            return;
        }

        if self.left_index >= store_count {
            self.left_index = 0;
        }
        if self.right_index >= store_count {
            self.right_index = store_count.saturating_sub(1);
        }
        if store_count >= 2 && self.left_index == self.right_index {
            self.right_index = (self.left_index + 1) % store_count;
        }
    }

    /// Take any pending compare request previously triggered by the UI. Returns
    /// (left_index, right_index, options, verbose) if a run was requested and
    /// clears the pending flag.
    pub fn take_pending_run(&mut self) -> Option<(usize, usize, ComparisonOptions, bool)> {
        if self.pending_run {
            self.pending_run = false;
            // Clear any previous result when issuing a new run.
            self.result = None;
            return Some((
                self.left_index,
                self.right_index,
                self.options.clone(),
                self.verbose,
            ));
        }
        None
    }

    /// Mark the tool as running (a background worker has started the compare).
    pub fn start_running(&mut self) {
        self.running = true;
    }

    /// Store the comparison result and mark the tool as not running.
    pub fn set_result(&mut self, result: ComparisonResult) {
        self.result = Some(result);
        self.running = false;
    }

    /// Return a reference to the last comparison result, if any.
    pub fn result(&self) -> Option<&ComparisonResult> {
        self.result.as_ref()
    }

    /// Mark the tool as not running any more (worker finished).
    pub fn stop_running(&mut self) {
        self.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::product::Product;
    use crate::zarr::ZarrStore;

    #[test]
    fn product_label_uses_final_path_segment() {
        let store = Product::Zarr(ZarrStore {
            storage: std::sync::Arc::new(
                zarrs::filesystem::FilesystemStore::new(".").expect("store"),
            ),
            root_path: "/data/S03OLCEFR_20230509.zarr".to_string(),
            tree: crate::zarr::tree::ZarrTree {
                root: crate::zarr::ZarrTreeNode {
                    name: "/".to_string(),
                    path: "/".to_string(),
                    kind: crate::zarr::ZarrNodeKind::Group {
                        attributes: Default::default(),
                    },
                    children: vec![],
                },
            },
        });
        assert_eq!(product_label(&store), "S03OLCEFR_20230509.zarr");
    }
}
