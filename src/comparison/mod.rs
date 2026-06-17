//! Side-by-side and difference views across open EOPF Zarr products.

use eframe::egui;

/// Comparison workspace (stub — full implementation pending).
#[derive(Default)]
pub struct ComparisonTool {
    open: bool,
}

impl ComparisonTool {
    pub fn show(&mut self) {
        self.open = true;
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }

        let mut keep_open = self.open;
        egui::Window::new("Comparison")
            .collapsible(true)
            .resizable(true)
            .default_width(480.0)
            .default_height(320.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut keep_open)
            .show(ctx, |ui| {
                ui.heading("Comparison");
                ui.separator();
                ui.label("Compare variables across open products.");
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("This tool is not implemented yet.")
                        .weak()
                        .italics(),
                );
            });
        self.open = keep_open;
    }
}
