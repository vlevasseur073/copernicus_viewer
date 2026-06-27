use anyhow::{Result, bail};

/// Sentinel-3 SAFE filename token → EOPF product type.
const S3_PATTERNS: &[(&str, &str)] = &[
    ("SL_2_LST", "S03SLSLST"),
    ("SL_2_FRP", "S03SLSFRP"),
    ("SL_1_RBT", "S03SLSRBT"),
    ("OL_2_LFR", "S03OLCLFR"),
    ("OL_2_LRR", "S03OLCLRR"),
    ("OL_2_WFR", "S03OLCLFR"),
    ("OL_2_WRR", "S03OLCLRR"),
    ("OL_1_EFR", "S03OLCEFR"),
    ("OL_1_ERR", "S03OLCERR"),
    ("OL_1_SPC", "S03OLCSPC"),
    ("OL_1_RAC", "S03OLCRAC"),
    ("SY_2_AOD", "S03SYNAOD"),
    ("SY_2_VGP", "S03SYNVGT"),
    ("SY_2_VG1", "S03SYNV01"),
    ("SY_2_V10", "S03SYNV10"),
    ("SY_2_VGK", "S03SYNVGK"),
    ("SY_2_SDR", "S03SYNSDR"),
];

/// Detect EOPF product type from a `.SEN3` directory or file name.
pub fn detect_product_type(name: &str) -> Result<String> {
    let upper = name.to_uppercase();
    if !upper.contains("S3") {
        bail!("not a Sentinel-3 SAFE product name: {name}");
    }

    for (token, ptype) in S3_PATTERNS {
        if upper.contains(token) {
            return Ok((*ptype).to_string());
        }
    }

    bail!("unsupported Sentinel-3 SAFE product type for {name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sl_lst() {
        let name = "S3A_SL_2_LST____20260622T102053_20260622T102353_20260622T123949_0179_141_008_2160_PS1_O_NR_005.SEN3";
        assert_eq!(detect_product_type(name).unwrap(), "S03SLSLST");
    }
}
