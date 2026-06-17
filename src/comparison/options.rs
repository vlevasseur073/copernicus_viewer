/// Options mirroring `compare_product_datatrees` in sentineltoolbox.
#[derive(Clone, Debug)]
pub struct CompareOptions {
    pub relative: bool,
    pub absolute: bool,
    pub threshold: f64,
    pub threshold_packed: f64,
    pub threshold_nb_outliers: f64,
    pub threshold_coverage: f64,
    pub structure: bool,
    pub data: bool,
    pub flags: bool,
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
