use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tracing::debug;

use zarrs_object_store::object_store::aws::AmazonS3;

use crate::zarr::error::IoError;

/// S3 connection parameters resolved from config file or environment.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// Access key ID for the configured endpoint.
    pub access_key_id: String,
    /// Secret access key.
    pub secret_access_key: String,
    /// AWS or custom S3 region name.
    pub region: String,
    /// S3-compatible endpoint URL.
    pub endpoint: String,
}

/// S3 client scoped to a bucket prefix (re-exported alias for zarrs-object-store).
pub type PrefixedS3 = zarrs_object_store::object_store::prefix::PrefixStore<AmazonS3>;

impl S3Config {
    /// Resolve S3 credentials following the priority chain:
    ///
    /// 1. Explicit config file path
    /// 2. Default config at `%APPDATA%\cp-rs\s3.conf` (Windows) or
    ///    `~/.config/cp-rs/s3.conf` (Unix)
    /// 3. `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` / `S3_ENDPOINT` / `S3_REGION`
    /// 4. `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_ENDPOINT_URL` / `AWS_REGION`
    ///
    /// When reading from an INI file, `bucket` is used to select the
    /// matching `[section]`.  If no section matches the bucket name, the
    /// file is skipped and resolution continues with environment variables.
    pub fn resolve(bucket: &str, config_path: Option<&Path>) -> Result<Self, IoError> {
        if let Some(path) = config_path
            && let Some(cfg) = Self::from_ini_file(path, bucket)?
        {
            debug!("resolved S3 credentials from config file: {path:?}");
            return Ok(cfg);
        }

        if let Some(default_path) = default_config_path()
            && default_path.exists()
            && let Some(cfg) = Self::from_ini_file(&default_path, bucket)?
        {
            debug!("resolved S3 credentials from default config file: {default_path:?}");
            return Ok(cfg);
        }

        if let Some(cfg) = Self::from_env_prefix(
            "S3_ACCESS_KEY_ID",
            "S3_SECRET_ACCESS_KEY",
            "S3_ENDPOINT",
            "S3_REGION",
        ) {
            debug!("resolved S3 credentials from environment variables");
            return Ok(cfg);
        }

        if let Some(cfg) = Self::from_env_prefix(
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_ENDPOINT_URL",
            "AWS_REGION",
        ) {
            debug!("resolved S3 credentials from AWS environment variables");
            return Ok(cfg);
        }

        Err(IoError::S3Credentials(format!(
            "no S3 credentials found \
                 place a config at {}, \
                 or set S3_*/AWS_* environment variables",
            default_config_hint()
        )))
    }

    /// Build a bare `AmazonS3` client from these credentials.
    pub fn build_s3_client(&self, bucket: &str) -> Result<AmazonS3, IoError> {
        zarrs_object_store::object_store::aws::AmazonS3Builder::new()
            .with_bucket_name(bucket)
            .with_endpoint(&self.endpoint)
            .with_access_key_id(&self.access_key_id)
            .with_secret_access_key(&self.secret_access_key)
            .with_region(&self.region)
            .build()
            .map_err(|e| IoError::S3Client(format!("failed to build S3 client: {e}")))
    }

    /// Build an `AmazonS3` client wrapped in a prefix store so that all
    /// operations are scoped to `prefix` within the bucket.
    ///
    /// The underlying `AmazonS3` is fetched from the per-bucket cache
    /// ([`get_or_create_s3_client`]), so multiple prefixed stores on the same
    /// bucket share one HTTP connection pool.
    pub fn build_prefixed_s3_client(
        &self,
        bucket: &str,
        prefix: &str,
    ) -> Result<PrefixedS3, IoError> {
        let s3 = get_or_create_s3_client(self, bucket)?;
        Ok(zarrs_object_store::object_store::prefix::PrefixStore::new(
            s3, prefix,
        ))
    }

    /// List bucket names from configured INI sections in the S3 config file(s).
    pub fn list_configured_buckets(config_path: Option<&Path>) -> Result<Vec<String>, IoError> {
        let mut buckets = Vec::new();

        if let Some(path) = config_path {
            buckets.extend(Self::bucket_names_from_ini_file(path)?);
        }

        if let Some(default_path) = default_config_path()
            && default_path.exists()
        {
            let from_default = Self::bucket_names_from_ini_file(&default_path)?;
            for name in from_default {
                if !buckets.iter().any(|b| b == &name) {
                    buckets.push(name);
                }
            }
        }

        buckets.sort();
        Ok(buckets)
    }

    fn bucket_names_from_ini_file(path: &Path) -> Result<Vec<String>, IoError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            IoError::S3Credentials(format!("cannot read S3 config {}: {e}", path.display()))
        })?;
        let mut names: Vec<String> = parse_ini_sections(&content)
            .into_iter()
            .map(|section| section.name)
            .collect();
        names.sort();
        Ok(names)
    }

    /// Parse an rclone-style INI file, selecting the section whose name
    /// matches `bucket`.  Returns `Ok(None)` when the file is valid but
    /// contains no section matching the bucket name, so the caller can
    /// continue down the fallback chain.
    fn from_ini_file(path: &Path, bucket: &str) -> Result<Option<Self>, IoError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            IoError::S3Credentials(format!("cannot read S3 config {}: {e}", path.display()))
        })?;

        let sections = parse_ini_sections(&content);

        let section = match sections.iter().find(|s| s.name == bucket) {
            Some(s) => s,
            None => return Ok(None),
        };

        let access_key_id = section.get("access_key_id").ok_or_else(|| {
            IoError::S3Credentials(format!(
                "missing 'access_key_id' in [{}] of {}",
                section.name,
                path.display()
            ))
        })?;
        let secret_access_key = section.get("secret_access_key").ok_or_else(|| {
            IoError::S3Credentials(format!(
                "missing 'secret_access_key' in [{}] of {}",
                section.name,
                path.display()
            ))
        })?;
        let region = section.get("region").ok_or_else(|| {
            IoError::S3Credentials(format!(
                "missing 'region' in [{}] of {}",
                section.name,
                path.display()
            ))
        })?;
        let endpoint = section.get("endpoint").ok_or_else(|| {
            IoError::S3Credentials(format!(
                "missing 'endpoint' in [{}] of {}",
                section.name,
                path.display()
            ))
        })?;

        Ok(Some(Self {
            access_key_id: access_key_id.to_string(),
            secret_access_key: secret_access_key.to_string(),
            region: region.to_string(),
            endpoint: endpoint.to_string(),
        }))
    }

    fn from_env_prefix(ak_var: &str, sk_var: &str, ep_var: &str, rg_var: &str) -> Option<Self> {
        let access_key_id = env::var(ak_var).ok()?;
        let secret_access_key = env::var(sk_var).ok()?;
        let endpoint = env::var(ep_var).ok()?;
        let region = env::var(rg_var).ok()?;
        Some(Self {
            access_key_id,
            secret_access_key,
            region,
            endpoint,
        })
    }
}

/// Default location for `s3.conf` when no explicit path is provided.
fn default_config_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var_os("APPDATA").map(|appdata| Path::new(&appdata).join("cp-rs").join("s3.conf"))
    }
    #[cfg(not(windows))]
    {
        env::var_os("HOME").map(|home| {
            Path::new(&home)
                .join(".config")
                .join("cp-rs")
                .join("s3.conf")
        })
    }
}

fn default_config_hint() -> &'static str {
    #[cfg(windows)]
    {
        "%APPDATA%\\cp-rs\\s3.conf"
    }
    #[cfg(not(windows))]
    {
        "~/.config/cp-rs/s3.conf"
    }
}

// ---------------------------------------------------------------------------
// Per-bucket S3 client cache
// ---------------------------------------------------------------------------

static S3_CLIENT_CACHE: OnceLock<Mutex<HashMap<String, AmazonS3>>> = OnceLock::new();

/// Return a cached `AmazonS3` client for `bucket`, creating one from `config`
/// on first access.  All stores targeting the same bucket share a single
/// underlying HTTP connection pool.
pub fn get_or_create_s3_client(config: &S3Config, bucket: &str) -> Result<AmazonS3, IoError> {
    let cache = S3_CLIENT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().expect("S3 client cache lock poisoned");
    if let Some(client) = map.get(bucket) {
        debug!(bucket, "reusing cached S3 client");
        return Ok(client.clone());
    }
    let client = config.build_s3_client(bucket)?;
    map.insert(bucket.to_string(), client.clone());
    debug!(bucket, "created and cached new S3 client");
    Ok(client)
}

// ---------------------------------------------------------------------------
// INI parser helpers
// ---------------------------------------------------------------------------

struct IniSection {
    name: String,
    entries: Vec<(String, String)>,
}

impl IniSection {
    fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Parse an rclone-style INI file into a list of named sections.
/// Lines before the first `[section]` header are ignored.
fn parse_ini_sections(content: &str) -> Vec<IniSection> {
    let mut sections = Vec::new();
    let mut current: Option<IniSection> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            if let Some(sec) = current.take() {
                sections.push(sec);
            }
            let name = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_string();
            current = Some(IniSection {
                name,
                entries: Vec::new(),
            });
            continue;
        }
        if let Some(ref mut sec) = current
            && let Some((key, value)) = line.split_once('=')
        {
            sec.entries
                .push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    if let Some(sec) = current {
        sections.push(sec);
    }

    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_config_path_follows_platform_convention() {
        #[cfg(windows)]
        {
            let appdata = env::var_os("APPDATA").expect("APPDATA must be set");
            let path = default_config_path().expect("default config path");
            assert_eq!(path, Path::new(&appdata).join("cp-rs").join("s3.conf"));
        }
        #[cfg(not(windows))]
        {
            let home = env::var_os("HOME").expect("HOME must be set");
            let path = default_config_path().expect("default config path");
            assert_eq!(
                path,
                Path::new(&home)
                    .join(".config")
                    .join("cp-rs")
                    .join("s3.conf")
            );
        }
    }

    #[test]
    fn list_configured_buckets_reads_all_sections() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            "[bucket-a]\n\
             type = s3\n\
             access_key_id = AK_A\n\
             secret_access_key = SK_A\n\
             region = eu-west-1\n\
             endpoint = https://s3.a.example.com\n\
             \n\
             [bucket-b]\n\
             type = s3\n\
             access_key_id = AK_B\n\
             secret_access_key = SK_B\n\
             region = us-east-1\n\
             endpoint = https://s3.b.example.com"
        )
        .unwrap();

        let buckets = S3Config::list_configured_buckets(Some(tmp.path())).unwrap();
        assert!(buckets.contains(&"bucket-a".to_string()));
        assert!(buckets.contains(&"bucket-b".to_string()));
    }

    #[test]
    fn parse_ini_selects_matching_section() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            "[bucket-a]\n\
             type = s3\n\
             access_key_id = AK_A\n\
             secret_access_key = SK_A\n\
             region = eu-west-1\n\
             endpoint = https://s3.a.example.com\n\
             \n\
             [bucket-b]\n\
             type = s3\n\
             access_key_id = AK_B\n\
             secret_access_key = SK_B\n\
             region = us-east-1\n\
             endpoint = https://s3.b.example.com"
        )
        .unwrap();

        let cfg = S3Config::from_ini_file(tmp.path(), "bucket-b")
            .unwrap()
            .expect("section bucket-b should be found");
        assert_eq!(cfg.access_key_id, "AK_B");
        assert_eq!(cfg.secret_access_key, "SK_B");
        assert_eq!(cfg.region, "us-east-1");
        assert_eq!(cfg.endpoint, "https://s3.b.example.com");
    }

    #[test]
    fn parse_ini_returns_none_when_no_section_matches() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            "[my-remote]\n\
             type = s3\n\
             access_key_id = AKID\n\
             secret_access_key = SKEY\n\
             region = eu-west-1\n\
             endpoint = https://s3.example.com"
        )
        .unwrap();

        let result = S3Config::from_ini_file(tmp.path(), "unknown-bucket").unwrap();
        assert!(result.is_none());
    }
}
