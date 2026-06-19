//! Main egui application: hierarchy browser, inspector, plot panel, and comparison tool.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use eframe::egui;

use copernicus_viewer::comparison::ComparisonTool;
use copernicus_viewer::display::{render_inspector, InspectorView};
use copernicus_viewer::plot::{load_plot_data, shared_progress, PlotLoadResult, PlotPanel};
use copernicus_viewer::zarr::{
    open_store, resolve_zarr_product_path, ZarrNodeKind, ZarrStore, ZarrTreeNode,
};
#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectedNode {
    store_index: usize,
    path: String,
}

struct OpenProductDialog {
    show: bool,
    path: String,
    browser_location: crate::file_browser::BrowserLocation,
    browse_request_id: u64,
    browse_items: Option<Result<Vec<crate::file_browser::BrowserItem>, String>>,
    browse_loading: bool,
}

impl OpenProductDialog {
    fn new() -> Self {
        Self {
            show: false,
            path: String::new(),
            browser_location: crate::file_browser::BrowserLocation::Local(
                crate::file_browser::home_dir().unwrap_or_else(|| PathBuf::from("/")),
            ),
            browse_request_id: 0,
            browse_items: None,
            browse_loading: false,
        }
    }
}

struct PendingNativeOpen {
    path_hint: String,
}

enum LoadMessage {
    StoreReady {
        location: String,
        result: Result<ZarrStore, String>,
    },
    BrowseListReady {
        request_id: u64,
        location: crate::file_browser::BrowserLocation,
        result: Result<Vec<crate::file_browser::BrowserItem>, String>,
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

/// Root egui application state for the Copernicus Viewer window.
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
    comparison: ComparisonTool,
    demo_capture: Option<crate::demo_capture::DemoCapture>,
}

impl CopernicusViewer {
    /// Create the viewer and asynchronously open any `initial_locations` (paths or `s3://` URIs).
    pub fn new(initial_locations: Vec<String>) -> Self {
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
            open_product_dialog: OpenProductDialog::new(),
            comparison: ComparisonTool::default(),
            demo_capture: crate::demo_capture::DemoCapture::from_env(),
        };

        for location in initial_locations {
            app.open_path(location);
        }

        app
    }

    fn show_open_product_dialog(&mut self) {
        let last_root = self.stores.last().map(|store| store.root_path.as_str());
        let store_root = last_root
            .filter(|root| !root.starts_with("s3://"))
            .map(PathBuf::from);
        self.open_product_dialog.browser_location =
            crate::file_browser::initial_browser_location(
                &self.open_product_dialog.path,
                store_root.as_deref(),
            );
        if self.open_product_dialog.path.is_empty() {
            if let Some(root) = last_root {
                self.open_product_dialog.path = root.to_string();
            }
        }
        self.open_product_dialog.browse_items = None;
        self.open_product_dialog.show = true;
        self.request_browse_list();
    }

    fn request_browse_list(&mut self) {
        self.open_product_dialog.browse_request_id += 1;
        let request_id = self.open_product_dialog.browse_request_id;
        let location = self.open_product_dialog.browser_location.clone();
        self.open_product_dialog.browse_loading = true;

        let tx = self.load_tx.clone();
        thread::spawn(move || {
            let result = crate::s3_browser::list_browser_items(&location);
            let _ = tx.send(LoadMessage::BrowseListReady {
                request_id,
                location,
                result,
            });
        });
    }

    fn set_browser_location(&mut self, location: crate::file_browser::BrowserLocation) {
        self.open_product_dialog.browser_location = location;
        self.request_browse_list();
    }

    fn request_native_open(&mut self, path_hint: String) {
        self.pending_native_open = Some(PendingNativeOpen { path_hint });
    }

    fn open_path(&mut self, location: String) {
        let trimmed = location.trim();
        if trimmed.is_empty() {
            return;
        }

        if !trimmed.starts_with("s3://") {
            let canonical = resolve_zarr_product_path(Path::new(trimmed))
                .display()
                .to_string();
            if self.stores.iter().any(|store| store.root_path == canonical) {
                self.status_message = format!("Already open: {canonical}");
                return;
            }
        } else if self.stores.iter().any(|store| store.root_path == trimmed) {
            self.status_message = format!("Already open: {trimmed}");
            return;
        }

        self.status_message = format!("Opening {trimmed}…");

        let input = trimmed.to_string();
        let tx = self.load_tx.clone();
        thread::spawn(move || {
            let result = open_store(&input).map_err(|e| e.to_string());
            let _ = tx.send(LoadMessage::StoreReady {
                location: input,
                result,
            });
        });
    }

    fn open_product_dialog_ui(&mut self, ctx: &egui::Context) {
        if !self.open_product_dialog.show {
            return;
        }

        let mut submit_path: Option<String> = None;
        let mut keep_open = true;
        let mut selected_path = self.open_product_dialog.path.clone();
        let mut navigate_to: Option<crate::file_browser::BrowserLocation> = None;
        let is_s3 = self.open_product_dialog.browser_location.is_s3();

        let window = egui::Window::new("Open Zarr product")
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .default_height(460.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("EOPF Zarr directory, .zarr.zip archive, or s3:// URI:");
                ui.horizontal(|ui| {
                    ui.label("Location:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.open_product_dialog.path)
                            .desired_width(f32::INFINITY)
                            .hint_text("/path/to/product.zarr or s3://bucket/path/product.zarr"),
                    );
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let can_go_up = self.open_product_dialog.browser_location.can_go_up();
                    ui.add_enabled_ui(can_go_up, |ui| {
                        if ui.button("⬆ Up").clicked() {
                            if let Some(parent) = self.open_product_dialog.browser_location.go_up()
                            {
                                navigate_to = Some(parent);
                            }
                        }
                    });

                    if ui.button("🏠 Home").clicked() {
                        navigate_to = Some(if is_s3 {
                            crate::file_browser::BrowserLocation::S3Root
                        } else {
                            crate::file_browser::BrowserLocation::Local(
                                crate::file_browser::home_dir()
                                    .unwrap_or_else(|| PathBuf::from("/")),
                            )
                        });
                    }

                    if ui.button("Local").clicked() {
                        navigate_to = Some(crate::file_browser::BrowserLocation::Local(
                            crate::file_browser::initial_browser_dir(
                                &self.open_product_dialog.path,
                                None,
                            ),
                        ));
                    }

                    if ui.button("S3").clicked() {
                        navigate_to = Some(crate::file_browser::BrowserLocation::S3Root);
                    }

                    if !is_s3
                        && ui.button("System picker…").clicked()
                    {
                        self.request_native_open(self.open_product_dialog.path.clone());
                    }
                });

                ui.label(
                    egui::RichText::new(format!(
                        "Browse: {}",
                        self.open_product_dialog.browser_location.display_label()
                    ))
                    .strong(),
                );
                ui.label(
                    egui::RichText::new(if is_s3 {
                        "Select a .zarr prefix. Double-click a folder to open it, \
                         or double-click a product to load it."
                    } else {
                        "Select a .zarr folder or .zip archive. Double-click a folder to open it, \
                         or double-click a product to load it."
                    })
                    .small()
                    .weak(),
                );

                if self.open_product_dialog.browse_loading {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Loading…").weak());
                } else if let Some(result) = &self.open_product_dialog.browse_items {
                    match result {
                        Ok(items) if items.is_empty() => {
                            ui.add_space(8.0);
                            let message = if matches!(
                                self.open_product_dialog.browser_location,
                                crate::file_browser::BrowserLocation::S3Root
                            ) {
                                "No buckets in s3.conf — add [bucket-name] sections \
                                 or type s3://bucket/path above."
                            } else if is_s3 {
                                "No child prefixes here."
                            } else {
                                "No .zarr folders or zip archives here."
                            };
                            ui.label(egui::RichText::new(message).weak());
                        }
                        Ok(items) => {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .max_height(240.0)
                                .show(ui, |ui| {
                                    for item in items {
                                        let location = item.location();
                                        match &item {
                                            crate::file_browser::BrowserItem::Directory {
                                                name,
                                                zarr_product,
                                                ..
                                            } => {
                                                let label = if *zarr_product {
                                                    format!("📦  {name}")
                                                } else {
                                                    format!("📁  {name}")
                                                };
                                                let selected = selected_path == location;
                                                let response = ui.selectable_label(selected, label);
                                                if response.clicked() {
                                                    selected_path = location.to_string();
                                                    self.open_product_dialog.path =
                                                        selected_path.clone();
                                                }
                                                if response.double_clicked() {
                                                    if *zarr_product {
                                                        submit_path = Some(location.to_string());
                                                        keep_open = false;
                                                    } else if let Some(next) =
                                                        crate::file_browser::BrowserLocation::from_path_hint(location)
                                                    {
                                                        navigate_to = Some(next);
                                                        selected_path.clear();
                                                        self.open_product_dialog.path.clear();
                                                    }
                                                }
                                            }
                                            crate::file_browser::BrowserItem::ZipArchive {
                                                name,
                                                ..
                                            } => {
                                                let label = format!("🗜  {name}");
                                                let selected = selected_path == location;
                                                let response = ui.selectable_label(selected, label);
                                                if response.clicked() {
                                                    selected_path = location.to_string();
                                                    self.open_product_dialog.path =
                                                        selected_path.clone();
                                                }
                                                if response.double_clicked() {
                                                    submit_path = Some(location.to_string());
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
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let can_open = !self.open_product_dialog.path.trim().is_empty();
                    ui.add_enabled_ui(can_open, |ui| {
                        if ui.button("Open").clicked() {
                            submit_path =
                                Some(self.open_product_dialog.path.trim().to_string());
                            keep_open = false;
                        }
                    });
                    if ui.button("Cancel").clicked() {
                        keep_open = false;
                    }
                });
            });

        if let Some(location) = navigate_to {
            self.set_browser_location(location);
        }

        if window.is_none() || !keep_open {
            self.open_product_dialog.show = false;
        }

        if let Some(path) = submit_path {
            self.open_path(path);
        }
    }

    fn product_name(store: &ZarrStore) -> String {
        let root = &store.root_path;
        if let Some(rest) = root.strip_prefix("s3://") {
            if let Some(name) = rest.rsplit('/').next().filter(|s| !s.is_empty()) {
                return name.to_string();
            }
        }
        PathBuf::from(root)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "product".to_string())
    }

    fn close_product(&mut self, store_index: usize) {
        if store_index >= self.stores.len() {
            return;
        }

        let closed_name = Self::product_name(&self.stores[store_index]);
        self.stores.remove(store_index);

        let selection_was_closed = self
            .selected
            .as_ref()
            .is_some_and(|sel| sel.store_index == store_index);

        if let Some(sel) = &self.selected {
            if sel.store_index > store_index {
                self.selected = Some(SelectedNode {
                    store_index: sel.store_index - 1,
                    path: sel.path.clone(),
                });
            }
        }

        if let Some((idx, path)) = self.pending_plot.clone() {
            if idx == store_index {
                self.pending_plot = None;
            } else if idx > store_index {
                self.pending_plot = Some((idx - 1, path));
            }
        }

        if selection_was_closed {
            self.selected = None;
            self.pending_plot = None;
            self.plot_panel.clear();
            self.inspector = InspectorView::default();

            if let Some(store) = self.stores.first() {
                let root = store.tree.root.clone();
                self.select_node(0, &root);
            }
        }

        let count = self.stores.len();
        if count == 0 {
            self.status_message = format!("Closed {closed_name}");
        } else {
            self.status_message = format!(
                "Closed {closed_name} ({count} product{} open)",
                if count == 1 { "" } else { "s" }
            );
        }
    }

    fn close_product_for_selection(&mut self) {
        let index = self
            .selected
            .as_ref()
            .map(|sel| sel.store_index)
            .unwrap_or_else(|| self.stores.len().saturating_sub(1));
        self.close_product(index);
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

        if let ZarrNodeKind::Array {
            shape, attributes, ..
        } = &node.kind
        {
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
            let result = load_plot_data(&storage, &tree, &kind, &request, Some(progress))
                .map_err(|e| e.to_string());
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
                LoadMessage::BrowseListReady {
                    request_id,
                    location,
                    result,
                } => {
                    if request_id != self.open_product_dialog.browse_request_id {
                        continue;
                    }
                    if location != self.open_product_dialog.browser_location {
                        continue;
                    }
                    self.open_product_dialog.browse_loading = false;
                    self.open_product_dialog.browse_items = Some(result);
                }
                LoadMessage::StoreReady { location, result } => match result {
                    Ok(store) => {
                        if self
                            .stores
                            .iter()
                            .any(|existing| existing.root_path == store.root_path)
                        {
                            self.status_message =
                                format!("Already open: {}", store.root_path);
                            continue;
                        }
                        let is_first = self.stores.is_empty();
                        let root_path = store.root_path.clone();
                        self.stores.push(Arc::new(store));
                        let count = self.stores.len();
                        self.status_message = format!(
                            "Loaded {root_path} ({count} product{} open)",
                            if count == 1 { "" } else { "s" }
                        );
                        if is_first {
                            let root = self.stores[0].tree.root.clone();
                            self.select_node(0, &root);
                        }
                    }
                    Err(err) => {
                        self.status_message =
                            format!("Failed to open {location}: {err}");
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
        self.selected
            .as_ref()
            .is_some_and(|sel| sel.store_index == store_index && sel.path == node.path)
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

        let mut to_close: Option<usize> = None;

        for (store_index, product_name) in products {
            let root = self.stores[store_index].tree.root.clone();
            ui.horizontal(|ui| {
                if ui
                    .small_button("✕")
                    .on_hover_text("Close product")
                    .clicked()
                {
                    to_close = Some(store_index);
                }

                let response = egui::CollapsingHeader::new(format!("📦 {product_name}"))
                    .id_salt(format!("product_{store_index}"))
                    .default_open(self.stores.len() <= 2)
                    .show(ui, |ui| {
                        self.tree_ui(ui, store_index, &root);
                    });

                if response.header_response.double_clicked() {
                    self.select_node(store_index, &root);
                }
            });
        }

        if let Some(store_index) = to_close {
            self.close_product(store_index);
        }
    }
}

impl eframe::App for CopernicusViewer {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(demo) = &mut self.demo_capture {
            demo.handle_events(ctx);
            let action = demo.tick(ctx, &self.stores, &self.plot_panel, &self.comparison);
            match action {
                Some(crate::demo_capture::DemoAction::SelectLst) => {
                    if let Some(node) = self.stores[0]
                        .tree
                        .root
                        .find_by_path("/measurements/lst")
                        .cloned()
                    {
                        self.select_node(0, &node);
                    }
                }
                Some(crate::demo_capture::DemoAction::RunComparison) => {
                    self.comparison.open_and_compare(0, 1, &self.stores);
                }
                Some(crate::demo_capture::DemoAction::Close) => {}
                None => {}
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let area = ui.clip_rect().size();
        if area.x <= 0.0 || area.y <= 0.0 {
            return;
        }

        self.poll_background_tasks(ui.ctx());

        if let Some(pending) = self.pending_native_open.take() {
            let kind = crate::platform::zarr_native_pick_for_hint(&pending.path_hint);
            if let Some(path) = crate::platform::pick_zarr_product(frame, kind) {
                self.open_product_dialog.show = false;
                self.open_path(path.display().to_string());
            }
        }

        egui::Panel::top("menu").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open Zarr…").clicked() {
                        self.show_open_product_dialog();
                        ui.close();
                    }
                    ui.add_enabled_ui(!self.stores.is_empty(), |ui| {
                        if ui.button("Close product").clicked() {
                            self.close_product_for_selection();
                            ui.close();
                        }
                    });
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Tools", |ui| {
                    if ui.button("Comparison…").clicked() {
                        self.comparison.show(self.stores.len());
                        ui.close();
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
        self.comparison.ui(ui.ctx(), &self.stores);
    }
}
