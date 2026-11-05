use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cad::{add_raster, sketch_to_entities, update_raster_transform};
use crate::calibration::{level_transform, solve_calibration};
use crate::fit::{fit_native_sketch, fit_status};
use crate::mask::{mask_from_lines, mask_params, MaskCommand, MaskMode, SharedMask};
use crate::native::{selected_sketch, update_fit_entities};
use crate::pdf::{load as load_pdf, PdfDocument};
use crate::raster::{decode_raster, raster_to_world, world_to_raster, RasterData, RasterTransform};
use crate::state::{lock_state, SharedState};
use ocs_plugin_api::dialog::{DialogControl, DialogDef, DialogResponse, DialogValue};
use ocs_plugin_api::host::{CommandStep, Handle, HostApi, InteractiveCommand, PreviewWire};
use rf_core::calibration::{CalibrationSegment, SegmentOrientation};
use rf_core::geometry::Point;

pub(crate) fn dispatch(host: &mut dyn HostApi, state: &SharedState, cmd: &str) -> bool {
    if lock_state(state)
        .map(|state| state.restore_pending)
        .unwrap_or(false)
    {
        if let Err(error) = restore_workspace(host, state) {
            host.push_error(&format!("RasterFit workspace restore failed: {error}"));
        }
        if let Ok(mut state) = lock_state(state) {
            state.restore_pending = false;
        }
    }
    if let Some(path) = command_path(cmd, "RF_OPEN") {
        let result = path
            .ok_or_else(missing_path)
            .and_then(|path| open_raster(host, state, path));
        report(host, state, result);
        return true;
    }
    if let Some(path) = command_path(cmd, "RF_OPEN_PDF") {
        let result = path
            .ok_or_else(missing_path)
            .and_then(|path| open_pdf(host, state, path));
        report(host, state, result);
        return true;
    }
    if let Some(length) = command_length(cmd, "RF_CALIBRATE") {
        let result = start_calibration(host, state, length);
        report(host, state, result);
        return true;
    }
    match cmd {
        "RF_LINE" => {
            let result = crate::drawing::start(host, state, crate::drawing::DrawKind::Line);
            report(host, state, result);
            true
        }
        "RF_POLYLINE" => {
            let result = crate::drawing::start(host, state, crate::drawing::DrawKind::Polyline);
            report(host, state, result);
            true
        }
        "RF_IMAGE_ADJUST" => {
            let result = crate::image_tools::start_image_adjustments(host, state);
            report(host, state, result);
            true
        }
        "RF_IMAGE_APPLY" => {
            let result = crate::image_tools::apply_image_adjustments(host, state);
            report(host, state, result);
            true
        }
        "RF_CALIBRATE_LENGTH" => {
            let result = start_calibration_dialog(host, state);
            report(host, state, result);
            true
        }
        "RF_CALIBRATE_APPLY" => {
            let result = apply_calibration(host, state);
            report(host, state, result);
            true
        }
        "RF_LEVEL" => {
            let result = start_level(host, state);
            report(host, state, result);
            true
        }
        "RF_LEVEL_APPLY" => {
            let result = apply_level(host, state);
            report(host, state, result);
            true
        }
        "RF_ORIGIN" => {
            let result = start_origin(host, state);
            report(host, state, result);
            true
        }
        "RF_ORIGIN_APPLY" => {
            let result = apply_origin(host, state);
            report(host, state, result);
            true
        }
        "RF_ORIGIN_RESET" => {
            let result = apply_origin_value(host, state, [0.0, 0.0]);
            report(host, state, result);
            true
        }
        "RF_GUIDES" => {
            let result = crate::overlay::toggle_guides(host, state);
            report(host, state, result);
            true
        }
        "RF_MASK_SYNC" => {
            let result = crate::overlay::sync_mask(host, state)
                .map(|count| format!("RasterFit mask visible: {count} filled region(s)"));
            report(host, state, result);
            true
        }
        "RF_PAGE_PREV" => {
            let result = change_pdf_page(host, state, -1);
            report(host, state, result);
            true
        }
        "RF_PAGE_NEXT" => {
            let result = change_pdf_page(host, state, 1);
            report(host, state, result);
            true
        }
        "RF_MASK_BRUSH" => {
            let result = crate::dialogs::start_brush_size(host, state);
            report(host, state, result);
            true
        }
        "RF_MASK_BRUSH_APPLY" => {
            let result = lock_state(state).and_then(|state| {
                state
                    .mask
                    .lock()
                    .map(|mask| format!("RasterFit mask brush radius: {:.1} px", mask.brush_radius))
                    .map_err(|_| "mask state is unavailable".to_string())
            });
            report(host, state, result);
            true
        }
        "RF_MASK_INCLUDE" => {
            let result = start_mask(host, state, MaskMode::Include);
            report(host, state, result);
            true
        }
        "RF_MASK_EXCLUDE" => {
            let result = start_mask(host, state, MaskMode::Exclude);
            report(host, state, result);
            true
        }
        "RF_MASK_RECT" => {
            let result = start_mask(host, state, MaskMode::Rect);
            report(host, state, result);
            true
        }
        "RF_MASK_POLYGON" => {
            let result = start_mask(host, state, MaskMode::Polygon);
            report(host, state, result);
            true
        }
        "RF_MASK_ERASE" => {
            let result = start_mask(host, state, MaskMode::Eraser);
            report(host, state, result);
            true
        }
        "RF_MASK_FROM_SELECTION" => {
            let result = mask_from_selection(host, state);
            report(host, state, result);
            true
        }
        "RF_MASK_CLEAR" => {
            let result = clear_mask(host, state);
            report(host, state, result);
            true
        }
        "RF_CONSTRAINT_COINCIDENT" => {
            let result = crate::constraints::create_constraint_from_selection(host, false)
                .map(|_| "Native coincident constraint created".to_string());
            report(host, state, result);
            true
        }
        "RF_CONSTRAINT_TANGENT" => {
            let result = crate::constraints::create_constraint_from_selection(host, true)
                .map(|_| "Native tangent constraint created".to_string());
            report(host, state, result);
            true
        }
        "RF_MEASURE_DISTANCE" => {
            let result = native_command(host, "DIST", "Distance measurement");
            report(host, state, result);
            true
        }
        "RF_MEASURE_ANGLE" => {
            let result = native_command(host, "MEASUREANGLE", "Angle measurement");
            report(host, state, result);
            true
        }
        "RF_HATCH" => {
            let result = native_command(host, "HATCH", "Hatch creation");
            report(host, state, result);
            true
        }
        "RF_FIT_OPTIONS" => {
            let result = crate::dialogs::start_fit_options(host, state);
            report(host, state, result);
            true
        }
        "RF_HEATMAP" => {
            let restore = lock_state(state)
                .map(|state| state.heatmap_active)
                .unwrap_or(false);
            let result = crate::analysis::heatmap_command(host, state, restore);
            if result.is_ok() {
                if let Ok(mut state) = lock_state(state) {
                    state.heatmap_active = !restore;
                }
            }
            let result = result.map(|count| {
                format!(
                    "RasterFit heatmap {} for {count} entities",
                    if restore { "restored" } else { "enabled" }
                )
            });
            report(host, state, result);
            true
        }
        "RF_EXPORT_REPORT" => {
            let result = crate::dialogs::start_report_export(host, state);
            report(host, state, result);
            true
        }
        "RF_FIT_PREVIEW" => {
            let result = crate::preview::start_fit_preview(host, state);
            report(host, state, result);
            true
        }
        "RF_FIT" => {
            let result = fit_command(host, state);
            report(host, state, result);
            true
        }
        "RF_STATUS" => {
            crate::panel::toggle(host, state);
            report(host, state, status(state));
            true
        }
        _ => false,
    }
}

fn open_raster(host: &mut dyn HostApi, state: &SharedState, path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read raster '{}': {error}", path.display()))?;
    let raster = decode_raster(&bytes)
        .map_err(|error| format!("failed to decode raster '{}': {error}", path.display()))?;
    let width = raster.width;
    let height = raster.height;
    install_raster(
        host,
        state,
        raster,
        path.to_path_buf(),
        path.to_path_buf(),
        None,
        0,
    )?;
    Ok(format!(
        "RasterFit opened {}x{} raster; calibration required",
        width, height
    ))
}

fn open_pdf(host: &mut dyn HostApi, state: &SharedState, path: &Path) -> Result<String, String> {
    let document = load_pdf(path)?;
    if document.page_count() == 0 {
        return Err("PDF has no pages".to_string());
    }
    let page_count = document.page_count();
    let rendered = document.render_page(0, 150.0)?;
    let dpi = rendered.dpi;
    let display_path = write_png_cache(path, 0, &rendered.png)?;
    install_raster(
        host,
        state,
        rendered.raster,
        display_path,
        path.to_path_buf(),
        Some(document),
        0,
    )?;
    Ok(format!(
        "RasterFit opened PDF page 1/{page_count} at {dpi} DPI; calibration required"
    ))
}

fn change_pdf_page(
    host: &mut dyn HostApi,
    state: &SharedState,
    delta: isize,
) -> Result<String, String> {
    let (target, page_count, source_path) = {
        let state = lock_state(state)?;
        let document = state
            .pdf
            .as_ref()
            .ok_or_else(|| "open a PDF before changing pages".to_string())?;
        let page_count = document.page_count();
        let target = (state.page_index as isize + delta).clamp(0, page_count as isize - 1) as usize;
        let source_path = state
            .source_path
            .clone()
            .ok_or_else(|| "PDF source path is unavailable".to_string())?;
        (target, page_count, source_path)
    };
    let current = lock_state(state)?.page_index;
    if target == current {
        return Ok(format!("RasterFit PDF page {}/{}", target + 1, page_count));
    }
    let rendered = {
        let state = lock_state(state)?;
        state
            .pdf
            .as_ref()
            .expect("PDF checked above")
            .render_page(target, 150.0)?
    };
    let display_path = write_png_cache(&source_path, target, &rendered.png)?;
    let document = lock_state(state)?.pdf.take();
    install_raster(
        host,
        state,
        rendered.raster,
        display_path,
        source_path,
        document,
        target,
    )?;
    Ok(format!(
        "RasterFit PDF page {}/{}; calibration required",
        target + 1,
        page_count
    ))
}

fn install_raster(
    host: &mut dyn HostApi,
    state: &SharedState,
    raster: RasterData,
    raster_path: PathBuf,
    source_path: PathBuf,
    pdf: Option<PdfDocument>,
    page_index: usize,
) -> Result<(), String> {
    crate::overlay::clear_mask(host, state)?;
    crate::overlay::clear_guides(host, state)?;
    let handle = add_raster(host, &raster_path, &raster, RasterTransform::default())?;
    crate::overlay::raster_to_back(host, handle)?;
    let mut state = lock_state(state)?;
    state.original_raster = Some(raster.clone());
    state.preprocess = Default::default();
    state.raster = Some(raster);
    state.raster_handle = Some(handle);
    state.raster_path = Some(raster_path);
    state.source_path = Some(source_path);
    state.pdf = pdf;
    state.page_index = page_index;
    state.transform = RasterTransform::default();
    state.calibrated = false;
    state.segments.clear();
    state.pending_segment = None;
    state.pending_calibration = None;
    state.pending_level = None;
    state.pending_origin = None;
    state.last_fit = None;
    state.mask = SharedMask::default();
    Ok(())
}

pub(crate) fn restore_workspace(
    host: &mut dyn HostApi,
    state: &SharedState,
) -> Result<bool, String> {
    let snapshot = match crate::workspace::load_workspace(host) {
        Ok(snapshot) => snapshot,
        Err(_) => return Ok(false),
    };
    let (raster, pdf, raster_path) = match snapshot.source_kind {
        crate::workspace::SourceKind::Image => {
            let bytes = fs::read(&snapshot.source_path).map_err(|error| {
                format!(
                    "failed to restore raster '{}': {error}",
                    snapshot.source_path.display()
                )
            })?;
            let raster = decode_raster(&bytes)?;
            let raster_path = if snapshot.raster_path.exists() {
                snapshot.raster_path.clone()
            } else {
                snapshot.source_path.clone()
            };
            (raster, None, raster_path)
        }
        crate::workspace::SourceKind::Pdf => {
            let document = load_pdf(&snapshot.source_path)?;
            let rendered = document.render_page(snapshot.page_index, snapshot.dpi)?;
            let raster_path = if snapshot.raster_path.exists() {
                snapshot.raster_path.clone()
            } else {
                write_png_cache(&snapshot.source_path, snapshot.page_index, &rendered.png)?
            };
            (rendered.raster, Some(document), raster_path)
        }
    };
    let original_raster = raster;
    let raster = crate::preprocess::apply(&original_raster, snapshot.preprocess)?;
    let raster_path = if snapshot.preprocess != crate::preprocess::PreprocessSettings::default() {
        crate::image_tools::write_processed_png(&raster, Some(snapshot.source_path.as_path()))?
    } else {
        raster_path
    };

    let saved_handle = crate::workspace::raster_handle(&snapshot);
    let exists = host
        .entities_batch(&[saved_handle])?
        .into_iter()
        .next()
        .flatten()
        .is_some();
    let handle = if exists {
        update_raster_transform(host, saved_handle, snapshot.transform)?;
        saved_handle
    } else {
        add_raster(host, &raster_path, &raster, snapshot.transform)?
    };

    let shared = state;
    let mut state = lock_state(state)?;
    state.original_raster = Some(original_raster);
    state.preprocess = snapshot.preprocess;
    state.raster = Some(raster);
    state.raster_handle = Some(handle);
    state.raster_path = Some(raster_path);
    state.source_path = Some(snapshot.source_path);
    state.pdf = pdf;
    state.page_index = snapshot.page_index;
    state.transform = snapshot.transform;
    state.calibrated = snapshot.calibrated;
    state.segments = snapshot.segments;
    state.pending_segment = None;
    state.pending_calibration = None;
    state.pending_level = None;
    state.pending_origin = None;
    state.mask_overlay_handles = snapshot
        .mask_overlay_handles
        .iter()
        .map(|handle| Handle::new(*handle))
        .collect();
    state.guides_enabled = snapshot.guides_enabled;
    state.guide_handles = snapshot
        .guide_handles
        .iter()
        .map(|handle| Handle::new(*handle))
        .collect();
    state.fit_options = snapshot.fit_options;
    state.last_fit = snapshot.last_fit;
    state.last_origins = snapshot
        .last_origins
        .iter()
        .map(|handle| Handle::new(*handle))
        .collect();
    state.original_colors.clear();
    state.mask = SharedMask::default();
    {
        let mut mask = state
            .mask
            .lock()
            .map_err(|_| "mask state is unavailable".to_string())?;
        mask.strokes = snapshot.mask_strokes;
        mask.enabled = snapshot.mask_enabled;
        mask.brush_radius = snapshot.mask_brush_radius;
    }
    state.restore_pending = false;
    drop(state);
    crate::overlay::restore(host, shared)?;
    Ok(true)
}

fn write_png_cache(source: &Path, page_index: usize, png: &[u8]) -> Result<PathBuf, String> {
    let directory = std::env::temp_dir().join("rasterfit-ocs");
    fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "could not create PDF cache '{}': {error}",
            directory.display()
        )
    })?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_nanos();
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("document");
    let path = directory.join(format!(
        "{}-page-{}-{}-{unique}.png",
        stem,
        page_index + 1,
        std::process::id()
    ));
    fs::write(&path, png)
        .map_err(|error| format!("could not cache PDF page '{}': {error}", path.display()))?;
    Ok(path)
}

fn start_mask(
    host: &mut dyn HostApi,
    state: &SharedState,
    mode: MaskMode,
) -> Result<String, String> {
    let (mask, width, height, transform) = {
        let state = lock_state(state)?;
        let raster = state
            .raster
            .as_ref()
            .ok_or_else(|| "open a raster before editing the mask".to_string())?;
        (
            state.mask.clone(),
            raster.width,
            raster.height,
            state.transform,
        )
    };
    host.start_interactive(Box::new(MaskCommand::new(
        mask, mode, width, height, transform,
    )));
    Ok("RasterFit mask command started".to_string())
}

fn mask_from_selection(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    let handles = host.selection();
    if handles.is_empty() {
        return Err("select native LINE entities for the mask".to_string());
    }
    let entities = host.entities_batch(&handles)?;
    let entities = entities
        .into_iter()
        .map(|entity| entity.ok_or_else(|| "selected entity no longer exists".to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let (height, transform) = {
        let state = lock_state(state)?;
        let raster = state
            .raster
            .as_ref()
            .ok_or_else(|| "open a raster before creating a mask".to_string())?;
        (raster.height, state.transform)
    };
    let strokes = mask_from_lines(&entities, transform, height)?;
    if strokes.is_empty() {
        return Err("selection has no native LINE entities for the mask".to_string());
    }
    let count = strokes.len();
    let shared = state;
    let state = lock_state(state)?;
    let mut mask = state
        .mask
        .lock()
        .map_err(|_| "mask state is unavailable".to_string())?;
    mask.strokes.extend(strokes);
    mask.enabled = true;
    drop(mask);
    drop(state);
    crate::overlay::sync_mask(host, shared)?;
    Ok(format!("RasterFit mask created from {count} line(s)"))
}

fn clear_mask(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    {
        let state = lock_state(state)?;
        let mut mask = state
            .mask
            .lock()
            .map_err(|_| "mask state is unavailable".to_string())?;
        mask.strokes.clear();
        mask.enabled = false;
    }
    crate::overlay::clear_mask(host, state)?;
    Ok("RasterFit mask cleared".to_string())
}

fn start_calibration(
    host: &mut dyn HostApi,
    state: &SharedState,
    known_length: Option<f64>,
) -> Result<String, String> {
    start_point_command(host, state, known_length, false)?;
    Ok("RasterFit calibration: pick the first point".to_string())
}

fn start_level(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    start_point_command(host, state, None, true)?;
    Ok("RasterFit level: pick the first point on a raster line".to_string())
}

fn start_point_command(
    host: &mut dyn HostApi,
    state: &SharedState,
    known_length: Option<f64>,
    level: bool,
) -> Result<(), String> {
    let (raster, transform) = {
        let state = lock_state(state)?;
        let raster = state
            .raster
            .clone()
            .ok_or_else(|| "open a raster before calibration or leveling".to_string())?;
        (raster, state.transform)
    };
    host.start_interactive(Box::new(CalibrationCommand {
        state: state.clone(),
        width: raster.width,
        height: raster.height,
        raster,
        transform,
        known_length,
        level,
        first_world: None,
        first_raster: None,
    }));
    Ok(())
}

fn start_calibration_dialog(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    if lock_state(state)?.pending_segment.is_none() {
        return Err("pick two calibration points first".to_string());
    }
    host.start_interactive(Box::new(CalibrationLengthCommand {
        state: state.clone(),
    }));
    Ok("RasterFit calibration: enter the segment length".to_string())
}

fn apply_calibration(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    let (transform, segments, handle) = {
        let state = lock_state(state)?;
        let (transform, segments) = state
            .pending_calibration
            .clone()
            .ok_or_else(|| "submit the calibration dialog first".to_string())?;
        let handle = state
            .raster_handle
            .ok_or_else(|| "the RasterFit raster is not attached to the document".to_string())?;
        (transform, segments, handle)
    };
    update_raster_transform(host, handle, transform)?;

    let count = segments.len();
    let mut state_guard = lock_state(state)?;
    state_guard.segments = segments;
    state_guard.transform = transform;
    state_guard.calibrated = true;
    state_guard.pending_segment = None;
    state_guard.pending_calibration = None;
    drop(state_guard);
    crate::overlay::sync_all(host, state)?;
    Ok(format!(
        "RasterFit calibration applied from {count} segment(s): {:.6} mm/px, rotation {:.3} deg",
        transform.sx,
        transform.theta.to_degrees()
    ))
}

fn apply_level(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    let (transform, handle) = {
        let state = lock_state(state)?;
        let transform = state
            .pending_level
            .ok_or_else(|| "pick two leveling points first".to_string())?;
        let handle = state
            .raster_handle
            .ok_or_else(|| "the RasterFit raster is not attached to the document".to_string())?;
        (transform, handle)
    };
    update_raster_transform(host, handle, transform)?;
    let mut state_guard = lock_state(state)?;
    state_guard.transform = transform;
    state_guard.pending_level = None;
    drop(state_guard);
    crate::overlay::sync_all(host, state)?;
    Ok(format!(
        "RasterFit leveled to the nearest axis: rotation {:.3} deg",
        transform.theta.to_degrees()
    ))
}

fn start_origin(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    if lock_state(state)?.raster_handle.is_none() {
        return Err("open a raster before setting its origin".to_string());
    }
    host.start_interactive(Box::new(OriginCommand {
        state: state.clone(),
    }));
    Ok("RasterFit origin: pick the new bottom-left raster origin".to_string())
}

fn apply_origin(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    let origin = lock_state(state)?
        .pending_origin
        .ok_or_else(|| "pick the new raster origin first".to_string())?;
    apply_origin_value(host, state, origin)
}

fn apply_origin_value(
    host: &mut dyn HostApi,
    state: &SharedState,
    origin: [f64; 2],
) -> Result<String, String> {
    if origin.iter().any(|coordinate| !coordinate.is_finite()) {
        return Err("raster origin must be finite".to_string());
    }
    let (mut transform, handle) = {
        let state = lock_state(state)?;
        let handle = state
            .raster_handle
            .ok_or_else(|| "the RasterFit raster is not attached to the document".to_string())?;
        (state.transform, handle)
    };
    transform.origin = origin;
    update_raster_transform(host, handle, transform)?;
    let mut state_guard = lock_state(state)?;
    state_guard.transform = transform;
    state_guard.pending_origin = None;
    drop(state_guard);
    crate::overlay::sync_all(host, state)?;
    Ok(format!(
        "RasterFit origin: {:.3}, {:.3} mm; scale and rotation unchanged",
        origin[0], origin[1]
    ))
}

fn calibration_dialog() -> DialogDef {
    DialogDef::new("RasterFit calibration")
        .submit_label("Apply segment")
        .with_control(DialogControl::Number {
            id: "length".to_string(),
            label: "Real length (mm)".to_string(),
            default: 100.0,
            min: Some(0.000001),
            max: None,
            step: Some(1.0),
        })
        .with_control(DialogControl::Choice {
            id: "orientation".to_string(),
            label: "Orientation".to_string(),
            options: vec![
                "Free".to_string(),
                "Horizontal".to_string(),
                "Vertical".to_string(),
                "Angle".to_string(),
            ],
            default_index: 0,
        })
        .with_control(DialogControl::Number {
            id: "angle".to_string(),
            label: "World angle (degrees, for Angle orientation)".to_string(),
            default: 0.0,
            min: None,
            max: None,
            step: Some(15.0),
        })
        .width(420.0)
}

fn calibration_values(
    values: &HashMap<String, DialogValue>,
) -> Result<(f64, SegmentOrientation), String> {
    let length = number_value(values, "length")?;
    if !length.is_finite() || length <= 0.0 {
        return Err("calibration length must be a positive finite number".to_string());
    }
    let orientation = match choice_value(values, "orientation")? {
        0 => SegmentOrientation::Free,
        1 => SegmentOrientation::Horizontal,
        2 => SegmentOrientation::Vertical,
        3 => SegmentOrientation::Angle(number_value(values, "angle")?.to_radians()),
        index => return Err(format!("unknown calibration orientation {index}")),
    };
    Ok((length, orientation))
}

fn fit_command(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    let (raster, transform, options) = {
        let state = lock_state(state)?;
        if !state.calibrated {
            return Err("calibrate the raster before fitting".to_string());
        }
        (
            state
                .raster
                .clone()
                .ok_or_else(|| "open a raster before fitting".to_string())?,
            state.transform,
            state.fit_options,
        )
    };

    if host.selection().is_empty() {
        let pending = lock_state(state)?.pending_fit_selection.clone();
        if !pending.is_empty() {
            host.set_selection(&pending)?;
        }
    }
    let mut native = selected_sketch(host, transform, raster.height)?
        .ok_or_else(|| "select native LINE/CIRCLE/ARC entities before fitting".to_string())?;
    let native_constraints = crate::constraints::read_native_constraints(host.document(), &native)?;
    let constraint_summary = if native_constraints.mapped + native_constraints.skipped > 0 {
        format!(
            "; native constraints: {} mapped, {} skipped",
            native_constraints.mapped, native_constraints.skipped
        )
    } else {
        String::new()
    };
    native_constraints.apply_to_sketch(&mut native.sketch);
    let origins = native.primitive_origins.clone();
    let (strokes, params) = {
        let state = lock_state(state)?;
        let mask = state
            .mask
            .lock()
            .map_err(|_| "mask state is unavailable".to_string())?;
        (mask.strokes.clone(), mask_params(&mask))
    };
    let mask = params.enabled.then_some((strokes.as_slice(), params));
    let output = fit_native_sketch(&native.sketch, &raster, mask, options)?;
    let entities = sketch_to_entities(&output.sketch, transform, raster.height)?;
    let message = fit_status(&output.result);
    let warnings = fit_warnings(&output.result);
    let count = update_fit_entities(
        host,
        native.originals,
        entities,
        &origins,
        &output.result,
        transform,
        raster.height,
        options.fit_geometry,
    )?;
    let mut state = lock_state(state)?;
    state.last_fit = Some(output.result);
    state.last_origins = origins;
    state.pending_fit_selection.clear();
    drop(state);
    Ok(format!(
        "{message}; updated {count} selected CAD entities{warnings}{constraint_summary}"
    ))
}

fn native_command(host: &mut dyn HostApi, command: &str, label: &str) -> Result<String, String> {
    if host.execute_command(command) {
        Ok(format!("{label}: native OpenCADStudio command started"))
    } else {
        Err(format!("OpenCADStudio could not start {command}"))
    }
}

fn status(state: &SharedState) -> Result<String, String> {
    let state = lock_state(state)?;
    let raster = state
        .raster
        .as_ref()
        .map(|raster| format!("{}x{}", raster.width, raster.height))
        .unwrap_or_else(|| "none".to_string());
    let fit = state
        .last_fit
        .as_ref()
        .map(fit_status)
        .unwrap_or_else(|| "none".to_string());
    let page = state
        .pdf
        .as_ref()
        .map(|pdf| format!("{}/{}", state.page_index + 1, pdf.page_count()))
        .unwrap_or_else(|| "n/a".to_string());
    let diagnostics = state
        .raster
        .as_ref()
        .map(|raster| {
            crate::calibration::diagnostics(
                &state.segments,
                state.transform,
                raster.width,
                raster.height,
            )
        })
        .unwrap_or_else(|| "calibration diagnostics: no raster".to_string());
    Ok(format!(
        "RasterFit — raster: {raster}; PDF page: {page}; calibrated: {}; scale: {:.6} mm/px; rotation: {:.3} deg; segments: {}; last fit: {fit}; {diagnostics}",
        state.calibrated,
        state.transform.sx,
        state.transform.theta.to_degrees(),
        state.segments.len()
    ))
}

fn report(host: &mut dyn HostApi, state: &SharedState, result: Result<String, String>) {
    match result {
        Ok(message) => {
            if let Err(error) = crate::workspace::save_workspace(host, state) {
                host.push_error(&format!("RasterFit workspace save failed: {error}"));
            }
            crate::panel::sync(host, state);
            host.push_info(&message);
        }
        Err(error) => host.push_error(&format!("RasterFit: {error}")),
    }
}

fn fit_warnings(result: &rf_core::fitter::FitterResult) -> String {
    let mut warnings = Vec::new();
    if !result.converged {
        warnings.push("fit did not converge");
    }
    if result.gauge_warning {
        warnings.push("selection has no position/orientation anchor");
    }
    if result.uncovered_ratio > 0.5 {
        warnings.push("more than half of selected geometry samples are uncovered");
    }
    if warnings.is_empty() {
        String::new()
    } else {
        format!("; warning: {}", warnings.join(", "))
    }
}

fn command_path<'a>(cmd: &'a str, command: &str) -> Option<Option<&'a Path>> {
    if cmd == command {
        return Some(None);
    }
    cmd.strip_prefix(command)
        .filter(|suffix| suffix.starts_with(char::is_whitespace))
        .map(|suffix| suffix.trim_start())
        .filter(|path| !path.is_empty())
        .map(|path| Some(Path::new(path)))
}

fn command_length(cmd: &str, command: &str) -> Option<Option<f64>> {
    if cmd == command {
        return Some(None);
    }
    cmd.strip_prefix(command)
        .filter(|suffix| suffix.starts_with(char::is_whitespace))
        .and_then(|suffix| {
            suffix
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok())
        })
        .filter(|length| length.is_finite() && *length > 0.0)
        .map(Some)
}

fn missing_path() -> String {
    "choose a file through the RasterFit ribbon file dialog".to_string()
}

fn number_value(values: &HashMap<String, DialogValue>, id: &str) -> Result<f64, String> {
    match values.get(id) {
        Some(DialogValue::Number(value)) => Ok(*value),
        _ => Err(format!("calibration dialog did not return numeric '{id}'")),
    }
}

fn choice_value(values: &HashMap<String, DialogValue>, id: &str) -> Result<usize, String> {
    match values.get(id) {
        Some(DialogValue::Choice { index, .. }) => Ok(*index),
        _ => Err(format!("calibration dialog did not return choice '{id}'")),
    }
}

fn prepare_pending_calibration(
    state: &SharedState,
    length: f64,
    orientation: SegmentOrientation,
) -> Result<(), String> {
    let mut state = lock_state(state)?;
    let [a, b] = state
        .pending_segment
        .ok_or_else(|| "pick two calibration points first".to_string())?;
    let mut segments = state.segments.clone();
    segments.push(CalibrationSegment {
        a,
        b,
        length,
        orientation,
    });
    let transform = solve_calibration(&segments, state.transform)?;
    state.pending_calibration = Some((transform, segments));
    Ok(())
}

struct OriginCommand {
    state: SharedState,
}

impl InteractiveCommand for OriginCommand {
    fn prompt(&self) -> String {
        "RasterFit: pick the new bottom-left raster origin".to_string()
    }

    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        if !pt[0].is_finite() || !pt[1].is_finite() {
            return CommandStep::NeedPoint;
        }
        match lock_state(&self.state) {
            Ok(mut state) => {
                state.pending_origin = Some([pt[0], pt[1]]);
                CommandStep::Dispatch("RF_ORIGIN_APPLY".to_string())
            }
            Err(_) => CommandStep::Cancel,
        }
    }

    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        vec![
            PreviewWire::line(
                [pt[0] - 10.0, pt[1], 0.0],
                [pt[0] + 10.0, pt[1], 0.0],
                Some([1.0, 0.2, 0.2, 1.0]),
            ),
            PreviewWire::line(
                [pt[0], pt[1] - 10.0, 0.0],
                [pt[0], pt[1] + 10.0, 0.0],
                Some([0.2, 1.0, 0.4, 1.0]),
            ),
        ]
    }
}

struct CalibrationLengthCommand {
    state: SharedState,
}

impl InteractiveCommand for CalibrationLengthCommand {
    fn prompt(&self) -> String {
        "RasterFit: enter calibration segment length".to_string()
    }

    fn on_point(&mut self, _pt: [f64; 3]) -> CommandStep {
        CommandStep::Cancel
    }

    fn modal_dialog(&self) -> Option<DialogDef> {
        Some(calibration_dialog())
    }

    fn on_dialog_response(&mut self, response: DialogResponse) -> CommandStep {
        let DialogResponse::Submitted(values) = response else {
            if let Ok(mut state) = lock_state(&self.state) {
                state.pending_segment = None;
                state.pending_calibration = None;
            }
            return CommandStep::Done;
        };
        let prepared = calibration_values(&values).and_then(|(length, orientation)| {
            prepare_pending_calibration(&self.state, length, orientation)
        });
        match prepared {
            Ok(()) => CommandStep::Dispatch("RF_CALIBRATE_APPLY".to_string()),
            Err(_) => CommandStep::Cancel,
        }
    }
}

struct CalibrationCommand {
    state: SharedState,
    width: usize,
    height: usize,
    raster: RasterData,
    transform: RasterTransform,
    known_length: Option<f64>,
    level: bool,
    first_world: Option<[f64; 3]>,
    first_raster: Option<Point>,
}

impl CalibrationCommand {
    fn snap_raster(&self, pt: [f64; 3]) -> Option<Point> {
        let point = world_to_raster(self.transform, self.height, pt)?;
        rf_core::ridge::snap_to_ridge(
            &self.raster.gray,
            self.width,
            self.height,
            point.x,
            point.y,
            8.0,
        )
        .or(Some(point))
    }
}

impl InteractiveCommand for CalibrationCommand {
    fn prompt(&self) -> String {
        match (self.level, self.first_world.is_some()) {
            (true, false) => "RasterFit: pick first leveling point".to_string(),
            (true, true) => "RasterFit: pick second leveling point".to_string(),
            (false, false) => "RasterFit: pick first calibration point".to_string(),
            (false, true) => "RasterFit: pick second calibration point".to_string(),
        }
    }

    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let Some(raster) = self.snap_raster(pt) else {
            return CommandStep::Cancel;
        };
        if raster.x < 0.0
            || raster.y < 0.0
            || raster.x > self.width as f64
            || raster.y > self.height as f64
        {
            return CommandStep::NeedPoint;
        }
        let Some(first) = self.first_raster else {
            self.first_world = Some(raster_to_world(self.transform, self.height, raster));
            self.first_raster = Some(raster);
            return CommandStep::NeedPoint;
        };
        if first.distance(raster) <= if self.level { 1.0 } else { f64::EPSILON } {
            return CommandStep::NeedPoint;
        }
        if self.level {
            return match level_transform(self.transform, first, raster) {
                Ok(transform) => match lock_state(&self.state) {
                    Ok(mut state) => {
                        state.pending_level = Some(transform);
                        CommandStep::Dispatch("RF_LEVEL_APPLY".to_string())
                    }
                    Err(_) => CommandStep::Cancel,
                },
                Err(_) => CommandStep::NeedPoint,
            };
        }
        match lock_state(&self.state) {
            Ok(mut state) => {
                state.pending_segment = Some([first, raster]);
                drop(state);
                if let Some(length) = self.known_length {
                    match prepare_pending_calibration(&self.state, length, SegmentOrientation::Free)
                    {
                        Ok(()) => CommandStep::Dispatch("RF_CALIBRATE_APPLY".to_string()),
                        Err(_) => CommandStep::Cancel,
                    }
                } else {
                    CommandStep::Dispatch("RF_CALIBRATE_LENGTH".to_string())
                }
            }
            Err(_) => CommandStep::Cancel,
        }
    }

    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        let point = self
            .snap_raster(pt)
            .map(|point| raster_to_world(self.transform, self.height, point))
            .unwrap_or(pt);
        self.first_world
            .map(|first| vec![PreviewWire::line(first, point, None)])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_paths_preserve_spaces_and_case() {
        assert_eq!(command_path("RF_OPEN", "RF_OPEN"), Some(None));
        assert_eq!(
            command_path("RF_OPEN /tmp/My Drawing.rasterfit.zip", "RF_OPEN"),
            Some(Some(Path::new("/tmp/My Drawing.rasterfit.zip")))
        );
        assert!(command_path("RF_OPEN_OTHER", "RF_OPEN").is_none());
        assert!(command_path("RF_OPEN   ", "RF_OPEN").is_none());
    }

    #[test]
    fn origin_picker_prepares_native_translation() {
        let state = SharedState::default();
        let mut command = OriginCommand {
            state: state.clone(),
        };
        assert_eq!(command.on_cursor_move([12.0, -8.0, 0.0]).len(), 2);
        assert!(matches!(
            command.on_point([12.0, -8.0, 0.0]),
            CommandStep::Dispatch(command) if command == "RF_ORIGIN_APPLY"
        ));
        assert_eq!(
            lock_state(&state).unwrap().pending_origin,
            Some([12.0, -8.0])
        );
    }

    #[test]
    fn interactive_calibration_collects_two_points() {
        let state = SharedState::default();
        let mut command = CalibrationCommand {
            state: state.clone(),
            width: 100,
            height: 100,
            transform: RasterTransform::default(),
            level: false,
            raster: RasterData {
                gray: vec![255; 100 * 100],
                width: 100,
                height: 100,
            },
            known_length: None,
            first_world: None,
            first_raster: None,
        };
        assert!(matches!(
            command.on_point([10.0, 90.0, 0.0]),
            CommandStep::NeedPoint
        ));
        assert!(matches!(
            command.on_point([60.0, 90.0, 0.0]),
            CommandStep::Dispatch(_)
        ));
        let pending = lock_state(&state).unwrap().pending_segment.unwrap();
        assert_eq!(pending[0], Point::new(10.0, 10.0));
        assert_eq!(pending[1], Point::new(60.0, 10.0));
    }

    #[test]
    fn command_line_length_skips_the_modal() {
        assert_eq!(
            command_length("RF_CALIBRATE 100", "RF_CALIBRATE"),
            Some(Some(100.0))
        );
        assert_eq!(
            command_length("RF_CALIBRATE 100 0,0 50,0", "RF_CALIBRATE"),
            Some(Some(100.0))
        );
        let state = SharedState::default();
        let mut command = CalibrationCommand {
            state: state.clone(),
            width: 100,
            height: 100,
            transform: RasterTransform::default(),
            level: false,
            raster: RasterData {
                gray: vec![255; 100 * 100],
                width: 100,
                height: 100,
            },
            known_length: Some(100.0),
            first_world: None,
            first_raster: None,
        };
        command.on_point([0.0, 100.0, 0.0]);
        assert!(matches!(
            command.on_point([50.0, 100.0, 0.0]),
            CommandStep::Dispatch(command) if command == "RF_CALIBRATE_APPLY"
        ));
        let state = lock_state(&state).unwrap();
        let (transform, _) = state.pending_calibration.as_ref().unwrap();
        assert_eq!(transform.sx, 2.0);
    }

    #[test]
    fn interactive_level_prepares_only_the_raster_rotation() {
        let state = SharedState::default();
        let mut command = CalibrationCommand {
            state: state.clone(),
            width: 100,
            height: 100,
            transform: RasterTransform::default(),
            level: true,
            raster: RasterData {
                gray: vec![255; 100 * 100],
                width: 100,
                height: 100,
            },
            known_length: None,
            first_world: None,
            first_raster: None,
        };
        assert!(command.prompt().contains("leveling"));
        assert!(matches!(
            command.on_point([10.0, 90.0, 0.0]),
            CommandStep::NeedPoint
        ));
        assert!(matches!(
            command.on_point([60.0, 80.0, 0.0]),
            CommandStep::Dispatch(command) if command == "RF_LEVEL_APPLY"
        ));
        let state = lock_state(&state).unwrap();
        let transform = state.pending_level.unwrap();
        assert!((transform.theta - 0.2f64.atan()).abs() < 1e-12);
        assert!(state.pending_segment.is_none());
        assert!(state.pending_calibration.is_none());
    }

    #[test]
    fn calibration_picker_snaps_to_a_real_ridge() {
        let mut gray = vec![255; 100 * 100];
        for x in 20..80 {
            gray[50 * 100 + x] = 0;
        }
        let command = CalibrationCommand {
            state: SharedState::default(),
            width: 100,
            height: 100,
            raster: RasterData {
                gray,
                width: 100,
                height: 100,
            },
            transform: RasterTransform::default(),
            known_length: None,
            level: false,
            first_world: None,
            first_raster: None,
        };
        let snapped = command.snap_raster([40.0, 51.0, 0.0]).unwrap();
        assert!((snapped.x - 40.0).abs() < 1.0);
        assert!((snapped.y - 50.0).abs() < 1.0);
    }

    #[test]
    fn modal_calibration_prepares_the_host_apply_step() {
        let state = SharedState::default();
        lock_state(&state).unwrap().pending_segment =
            Some([Point::new(0.0, 0.0), Point::new(100.0, 0.0)]);
        let mut command = CalibrationLengthCommand {
            state: state.clone(),
        };
        assert!(command.modal_dialog().is_some());
        let values = HashMap::from([
            ("length".to_string(), DialogValue::Number(50.0)),
            (
                "orientation".to_string(),
                DialogValue::Choice {
                    index: 1,
                    text: "Horizontal".to_string(),
                },
            ),
            ("angle".to_string(), DialogValue::Number(0.0)),
        ]);
        assert!(matches!(
            command.on_dialog_response(DialogResponse::Submitted(values)),
            CommandStep::Dispatch(command) if command == "RF_CALIBRATE_APPLY"
        ));
        let state = lock_state(&state).unwrap();
        let (transform, segments) = state.pending_calibration.as_ref().unwrap();
        assert_eq!(transform.sx, 0.5);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].orientation, SegmentOrientation::Horizontal);
    }
}
