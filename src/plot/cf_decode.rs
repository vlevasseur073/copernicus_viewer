use ndarray::ArrayD;
use serde_json::{Map, Value};

/// Parsed CF encoding metadata from array attributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CfEncoding {
    /// Missing value in stored (encoded) space.
    pub fill_value: Option<f64>,
    /// Scale applied after fill masking: `decoded = scale_factor * encoded + add_offset`.
    pub scale_factor: Option<f64>,
    /// Offset applied after fill masking.
    pub add_offset: Option<f64>,
}

/// Parse CF fill value and scale/offset attributes.
pub fn parse_cf_encoding(
    attributes: &Map<String, Value>,
    zarr_fill_value: Option<&Value>,
) -> CfEncoding {
    CfEncoding {
        fill_value: resolve_fill_value(zarr_fill_value, attributes),
        scale_factor: parse_attr_number(attributes, "scale_factor"),
        add_offset: parse_attr_number(attributes, "add_offset"),
    }
}

/// Mask fill values and apply scale/offset to produce decoded physical values.
pub fn apply_cf_decode(values: &ArrayD<f64>, encoding: &CfEncoding) -> ArrayD<f64> {
    let scale = encoding.scale_factor.unwrap_or(1.0);
    let offset = encoding.add_offset.unwrap_or(0.0);
    let has_transform = encoding.scale_factor.is_some() || encoding.add_offset.is_some();

    values.mapv(|value| {
        if !value.is_finite() {
            return f64::NAN;
        }
        if is_fill_value(value, encoding.fill_value) {
            return f64::NAN;
        }
        if has_transform {
            scale * value + offset
        } else {
            value
        }
    })
}

/// Resolve fill value from Zarr array metadata, falling back to CF attributes.
pub fn resolve_fill_value(
    zarr_fill_value: Option<&Value>,
    attributes: &Map<String, Value>,
) -> Option<f64> {
    if let Some(value) = zarr_fill_value
        && let Some(parsed) = parse_fill_value_from_json(value)
    {
        return Some(parsed);
    }

    attributes
        .get("_FillValue")
        .or_else(|| attributes.get("fill_value"))
        .and_then(parse_fill_value_from_json)
}

/// Parse `_FillValue` or `fill_value` from array attributes only.
pub fn parse_fill_value(attributes: &Map<String, Value>) -> Option<f64> {
    resolve_fill_value(None, attributes)
}

fn parse_fill_value_from_json(value: &Value) -> Option<f64> {
    parse_json_number(value)
}

/// Returns `true` when `value` matches the CF fill value within tolerance.
pub fn is_fill_value(value: f64, fill_value: Option<f64>) -> bool {
    fill_value
        .map(|fill| (value - fill).abs() <= 1e-9)
        .unwrap_or(false)
}

fn parse_attr_number(attributes: &Map<String, Value>, key: &str) -> Option<f64> {
    attributes.get(key).and_then(parse_json_number)
}

fn parse_json_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::IxDyn;
    use serde_json::json;

    #[test]
    fn prefers_zarr_fill_value_over_attributes() {
        let attrs = Map::new();
        assert_eq!(
            resolve_fill_value(Some(&json!(65535)), &attrs),
            Some(65535.0)
        );
    }

    #[test]
    fn falls_back_to_attributes_when_zarr_fill_missing() {
        let attrs = json!({ "_FillValue": 255 }).as_object().unwrap().clone();
        assert_eq!(resolve_fill_value(None, &attrs), Some(255.0));
    }

    #[test]
    fn parses_fill_value_from_either_attribute_key() {
        let attrs = json!({ "_FillValue": 255 }).as_object().unwrap().clone();
        assert_eq!(parse_fill_value(&attrs), Some(255.0));

        let attrs = json!({ "fill_value": -999 }).as_object().unwrap().clone();
        assert_eq!(parse_fill_value(&attrs), Some(-999.0));
    }

    #[test]
    fn masks_fill_and_applies_scale_offset() {
        let encoding = CfEncoding {
            fill_value: Some(65535.0),
            scale_factor: Some(0.01),
            add_offset: Some(273.15),
        };
        let values =
            ArrayD::from_shape_vec(IxDyn(&[2, 2]), vec![0.0, 100.0, 200.0, 65535.0]).unwrap();
        let decoded = apply_cf_decode(&values, &encoding);

        assert!((decoded[[0, 0]] - 273.15).abs() < 1e-9);
        assert!((decoded[[0, 1]] - 274.15).abs() < 1e-9);
        assert!((decoded[[1, 0]] - 275.15).abs() < 1e-9);
        assert!(decoded[[1, 1]].is_nan());
    }

    #[test]
    fn masks_fill_without_transform() {
        let encoding = CfEncoding {
            fill_value: Some(255.0),
            scale_factor: None,
            add_offset: None,
        };
        let values = ArrayD::from_shape_vec(IxDyn(&[3]), vec![1.0, 255.0, 4.0]).unwrap();
        let decoded = apply_cf_decode(&values, &encoding);

        assert_eq!(decoded[[0]], 1.0);
        assert!(decoded[[1]].is_nan());
        assert_eq!(decoded[[2]], 4.0);
    }

    #[test]
    fn preserves_existing_nan() {
        let encoding = CfEncoding {
            fill_value: None,
            scale_factor: Some(2.0),
            add_offset: Some(1.0),
        };
        let values = ArrayD::from_shape_vec(IxDyn(&[2]), vec![f64::NAN, 3.0]).unwrap();
        let decoded = apply_cf_decode(&values, &encoding);

        assert!(decoded[[0]].is_nan());
        assert!((decoded[[1]] - 7.0).abs() < 1e-9);
    }
}
