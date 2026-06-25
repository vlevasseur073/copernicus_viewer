//! Array plotting: async loading, geo-referenced heatmaps, and CF flag visualization.

pub mod cf_decode;
pub mod flags;
pub mod georef;
pub mod loader;
pub mod panel;

pub use cf_decode::{CfEncoding, apply_cf_decode, parse_cf_encoding};

pub use flags::{CfFlagMode, CfFlags, FlagSelection, parse_cf_flags};
pub use georef::{GeorefInfo, axis_label, extent_description};
pub use loader::{
    PlotData, PlotLoadResult, PlotRequest, ProgressCallback, load_plot_data, mutex_progress,
    shared_progress,
};
pub use panel::PlotPanel;
