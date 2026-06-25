//! Copernicus Viewer — GUI binary entry point.
//!
//! Initializes the platform layer, logging, and egui/eframe, then runs
//! [`app::CopernicusViewer`] with optional product paths from the command line.

mod app;
mod branding;
mod demo_capture;
mod file_browser;
mod help_dialog;
mod platform;
mod s3_browser;
mod s3_config_dialog;

fn main() -> eframe::Result<()> {
    platform::init();
    env_logger::init();
    platform::log_startup();

    let locations: Vec<String> = std::env::args().skip(1).collect();

    eframe::run_native(
        "Copernicus Viewer",
        platform::native_options(),
        Box::new(move |cc| {
            platform::configure_egui(cc);
            Ok(Box::new(app::CopernicusViewer::new(locations.clone())))
        }),
    )
}
