use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ocs_plugin_api::host::acadrust::types::Color;
use ocs_plugin_api::host::Handle;
use rf_core::calibration::CalibrationSegment;
use rf_core::fitter::FitterResult;

use crate::fit::FitOptions;
use crate::mask::SharedMask;
use crate::pdf::PdfDocument;
use crate::preprocess::PreprocessSettings;
use crate::raster::{RasterData, RasterTransform};

#[derive(Default)]
pub(crate) struct PluginState {
    pub raster: Option<RasterData>,
    pub original_raster: Option<RasterData>,
    pub preprocess: PreprocessSettings,
    pub raster_handle: Option<Handle>,
    pub raster_path: Option<PathBuf>,
    pub source_path: Option<PathBuf>,
    pub pdf: Option<PdfDocument>,
    pub page_index: usize,
    pub mask: SharedMask,
    pub transform: RasterTransform,
    pub calibrated: bool,
    pub segments: Vec<CalibrationSegment>,
    pub pending_segment: Option<[rf_core::geometry::Point; 2]>,
    pub pending_calibration: Option<(RasterTransform, Vec<CalibrationSegment>)>,
    pub pending_level: Option<RasterTransform>,
    pub pending_origin: Option<[f64; 2]>,
    pub mask_overlay_handles: Vec<Handle>,
    pub guides_enabled: bool,
    pub guide_handles: Vec<Handle>,
    pub fit_options: FitOptions,
    pub last_fit: Option<FitterResult>,
    pub last_origins: Vec<Handle>,
    pub original_colors: Vec<(Handle, Color)>,
    pub heatmap_active: bool,
    pub restore_pending: bool,
    pub pending_fit_selection: Vec<Handle>,
}

pub(crate) type SharedState = Arc<Mutex<PluginState>>;

pub(crate) fn lock_state(
    state: &SharedState,
) -> Result<std::sync::MutexGuard<'_, PluginState>, String> {
    state
        .lock()
        .map_err(|_| "RasterFit state is unavailable after a previous failure".to_string())
}
