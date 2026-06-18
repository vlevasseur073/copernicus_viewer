mod app;
mod demo_capture;
mod file_browser;
mod platform;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    platform::init();
    env_logger::init();
    platform::log_startup();

    let paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();

    eframe::run_native(
        "Copernicus Viewer",
        platform::native_options(),
        Box::new(move |cc| {
            platform::configure_egui(cc);
            Ok(Box::new(app::CopernicusViewer::new(paths.clone())))
        }),
    )
}
