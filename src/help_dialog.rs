//! About dialog: project description, contact, and application logo.

use eframe::egui;

const APP_DESCRIPTION: &str = "Copernicus Viewer is a lightweight GUI for exploring and \
    visualizing Earth observation products from the Copernicus ecosystem, with a focus on \
    EOPF Zarr stores (.zarr directories, .zarr.zip archives, and s3:// URIs).";

const AUTHOR_NAME: &str = "Vincent Levasseur";
const AUTHOR_EMAIL: &str = "vince.levasseur@protonmail.com";
const REPOSITORY_URL: &str = env!("CARGO_PKG_REPOSITORY");

#[derive(Default)]
pub struct HelpDialog {
    open: bool,
}

impl HelpDialog {
    pub fn show(&mut self) {
        self.open = true;
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }

        let viewport = ctx.input(|input| {
            input
                .viewport()
                .inner_rect
                .unwrap_or_else(|| input.content_rect())
        });
        let max_size = egui::vec2(
            (viewport.width() - 48.0).clamp(360.0, viewport.width()),
            (viewport.height() - 48.0).clamp(280.0, viewport.height()),
        );
        let default_size = egui::vec2(420.0_f32.min(max_size.x), 520.0_f32.min(max_size.y));

        let mut keep_open = self.open;
        let mut dismiss = false;
        egui::Window::new("About Copernicus Viewer")
            .collapsible(false)
            .resizable(false)
            .default_size(default_size)
            .max_size(max_size)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut keep_open)
            .show(ctx, |ui| {
                ui.set_width(ui.available_width());

                ui.vertical_centered(|ui| {
                    let texture = crate::branding::logo_texture(ui.ctx());
                    let size = texture.size_vec2();
                    let max_height = 128.0;
                    let scale = (max_height / size.y).min(1.0);
                    ui.add(
                        egui::Image::new((texture.id(), size * scale))
                            .fit_to_exact_size(size * scale),
                    );
                    ui.heading("Copernicus Viewer");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                });

                ui.add_space(12.0);
                ui.label(APP_DESCRIPTION);
                ui.add_space(12.0);
                ui.label(format!("Author: {AUTHOR_NAME}"));
                ui.horizontal(|ui| {
                    ui.label("Contact:");
                    ui.hyperlink_to(AUTHOR_EMAIL, format!("mailto:{AUTHOR_EMAIL}"));
                });
                ui.hyperlink_to("Source repository", REPOSITORY_URL);
                ui.add_space(12.0);

                ui.with_layout(egui::Layout::top_down_justified(egui::Align::RIGHT), |ui| {
                    if ui.button("Close").clicked() {
                        dismiss = true;
                    }
                });
            });

        self.open = keep_open && !dismiss;
    }
}
