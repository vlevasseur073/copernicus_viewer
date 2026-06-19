use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use zarrs_object_store::object_store::ObjectStoreExt;
use zarrs_object_store::object_store::path::Path as ObjectPath;

use super::creds::S3Config;
use super::runtime::shared_runtime;

/// Normalize a user-selected path to the Zarr product root (.zarr directory or .zarr.zip file).
pub fn resolve_zarr_product_path(path: &Path) -> PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }

    let mut current = path.to_path_buf();
    loop {
        if is_local_zarr_root(&current) {
            return current;
        }
        if current
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".zarr"))
        {
            return current;
        }
        match current.parent() {
            Some(parent) if parent != current.as_path() => current = parent.to_path_buf(),
            _ => return path.to_path_buf(),
        }
    }
}

fn is_local_zarr_root(path: &Path) -> bool {
    path.is_dir() && (path.join(".zgroup").exists() || path.join(".zmetadata").exists())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductLocation {
    Local(PathBuf),
    S3 { bucket: String, prefix: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProduct {
    pub canonical_id: String,
    pub location: ProductLocation,
}

/// Parse a user-provided product location string (local path or `s3://` URI).
pub fn parse_product_location(input: &str) -> Result<ProductLocation> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("product location cannot be empty");
    }

    if let Some(rest) = trimmed.strip_prefix("s3://") {
        return parse_s3_uri(rest);
    }

    Ok(ProductLocation::Local(PathBuf::from(trimmed)))
}

/// Normalize a product location to its Zarr root and canonical identifier.
pub fn resolve_product_location(loc: ProductLocation) -> Result<ResolvedProduct> {
    match loc {
        ProductLocation::Local(path) => {
            let resolved = resolve_zarr_product_path(&path);
            Ok(ResolvedProduct {
                canonical_id: resolved.display().to_string(),
                location: ProductLocation::Local(resolved),
            })
        }
        ProductLocation::S3 { bucket, prefix } => {
            let resolved_prefix = resolve_s3_zarr_root(&bucket, &prefix)?;
            let canonical_id = format_s3_uri(&bucket, &resolved_prefix);
            Ok(ResolvedProduct {
                canonical_id,
                location: ProductLocation::S3 {
                    bucket,
                    prefix: resolved_prefix,
                },
            })
        }
    }
}

fn parse_s3_uri(rest: &str) -> Result<ProductLocation> {
    if rest.is_empty() {
        bail!("invalid s3 URI: missing bucket name");
    }

    let (bucket, prefix) = match rest.split_once('/') {
        Some((bucket, prefix)) if !bucket.is_empty() => (bucket, normalize_prefix(prefix)),
        Some(_) => bail!("invalid s3 URI: missing bucket name"),
        None => (rest, String::new()),
    };

    Ok(ProductLocation::S3 {
        bucket: bucket.to_string(),
        prefix,
    })
}

fn normalize_prefix(prefix: &str) -> String {
    prefix.trim_matches('/').to_string()
}

pub fn format_s3_uri(bucket: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        format!("s3://{bucket}")
    } else {
        format!("s3://{bucket}/{prefix}")
    }
}

pub fn parent_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return None;
    }
    match prefix.rsplit_once('/') {
        Some(("", _)) => Some(String::new()),
        Some((parent, _)) => Some(parent.to_string()),
        None => Some(String::new()),
    }
}

pub fn s3_config_path() -> Option<PathBuf> {
    for var in ["COPERNICUS_VIEWER_S3_CONFIG", "S3_CONFIG"] {
        if let Ok(path) = std::env::var(var) {
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

fn resolve_s3_zarr_root(bucket: &str, prefix: &str) -> Result<String> {
    let config = S3Config::resolve(bucket, s3_config_path().as_deref())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let runtime = shared_runtime();

    let mut current = prefix.to_string();
    loop {
        if current.ends_with(".zarr") {
            return Ok(current);
        }

        let is_root = runtime
            .block_on(s3_prefix_is_zarr_root(&config, bucket, &current))
            .with_context(|| format!("failed to probe s3://{bucket}/{current}"))?;
        if is_root {
            return Ok(current);
        }

        match parent_prefix(&current) {
            Some(parent) => current = parent,
            None => bail!(
                "could not find zarr root under s3://{bucket}/{prefix} \
                 (no .zgroup, .zmetadata, or .zarr suffix found)"
            ),
        }
    }
}

async fn s3_prefix_is_zarr_root(config: &S3Config, bucket: &str, prefix: &str) -> Result<bool> {
    let store = config
        .build_prefixed_s3_client(bucket, prefix)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    for marker in [".zgroup", ".zmetadata"] {
        match store.head(&ObjectPath::from(marker)).await {
            Ok(_) => return Ok(true),
            Err(zarrs_object_store::object_store::Error::NotFound { .. }) => {}
            Err(err) => {
                return Err(anyhow::anyhow!(
                    "failed to check for {marker} at s3://{bucket}/{prefix}: {err}"
                ));
            }
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_s3_uri_with_prefix() {
        let loc = parse_product_location("s3://my-bucket/eopf/products/S03.zarr/measurements")
            .unwrap();
        assert_eq!(
            loc,
            ProductLocation::S3 {
                bucket: "my-bucket".to_string(),
                prefix: "eopf/products/S03.zarr/measurements".to_string(),
            }
        );
    }

    #[test]
    fn parses_s3_uri_strips_trailing_slash() {
        let loc = parse_product_location("s3://my-bucket/eopf/product.zarr/").unwrap();
        assert_eq!(
            loc,
            ProductLocation::S3 {
                bucket: "my-bucket".to_string(),
                prefix: "eopf/product.zarr".to_string(),
            }
        );
    }

    #[test]
    fn parses_s3_uri_bucket_only() {
        let loc = parse_product_location("s3://my-bucket").unwrap();
        assert_eq!(
            loc,
            ProductLocation::S3 {
                bucket: "my-bucket".to_string(),
                prefix: String::new(),
            }
        );
    }

    #[test]
    fn parses_local_path() {
        let loc = parse_product_location("/data/product.zarr").unwrap();
        assert_eq!(
            loc,
            ProductLocation::Local(PathBuf::from("/data/product.zarr"))
        );
    }

    #[test]
    fn rejects_empty_location() {
        assert!(parse_product_location("  ").is_err());
    }

    #[test]
    fn rejects_malformed_s3_uri() {
        assert!(parse_product_location("s3://").is_err());
        assert!(parse_product_location("s3:///no-bucket/key").is_err());
    }

    #[test]
    fn parent_prefix_walks_up() {
        assert_eq!(
            parent_prefix("a/b/c.zarr"),
            Some("a/b".to_string())
        );
        assert_eq!(parent_prefix("product.zarr"), Some(String::new()));
        assert_eq!(parent_prefix(""), None);
    }

    #[test]
    fn format_s3_uri_omits_trailing_slash() {
        assert_eq!(format_s3_uri("b", "p.zarr"), "s3://b/p.zarr");
        assert_eq!(format_s3_uri("b", ""), "s3://b");
    }

    #[test]
    fn resolves_local_nested_path() {
        let root = PathBuf::from("/data/product.zarr");
        let resolved = resolve_product_location(ProductLocation::Local(
            root.join("measurements/image"),
        ))
        .unwrap();
        assert_eq!(resolved.canonical_id, root.display().to_string());
        assert_eq!(resolved.location, ProductLocation::Local(root));
    }

    #[test]
    #[ignore = "requires S3_TEST_URI and credentials in ~/.config/cp-rs/s3.conf or env"]
    fn opens_s3_product_when_configured() {
        let uri = std::env::var("S3_TEST_URI").expect("S3_TEST_URI not set");
        let loc = parse_product_location(&uri).expect("parse uri");
        let resolved = resolve_product_location(loc).expect("resolve");
        assert!(resolved.canonical_id.starts_with("s3://"));
    }
}
