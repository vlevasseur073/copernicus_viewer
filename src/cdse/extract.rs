//! Extract CDSE product zip archives into a temporary directory for opening.
//!
//! Supports EOPF Zarr zips and Sentinel-3 SAFE (`.SEN3`) zips.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use zip::ZipArchive;

/// Downloaded (and optionally extracted) product ready to open in the viewer.
pub struct CdsePreparedProduct {
    /// Path passed to [`crate::product::open_product`] (extracted product root when from zip).
    pub open_path: PathBuf,
    /// Original zip path when extraction was performed.
    pub zip_path: Option<PathBuf>,
    /// Keeps the temp extraction directory alive until dropped by the app.
    pub extract_guard: Option<TempDir>,
}

/// Prepare a downloaded CDSE path for opening: unzip archives into a temp dir.
pub fn prepare_downloaded_product(path: PathBuf) -> Result<CdsePreparedProduct, String> {
    if !is_zip_path(&path) {
        return Ok(CdsePreparedProduct {
            open_path: path,
            zip_path: None,
            extract_guard: None,
        });
    }

    let (guard, root) = extract_product_zip_to_temp(&path)?;
    Ok(CdsePreparedProduct {
        open_path: root,
        zip_path: Some(path),
        extract_guard: Some(guard),
    })
}

fn is_zip_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

fn extract_product_zip_to_temp(zip_path: &Path) -> Result<(TempDir, PathBuf), String> {
    let temp = TempDir::new().map_err(|err| format!("failed to create temp dir: {err}"))?;
    extract_zip(zip_path, temp.path())?;
    let root = find_product_root(temp.path())?;
    Ok((temp, root))
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("failed to open zip {}: {err}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| format!("failed to read zip {}: {err}", zip_path.display()))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("failed to read zip entry {index}: {err}"))?;
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        let out_path = dest.join(enclosed);

        if entry.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|err| format!("failed to create {}: {err}", out_path.display()))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        let mut outfile = File::create(&out_path)
            .map_err(|err| format!("failed to create {}: {err}", out_path.display()))?;
        io::copy(&mut entry, &mut outfile)
            .map_err(|err| format!("failed to extract {}: {err}", out_path.display()))?;
    }

    Ok(())
}

fn find_product_root(extract_root: &Path) -> Result<PathBuf, String> {
    if is_product_dir(extract_root) {
        return accept_product_dir(extract_root);
    }

    let mut zarr_candidates = Vec::new();
    let mut safe_candidates = Vec::new();
    collect_product_candidates(
        extract_root,
        0,
        4,
        &mut zarr_candidates,
        &mut safe_candidates,
    )
    .map_err(|err| format!("failed to scan extracted archive: {err}"))?;

    if let Some(path) = prefer_zarr_candidate(&zarr_candidates) {
        return Ok(path);
    }
    if let Some(path) = prefer_safe_candidate(&safe_candidates) {
        return accept_product_dir(&path);
    }

    // Single top-level directory wrapper (common for CDSE zips).
    let mut children = fs::read_dir(extract_root)
        .map_err(|err| format!("failed to read {}: {err}", extract_root.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    children.sort();
    if children.len() == 1 {
        return find_product_root(&children[0]);
    }

    Err(format!(
        "no Zarr (.zarr) or SAFE (.SEN3) product root found under {}",
        extract_root.display()
    ))
}

fn accept_product_dir(path: &Path) -> Result<PathBuf, String> {
    #[cfg(not(feature = "safe"))]
    if is_safe_dir(path) {
        return Err(format!(
            "found Sentinel-3 SAFE product at {}, but SAFE support is disabled \
             (rebuild with --features safe)",
            path.display()
        ));
    }
    Ok(path.to_path_buf())
}

fn is_product_dir(path: &Path) -> bool {
    is_zarr_dir(path) || is_safe_dir(path)
}

fn collect_product_candidates(
    current: &Path,
    depth: usize,
    max_depth: usize,
    zarr_out: &mut Vec<PathBuf>,
    safe_out: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    if is_zarr_dir(current) || is_zarr_named_dir(current) {
        zarr_out.push(current.to_path_buf());
        if is_zarr_dir(current) {
            return Ok(());
        }
    }
    if is_safe_dir(current) {
        safe_out.push(current.to_path_buf());
        return Ok(());
    }
    if !current.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_product_candidates(&path, depth + 1, max_depth, zarr_out, safe_out)?;
        }
    }
    Ok(())
}

fn prefer_zarr_candidate(candidates: &[PathBuf]) -> Option<PathBuf> {
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates[0].clone());
    }
    candidates
        .iter()
        .find(|path| is_zarr_named_dir(path) && is_zarr_dir(path))
        .or_else(|| candidates.iter().find(|path| is_zarr_dir(path)))
        .or_else(|| candidates.iter().find(|path| is_zarr_named_dir(path)))
        .cloned()
}

fn prefer_safe_candidate(candidates: &[PathBuf]) -> Option<PathBuf> {
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates[0].clone());
    }
    candidates
        .iter()
        .find(|path| is_safe_dir(path))
        .cloned()
        .or_else(|| candidates.first().cloned())
}

fn is_zarr_named_dir(path: &Path) -> bool {
    path.is_dir()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".zarr"))
}

fn is_zarr_dir(path: &Path) -> bool {
    path.is_dir()
        && (path.join(".zgroup").is_file()
            || path.join(".zmetadata").is_file()
            || path.join("zarr.json").is_file())
}

fn is_safe_dir(path: &Path) -> bool {
    path.is_dir()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".SEN3"))
        && path.join("xfdumanifest.xml").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    fn write_zarr_zip(zip_path: &Path, inner_root: &str) {
        let file = File::create(zip_path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.add_directory(format!("{inner_root}/"), options)
            .expect("dir");
        zip.start_file(format!("{inner_root}/.zgroup"), options)
            .expect("start");
        zip.write_all(br#"{"zarr_format":2}"#).expect("write");
        zip.start_file(format!("{inner_root}/.zattrs"), options)
            .expect("start attrs");
        zip.write_all(br#"{}"#).expect("write attrs");
        zip.finish().expect("finish");
    }

    fn write_safe_zip(zip_path: &Path, inner_root: &str) {
        let file = File::create(zip_path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.add_directory(format!("{inner_root}/"), options)
            .expect("dir");
        zip.start_file(format!("{inner_root}/xfdumanifest.xml"), options)
            .expect("start");
        zip.write_all(br#"<?xml version="1.0"?><xfdu:XFDU xmlns:xfdu="urn:ccsds:schema:xfdu:1"/>"#)
            .expect("write");
        zip.finish().expect("finish");
    }

    #[test]
    fn extracts_nested_zarr_directory() {
        let dir = tempfile::tempdir().expect("temp");
        let zip_path = dir.path().join("product.zarr.zip");
        write_zarr_zip(&zip_path, "S3OLCEFR_demo.zarr");

        let prepared = prepare_downloaded_product(zip_path.clone()).expect("prepare");
        assert!(prepared.open_path.ends_with("S3OLCEFR_demo.zarr"));
        assert!(prepared.open_path.join(".zgroup").is_file());
        assert_eq!(prepared.zip_path.as_deref(), Some(zip_path.as_path()));
        assert!(prepared.extract_guard.is_some());
    }

    #[test]
    #[cfg(feature = "safe")]
    fn extracts_nested_safe_directory() {
        let dir = tempfile::tempdir().expect("temp");
        let zip_path = dir.path().join("product.SAFE.zip");
        let inner = "S3B_OL_2_WFR____20200101T000000_20200101T000300_20200102T120000_0179_000_000______MAR_O_NT_002.SEN3";
        write_safe_zip(&zip_path, inner);

        let prepared = prepare_downloaded_product(zip_path.clone()).expect("prepare");
        assert!(prepared.open_path.ends_with(inner));
        assert!(prepared.open_path.join("xfdumanifest.xml").is_file());
        assert_eq!(prepared.zip_path.as_deref(), Some(zip_path.as_path()));
        assert!(prepared.extract_guard.is_some());
    }

    #[test]
    #[cfg(not(feature = "safe"))]
    fn rejects_nested_safe_directory_without_safe_feature() {
        let dir = tempfile::tempdir().expect("temp");
        let zip_path = dir.path().join("product.SAFE.zip");
        let inner = "S3B_OL_2_WFR____20200101T000000_20200101T000300_20200102T120000_0179_000_000______MAR_O_NT_002.SEN3";
        write_safe_zip(&zip_path, inner);

        match prepare_downloaded_product(zip_path) {
            Ok(_) => panic!("expected SAFE extraction to fail without the safe feature"),
            Err(err) => assert!(err.contains("SAFE support is disabled")),
        }
    }

    #[test]
    fn non_zip_paths_pass_through() {
        let path = PathBuf::from("/tmp/already.zarr");
        let prepared = prepare_downloaded_product(path.clone()).expect("prepare");
        assert_eq!(prepared.open_path, path);
        assert!(prepared.zip_path.is_none());
        assert!(prepared.extract_guard.is_none());
    }
}
