mod app;
mod platform;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    platform::init();
    env_logger::init();

    if platform::is_wsl() {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            log::info!("WSLg/Wayland detected — using native GPU windowing");
        } else {
            log::info!(
                "WSL X11 detected — OpenGL software rendering enabled (set COPERNICUS_VIEWER_GL=hardware to override)"
            );
        }
    }

    let path = std::env::args().nth(1).map(PathBuf::from);

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        hardware_acceleration: platform::hardware_acceleration(),
        vsync: platform::vsync_enabled(),
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 480.0])
            .with_title("Copernicus Viewer — EOPF Zarr"),
        ..Default::default()
    };

    eframe::run_native(
        "Copernicus Viewer",
        options,
        Box::new(move |cc| Ok(Box::new(app::CopernicusViewer::new(cc, path.clone())))),
    )
}
