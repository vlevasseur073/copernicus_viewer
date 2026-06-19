//! Native window, GPU backend, and egui rendering setup.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, IconData};

const APP_ID: &str = "eu.copernicus.copernicus-viewer";
const WINDOW_TITLE: &str = "Copernicus Viewer — EOPF Zarr";

/// OpenGL / GPU profile selected from the environment and host OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpuProfile {
    /// Mesa llvmpipe + Glow — WSL over X11 (stable).
    Software,
    /// Glow + WSLg GPU — default on WSL Wayland.
    WslGpu,
    /// wgpu — Linux desktop, macOS, and Windows.
    Native,
}

impl GpuProfile {
    fn detect() -> Self {
        match std::env::var("COPERNICUS_VIEWER_GL").as_deref() {
            Ok("software") => {
                if is_wsl() && use_wayland() {
                    // llvmpipe is incompatible with glutin on WSLg/Wayland.
                    Self::WslGpu
                } else {
                    Self::Software
                }
            }
            Ok("hardware") => {
                if is_wsl() {
                    Self::WslGpu
                } else {
                    Self::Native
                }
            }
            // Expert override: force wgpu even on WSL (may fail with EGL/ZINK).
            Ok("native") | Ok("wgpu") => Self::Native,
            Ok("auto") | Err(_) | _ => {
                if is_wsl() {
                    if use_wayland() {
                        Self::WslGpu
                    } else {
                        Self::Software
                    }
                } else {
                    Self::Native
                }
            }
        }
    }

    fn renderer(self) -> eframe::Renderer {
        match self {
            Self::Native => eframe::Renderer::Wgpu,
            Self::Software | Self::WslGpu => eframe::Renderer::Glow,
        }
    }

    fn hardware_acceleration(self) -> eframe::HardwareAcceleration {
        match self {
            Self::Software => eframe::HardwareAcceleration::Off,
            Self::Native | Self::WslGpu => eframe::HardwareAcceleration::Preferred,
        }
    }

    fn vsync(self) -> bool {
        // Vsync + broken EGL drivers can destabilize resize on WSL2 over X11.
        !matches!(self, Self::Software if is_wsl() && !use_wayland())
    }

    /// Mesa llvmpipe env vars only work on X11 — not on WSLg/Wayland.
    fn needs_mesa_software_env(self) -> bool {
        matches!(self, Self::Software) && !use_wayland()
    }
}

/// Configure the process environment before eframe / winit start.
///
/// Override with `COPERNICUS_VIEWER_GL`:
/// - `software` — Mesa llvmpipe + Glow on X11; on WSLg falls back to GPU Glow
/// - `hardware` — Glow with GPU on WSL; wgpu elsewhere
/// - `native` / `wgpu` — force wgpu (may fail on WSL with EGL errors)
/// - `auto` — WSLg: Glow+GPU, WSL X11: Glow+llvmpipe, else: wgpu
pub fn init() {
    check_linux_windowing_deps();

    let profile = GpuProfile::detect();

    if std::env::var("COPERNICUS_VIEWER_GL").as_deref() == Ok("software")
        && is_wsl()
        && use_wayland()
    {
        log::warn!(
            "COPERNICUS_VIEWER_GL=software is not supported on WSLg/Wayland; using GPU OpenGL (Glow)"
        );
    }

    if profile.needs_mesa_software_env() {
        force_env("LIBGL_ALWAYS_SOFTWARE", "1");
        force_env("GALLIUM_DRIVER", "llvmpipe");
        force_env("MESA_LOADER_DRIVER_OVERRIDE", "llvmpipe");
    }
}

/// Log the detected GPU / OpenGL profile at startup.
pub fn log_startup() {
    let profile = GpuProfile::detect();
    match profile {
        GpuProfile::Software if is_wsl() => {
            log::info!("WSL X11 — software OpenGL (Glow / Mesa llvmpipe)");
        }
        GpuProfile::Software => {
            log::info!("Software OpenGL rendering (Glow / Mesa llvmpipe)");
        }
        GpuProfile::WslGpu => {
            log::info!("WSLg — GPU OpenGL via Glow");
        }
        GpuProfile::Native => {
            log::info!("GPU rendering (wgpu)");
        }
    }
}

/// All native window and renderer options in one place.
pub fn native_options() -> eframe::NativeOptions {
    let profile = GpuProfile::detect();

    eframe::NativeOptions {
        renderer: profile.renderer(),
        hardware_acceleration: profile.hardware_acceleration(),
        vsync: profile.vsync(),
        centered: true,
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_ID)
            .with_title(WINDOW_TITLE)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 480.0])
            .with_icon(Arc::new(window_icon())),
        ..Default::default()
    }
}

/// Tune egui paint settings once the render context exists.
pub fn configure_egui(cc: &eframe::CreationContext<'_>) {
    let ctx = &cc.egui_ctx;

    ctx.options_mut(|options| {
        options.theme_preference = egui::ThemePreference::System;
        options.tessellation_options.feathering = true;
        options.tessellation_options.feathering_size_in_pixels = 1.0;
        options.tessellation_options.round_text_to_pixels = true;
        options.tessellation_options.round_line_segments_to_pixels = true;
        options.tessellation_options.round_rects_to_pixels = true;
    });
}

/// Returns `true` when running under WSL (Linux `/proc/version` contains "microsoft").
pub fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|v| v.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

fn use_wayland() -> bool {
    env_nonempty("WAYLAND_DISPLAY")
        .or_else(|| env_nonempty("WAYLAND_SOCKET"))
        .is_some()
}

/// Simple teal-on-slate icon for the window and taskbar.
fn window_icon() -> IconData {
    const SIZE: u32 = 64;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let center = (SIZE as f32 - 1.0) / 2.0;
    let radius = SIZE as f32 * 0.34;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();

            let (r, g, b) = if dist <= radius + 1.0 {
                let t = ((radius + 1.0 - dist) / 1.0).clamp(0.0, 1.0);
                (lerp_u8(30, 0, t), lerp_u8(34, 148, t), lerp_u8(40, 168, t))
            } else {
                (30, 34, 40)
            };

            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = 255;
        }
    }

    IconData {
        width: SIZE,
        height: SIZE,
        rgba,
    }
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

#[cfg(target_os = "linux")]
fn check_linux_windowing_deps() {
    if use_wayland() || env_nonempty("DISPLAY").is_none() {
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

#[cfg(not(target_os = "linux"))]
fn check_linux_windowing_deps() {}

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
/// Native folder picker for `.zarr` directories.
    Directory,
    /// Native file picker for `.zarr.zip` archives.
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
