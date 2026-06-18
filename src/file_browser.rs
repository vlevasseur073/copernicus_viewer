use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum BrowserItem {
    Directory {
        name: String,
        path: PathBuf,
        zarr_product: bool,
    },
    ZipArchive {
        name: String,
        path: PathBuf,
    },
}

pub fn initial_browser_dir(path_hint: &str, store_root: Option<&Path>) -> PathBuf {
    if let Some(root) = store_root {
        if root.is_dir() {
            if is_zarr_product_dir(
                root.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                root,
            ) {
                if let Some(parent) = root.parent() {
                    if parent.is_dir() {
                        return parent.to_path_buf();
                    }
                }
            }
            return root.to_path_buf();
        }
        if let Some(parent) = root.parent() {
            if parent.is_dir() {
                return parent.to_path_buf();
            }
        }
    }

    let trimmed = path_hint.trim();
    if !trimmed.is_empty() {
        let path = PathBuf::from(trimmed);
        if path.is_dir() {
            if is_zarr_product_dir(
                path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                &path,
            ) {
                if let Some(parent) = path.parent() {
                    if parent.is_dir() {
                        return parent.to_path_buf();
                    }
                }
            }
            return path;
        }
        if let Some(parent) = path.parent() {
            if parent.is_dir() {
                return parent.to_path_buf();
            }
        }
    }

    home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn list_directory(dir: &Path) -> Result<Vec<BrowserItem>, String> {
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", dir.display()));
    }

    let read_dir =
        std::fs::read_dir(dir).map_err(|err| format!("Cannot read {}: {err}", dir.display()))?;

    let mut dirs = Vec::new();
    let mut zips = Vec::new();

    for entry in read_dir {
        let entry = entry.map_err(|err| format!("Cannot read directory entry: {err}"))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        if name.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            dirs.push(BrowserItem::Directory {
                zarr_product: is_zarr_product_dir(&name, &path),
                name,
                path,
            });
        } else if is_zarr_zip(&name) {
            zips.push(BrowserItem::ZipArchive { name, path });
        }
    }

    dirs.sort_by(|a, b| match (a, b) {
        (
            BrowserItem::Directory {
                zarr_product: za,
                name: a,
                ..
            },
            BrowserItem::Directory {
                zarr_product: zb,
                name: b,
                ..
            },
        ) => zb.cmp(za).then_with(|| a.cmp(b)),
        _ => std::cmp::Ordering::Equal,
    });
    zips.sort_by(|a, b| match (a, b) {
        (BrowserItem::ZipArchive { name: a, .. }, BrowserItem::ZipArchive { name: b, .. }) => {
            a.cmp(b)
        }
        _ => std::cmp::Ordering::Equal,
    });

    let mut items = dirs;
    items.extend(zips);
    Ok(items)
}

pub fn is_zarr_product_dir(name: &str, path: &Path) -> bool {
    name.ends_with(".zarr") || path.join(".zgroup").exists() || path.join(".zmetadata").exists()
}

pub fn is_zarr_zip(name: &str) -> bool {
    name.ends_with(".zarr.zip") || name.ends_with(".zip")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_zarr_product_by_suffix() {
        assert!(is_zarr_product_dir(
            "sample.zarr",
            Path::new("/tmp/sample.zarr")
        ));
        assert!(!is_zarr_product_dir("data", Path::new("/tmp/data")));
    }

    #[test]
    fn prefers_home_or_root_for_empty_hint() {
        let dir = initial_browser_dir("", None);
        assert!(dir.is_absolute());
    }
}
