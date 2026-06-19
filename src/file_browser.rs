use std::path::{Path, PathBuf};

use copernicus_viewer::zarr::{format_s3_uri, parent_prefix, parse_product_location, ProductLocation};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserLocation {
    Local(PathBuf),
    S3Root,
    S3 { bucket: String, prefix: String },
}

#[derive(Clone, Debug)]
pub enum BrowserItem {
    Directory {
        name: String,
        location: String,
        zarr_product: bool,
    },
    ZipArchive {
        name: String,
        location: String,
    },
}

impl BrowserItem {
    pub fn location(&self) -> &str {
        match self {
            BrowserItem::Directory { location, .. } | BrowserItem::ZipArchive { location, .. } => {
                location
            }
        }
    }
}

impl BrowserLocation {
    pub fn is_s3(&self) -> bool {
        matches!(self, Self::S3Root | Self::S3 { .. })
    }

    pub fn display_label(&self) -> String {
        match self {
            Self::Local(path) => path.display().to_string(),
            Self::S3Root => "s3:// (configured buckets)".to_string(),
            Self::S3 { bucket, prefix } => format_s3_uri(bucket, prefix),
        }
    }

    pub fn from_path_hint(hint: &str) -> Option<Self> {
        let trimmed = hint.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.starts_with("s3://") {
            let loc = parse_product_location(trimmed).ok()?;
            return Some(match loc {
                ProductLocation::S3 { bucket, prefix } => Self::S3 { bucket, prefix },
                ProductLocation::Local(path) => Self::Local(path),
            });
        }
        Some(Self::Local(PathBuf::from(trimmed)))
    }

    pub fn go_up(&self) -> Option<Self> {
        match self {
            Self::Local(path) => path.parent().map(|p| Self::Local(p.to_path_buf())),
            Self::S3Root => None,
            Self::S3 { bucket, prefix } => {
                if prefix.is_empty() {
                    Some(Self::S3Root)
                } else {
                    parent_prefix(prefix).map(|parent| Self::S3 {
                        bucket: bucket.clone(),
                        prefix: parent,
                    })
                }
            }
        }
    }

    pub fn can_go_up(&self) -> bool {
        match self {
            Self::Local(path) => path.parent().is_some_and(|p| p.is_dir()),
            Self::S3Root => false,
            Self::S3 { .. } => true,
        }
    }
}

pub fn initial_browser_location(path_hint: &str, store_root: Option<&Path>) -> BrowserLocation {
    let trimmed = path_hint.trim();
    if trimmed.starts_with("s3://") {
        if let Some(loc) = initial_s3_browser_location(trimmed) {
            return loc;
        }
    }

    if let Some(root) = store_root {
        if let Some(s) = root.to_str()
            && s.starts_with("s3://")
            && let Some(loc) = initial_s3_browser_location(s)
        {
            return loc;
        }
    }

    BrowserLocation::Local(initial_browser_dir(path_hint, store_root))
}

fn initial_s3_browser_location(uri: &str) -> Option<BrowserLocation> {
    let loc = parse_product_location(uri).ok()?;
    let ProductLocation::S3 { bucket, prefix } = loc else {
        return None;
    };

    let browse_prefix = if prefix.ends_with(".zarr") {
        parent_prefix(&prefix).unwrap_or_default()
    } else {
        prefix
    };

    Some(BrowserLocation::S3 {
        bucket,
        prefix: browse_prefix,
    })
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
    if !trimmed.is_empty() && !trimmed.starts_with("s3://") {
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
                location: path.display().to_string(),
            });
        } else if is_zarr_zip(&name) {
            zips.push(BrowserItem::ZipArchive {
                name,
                location: path.display().to_string(),
            });
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

    #[test]
    fn from_path_hint_parses_s3_uri() {
        let loc = BrowserLocation::from_path_hint("s3://my-bucket/eopf/product.zarr").unwrap();
        assert_eq!(
            loc,
            BrowserLocation::S3 {
                bucket: "my-bucket".to_string(),
                prefix: "eopf/product.zarr".to_string(),
            }
        );
    }

    #[test]
    fn from_path_hint_parses_local_path() {
        let loc = BrowserLocation::from_path_hint("/data/product.zarr").unwrap();
        assert_eq!(loc, BrowserLocation::Local(PathBuf::from("/data/product.zarr")));
    }

    #[test]
    fn s3_go_up_from_prefix_to_bucket_root() {
        let loc = BrowserLocation::S3 {
            bucket: "b".to_string(),
            prefix: "eopf/product.zarr".to_string(),
        };
        assert_eq!(
            loc.go_up(),
            Some(BrowserLocation::S3 {
                bucket: "b".to_string(),
                prefix: "eopf".to_string(),
            })
        );
    }

    #[test]
    fn s3_go_up_from_bucket_root_to_s3_root() {
        let loc = BrowserLocation::S3 {
            bucket: "b".to_string(),
            prefix: String::new(),
        };
        assert_eq!(loc.go_up(), Some(BrowserLocation::S3Root));
    }

    #[test]
    fn initial_s3_location_uses_parent_of_zarr_product() {
        let loc = initial_browser_location(
            "s3://bucket/eopf/product.zarr",
            None,
        );
        assert_eq!(
            loc,
            BrowserLocation::S3 {
                bucket: "bucket".to_string(),
                prefix: "eopf".to_string(),
            }
        );
    }
}
