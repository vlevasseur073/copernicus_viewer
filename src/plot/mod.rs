//! Array plotting: async loading, geo-referenced heatmaps, and CF flag visualization.

pub mod flags;
pub mod georef;
pub mod loader;
pub mod panel;

pub use flags::{parse_cf_flags, CfFlagMode, CfFlags, FlagSelection};
pub use georef::{axis_label, extent_description, GeorefInfo};
pub use loader::{
    load_plot_data, mutex_progress, shared_progress, PlotData, PlotLoadResult, PlotRequest,
    ProgressCallback,
};
pub use panel::PlotPanel;
