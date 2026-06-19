//! User-configurable thresholds and toggles for product comparison.
//!
//! Defaults mirror the sentineltoolbox `compare_product_datatrees` behaviour.

/// Options controlling which checks run and how numeric differences are judged.
#[derive(Clone, Debug)]
pub struct CompareOptions {
    /// Compare relative error for all variables (overrides auto mode).
    pub relative: bool,
    /// Compare absolute error for all variables (overrides auto mode).
    pub absolute: bool,
    /// Relative or absolute error threshold (depending on mode).
    pub threshold: f64,
    /// Multiplier applied to `scale_factor` for packed variables in absolute mode.
    pub threshold_packed: f64,
    /// Maximum allowed ratio of outlier pixels per variable.
    pub threshold_nb_outliers: f64,
    /// Maximum allowed valid-pixel coverage difference ratio per variable.
    pub threshold_coverage: f64,
    /// Compare group/array metadata and hierarchy layout.
    pub structure: bool,
    /// Compare measurement variable array values chunk-by-chunk.
    pub data: bool,
    /// Compare CF flag / mask variables bit-by-bit.
    pub flags: bool,
    /// Include chunk layout in structure checks (non-fatal warnings when shapes match).
    pub chunks: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            relative: true,
            absolute: false,
            threshold: 1.0e-6,
            threshold_packed: 1.5,
            threshold_nb_outliers: 0.01,
            threshold_coverage: 0.01,
            structure: false,
            data: true,
            flags: true,
            chunks: true,
        }
    }
}
