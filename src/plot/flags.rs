use ndarray::ArrayD;
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlagSelection {
    #[default]
    Raw,
    Flag(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CfFlagMode {
    Values,
    Masks,
}

#[derive(Clone, Debug)]
pub struct CfFlags {
    pub meanings: Vec<String>,
    pub codes: Vec<u64>,
    pub mode: CfFlagMode,
    pub fill_value: Option<f64>,
}

impl CfFlags {
    pub fn flag_label(&self, index: usize) -> String {
        let meaning = self.meanings.get(index).map(String::as_str).unwrap_or("?");
        format!("[{index}] {meaning} ({})", self.code_description(index))
    }

    pub fn code_description(&self, index: usize) -> String {
        let code = self.codes.get(index).copied().unwrap_or(0);
        match self.mode {
            CfFlagMode::Values => code.to_string(),
            CfFlagMode::Masks => {
                if code.is_power_of_two() {
                    format!("bit {}, mask {code}", code.trailing_zeros())
                } else {
                    format!("mask {code}")
                }
            }
        }
    }

    pub fn uses_bitmasks(&self) -> bool {
        self.mode == CfFlagMode::Masks
    }
}

/// Parse CF `flag_meanings` with `flag_values` or `flag_masks` from array attributes.
pub fn parse_cf_flags(attributes: &Map<String, Value>) -> Option<CfFlags> {
    let meanings = parse_meanings(attributes)?;
    let (codes, mode) = parse_codes(attributes)?;
    if meanings.len() != codes.len() {
        return None;
    }

    Some(CfFlags {
        meanings,
        codes,
        mode,
        fill_value: parse_fill_value(attributes),
    })
}

pub fn apply_flag_selection(values: &ArrayD<f64>, flags: &CfFlags, index: usize) -> ArrayD<f64> {
    let Some(&code) = flags.codes.get(index) else {
        return values.clone();
    };

    values.mapv(|value| {
        if !value.is_finite() {
            return f64::NAN;
        }
        if is_fill_value(value, flags.fill_value) {
            return f64::NAN;
        }

        match flags.mode {
            CfFlagMode::Values => {
                if values_equal(value, code) {
                    1.0
                } else {
                    0.0
                }
            }
            CfFlagMode::Masks => {
                if (to_bit_pattern(value) & code) != 0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    })
}

fn parse_meanings(attributes: &Map<String, Value>) -> Option<Vec<String>> {
    let value = attributes
        .get("flag_meanings")
        .or_else(|| attributes.get("flags_meanings"))?;
    let text = value.as_str()?;
    let meanings: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    if meanings.is_empty() {
        None
    } else {
        Some(meanings)
    }
}

fn parse_codes(attributes: &Map<String, Value>) -> Option<(Vec<u64>, CfFlagMode)> {
    if let Some(value) = attributes
        .get("flag_masks")
        .or_else(|| attributes.get("flags_masks"))
    {
        let codes = parse_numeric_list(value)?;
        return Some((codes, CfFlagMode::Masks));
    }

    if let Some(value) = attributes
        .get("flag_values")
        .or_else(|| attributes.get("flags_values"))
    {
        let codes = parse_numeric_list(value)?;
        return Some((codes, CfFlagMode::Values));
    }

    None
}

fn parse_numeric_list(value: &Value) -> Option<Vec<u64>> {
    match value {
        Value::String(text) => {
            let values = text
                .split_whitespace()
                .map(parse_u64_token)
                .collect::<Option<Vec<_>>>()?;
            if values.is_empty() { None } else { Some(values) }
        }
        Value::Array(values) => {
            let parsed = values.iter().map(parse_json_number).collect::<Option<Vec<_>>>()?;
            if parsed.is_empty() { None } else { Some(parsed) }
        }
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().map(|v| v as u64))
            .map(|v| vec![v]),
        _ => None,
    }
}

fn parse_u64_token(token: &str) -> Option<u64> {
    if let Some(stripped) = token.strip_prefix("0x").or_else(|| token.strip_prefix("0X")) {
        return u64::from_str_radix(stripped, 16).ok();
    }
    if let Some(stripped) = token.strip_prefix("0b").or_else(|| token.strip_prefix("0B")) {
        return u64::from_str_radix(stripped, 2).ok();
    }
    token.parse::<i64>().ok().map(|v| v as u64)
}

fn parse_json_number(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().map(|v| v as u64)),
        Value::String(text) => parse_u64_token(text),
        _ => None,
    }
}

fn parse_fill_value(attributes: &Map<String, Value>) -> Option<f64> {
    let value = attributes.get("_FillValue")?;
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn is_fill_value(value: f64, fill_value: Option<f64>) -> bool {
    fill_value
        .map(|fill| (value - fill).abs() <= 1e-9)
        .unwrap_or(false)
}

fn values_equal(value: f64, code: u64) -> bool {
    (value - code as f64).abs() <= 1e-9
}

fn to_bit_pattern(value: f64) -> u64 {
    if value >= 0.0 {
        value as u64
    } else {
        (value as i64) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_bitmask_metadata() {
        let attrs = json!({
            "flag_meanings": "good saturation cloud shadow",
            "flag_masks": "1 2 4 8",
            "_FillValue": 255
        })
        .as_object()
        .unwrap()
        .clone();

        let flags = parse_cf_flags(&attrs).expect("flags");
        assert_eq!(flags.meanings.len(), 4);
        assert!(flags.uses_bitmasks());
        assert_eq!(flags.codes, vec![1, 2, 4, 8]);
    }

    use ndarray::IxDyn;

    #[test]
    fn extracts_selected_bit_mask() {
        let flags = CfFlags {
            meanings: vec!["good".into(), "cloud".into()],
            codes: vec![1, 4],
            mode: CfFlagMode::Masks,
            fill_value: Some(255.0),
        };
        let values =
            ArrayD::from_shape_vec(IxDyn(&[2, 2]), vec![1.0, 5.0, 4.0, 255.0]).unwrap();
        let cloud = apply_flag_selection(&values, &flags, 1);
        assert_eq!(cloud[[0, 0]], 0.0);
        assert_eq!(cloud[[0, 1]], 1.0);
        assert_eq!(cloud[[1, 0]], 1.0);
        assert!(cloud[[1, 1]].is_nan());
    }

    #[test]
    fn extracts_exclusive_flag_values() {
        let flags = CfFlags {
            meanings: vec!["clear".into(), "cloudy".into()],
            codes: vec![0, 1],
            mode: CfFlagMode::Values,
            fill_value: None,
        };
        let values = ArrayD::from_shape_vec(IxDyn(&[3]), vec![0.0, 1.0, 2.0]).unwrap();
        let cloudy = apply_flag_selection(&values, &flags, 1);
        assert_eq!(
            cloudy,
            ArrayD::from_shape_vec(IxDyn(&[3]), vec![0.0, 1.0, 0.0]).unwrap()
        );
    }
}
