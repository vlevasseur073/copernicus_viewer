//! Download an S3-hosted Zarr product to a local `.zarr` directory.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::stream::{self, StreamExt, TryStreamExt};
use zarrs_object_store::object_store::{ObjectStore, ObjectStoreExt};

use super::creds::S3Config;
use super::error::IoError;
use super::location::{parse_product_location, resolve_product_location, s3_config_path, ProductLocation};
use super::runtime::shared_runtime;

const DOWNLOAD_CONCURRENCY: usize = 8;

/// Progress snapshot for an in-flight S3 product download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    /// Number of objects written so far.
    pub objects_done: usize,
    /// Total bytes written so far.
    pub bytes_done: u64,
    /// Key of the object most recently completed.
    pub current_key: String,
}

/// Callback invoked after each object is written locally.
pub type DownloadProgressCallback = Arc<dyn Fn(DownloadProgress) + Send + Sync>;

/// Returns true when `root_path` is an `s3://` product URI.
pub fn is_s3_product(root_path: &str) -> bool {
    root_path.starts_with("s3://")
}

/// Parse bucket and prefix from a canonical `s3://` product root path.
pub fn parse_s3_location(root_path: &str) -> Result<(String, String), IoError> {
    let loc = parse_product_location(root_path).map_err(|e| IoError::S3Download(e.to_string()))?;
    let resolved =
        resolve_product_location(loc).map_err(|e| IoError::S3Download(e.to_string()))?;
    match resolved.location {
        ProductLocation::S3 { bucket, prefix } => Ok((bucket, prefix)),
        ProductLocation::Local(_) => Err(IoError::S3Download("not an S3 product".into())),
    }
}

/// Download an S3 Zarr product into `dest_parent` / `<product-name>.zarr`.
///
/// Fails if the destination directory already exists. Reports progress after
/// each object via `progress` when provided.
pub fn download_s3_product(
    bucket: &str,
    prefix: &str,
    dest_parent: &Path,
    progress: Option<DownloadProgressCallback>,
) -> Result<PathBuf, IoError> {
    let bucket = bucket.to_string();
    let prefix = prefix.to_string();
    let dest_parent = dest_parent.to_path_buf();
    shared_runtime().block_on(async move {
        download_s3_product_async(&bucket, &prefix, &dest_parent, progress).await
    })
}

fn product_folder_name(prefix: &str) -> Result<String, IoError> {
    let name = prefix
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| IoError::S3Download("empty S3 prefix".into()))?;
    Ok(name.to_string())
}

async fn download_s3_product_async(
    bucket: &str,
    prefix: &str,
    dest_parent: &Path,
    progress: Option<DownloadProgressCallback>,
) -> Result<PathBuf, IoError> {
    let product_name = product_folder_name(prefix)?;
    let dest = dest_parent.join(&product_name);
    if dest.exists() {
        return Err(IoError::S3Download(format!(
            "destination already exists: {}",
            dest.display()
        )));
    }

    let config = S3Config::resolve(bucket, s3_config_path().as_deref())?;
    let store = config.build_prefixed_s3_client(bucket, prefix)?;

    std::fs::create_dir_all(&dest)?;

    let mut objects = Vec::new();
    let mut list_stream = store.list(None);
    while let Some(item) = list_stream.next().await {
        objects.push(
            item.map_err(|e| IoError::S3Download(format!("failed to list s3://{bucket}/{prefix}: {e}")))?,
        );
    }

    let counters = Arc::new(Mutex::new((0usize, 0u64)));

    stream::iter(objects)
        .map(|meta| {
            let store = store.clone();
            let dest = dest.clone();
            let counters = Arc::clone(&counters);
            let progress = progress.clone();
            async move {
                let key = meta.location.as_ref().to_string();
                let local_path = dest.join(&key);
                if let Some(parent) = local_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                let payload = store
                    .get(&meta.location)
                    .await
                    .map_err(|e| IoError::S3Download(format!("failed to get {key}: {e}")))?;
                let bytes = payload
                    .bytes()
                    .await
                    .map_err(|e| IoError::S3Download(format!("failed to read {key}: {e}")))?;
                let nbytes = bytes.len() as u64;
                std::fs::write(&local_path, &bytes)?;

                if let Some(callback) = progress {
                    let mut state = counters.lock().expect("download progress lock");
                    state.0 += 1;
                    state.1 += nbytes;
                    callback(DownloadProgress {
                        objects_done: state.0,
                        bytes_done: state.1,
                        current_key: key,
                    });
                }

                Ok::<(), IoError>(())
            }
        })
        .buffer_unordered(DOWNLOAD_CONCURRENCY)
        .try_collect::<Vec<()>>()
        .await?;

    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_folder_name_from_prefix() {
        assert_eq!(
            product_folder_name("eopf/products/S03OLCEFR_202309.zarr").unwrap(),
            "S03OLCEFR_202309.zarr"
        );
        assert_eq!(product_folder_name("product.zarr/").unwrap(), "product.zarr");
    }

    #[test]
    fn rejects_empty_prefix() {
        assert!(product_folder_name("").is_err());
    }

    #[test]
    fn is_s3_product_detects_uri() {
        assert!(is_s3_product("s3://bucket/product.zarr"));
        assert!(!is_s3_product("/data/product.zarr"));
    }

    #[test]
    fn rejects_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let product_dir = dir.path().join("product.zarr");
        std::fs::create_dir(&product_dir).unwrap();

        let err = download_s3_product("bucket", "product.zarr", dir.path(), None).unwrap_err();
        assert!(matches!(err, IoError::S3Download(_)));
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    #[ignore = "requires S3_TEST_URI and credentials in ~/.config/cp-rs/s3.conf or env"]
    fn download_s3_product_integration() {
        let uri = std::env::var("S3_TEST_URI").expect("S3_TEST_URI not set");
        let (bucket, prefix) = parse_s3_location(&uri).expect("parse");
        let dest_parent = tempfile::tempdir().expect("tempdir");
        let dest = download_s3_product(&bucket, &prefix, dest_parent.path(), None).expect("download");
        assert!(dest.join(".zgroup").exists() || dest.join(".zmetadata").exists());
    }
}
