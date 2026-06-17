use crate::display::footprint::{parse_product_footprint, ProductFootprint};
use crate::display::map::render_footprint_map;
use crate::display::stats::{ArrayPreview, ArrayStatistics};
use crate::display::stac::{render_attribute_tree, AttributeNode};
use crate::display::{format_node_repr, parse_root_attributes};
use crate::zarr::{ZarrNodeKind, ZarrTreeNode};

#[derive(Clone, Debug, Default)]
pub struct InspectorView {
    pub title: String,
    pub repr_body: String,
    pub footprint: Option<ProductFootprint>,
    pub root_attributes: Option<Vec<AttributeNode>>,
    pub stats: Option<ArrayStatistics>,
    pub preview: Option<ArrayPreview>,
}

impl InspectorView {
    pub fn from_node(node: &ZarrTreeNode, product_name: &str) -> Self {
        Self::from_node_with_root(node, product_name, None)
    }

    pub fn from_node_with_root(
        node: &ZarrTreeNode,
        product_name: &str,
        root: Option<&ZarrTreeNode>,
    ) -> Self {
        let repr = format_node_repr(node, product_name);
        let (root_attributes, footprint) = root_metadata(node, root);

        Self {
            title: repr.title,
            repr_body: repr.body,
            footprint,
            root_attributes,
            stats: None,
            preview: None,
        }
    }

    pub fn set_array_extras(&mut self, stats: ArrayStatistics, preview: ArrayPreview) {
        self.stats = Some(stats);
        self.preview = Some(preview);
    }

    pub fn clear_array_extras(&mut self) {
        self.stats = None;
        self.preview = None;
    }
}

fn root_metadata(
    node: &ZarrTreeNode,
    root: Option<&ZarrTreeNode>,
) -> (Option<Vec<AttributeNode>>, Option<ProductFootprint>) {
    let root_node = if node.path == "/" {
        Some(node)
    } else {
        root
    };

    let Some(root_node) = root_node else {
        return (None, None);
    };

    let ZarrNodeKind::Group { attributes } = &root_node.kind else {
        return (None, None);
    };

    let root_attributes = parse_root_attributes(root_node, None);
    let footprint = parse_product_footprint(attributes);
    (root_attributes, footprint)
}

pub fn render_inspector(ui: &mut egui::Ui, view: &InspectorView) {
    ui.monospace(&view.title);
    ui.separator();

    if let Some(footprint) = &view.footprint {
        render_footprint_map(ui, footprint);
        ui.separator();
    }

    if let Some(attributes) = &view.root_attributes {
        egui::CollapsingHeader::new("Product attributes")
            .default_open(false)
            .show(ui, |ui| {
                render_attribute_tree(ui, attributes, "root_attrs");
            });
        ui.separator();
    }

    egui::CollapsingHeader::new("Representation")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut view.repr_body.as_str())
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .interactive(false),
            );
        });

    if let Some(stats) = &view.stats {
        ui.separator();
        egui::CollapsingHeader::new("Statistics")
            .default_open(true)
            .show(ui, |ui| {
                render_stats_table(ui, stats);
            });
    }

    if let Some(preview) = &view.preview {
        ui.separator();
        egui::CollapsingHeader::new("Data preview")
            .default_open(true)
            .show(ui, |ui| {
                render_preview_table(ui, preview);
            });
    }
}

fn render_stats_table(ui: &mut egui::Ui, stats: &ArrayStatistics) {
    ui.label(
        egui::RichText::new("Computed on the loaded plot subset")
            .small()
            .weak(),
    );
    egui::Grid::new("stats_grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            stat_row(ui, "elements", stats.element_count.to_string());
            stat_row(ui, "finite", stats.finite_count.to_string());
            if stats.nan_count > 0 {
                stat_row(ui, "NaN", stats.nan_count.to_string());
            }
            if let Some(v) = stats.min {
                stat_row(ui, "min", format!("{v:.6}"));
            }
            if let Some(v) = stats.max {
                stat_row(ui, "max", format!("{v:.6}"));
            }
            if let Some(v) = stats.mean {
                stat_row(ui, "mean", format!("{v:.6}"));
            }
            if let Some(v) = stats.std_dev {
                stat_row(ui, "std", format!("{v:.6}"));
            }
        });
}

fn stat_row(ui: &mut egui::Ui, name: &str, value: String) {
    ui.label(name);
    ui.monospace(value);
    ui.end_row();
}

fn render_preview_table(ui: &mut egui::Ui, preview: &ArrayPreview) {
    if preview.rows.is_empty() {
        ui.label("(empty)");
        return;
    }

    egui::ScrollArea::horizontal().show(ui, |ui| {
        egui::Grid::new("preview_grid")
            .num_columns(preview.column_labels.len() + 1)
            .spacing([8.0, 2.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label("");
                for label in &preview.column_labels {
                    ui.monospace(label);
                }
                ui.end_row();

                for (row_idx, row) in preview.rows.iter().enumerate() {
                    ui.monospace(row_idx.to_string());
                    for cell in row {
                        ui.monospace(cell);
                    }
                    ui.end_row();
                }
            });
    });
}
