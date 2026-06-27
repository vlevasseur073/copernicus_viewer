//! Chunk-aligned array I/O for comparing two products on the reference grid.

use std::ops::Range;

use anyhow::{Context, Result};
use ndarray::ArrayD;
use zarrs::array::Array;
use zarrs::array::ArraySubset;
use zarrs::storage::ReadableListableStorage;

use crate::product::Product;

/// Chunk boundaries aligned on the reference product's chunk grid.
pub fn iter_reference_chunk_subsets(shape: &[u64], ref_chunks: &[u64]) -> Vec<Vec<Range<u64>>> {
    let chunks = effective_chunks(shape, ref_chunks);
    let per_dim: Vec<Vec<Range<u64>>> = shape
        .iter()
        .zip(chunks.iter())
        .map(|(&dim, &chunk)| chunk_ranges(dim, chunk))
        .collect();
    cartesian_ranges(&per_dim)
}

/// Effective chunk shape, falling back to full dimension sizes when metadata is missing.
pub fn effective_chunks(shape: &[u64], chunks: &[u64]) -> Vec<u64> {
    if chunks.len() == shape.len() && chunks.iter().all(|&c| c > 0) {
        return chunks.to_vec();
    }
    shape.to_vec()
}

fn chunk_ranges(dim: u64, chunk: u64) -> Vec<Range<u64>> {
    let chunk = chunk.max(1);
    let mut ranges = Vec::new();
    let mut start = 0u64;
    while start < dim {
        let end = (start + chunk).min(dim);
        ranges.push(start..end);
        start = end;
    }
    if ranges.is_empty() {
        ranges.push(0..dim.max(1));
    }
    ranges
}

fn cartesian_ranges(per_dim: &[Vec<Range<u64>>]) -> Vec<Vec<Range<u64>>> {
    if per_dim.is_empty() {
        return vec![Vec::new()];
    }

    let mut out = vec![Vec::new()];
    for dim_ranges in per_dim {
        let mut next = Vec::new();
        for prefix in &out {
            for range in dim_ranges {
                let mut combined = prefix.clone();
                combined.push(range.clone());
                next.push(combined);
            }
        }
        out = next;
    }
    out
}

/// Read matching logical slices from both products, iterating on the reference chunk grid.
pub fn for_each_aligned_chunk<F>(
    left: &Product,
    right: &Product,
    path: &str,
    shape: &[u64],
    ref_chunks: &[u64],
    mut f: F,
) -> Result<()>
where
    F: FnMut(&ArrayD<f64>, &ArrayD<f64>) -> Result<()>,
{
    for ranges in iter_reference_chunk_subsets(shape, ref_chunks) {
        let ref_data = left
            .read_array_subset_f64(path, &ranges)
            .with_context(|| format!("read reference chunk at {path}"))?;
        let new_data = right
            .read_array_subset_f64(path, &ranges)
            .with_context(|| format!("read new chunk at {path}"))?;
        if ref_data.len() != new_data.len() {
            anyhow::bail!(
                "aligned chunk size mismatch at {path} subset {ranges:?}: reference {} vs new {}",
                ref_data.len(),
                new_data.len()
            );
        }
        f(&ref_data, &new_data)?;
    }

    Ok(())
}

/// Read a full logical array from a Zarr-backed product (used by tests and tooling).
#[allow(dead_code)]
pub fn read_array_f64(
    storage: &ReadableListableStorage,
    path: &str,
    shape: &[u64],
    dtype_hint: &str,
) -> Result<ArrayD<f64>> {
    let array = Array::open(storage.clone(), path)
        .with_context(|| format!("failed to open array at {path}"))?;
    let ranges: Vec<Range<u64>> = shape.iter().map(|&dim| 0..dim).collect();
    let subset = ArraySubset::new_with_ranges(&ranges);
    read_zarr_subset_as_f64(&array, &subset, dtype_hint)
}

fn read_zarr_subset_as_f64(
    array: &Array<dyn zarrs::storage::ReadableListableStorageTraits>,
    subset: &ArraySubset,
    dtype_hint: &str,
) -> Result<ArrayD<f64>> {
    let normalized = dtype_hint.to_ascii_lowercase();

    macro_rules! read_as {
        ($t:ty) => {{
            let arr: ArrayD<$t> = array
                .retrieve_array_subset::<ArrayD<$t>>(subset)
                .with_context(|| {
                    format!("failed to read array as {}", std::any::type_name::<$t>())
                })?;
            return Ok(arr.mapv(|v| v as f64));
        }};
    }

    if normalized.contains("float64") || normalized.contains("<f8") || normalized.contains("|f8") {
        read_as!(f64);
    }
    if normalized.contains("float32") || normalized.contains("<f4") || normalized.contains("|f4") {
        read_as!(f32);
    }
    if normalized.contains("uint64") || normalized.contains("<u8") || normalized.contains("|u8") {
        read_as!(u64);
    }
    if normalized.contains("uint32") || normalized.contains("<u4") || normalized.contains("|u4") {
        read_as!(u32);
    }
    if normalized.contains("uint16") || normalized.contains("<u2") || normalized.contains("|u2") {
        read_as!(u16);
    }
    if normalized.contains("uint8") || normalized.contains("<u1") || normalized.contains("|u1") {
        read_as!(u8);
    }
    if normalized.contains("int64") || normalized.contains("<i8") || normalized.contains("|i8") {
        read_as!(i64);
    }
    if normalized.contains("int32") || normalized.contains("<i4") || normalized.contains("|i4") {
        read_as!(i32);
    }
    if normalized.contains("int16") || normalized.contains("<i2") || normalized.contains("|i2") {
        read_as!(i16);
    }
    if normalized.contains("int8") || normalized.contains("<i1") || normalized.contains("|i1") {
        read_as!(i8);
    }

    anyhow::bail!("unsupported data type for comparison: {dtype_hint}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_chunk_grid_uses_reference_boundaries() {
        let shape = vec![1200];
        let ref_chunks = vec![600];
        let subsets = iter_reference_chunk_subsets(&shape, &ref_chunks);
        assert_eq!(subsets.len(), 2);
        assert_eq!(subsets[0], vec![0..600]);
        assert_eq!(subsets[1], vec![600..1200]);
    }

    #[test]
    fn reference_chunk_grid_is_independent_of_new_chunking() {
        let shape = vec![1200, 1200];
        let ref_chunks = vec![600, 600];
        let subsets = iter_reference_chunk_subsets(&shape, &ref_chunks);
        assert_eq!(subsets.len(), 4);
        assert!(subsets.contains(&vec![0..600, 0..600]));
        assert!(subsets.contains(&vec![600..1200, 600..1200]));
    }

    #[test]
    fn uneven_reference_chunks_are_respected() {
        let shape = vec![1200];
        let ref_chunks = vec![601];
        let subsets = iter_reference_chunk_subsets(&shape, &ref_chunks);
        assert_eq!(subsets.len(), 2);
        assert_eq!(subsets[0], vec![0..601]);
        assert_eq!(subsets[1], vec![601..1200]);
    }
}
