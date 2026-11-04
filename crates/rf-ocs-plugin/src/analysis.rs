use std::collections::HashMap;
use std::fs;

use ocs_plugin_api::host::acadrust::types::Color;
use ocs_plugin_api::host::{Handle, HostApi};
use rf_core::fitter::{FitterResult, PrimitiveFitStat};
use rf_core::report::{summary_csv, summary_json};

use crate::state::{lock_state, SharedState};

#[derive(Debug, Clone, Copy)]
pub(crate) enum ReportFormat {
    Json,
    Csv,
}

pub(crate) fn report_text(result: &FitterResult, format: ReportFormat) -> String {
    match format {
        ReportFormat::Json => summary_json(result),
        ReportFormat::Csv => summary_csv(result),
    }
}

pub(crate) fn export_report(
    state: &SharedState,
    path: &str,
    format: ReportFormat,
) -> Result<usize, String> {
    if path.trim().is_empty() {
        return Err("choose a report output path".to_string());
    }
    let result = lock_state(state)?
        .last_fit
        .clone()
        .ok_or_else(|| "run fit before exporting a report".to_string())?;
    let text = report_text(&result, format);
    fs::write(path, &text).map_err(|error| format!("could not write report '{path}': {error}"))?;
    Ok(text.len())
}

pub(crate) fn heatmap_command(
    host: &mut dyn HostApi,
    state: &SharedState,
    restore: bool,
) -> Result<usize, String> {
    let (fit, origins) = {
        let state = lock_state(state)?;
        let fit = state
            .last_fit
            .clone()
            .ok_or_else(|| "run fit before showing a heatmap".to_string())?;
        (fit, state.last_origins.clone())
    };
    let colors = heatmap_colors(&fit, &origins);
    if colors.is_empty() {
        return Err("last fit has no native primitive statistics".to_string());
    }
    let handles: Vec<Handle> = colors.keys().copied().collect();
    let entities = host.entities_batch(&handles)?;
    let mut updated = Vec::new();
    let mut backups = Vec::new();
    for (handle, entity) in handles.iter().zip(entities) {
        let mut entity = entity.ok_or_else(|| format!("fit entity {handle} no longer exists"))?;
        let color = entity.common().color;
        backups.push((*handle, color));
        if restore {
            entity.common_mut().color = original_color(state, *handle)?;
        } else if let Some(color) = colors.get(handle) {
            entity.common_mut().color = *color;
        }
        updated.push(entity);
    }
    let count = updated.len();
    host.update_entities_transaction("RasterFit heatmap", updated)?;
    host.bump_geometry();
    host.set_dirty();
    if !restore {
        save_original_colors(state, backups)?;
    }
    Ok(count)
}

fn original_color(state: &SharedState, handle: Handle) -> Result<Color, String> {
    let backups = original_colors(state)?;
    backups
        .into_iter()
        .find(|(candidate, _)| *candidate == handle)
        .map(|(_, color)| color)
        .ok_or_else(|| format!("original color for entity {handle} is unavailable"))
}

fn save_original_colors(state: &SharedState, backups: Vec<(Handle, Color)>) -> Result<(), String> {
    let mut state = lock_state(state)?;
    let mut current = original_colors_from_state(&state);
    for (handle, color) in backups {
        if !current.iter().any(|(candidate, _)| *candidate == handle) {
            current.push((handle, color));
        }
    }
    state.original_colors = current;
    Ok(())
}

fn original_colors(state: &SharedState) -> Result<Vec<(Handle, Color)>, String> {
    Ok(lock_state(state)?.original_colors.clone())
}

fn original_colors_from_state(state: &crate::state::PluginState) -> Vec<(Handle, Color)> {
    state.original_colors.clone()
}

fn heatmap_colors(fit: &FitterResult, origins: &[Handle]) -> HashMap<Handle, Color> {
    let mut worst: HashMap<Handle, &PrimitiveFitStat> = HashMap::new();
    for (stat, handle) in fit.primitive_stats.iter().zip(origins.iter().copied()) {
        worst
            .entry(handle)
            .and_modify(|current| {
                if stat.rms_px > current.rms_px {
                    *current = stat;
                }
            })
            .or_insert(stat);
    }
    worst
        .into_iter()
        .map(|(handle, stat)| (handle, heatmap_color(stat)))
        .collect()
}

fn heatmap_color(stat: &PrimitiveFitStat) -> Color {
    if stat.low_coverage_warning || stat.rms_px >= 1.0 {
        Color::from_rgb(229, 57, 53)
    } else if stat.rms_px >= 0.3 {
        Color::from_rgb(255, 193, 7)
    } else {
        Color::from_rgb(76, 207, 111)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(id: u32, rms: f64) -> PrimitiveFitStat {
        PrimitiveFitStat {
            kind: "line".to_string(),
            id,
            total_samples: 10,
            matched_samples: 9,
            coverage: 0.9,
            rms_px: rms,
            low_coverage_warning: false,
        }
    }

    fn fit() -> FitterResult {
        FitterResult {
            params: Vec::new(),
            transform: [0.0; 3],
            scale: 1.0,
            rms_px: 0.0,
            max_constraint: 0.0,
            iterations: 1,
            converged: true,
            gauge_warning: false,
            residuals: vec![0.0, 1.0],
            total_samples: 2,
            matched_samples: 2,
            inlier_ratio: 1.0,
            uncovered_ratio: 0.0,
            mask_too_narrow: false,
            transform_stddev: [0.0; 3],
            primitive_stats: vec![stat(1, 0.1), stat(2, 0.5), stat(3, 1.5)],
        }
    }

    #[test]
    fn report_text_has_json_and_csv_variants() {
        assert!(report_text(&fit(), ReportFormat::Json).contains("rms_px"));
        assert!(report_text(&fit(), ReportFormat::Csv).contains("primitive_kind"));
    }

    #[test]
    fn export_writes_a_real_json_report() {
        let state = SharedState::default();
        lock_state(&state).unwrap().last_fit = Some(fit());
        let path = std::env::temp_dir().join(format!(
            "rasterfit-report-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bytes = export_report(&state, path.to_str().unwrap(), ReportFormat::Json).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(bytes, text.len());
        assert!(text.contains("\"rms_px\""));
    }

    #[test]
    fn heatmap_maps_rms_and_worst_statistic_per_handle() {
        let colors = heatmap_colors(&fit(), &[Handle::new(1), Handle::new(2)]);
        assert_eq!(colors[&Handle::new(1)], Color::from_rgb(76, 207, 111));
        assert_eq!(colors[&Handle::new(2)], Color::from_rgb(255, 193, 7));
        assert_eq!(colors.len(), 2);
        assert_eq!(heatmap_color(&stat(3, 1.5)), Color::from_rgb(229, 57, 53));
    }
}
