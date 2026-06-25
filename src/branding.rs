//! Application logo embedded from `assets/copernicus_viewer_logo.png`.

use eframe::egui::{self, ColorImage, IconData};

// const LOGO_PNG: &[u8] = include_bytes!("../assets/copernicus_viewer_logo.png");
const LOGO_PNG: &[u8] = include_bytes!("../assets/cp-viewer-icon-logo.jpg");
const APP_LOGO_TEXTURE_ID: &str = "copernicus_viewer_logo";
pub const APP_ID: &str = "io.github.vlevasseur073.CopernicusViewer";

const ICON_SIZES: [u32; 5] = [32, 48, 64, 128, 256];
const WINDOW_ICON_SIZE: u32 = ICON_SIZES[4];

fn logo_rgba_image() -> image::RgbaImage {
    image::load_from_memory(LOGO_PNG)
        .expect("failed to decode application logo")
        .into_rgba8()
}

fn square_source() -> image::RgbaImage {
    let img = logo_rgba_image();
    let (width, height) = img.dimensions();
    let side = width.min(height);
    let x = (width - side) / 2;
    let y = (height - side) / 2;
    image::imageops::crop_imm(&img, x, y, side, side).to_image()
}

fn resize_square(source: &image::RgbaImage, size: u32) -> image::RgbaImage {
    image::imageops::resize(source, size, size, image::imageops::FilterType::Lanczos3)
}

/// Window and taskbar icon for the native viewport (X11 / Windows).
pub fn window_icon() -> IconData {
    let resized = resize_square(&square_source(), WINDOW_ICON_SIZE);
    IconData {
        width: WINDOW_ICON_SIZE,
        height: WINDOW_ICON_SIZE,
        rgba: resized.into_raw(),
    }
}

/// Load (or reuse) the logo texture for in-app UI.
pub fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    ctx.data(|data| data.get_temp::<egui::TextureHandle>(egui::Id::new(APP_LOGO_TEXTURE_ID)))
        .unwrap_or_else(|| {
            let img = logo_rgba_image();
            let (width, height) = img.dimensions();
            let color_image = ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                &img.into_raw(),
            );
            let texture = ctx.load_texture(
                APP_LOGO_TEXTURE_ID,
                color_image,
                egui::TextureOptions::LINEAR,
            );
            ctx.data_mut(|data| {
                data.insert_temp(egui::Id::new(APP_LOGO_TEXTURE_ID), texture.clone());
            });
            texture
        })
}

/// On GNOME/Wayland the title-bar icon comes from a matching `.desktop` entry,
/// not from `set_window_icon`. Install/update one under `~/.local/share`.
#[cfg(target_os = "linux")]
pub fn ensure_linux_desktop_integration() {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };

    let apps_dir = home.join(".local/share/applications");
    let icons_root = home.join(".local/share/icons/hicolor");
    let desktop_path = apps_dir.join(format!("{APP_ID}.desktop"));
    let icon_basename = format!("{APP_ID}.png");

    if fs::create_dir_all(&apps_dir).is_err() {
        return;
    }

    ensure_hicolor_theme(&icons_root);

    let source = square_source();
    for size in ICON_SIZES {
        let dir = icons_root.join(format!("{size}x{size}/apps"));
        if fs::create_dir_all(&dir).is_err() {
            return;
        }
        let path = dir.join(&icon_basename);
        if resize_square(&source, size).save(&path).is_err() {
            return;
        }
    }

    let exec = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "copernicus_viewer".to_string());

    let desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Copernicus Viewer\n\
         GenericName=EOPF Zarr viewer\n\
         Comment=GUI viewer for Copernicus EOPF Zarr products\n\
         Exec={exec} %F\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=Science;Education;\n\
         StartupNotify=true\n\
         StartupWMClass={APP_ID}\n"
    );

    if fs::File::create(&desktop_path)
        .and_then(|mut file| file.write_all(desktop.as_bytes()))
        .is_err()
    {
        return;
    }

    refresh_linux_icon_cache(&icons_root);
    refresh_linux_desktop_database(&apps_dir);
}

#[cfg(target_os = "linux")]
fn ensure_hicolor_theme(icons_root: &std::path::Path) {
    use std::fs;

    let index_path = icons_root.join("index.theme");
    if index_path.is_file() {
        return;
    }

    if fs::create_dir_all(icons_root).is_err() {
        return;
    }

    let directories = ICON_SIZES
        .iter()
        .map(|size| format!("{size}x{size}/apps"))
        .collect::<Vec<_>>()
        .join(",");

    let theme = format!(
        "[Icon Theme]\n\
         Name=Hicolor\n\
         Comment=Fallback icon theme\n\
         Directories={directories}\n\n"
    );

    let mut body = theme;
    for size in ICON_SIZES {
        body.push_str(&format!(
            "[{size}x{size}/apps]\n\
             Size={size}\n\
             Type=Fixed\n\n"
        ));
    }

    let _ = fs::write(index_path, body);
}

#[cfg(target_os = "linux")]
fn refresh_linux_icon_cache(icons_root: &std::path::Path) {
    use std::process::Command;

    let _ = Command::new("gtk-update-icon-cache")
        .arg("-f")
        .arg("-t")
        .arg(icons_root)
        .status();
}

#[cfg(target_os = "linux")]
fn refresh_linux_desktop_database(apps_dir: &std::path::Path) {
    use std::process::Command;

    let _ = Command::new("update-desktop-database")
        .arg(apps_dir)
        .status();
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_linux_desktop_integration() {}
