//! S3 prefix listing for the in-app open-product browser.

use crate::file_browser::BrowserItem;
use copernicus_viewer::zarr::creds::S3Config;
use copernicus_viewer::zarr::location::{format_s3_uri, s3_config_path};
use copernicus_viewer::zarr::runtime::shared_runtime;
use copernicus_viewer::zarr::error::IoError;

use zarrs_object_store::object_store::path::Path as ObjectPath;
use zarrs_object_store::object_store::ObjectStore;

/// List browser entries for a local directory, S3 bucket root, or S3 prefix.
pub fn list_browser_items(
    location: &crate::file_browser::BrowserLocation,
) -> Result<Vec<BrowserItem>, String> {
    match location {
        crate::file_browser::BrowserLocation::S3Root => list_configured_buckets(),
        crate::file_browser::BrowserLocation::S3 { bucket, prefix } => {
            list_s3_prefix(bucket, prefix)
        }
        crate::file_browser::BrowserLocation::Local(path) => {
            crate::file_browser::list_directory(path)
        }
    }
}

fn list_configured_buckets() -> Result<Vec<BrowserItem>, String> {
    let buckets = S3Config::list_configured_buckets(s3_config_path().as_deref())
        .map_err(|e: IoError| e.to_string())?;

    let mut items: Vec<BrowserItem> = buckets
        .into_iter()
        .map(|name| BrowserItem::Directory {
            name: name.clone(),
            location: format_s3_uri(&name, ""),
            zarr_product: false,
        })
        .collect();

    items.sort_by(|a, b| match (a, b) {
        (
            BrowserItem::Directory { name: a, .. },
            BrowserItem::Directory { name: b, .. },
        ) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    });

    Ok(items)
}

fn list_s3_prefix(bucket: &str, prefix: &str) -> Result<Vec<BrowserItem>, String> {
    let bucket = bucket.to_string();
    let prefix = prefix.to_string();
    shared_runtime().block_on(async move { list_s3_prefix_async(&bucket, &prefix).await })
}

async fn list_s3_prefix_async(bucket: &str, prefix: &str) -> Result<Vec<BrowserItem>, String> {
    let config = S3Config::resolve(bucket, s3_config_path().as_deref())
        .map_err(|e: IoError| e.to_string())?;
    let store = config
        .build_prefixed_s3_client(bucket, prefix)
        .map_err(|e: IoError| e.to_string())?;

    let listing = store
        .list_with_delimiter(Some(&ObjectPath::from("")))
        .await
        .map_err(|e| format!("failed to list s3://{bucket}/{prefix}: {e}"))?;

    let mut items = Vec::new();
    for child_prefix in listing.common_prefixes {
        let child_path = child_prefix.as_ref();
        let name = child_path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(child_path)
            .to_string();

        if name.is_empty() || name.starts_with('.') {
            continue;
        }

        let child_prefix = join_prefix(prefix, &name);
        items.push(BrowserItem::Directory {
            zarr_product: name.ends_with(".zarr"),
            name,
            location: format_s3_uri(bucket, &child_prefix),
        });
    }

    items.sort_by(|a, b| match (a, b) {
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

    Ok(items)
}

fn join_prefix(base: &str, child: &str) -> String {
    if base.is_empty() {
        child.to_string()
    } else {
        format!("{base}/{child}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_prefix_handles_empty_base() {
        assert_eq!(join_prefix("", "foo"), "foo");
        assert_eq!(join_prefix("a", "b"), "a/b");
    }

    #[test]
    #[ignore = "requires S3_TEST_URI and credentials in ~/.config/cp-rs/s3.conf or env"]
    fn list_s3_prefix_integration() {
        let uri = std::env::var("S3_TEST_URI").expect("S3_TEST_URI not set");
        let loc = copernicus_viewer::zarr::parse_product_location(&uri).expect("parse");
        let copernicus_viewer::zarr::ProductLocation::S3 { bucket, prefix } = loc else {
            panic!("expected s3 location");
        };
        let parent = copernicus_viewer::zarr::parent_prefix(&prefix).unwrap_or_default();
        let items = list_s3_prefix(&bucket, &parent).expect("list");
        assert!(!items.is_empty());
    }
}
