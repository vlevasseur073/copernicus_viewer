use std::ops::Range;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ndarray::ArrayD;
use netcdf;
use netcdf::AttributeValue;
use netcdf::types::{FloatType, IntType, NcVariableType};
use serde_json::{Map, Number, Value};

use super::mapping::ArrayMeta;

/// Probe NetCDF variable metadata without reading array payload.
pub fn probe_variable(
    nc_path: &Path,
    var_name: &str,
    chunk_sizes: &[(String, u64)],
) -> Result<ArrayMeta> {
    let file = netcdf::open(nc_path).with_context(|| format!("open {}", nc_path.display()))?;
    let var = file
        .variable(var_name)
        .with_context(|| format!("variable {var_name} in {}", nc_path.display()))?;

    let shape = variable_shape(&var);
    let dimension_names: Vec<String> = var
        .dimensions()
        .iter()
        .map(|d| d.name().to_string())
        .collect();

    let chunks = effective_chunks(&dimension_names, &shape, chunk_sizes);
    let attributes = netcdf_attrs_to_json(&var);
    let fill_value = attributes
        .get("_FillValue")
        .or_else(|| attributes.get("missing_value"))
        .cloned();

    Ok(ArrayMeta {
        shape,
        chunks,
        dtype: netcdf_dtype_name(&var),
        dimension_names,
        attributes,
        fill_value,
    })
}

fn variable_shape(var: &netcdf::Variable) -> Vec<u64> {
    var.dimensions().iter().map(|d| d.len() as u64).collect()
}

fn effective_chunks(
    dim_names: &[String],
    shape: &[u64],
    chunk_sizes: &[(String, u64)],
) -> Vec<u64> {
    let lookup: std::collections::HashMap<_, _> = chunk_sizes.iter().cloned().collect();
    dim_names
        .iter()
        .zip(shape.iter())
        .map(|(name, &dim)| lookup.get(name).copied().unwrap_or(dim).max(1))
        .collect()
}

/// Read a NetCDF variable subset as `f64`.
pub fn read_subset_f64(
    nc_path: &Path,
    var_name: &str,
    ranges: &[Range<u64>],
) -> Result<ArrayD<f64>> {
    let file = netcdf::open(nc_path).with_context(|| format!("open {}", nc_path.display()))?;
    let var = file
        .variable(var_name)
        .with_context(|| format!("variable {var_name} in {}", nc_path.display()))?;

    let shape = variable_shape(&var);
    if ranges.len() != shape.len() {
        bail!(
            "range count {} does not match rank {} for {var_name}",
            ranges.len(),
            shape.len()
        );
    }

    for (i, range) in ranges.iter().enumerate() {
        let dim = shape[i];
        if range.end > dim {
            bail!(
                "range end {} exceeds dimension size {dim} at axis {i}",
                range.end
            );
        }
    }

    read_typed_subset(&var, ranges)
}

fn read_typed_subset(var: &netcdf::Variable, ranges: &[Range<u64>]) -> Result<ArrayD<f64>> {
    let slices: Vec<_> = ranges
        .iter()
        .map(|r| (r.start as usize)..(r.end as usize))
        .collect();

    macro_rules! try_read {
        ($t:ty) => {
            if let Ok(values) = var.get::<$t, _>(slices.as_slice()) {
                return Ok(values.mapv(|v| v as f64));
            }
        };
    }

    try_read!(f64);
    try_read!(f32);
    try_read!(i64);
    try_read!(i32);
    try_read!(i16);
    try_read!(i8);
    try_read!(u64);
    try_read!(u32);
    try_read!(u16);
    try_read!(u8);

    bail!("unsupported NetCDF variable type for {}", var.name())
}

fn netcdf_dtype_name(var: &netcdf::Variable) -> String {
    match var.vartype() {
        NcVariableType::Float(FloatType::F32) => "float32".to_string(),
        NcVariableType::Float(FloatType::F64) => "float64".to_string(),
        NcVariableType::Int(IntType::I8) => "int8".to_string(),
        NcVariableType::Int(IntType::I16) => "int16".to_string(),
        NcVariableType::Int(IntType::I32) => "int32".to_string(),
        NcVariableType::Int(IntType::I64) => "int64".to_string(),
        NcVariableType::Int(IntType::U8) => "uint8".to_string(),
        NcVariableType::Int(IntType::U16) => "uint16".to_string(),
        NcVariableType::Int(IntType::U32) => "uint32".to_string(),
        NcVariableType::Int(IntType::U64) => "uint64".to_string(),
        NcVariableType::String => "string".to_string(),
        NcVariableType::Char => "char".to_string(),
        other => format!("{other:?}"),
    }
}

fn netcdf_attrs_to_json(var: &netcdf::Variable) -> Map<String, Value> {
    let mut attrs = Map::new();
    for attr in var.attributes() {
        if let Ok(value) = attr.value()
            && let Some(json) = attr_value_to_json(&value)
        {
            attrs.insert(attr.name().to_string(), json);
        }
    }
    attrs
}

fn attr_value_to_json(value: &AttributeValue) -> Option<Value> {
    match value {
        AttributeValue::Str(s) => Some(Value::String(s.clone())),
        AttributeValue::Strs(v) if v.len() == 1 => Some(Value::String(v[0].clone())),
        AttributeValue::Strs(v) => Some(Value::Array(
            v.iter().map(|s| Value::String(s.clone())).collect(),
        )),
        AttributeValue::Float(v) => serde_json::Number::from_f64(*v as f64).map(Value::Number),
        AttributeValue::Double(v) => serde_json::Number::from_f64(*v).map(Value::Number),
        AttributeValue::Schar(v) => Some(Value::Number(Number::from(*v))),
        AttributeValue::Short(v) => Some(Value::Number(Number::from(*v))),
        AttributeValue::Int(v) => Some(Value::Number(Number::from(*v))),
        AttributeValue::Longlong(v) => Some(Value::Number(Number::from(*v))),
        AttributeValue::Uchar(v) => Some(Value::Number(Number::from(*v))),
        AttributeValue::Ushort(v) => Some(Value::Number(Number::from(*v))),
        AttributeValue::Uint(v) => Some(Value::Number(Number::from(*v))),
        AttributeValue::Ulonglong(v) => Some(Value::Number(Number::from(*v))),
        AttributeValue::Floats(v) => Some(Value::Array(
            v.iter()
                .filter_map(|x| serde_json::Number::from_f64(*x as f64).map(Value::Number))
                .collect(),
        )),
        AttributeValue::Doubles(v) => Some(Value::Array(
            v.iter()
                .filter_map(|x| serde_json::Number::from_f64(*x).map(Value::Number))
                .collect(),
        )),
        AttributeValue::Schars(v) => Some(Value::Array(
            v.iter().map(|x| Value::Number(Number::from(*x))).collect(),
        )),
        AttributeValue::Shorts(v) => Some(Value::Array(
            v.iter().map(|x| Value::Number(Number::from(*x))).collect(),
        )),
        AttributeValue::Ints(v) => Some(Value::Array(
            v.iter().map(|x| Value::Number(Number::from(*x))).collect(),
        )),
        AttributeValue::Longlongs(v) => Some(Value::Array(
            v.iter().map(|x| Value::Number(Number::from(*x))).collect(),
        )),
        AttributeValue::Uchars(v) => Some(Value::Array(
            v.iter().map(|x| Value::Number(Number::from(*x))).collect(),
        )),
        AttributeValue::Ushorts(v) => Some(Value::Array(
            v.iter().map(|x| Value::Number(Number::from(*x))).collect(),
        )),
        AttributeValue::Uints(v) => Some(Value::Array(
            v.iter().map(|x| Value::Number(Number::from(*x))).collect(),
        )),
        AttributeValue::Ulonglongs(v) => Some(Value::Array(
            v.iter().map(|x| Value::Number(Number::from(*x))).collect(),
        )),
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn probe_lst_in_tiny_product() {
        let nc = Path::new(
            "/home/vincent/Codes/Acri/sentineltoolbox/sentineltoolbox/testing/data/tiny_products/S3A_SL_2_LST____20191227T124111_20191227T124411_20221209T133218_0179_053_109______PS1_D_NR_004.SEN3/LST_in.nc",
        );
        if !nc.exists() {
            return;
        }
        let meta = probe_variable(nc, "LST", &[]).expect("probe tiny LST");
        assert_eq!(meta.shape.len(), 2);
    }

    #[test]
    fn probe_lst_in_example_product() {
        let nc = Path::new(
            "/home/vincent/Data/SLSTR/S3A_SL_2_LST____20260622T102053_20260622T102353_20260622T123949_0179_141_008_2160_PS1_O_NR_005.SEN3/LST_in.nc",
        );
        if !nc.exists() {
            return;
        }
        let meta = probe_variable(nc, "LST", &[]).expect("probe LST");
        assert!(!meta.shape.is_empty());
    }
}
