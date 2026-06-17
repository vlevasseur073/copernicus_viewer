//! Platform-specific setup, mainly for WSL2 / remote X11 stability.

use std::path::PathBuf;

/// Configure OpenGL/windowing before eframe initializes glutin.
///
/// Override with `COPERNICUS_VIEWER_GL`:
/// - `software` — force Mesa llvmpipe (recommended on WSL2 + X11)
/// - `hardware` — leave driver selection to the system
/// - `auto`   — software on WSL+X11, hardware on WSLg/Wayland (default)
pub fn init() {
    check_linux_windowing_deps();

    if !use_wayland() && should_use_software_gl() {
        force_env("LIBGL_ALWAYS_SOFTWARE", "1");
        force_env("GALLIUM_DRIVER", "llvmpipe");
        force_env("MESA_LOADER_DRIVER_OVERRIDE", "llvmpipe");
    }
}

pub fn hardware_acceleration() -> eframe::HardwareAcceleration {
    match std::env::var("COPERNICUS_VIEWER_GL").as_deref() {
        Ok("software") => eframe::HardwareAcceleration::Off,
        Ok("hardware") => eframe::HardwareAcceleration::Preferred,
        _ if use_wayland() => eframe::HardwareAcceleration::Preferred,
        _ if is_wsl() => eframe::HardwareAcceleration::Off,
        _ => eframe::HardwareAcceleration::Preferred,
    }
}

pub fn vsync_enabled() -> bool {
    // Vsync + broken EGL drivers can destabilize resize on WSL2 over X11.
    !(is_wsl() && !use_wayland() && should_use_software_gl())
}

pub fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|v| v.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

fn should_use_software_gl() -> bool {
    match std::env::var("COPERNICUS_VIEWER_GL").as_deref() {
        Ok("software") => true,
        Ok("hardware") => false,
        Ok("auto") | Err(_) => is_wsl(),
        _ => is_wsl(),
    }
}

fn use_wayland() -> bool {
    env_nonempty("WAYLAND_DISPLAY")
        .or_else(|| env_nonempty("WAYLAND_SOCKET"))
        .is_some()
}

/// Fail early with a helpful message when running on X11 without libxkbcommon-x11.
fn check_linux_windowing_deps() {
    #[cfg(target_os = "linux")]
    {
        if use_wayland() {
            return;
        }

        if env_nonempty("DISPLAY").is_none() {
            return;
        }

        if find_library("libxkbcommon-x11.so.0").is_some() {
            return;
        }

        eprintln!(
            "\
Copernicus Viewer could not find libxkbcommon-x11 (required for X11 windowing).

On Ubuntu / Debian / WSL, install it with:
  sudo apt install libxkbcommon-x11-0

On WSLg, prefer the native Wayland session (WAYLAND_DISPLAY is usually set).
If you use an external X server, the package above is required.
"
        );
        std::process::exit(1);
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn find_library(name: &str) -> Option<PathBuf> {
    let dirs = [
        "/lib/x86_64-linux-gnu",
        "/usr/lib/x86_64-linux-gnu",
        "/lib/aarch64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
    ];
    for dir in dirs {
        let path = PathBuf::from(dir).join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn force_env(key: &str, value: &str) {
    // SAFETY: called once at process startup before any threads or GL init.
    unsafe {
        std::env::set_var(key, value);
    }
}

/// Which native picker to use for an EOPF Zarr product.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZarrNativePick {
    Directory,
    ZipArchive,
}

/// Infer the native picker from a path hint (typed path or current selection).
pub fn zarr_native_pick_for_hint(path_hint: &str) -> ZarrNativePick {
    let trimmed = path_hint.trim();
    if trimmed.ends_with(".zip") {
        ZarrNativePick::ZipArchive
    } else {
        ZarrNativePick::Directory
    }
}

/// Native picker for EOPF Zarr products. Must run on the main/UI thread.
#[cfg(not(target_arch = "wasm32"))]
pub fn pick_zarr_product(frame: &eframe::Frame, kind: ZarrNativePick) -> Option<PathBuf> {
    match kind {
        ZarrNativePick::Directory => rfd::FileDialog::new()
            .set_title("Select EOPF Zarr folder")
            .set_parent(frame)
            .pick_folder(),
        ZarrNativePick::ZipArchive => rfd::FileDialog::new()
            .set_title("Select EOPF Zarr zip archive")
            .set_parent(frame)
            .add_filter("Zarr zip archive", &["zip"])
            .pick_file(),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn pick_zarr_product(_frame: &eframe::Frame, _kind: ZarrNativePick) -> Option<PathBuf> {
    None
}
