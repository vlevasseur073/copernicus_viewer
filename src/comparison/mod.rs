//! Side-by-side and difference views across open EOPF Zarr products.

mod compare;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;

use crate::zarr::ZarrStore;

pub use compare::{compare_products, ComparisonResult};

/// Comparison workspace: pick two open products and run [`compare_products`].
#[derive(Default)]
pub struct ComparisonTool {
    open: bool,
    left_index: usize,
    right_index: usize,
    result: Option<ComparisonResult>,
}

pub(crate) fn product_label(store: &ZarrStore) -> String {
    PathBuf::from(&store.root_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| store.root_path.clone())
}

impl ComparisonTool {
    pub fn show(&mut self, store_count: usize) {
        self.open = true;
        self.clamp_indices(store_count);
    }

    pub fn ui(&mut self, ctx: &egui::Context, stores: &[Arc<ZarrStore>]) {
        if !self.open {
            return;
        }

        self.clamp_indices(stores.len());

        let mut keep_open = self.open;
        egui::Window::new("Comparison")
            .collapsible(true)
            .resizable(true)
            .default_width(520.0)
            .default_height(360.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut keep_open)
            .show(ctx, |ui| {
                ui.heading("Comparison");
                ui.separator();
                ui.label("Select two open products to compare:");

                if stores.len() < 2 {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Open at least two products to use this tool.")
                            .weak(),
                    );
                    return;
                }

                ui.add_space(8.0);
                let mut left_index = self.left_index;
                let mut right_index = self.right_index;
                Self::product_selector(ui, stores, "Product A", &mut left_index);
                ui.add_space(4.0);
                Self::product_selector(ui, stores, "Product B", &mut right_index);
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

                ui.add_space(12.0);
                ui.add_enabled_ui(!same_product, |ui| {
                    if ui.button("Compare").clicked() {
                        let left = &stores[self.left_index];
                        let right = &stores[self.right_index];
                        self.result = Some(compare_products(left, right));
                    }
                });

                if let Some(result) = &self.result {
                    ui.add_space(12.0);
                    ui.separator();
                    ui.label(&result.summary);
                }
            });
        self.open = keep_open;
    }

    fn product_selector(
        ui: &mut egui::Ui,
        stores: &[Arc<ZarrStore>],
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_label_uses_final_path_segment() {
        let store = ZarrStore {
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
        };
        assert_eq!(product_label(&store), "S03OLCEFR_20230509.zarr");
    }
}
