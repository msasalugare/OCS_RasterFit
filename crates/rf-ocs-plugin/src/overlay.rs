use ocs_plugin_api::host::acadrust::entities::{
    BoundaryEdge, BoundaryPath, Hatch, PolylineEdge, XLine,
};
use ocs_plugin_api::host::acadrust::types::{Color, Transparency, Vector2, Vector3};
use ocs_plugin_api::host::{
    CommandRequest, EntityType, ExtendedDataRecord, Handle, HostApi, XDataValue,
};
use rf_core::mask::StrokeMode;

use crate::mask::stroke_fill_polygons;
use crate::state::{lock_state, SharedState};

pub(crate) const MASK_LAYER: &str = "RASTERFIT-MASK";
pub(crate) const GUIDES_LAYER: &str = "RASTERFIT-GUIDES";

pub(crate) fn sync_all(host: &mut dyn HostApi, state: &SharedState) -> Result<(), String> {
    sync_mask(host, state)?;
    sync_guides(host, state)?;
    Ok(())
}

pub(crate) fn sync_mask(host: &mut dyn HostApi, state: &SharedState) -> Result<usize, String> {
    let (old_handles, polygons) = {
        let state_guard = lock_state(state)?;
        let raster = state_guard.raster.clone();
        let mask = state_guard
            .mask
            .lock()
            .map_err(|_| "mask state is unavailable".to_string())?;
        let polygons = raster
            .as_ref()
            .map(|raster| {
                mask.strokes
                    .iter()
                    .flat_map(|stroke| {
                        stroke_fill_polygons(stroke, state_guard.transform, raster.height)
                            .into_iter()
                            .map(move |polygon| (stroke.mode, polygon))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        (state_guard.mask_overlay_handles.clone(), polygons)
    };

    host.begin_undo_group("RasterFit mask overlay");
    let result = (|| {
        remove_handles(host, &old_handles);
        let mut handles = Vec::with_capacity(polygons.len());
        for (mode, polygon) in polygons {
            let vertices = polygon
                .into_iter()
                .map(|point| Vector2::new(point[0], point[1]))
                .collect();
            let mut path = BoundaryPath::external();
            path.add_edge(BoundaryEdge::Polyline(PolylineEdge::new(vertices, true)));
            let mut hatch = Hatch::solid();
            hatch.paths.push(path);
            hatch.common.layer = MASK_LAYER.to_string();
            hatch.common.color = match mode {
                StrokeMode::Include => Color::from_rgb(76, 207, 111),
                StrokeMode::Exclude => Color::from_rgb(229, 57, 53),
            };
            hatch.common.transparency = Transparency::from_percent(0.65);
            let handle = host.add_entity(EntityType::Hatch(Box::new(hatch)));
            host.set_reference_only(handle, true)?;
            write_overlay_record(host, handle, "MASK")?;
            handles.push(handle);
        }
        draw_order(host, &handles, "FRONT")?;
        lock_state(state)?.mask_overlay_handles = handles.clone();
        host.bump_geometry();
        host.set_dirty();
        Ok(handles.len())
    })();
    host.end_undo_group();
    result
}

pub(crate) fn clear_mask(host: &mut dyn HostApi, state: &SharedState) -> Result<(), String> {
    let handles = lock_state(state)?.mask_overlay_handles.clone();
    remove_handles(host, &handles);
    lock_state(state)?.mask_overlay_handles.clear();
    host.bump_geometry();
    host.set_dirty();
    Ok(())
}

pub(crate) fn toggle_guides(host: &mut dyn HostApi, state: &SharedState) -> Result<String, String> {
    let enabled = {
        let mut state = lock_state(state)?;
        state.guides_enabled = !state.guides_enabled;
        state.guides_enabled
    };
    sync_guides(host, state)?;
    Ok(format!(
        "RasterFit guides {}",
        if enabled { "enabled" } else { "disabled" }
    ))
}

pub(crate) fn sync_guides(host: &mut dyn HostApi, state: &SharedState) -> Result<(), String> {
    let (enabled, origin, handles) = {
        let state = lock_state(state)?;
        (
            state.guides_enabled,
            state.transform.origin,
            state.guide_handles.clone(),
        )
    };
    if !enabled {
        remove_handles(host, &handles);
        lock_state(state)?.guide_handles.clear();
        host.bump_geometry();
        host.set_dirty();
        return Ok(());
    }

    let point = Vector3::new(origin[0], origin[1], 0.0);
    let existing = host.entities_batch(&handles)?;
    let can_update = handles.len() == 2
        && existing
            .iter()
            .all(|entity| matches!(entity, Some(EntityType::XLine(_))));
    let handles = if can_update {
        for ((handle, entity), color) in handles
            .iter()
            .copied()
            .zip(existing.into_iter().flatten())
            .zip([Color::from_rgb(229, 57, 53), Color::from_rgb(76, 207, 111)])
        {
            let EntityType::XLine(mut guide) = entity else {
                return Err("invalid RasterFit guide entity".to_string());
            };
            guide.base_point = point;
            guide.common.layer = GUIDES_LAYER.to_string();
            guide.common.color = color;
            if !host.update_entity(EntityType::XLine(guide)) {
                return Err("could not update RasterFit guides".to_string());
            }
            host.set_reference_only(handle, true)?;
        }
        handles
    } else {
        remove_handles(host, &handles);
        let mut created = Vec::new();
        for (mut guide, color) in [
            (XLine::horizontal(point), Color::from_rgb(229, 57, 53)),
            (XLine::vertical(point), Color::from_rgb(76, 207, 111)),
        ] {
            guide.common.layer = GUIDES_LAYER.to_string();
            guide.common.color = color;
            let handle = host.add_entity(EntityType::XLine(guide));
            host.set_reference_only(handle, true)?;
            write_overlay_record(host, handle, "GUIDE")?;
            created.push(handle);
        }
        created
    };
    draw_order(host, &handles, "FRONT")?;
    lock_state(state)?.guide_handles = handles;
    host.bump_geometry();
    host.set_dirty();
    Ok(())
}

pub(crate) fn clear_guides(host: &mut dyn HostApi, state: &SharedState) -> Result<(), String> {
    let handles = lock_state(state)?.guide_handles.clone();
    remove_handles(host, &handles);
    let mut state = lock_state(state)?;
    state.guide_handles.clear();
    state.guides_enabled = false;
    Ok(())
}

pub(crate) fn restore(host: &mut dyn HostApi, state: &SharedState) -> Result<(), String> {
    let (mask, guides, raster) = {
        let state = lock_state(state)?;
        (
            state.mask_overlay_handles.clone(),
            state.guide_handles.clone(),
            state.raster_handle,
        )
    };
    for handle in mask.iter().chain(&guides) {
        if host
            .entities_batch(&[*handle])?
            .into_iter()
            .next()
            .flatten()
            .is_some()
        {
            host.set_reference_only(*handle, true)?;
        }
    }
    draw_order(host, &mask, "FRONT")?;
    draw_order(host, &guides, "FRONT")?;
    if let Some(raster) = raster {
        raster_to_back(host, raster)?;
    }
    Ok(())
}

pub(crate) fn raster_to_back(host: &mut dyn HostApi, handle: Handle) -> Result<(), String> {
    draw_order(host, &[handle], "BACK")
}

fn remove_handles(host: &mut dyn HostApi, handles: &[Handle]) {
    for handle in handles {
        host.remove_entity(*handle);
    }
}

fn write_overlay_record(host: &mut dyn HostApi, handle: Handle, class: &str) -> Result<(), String> {
    let mut record = ExtendedDataRecord::new("RASTERFIT");
    record.add_value(XDataValue::String(format!("RASTERFIT-CLASS:{class}")));
    if host.write_record(handle, record) {
        Ok(())
    } else {
        Err(format!("could not mark RasterFit overlay entity {handle}"))
    }
}

fn draw_order(host: &mut dyn HostApi, handles: &[Handle], mode: &str) -> Result<(), String> {
    if handles.is_empty() {
        return Ok(());
    }
    let previous = host.selection();
    host.set_selection(handles)?;
    let outcome = host.run_command(CommandRequest::RunQuiet {
        line: format!("DRAWORDER {mode}"),
    });
    let restore = host.set_selection(&previous);
    let outcome = outcome?;
    if let Some(error) = outcome.error {
        return Err(format!("DRAWORDER {mode} failed: {error}"));
    }
    restore?;
    Ok(())
}
