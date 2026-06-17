use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use eframe::egui;

use copernicus_viewer::display::format_node_repr;
use copernicus_viewer::plot::{load_plot_data, PlotData, PlotPanel};
use copernicus_viewer::zarr::{open_store, ZarrNodeKind, ZarrStore, ZarrTreeNode};

enum LoadMessage {
    StoreReady(Result<ZarrStore, String>),
    PlotReady {
        path: String,
        result: Result<PlotData, String>,
    },
}

pub struct CopernicusViewer {
    store: Option<Arc<ZarrStore>>,
    selected_path: Option<String>,
    info_text: String,
    info_title: String,
    plot_panel: PlotPanel,
    status_message: String,
    load_tx: Sender<LoadMessage>,
    load_rx: Receiver<LoadMessage>,
    pending_plot_path: Option<String>,
}

impl CopernicusViewer {
    pub fn new(_cc: &eframe::CreationContext<'_>, initial_path: Option<PathBuf>) -> Self {
        let (load_tx, load_rx) = mpsc::channel();
        let mut app = Self {
            store: None,
            selected_path: None,
            info_text: "Open an EOPF Zarr product to begin.".to_string(),
            info_title: "Copernicus Viewer".to_string(),
            plot_panel: PlotPanel::default(),
            status_message: String::new(),
            load_tx,
            load_rx,
            pending_plot_path: None,
        };

        if let Some(path) = initial_path {
            app.open_path(path);
        }

        app
    }

    fn open_path(&mut self, path: PathBuf) {
        self.status_message = format!("Opening {}…", path.display());
        self.store = None;
        self.selected_path = None;
        self.plot_panel.clear();
        self.info_text = "Loading…".to_string();
        self.info_title = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Loading".to_string());

        let tx = self.load_tx.clone();
        thread::spawn(move || {
            let result = open_store(&path).map_err(|e| e.to_string());
            let _ = tx.send(LoadMessage::StoreReady(result));
        });
    }

    fn open_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Zarr", &["zarr", "zip"])
            .pick_folder()
            .or_else(|| {
                rfd::FileDialog::new()
                    .add_filter("Zarr Zip", &["zip"])
                    .pick_file()
            })
        {
            self.open_path(path);
        }
    }

    fn select_node(&mut self, node: &ZarrTreeNode) {
        self.selected_path = Some(node.path.clone());

        let product_name = self
            .store
            .as_ref()
            .map(|s| {
                PathBuf::from(&s.root_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "product".to_string())
            })
            .unwrap_or_else(|| "product".to_string());

        let repr = format_node_repr(node, &product_name);
        self.info_title = repr.title;
        self.info_text = repr.body;

        if let ZarrNodeKind::Array { shape, .. } = &node.kind {
            self.plot_panel.select_array(&node.path, shape);
            self.pending_plot_path = Some(node.path.clone());
            self.request_plot_load();
        } else {
            self.plot_panel.clear();
            self.pending_plot_path = None;
        }
    }

    fn request_plot_load(&mut self) {
        let Some(path) = self.pending_plot_path.clone() else {
            return;
        };

        let Some(store) = self.store.clone() else {
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
            }
        });

        let kind = node.kind.clone();
        let storage = store.storage.clone();
        let tx = self.load_tx.clone();

        thread::spawn(move || {
            let result = load_plot_data(&storage, &kind, &request).map_err(|e| e.to_string());
            let _ = tx.send(LoadMessage::PlotReady { path, result });
        });
    }

    fn poll_background_tasks(&mut self, ctx: &egui::Context) -> bool {
        let mut needs_repaint = false;

        while let Ok(msg) = self.load_rx.try_recv() {
            needs_repaint = true;
            match msg {
                LoadMessage::StoreReady(Ok(store)) => {
                    self.status_message = format!("Loaded {}", store.root_path);
                    self.store = Some(Arc::new(store));
                    self.select_node(&self.store.as_ref().unwrap().tree.root.clone());
                }
                LoadMessage::StoreReady(Err(err)) => {
                    self.status_message = err.clone();
                    self.info_title = "Error".to_string();
                    self.info_text = err;
                }
                LoadMessage::PlotReady { path, result } => {
                    if self.pending_plot_path.as_deref() != Some(path.as_str()) {
                        continue;
                    }
                    match result {
                        Ok(data) => self.plot_panel.set_plot_data(data),
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

    fn tree_ui(&mut self, ui: &mut egui::Ui, node: &ZarrTreeNode) {
        if node.path == "/" {
            for child in &node.children {
                self.tree_ui(ui, child);
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

        if node.children.is_empty() {
            let selected = self.selected_path.as_deref() == Some(node.path.as_str());
            let response = ui.selectable_label(selected, label);
            if response.clicked() || response.double_clicked() {
                self.select_node(node);
            }
        } else {
            let default_open = node.path == "/measurements" || node.path == "/conditions";
            let response = egui::CollapsingHeader::new(label)
                .id_salt(&node.path)
                .default_open(default_open)
                .show(ui, |ui| {
                    for child in &node.children {
                        self.tree_ui(ui, child);
                    }
                });

            if response.header_response.double_clicked() {
                self.select_node(node);
            }
        }
    }
}

impl eframe::App for CopernicusViewer {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Skip rendering while the window is minimized or mid-resize (zero-area viewport).
        let area = ui.clip_rect().size();
        if area.x < 8.0 || area.y < 8.0 {
            return;
        }

        self.poll_background_tasks(ui.ctx());

        egui::Panel::top("menu").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open Zarr…").clicked() {
                        self.open_file_dialog();
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
                        if let Some(store) = &self.store {
                            let root = store.tree.root.clone();
                            self.tree_ui(ui, &root);
                        } else {
                            ui.label("No product loaded.");
                        }
                    });
            });

        egui::Panel::left("inspector_panel")
            .default_size(380.0)
            .resizable(true)
            .show_inside(ui, |ui| {
                ui.heading("Inspector");
                ui.separator();
                ui.monospace(&self.info_title);
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.info_text.as_str())
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                    });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let ctx = ui.ctx().clone();
            self.plot_panel.ui(ui, &ctx);
        });
    }
}
