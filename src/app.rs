use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use eframe::egui;

use copernicus_viewer::display::{render_inspector, InspectorView};
use copernicus_viewer::plot::{load_plot_data, shared_progress, PlotLoadResult, PlotPanel};
use copernicus_viewer::zarr::{open_store, resolve_zarr_product_path, ZarrNodeKind, ZarrStore, ZarrTreeNode};

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectedNode {
    store_index: usize,
    path: String,
}

#[derive(Default)]
struct OpenProductDialog {
    show: bool,
    path: String,
    browser_dir: PathBuf,
}

struct PendingNativeOpen {
    path_hint: String,
}

enum LoadMessage {
    StoreReady {
        path: PathBuf,
        result: Result<ZarrStore, String>,
    },
    PlotProgress {
        store_index: usize,
        path: String,
        fraction: f32,
        message: String,
    },
    PlotReady {
        store_index: usize,
        path: String,
        result: Result<PlotLoadResult, String>,
    },
}

pub struct CopernicusViewer {
    stores: Vec<Arc<ZarrStore>>,
    selected: Option<SelectedNode>,
    inspector: InspectorView,
    plot_panel: PlotPanel,
    status_message: String,
    load_tx: Sender<LoadMessage>,
    load_rx: Receiver<LoadMessage>,
    pending_plot: Option<(usize, String)>,
    pending_native_open: Option<PendingNativeOpen>,
    open_product_dialog: OpenProductDialog,
}

impl CopernicusViewer {
    pub fn new(_cc: &eframe::CreationContext<'_>, initial_paths: Vec<PathBuf>) -> Self {
        let (load_tx, load_rx) = mpsc::channel();
        let mut app = Self {
            stores: Vec::new(),
            selected: None,
            inspector: InspectorView::default(),
            plot_panel: PlotPanel::default(),
            status_message: String::new(),
            load_tx,
            load_rx,
            pending_plot: None,
            pending_native_open: None,
            open_product_dialog: OpenProductDialog::default(),
        };

        for path in initial_paths {
            app.open_path(path);
        }

        app
    }

    fn show_open_product_dialog(&mut self) {
        let store_root = self
            .stores
            .last()
            .map(|store| PathBuf::from(&store.root_path));
        self.open_product_dialog.browser_dir = crate::file_browser::initial_browser_dir(
            &self.open_product_dialog.path,
            store_root.as_deref(),
        );
        if self.open_product_dialog.path.is_empty() {
            if let Some(root) = &store_root {
                self.open_product_dialog.path = root.display().to_string();
            }
        }
        self.open_product_dialog.show = true;
    }

    fn request_native_open(&mut self, path_hint: String) {
        self.pending_native_open = Some(PendingNativeOpen { path_hint });
    }

    fn open_path(&mut self, path: PathBuf) {
        let path = resolve_zarr_product_path(&path);
        let root_path = path.display().to_string();
        if self.stores.iter().any(|store| store.root_path == root_path) {
            self.status_message = format!("Already open: {root_path}");
            return;
        }

        self.status_message = format!("Opening {root_path}…");

        let tx = self.load_tx.clone();
        thread::spawn(move || {
            let result = open_store(&path).map_err(|e| e.to_string());
            let _ = tx.send(LoadMessage::StoreReady { path, result });
        });
    }

    fn open_product_dialog_ui(&mut self, ctx: &egui::Context) {
        if !self.open_product_dialog.show {
            return;
        }

        let mut submit_path: Option<PathBuf> = None;
        let mut keep_open = true;
        let mut selected_path = self.open_product_dialog.path.clone();

        let window = egui::Window::new("Open Zarr product")
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .default_height(460.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("EOPF Zarr directory or .zarr.zip archive:");
                ui.horizontal(|ui| {
                    ui.label("Location:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.open_product_dialog.path)
                            .desired_width(f32::INFINITY)
                            .hint_text("/path/to/product.zarr"),
                    );
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let can_go_up = self
                        .open_product_dialog
                        .browser_dir
                        .parent()
                        .is_some_and(|p| p.is_dir());
                    ui.add_enabled_ui(can_go_up, |ui| {
                        if ui.button("⬆ Up").clicked() {
                            if let Some(parent) = self.open_product_dialog.browser_dir.parent() {
                                self.open_product_dialog.browser_dir = parent.to_path_buf();
                            }
                        }
                    });

                    if ui.button("🏠 Home").clicked() {
                        self.open_product_dialog.browser_dir =
                            crate::file_browser::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                    }

                    if ui.button("System picker…").clicked() {
                        self.request_native_open(self.open_product_dialog.path.clone());
                    }
                });

                ui.label(
                    egui::RichText::new(format!(
                        "Browse: {}",
                        self.open_product_dialog.browser_dir.display()
                    ))
                    .strong(),
                );
                ui.label(
                    egui::RichText::new(
                        "Select a .zarr folder or .zip archive. Double-click a folder to open it, \
                         or double-click a product to load it.",
                    )
                    .small()
                    .weak(),
                );

                match crate::file_browser::list_directory(&self.open_product_dialog.browser_dir) {
                    Ok(items) if items.is_empty() => {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("No .zarr folders or zip archives here.")
                                .weak(),
                        );
                    }
                    Ok(items) => {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(240.0)
                            .show(ui, |ui| {
                                for item in items {
                                    match item {
                                        crate::file_browser::BrowserItem::Directory {
                                            name,
                                            path,
                                            zarr_product,
                                        } => {
                                            let label = if zarr_product {
                                                format!("📦  {name}")
                                            } else {
                                                format!("📁  {name}")
                                            };
                                            let selected = selected_path == path.display().to_string();
                                            let response =
                                                ui.selectable_label(selected, label);
                                            if response.clicked() {
                                                selected_path = path.display().to_string();
                                                self.open_product_dialog.path =
                                                    selected_path.clone();
                                            }
                                            if response.double_clicked() {
                                                if zarr_product {
                                                    submit_path = Some(path);
                                                    keep_open = false;
                                                } else {
                                                    self.open_product_dialog.browser_dir = path;
                                                    selected_path.clear();
                                                    self.open_product_dialog.path.clear();
                                                }
                                            }
                                        }
                                        crate::file_browser::BrowserItem::ZipArchive {
                                            name,
                                            path,
                                        } => {
                                            let label = format!("🗜  {name}");
                                            let selected = selected_path == path.display().to_string();
                                            let response =
                                                ui.selectable_label(selected, label);
                                            if response.clicked() {
                                                selected_path = path.display().to_string();
                                                self.open_product_dialog.path =
                                                    selected_path.clone();
                                            }
                                            if response.double_clicked() {
                                                submit_path = Some(path);
                                                keep_open = false;
                                            }
                                        }
                                    }
                                }
                            });
                    }
                    Err(err) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, err);
                    }
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let can_open = !self.open_product_dialog.path.trim().is_empty();
                    ui.add_enabled_ui(can_open, |ui| {
                        if ui.button("Open").clicked() {
                            submit_path = Some(PathBuf::from(
                                self.open_product_dialog.path.trim(),
                            ));
                            keep_open = false;
                        }
                    });
                    if ui.button("Cancel").clicked() {
                        keep_open = false;
                    }
                });
            });

        if window.is_none() || !keep_open {
            self.open_product_dialog.show = false;
        }

        if let Some(path) = submit_path {
            self.open_path(path);
        }
    }

    fn product_name(store: &ZarrStore) -> String {
        PathBuf::from(&store.root_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "product".to_string())
    }

    fn select_node(&mut self, store_index: usize, node: &ZarrTreeNode) {
        self.selected = Some(SelectedNode {
            store_index,
            path: node.path.clone(),
        });

        let store = self.stores.get(store_index);
        let root = store.map(|s| s.tree.root.clone());
        let product_name = store
            .as_ref()
            .map(|s| Self::product_name(s))
            .unwrap_or_else(|| "product".to_string());
        let mut inspector = if let Some(root) = &root {
            InspectorView::from_node_with_root(node, &product_name, Some(root))
        } else {
            InspectorView::from_node(node, &product_name)
        };
        if !matches!(node.kind, ZarrNodeKind::Array { .. }) {
            inspector.clear_array_extras();
        }
        self.inspector = inspector;

        if let ZarrNodeKind::Array { shape, attributes, .. } = &node.kind {
            self.plot_panel.select_array(&node.path, shape, attributes);
            self.pending_plot = Some((store_index, node.path.clone()));
            self.request_plot_load();
        } else {
            self.plot_panel.clear();
            self.pending_plot = None;
        }
    }

    fn request_plot_load(&mut self) {
        let Some((store_index, path)) = self.pending_plot.clone() else {
            return;
        };

        let Some(store) = self.stores.get(store_index).cloned() else {
            return;
        };

        let Some(node) = store.tree.root.find_by_path(&path) else {
            return;
        };

        let ZarrNodeKind::Array { .. } = &node.kind else {
            return;
        };

        let request = self.plot_panel.take_pending_request().unwrap_or_else(|| {
            copernicus_viewer::plot::PlotRequest {
                array_path: path.clone(),
                slice_indices: self.plot_panel.slice_indices().to_vec(),
                flag_selection: self.plot_panel.flag_selection(),
            }
        });

        let kind = node.kind.clone();
        let storage = store.storage.clone();
        let tree = store.tree.root.clone();
        let tx = self.load_tx.clone();
        let path_for_progress = path.clone();

        let (progress_tx, progress_rx) = mpsc::channel();
        let progress_forward_tx = tx.clone();
        let progress_forward_store = store_index;
        let progress_forward_path = path_for_progress.clone();
        thread::spawn(move || {
            while let Ok((fraction, message)) = progress_rx.recv() {
                let _ = progress_forward_tx.send(LoadMessage::PlotProgress {
                    store_index: progress_forward_store,
                    path: progress_forward_path.clone(),
                    fraction,
                    message,
                });
            }
        });

        thread::spawn(move || {
            let progress = shared_progress(progress_tx);
            let result =
                load_plot_data(&storage, &tree, &kind, &request, Some(progress)).map_err(|e| e.to_string());
            let _ = tx.send(LoadMessage::PlotReady {
                store_index,
                path,
                result,
            });
        });
    }

    fn plot_is_current(&self, store_index: usize, path: &str) -> bool {
        self.pending_plot
            .as_ref()
            .is_some_and(|(idx, p)| *idx == store_index && p == path)
    }

    fn poll_background_tasks(&mut self, ctx: &egui::Context) -> bool {
        let mut needs_repaint = false;

        while let Ok(msg) = self.load_rx.try_recv() {
            needs_repaint = true;
            match msg {
                LoadMessage::StoreReady { path, result } => match result {
                    Ok(store) => {
                        let is_first = self.stores.is_empty();
                        let root_path = store.root_path.clone();
                        self.stores.push(Arc::new(store));
                        let count = self.stores.len();
                        self.status_message =
                            format!("Loaded {root_path} ({count} product{} open)", if count == 1 { "" } else { "s" });
                        if is_first {
                            let root = self.stores[0].tree.root.clone();
                            self.select_node(0, &root);
                        }
                    }
                    Err(err) => {
                        self.status_message = format!("Failed to open {}: {err}", path.display());
                    }
                },
                LoadMessage::PlotProgress {
                    store_index,
                    path,
                    fraction,
                    message,
                } => {
                    if !self.plot_is_current(store_index, &path) {
                        continue;
                    }
                    self.plot_panel.set_load_progress(fraction, &message);
                }
                LoadMessage::PlotReady {
                    store_index,
                    path,
                    result,
                } => {
                    if !self.plot_is_current(store_index, &path) {
                        continue;
                    }
                    match result {
                        Ok(loaded) => {
                            self.inspector
                                .set_array_extras(loaded.stats.clone(), loaded.preview.clone());
                            self.plot_panel.set_load_result(loaded);
                        }
                        Err(err) => self.plot_panel.set_error(err),
                    }
                }
            }
        }

        if self.plot_panel.take_pending_request().is_some() {
            self.request_plot_load();
            needs_repaint = true;
        }

        if needs_repaint {
            ctx.request_repaint();
        }

        needs_repaint
    }

    fn node_is_selected(&self, store_index: usize, node: &ZarrTreeNode) -> bool {
        self.selected.as_ref().is_some_and(|sel| {
            sel.store_index == store_index && sel.path == node.path
        })
    }

    fn tree_ui(&mut self, ui: &mut egui::Ui, store_index: usize, node: &ZarrTreeNode) {
        if node.path == "/" {
            for child in &node.children {
                self.tree_ui(ui, store_index, child);
            }
            return;
        }

        let label = match &node.kind {
            ZarrNodeKind::Group { .. } => format!("📁 {}", node.name),
            ZarrNodeKind::Array { shape, dtype, .. } => {
                if shape.is_empty() {
                    format!("📊 {} (scalar, {dtype})", node.name)
                } else if shape.len() >= 2 {
                    format!(
                        "📊 {} [{}, {}] {dtype}",
                        node.name,
                        shape[shape.len() - 2],
                        shape[shape.len() - 1]
                    )
                } else {
                    format!("📊 {} {:?} {dtype}", node.name, shape)
                }
            }
        };

        let id = format!("{store_index}{}", node.path);

        if node.children.is_empty() {
            let selected = self.node_is_selected(store_index, node);
            let response = ui.selectable_label(selected, label);
            if response.clicked() || response.double_clicked() {
                self.select_node(store_index, node);
            }
        } else {
            let default_open = node.path == "/measurements" || node.path == "/conditions";
            let response = egui::CollapsingHeader::new(label)
                .id_salt(id)
                .default_open(default_open)
                .show(ui, |ui| {
                    for child in &node.children {
                        self.tree_ui(ui, store_index, child);
                    }
                });

            if response.header_response.double_clicked() {
                self.select_node(store_index, node);
            }
        }
    }

    fn products_tree_ui(&mut self, ui: &mut egui::Ui) {
        if self.stores.is_empty() {
            ui.label("No product loaded.");
            return;
        }

        let products: Vec<(usize, String)> = self
            .stores
            .iter()
            .enumerate()
            .map(|(index, store)| (index, Self::product_name(store)))
            .collect();

        for (store_index, product_name) in products {
            let root = self.stores[store_index].tree.root.clone();
            let response = egui::CollapsingHeader::new(format!("📦 {product_name}"))
                .id_salt(format!("product_{store_index}"))
                .default_open(self.stores.len() <= 2)
                .show(ui, |ui| {
                    self.tree_ui(ui, store_index, &root);
                });

            if response.header_response.double_clicked() {
                self.select_node(store_index, &root);
            }
        }
    }
}

impl eframe::App for CopernicusViewer {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let area = ui.clip_rect().size();
        if area.x < 8.0 || area.y < 8.0 {
            return;
        }

        self.poll_background_tasks(ui.ctx());

        if let Some(pending) = self.pending_native_open.take() {
            let kind = crate::platform::zarr_native_pick_for_hint(&pending.path_hint);
            if let Some(path) = crate::platform::pick_zarr_product(frame, kind) {
                self.open_product_dialog.show = false;
                self.open_path(path);
            }
        }

        egui::Panel::top("menu").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open Zarr…").clicked() {
                        self.show_open_product_dialog();
                        ui.close();
                    }
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });

        if !self.status_message.is_empty() {
            egui::Panel::bottom("status")
                .resizable(false)
                .show_inside(ui, |ui| {
                    ui.label(&self.status_message);
                });
        }

        egui::Panel::left("tree_panel")
            .default_size(260.0)
            .resizable(true)
            .show_inside(ui, |ui| {
                ui.heading("Hierarchy");
                ui.separator();
                ui.label(
                    egui::RichText::new("Click a variable to inspect and plot.")
                        .small()
                        .weak(),
                );
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.products_tree_ui(ui);
                    });
            });

        egui::Panel::left("inspector_panel")
            .default_size(380.0)
            .resizable(true)
            .show_inside(ui, |ui| {
                ui.heading("Inspector");
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        render_inspector(ui, &self.inspector);
                    });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let ctx = ui.ctx().clone();
            self.plot_panel.ui(ui, &ctx);
        });

        self.open_product_dialog_ui(ui.ctx());
    }
}
