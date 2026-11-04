use std::collections::HashMap;

use ocs_plugin_api::dialog::{DialogControl, DialogValue, PanelDef};
use ocs_plugin_api::host::{HostApi, HostNotification};

use crate::state::{lock_state, SharedState};

pub(crate) const PANEL_ID: &str = "rasterfit";

pub(crate) fn register(host: &mut dyn HostApi, state: &SharedState) {
    let definition = PanelDef::new(PANEL_ID, "RasterFit")
        .with_control(table(
            "summary",
            "Workspace",
            &["Property", "Value"],
            &[],
            false,
        ))
        .with_control(table(
            "segments",
            "Calibration segments",
            &["#", "Length", "Pixels", "Orientation"],
            &[],
            false,
        ))
        .with_control(DialogControl::Checkbox {
            id: "mask_enabled".to_string(),
            label: "Mask enabled".to_string(),
            default: false,
        })
        .with_control(DialogControl::Row {
            controls: vec![
                button("fit", "Fit", "RF_FIT"),
                button("preview", "Preview", "RF_FIT_PREVIEW"),
                button("options", "Options", "RF_FIT_OPTIONS"),
                button("image", "Image", "RF_IMAGE_ADJUST"),
            ],
            align_h: None,
            align_v: None,
        })
        .with_control(DialogControl::Row {
            controls: vec![
                button("report", "Export report", "RF_EXPORT_REPORT"),
                button("brush_size", "Brush size", "RF_MASK_BRUSH"),
                button("clear_mask", "Clear mask", "RF_MASK_CLEAR"),
            ],
            align_h: None,
            align_v: None,
        })
        .with_width(360.0)
        .with_command("RF_STATUS");
    host.register_panel(definition);
    host.toggle_panel(PANEL_ID, Some(false));
    sync(host, state);
}

pub(crate) fn toggle(host: &mut dyn HostApi, state: &SharedState) {
    host.toggle_panel(PANEL_ID, Some(true));
    sync(host, state);
}

pub(crate) fn sync(host: &mut dyn HostApi, state: &SharedState) {
    let Ok(state) = lock_state(state) else {
        return;
    };
    let mut values = HashMap::new();
    values.insert(
        "summary".to_string(),
        DialogValue::Table(summary_rows(&state)),
    );
    values.insert(
        "segments".to_string(),
        DialogValue::Table(segment_rows(&state)),
    );
    let mask_enabled = state.mask.lock().map(|mask| mask.enabled).unwrap_or(false);
    values.insert("mask_enabled".to_string(), DialogValue::Bool(mask_enabled));
    host.update_panel(PANEL_ID, values);
}

pub(crate) fn on_notification(state: &SharedState, notification: &HostNotification) {
    match notification {
        HostNotification::DocumentOpened { .. } => {
            if let Ok(mut state) = lock_state(state) {
                state.restore_pending = true;
                state.pending_fit_selection.clear();
            }
        }
        HostNotification::Cancel => {
            if let Ok(mut state) = lock_state(state) {
                state.pending_fit_selection.clear();
            }
        }
        HostNotification::PanelAction {
            panel_id,
            control_id,
            action,
            value,
        } if panel_id == PANEL_ID
            && control_id == "mask_enabled"
            && action == "change"
            && matches!(value, Some(DialogValue::Bool(_))) =>
        {
            let Some(DialogValue::Bool(enabled)) = value else {
                return;
            };
            if let Ok(state) = lock_state(state) {
                if let Ok(mut mask) = state.mask.lock() {
                    mask.enabled = *enabled;
                }
            }
        }
        _ => {}
    }
}

fn table(
    id: &str,
    label: &str,
    headers: &[&str],
    rows: &[Vec<String>],
    allow_edit: bool,
) -> DialogControl {
    DialogControl::Table {
        id: id.to_string(),
        label: Some(label.to_string()),
        headers: headers.iter().map(|header| header.to_string()).collect(),
        rows: rows.to_vec(),
        allow_add: allow_edit,
        allow_delete: allow_edit,
    }
}

fn button(id: &str, label: &str, command: &str) -> DialogControl {
    DialogControl::Button {
        id: id.to_string(),
        label: label.to_string(),
        action: Some(command.to_string()),
        align_h: None,
        width: None,
    }
}

fn summary_rows(state: &crate::state::PluginState) -> Vec<Vec<String>> {
    let fit = state.last_fit.as_ref();
    let page = state
        .pdf
        .as_ref()
        .map(|pdf| format!("{}/{}", state.page_index + 1, pdf.page_count()))
        .unwrap_or_else(|| "n/a".to_string());
    vec![
        row(
            "Raster",
            state
                .raster_path
                .as_deref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("none"),
        ),
        row("PDF page", &page),
        row("Calibrated", if state.calibrated { "yes" } else { "no" }),
        row("Scale (mm/px)", &format!("{:.6}", state.transform.sx)),
        row(
            "Rotation (deg)",
            &format!("{:.3}", state.transform.theta.to_degrees()),
        ),
        row(
            "Origin (mm)",
            &format!(
                "{:.3}, {:.3}",
                state.transform.origin[0], state.transform.origin[1]
            ),
        ),
        row(
            "RMS (px)",
            &fit.map(|fit| format!("{:.6}", fit.rms_px))
                .unwrap_or_else(|| "n/a".to_string()),
        ),
        row(
            "Coverage (%)",
            &fit.map(|fit| format!("{:.1}", 100.0 * (1.0 - fit.uncovered_ratio)))
                .unwrap_or_else(|| "n/a".to_string()),
        ),
        row(
            "Converged",
            &fit.map(|fit| fit.converged.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
        ),
        row(
            "Mask strokes",
            &state
                .mask
                .lock()
                .map(|mask| mask.strokes.len())
                .unwrap_or(0)
                .to_string(),
        ),
    ]
}

fn segment_rows(state: &crate::state::PluginState) -> Vec<Vec<String>> {
    state
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            vec![
                (index + 1).to_string(),
                format!("{:.3} mm", segment.length),
                format!("{:.2} px", segment.pixel_length()),
                format!("{:?}", segment.orientation),
            ]
        })
        .collect()
}

fn row(name: &str, value: &str) -> Vec<String> {
    vec![name.to_string(), value.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_and_segment_tables_reflect_workspace() {
        let mut state = crate::state::PluginState {
            calibrated: true,
            transform: crate::raster::RasterTransform {
                sx: 2.0,
                sy: 2.0,
                theta: 0.0,
                origin: [0.0, 0.0],
            },
            ..Default::default()
        };
        state
            .segments
            .push(rf_core::calibration::CalibrationSegment {
                a: rf_core::geometry::Point::new(0.0, 0.0),
                b: rf_core::geometry::Point::new(50.0, 0.0),
                length: 100.0,
                orientation: rf_core::calibration::SegmentOrientation::Horizontal,
            });
        assert!(summary_rows(&state)
            .iter()
            .any(|row| row[0] == "Scale (mm/px)" && row[1] == "2.000000"));
        let segments = segment_rows(&state);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0][2], "50.00 px");
    }

    #[test]
    fn panel_mask_checkbox_changes_shared_mask() {
        let state = SharedState::default();
        on_notification(
            &state,
            &HostNotification::PanelAction {
                panel_id: PANEL_ID.to_string(),
                control_id: "mask_enabled".to_string(),
                action: "change".to_string(),
                value: Some(DialogValue::Bool(true)),
            },
        );
        assert!(state.lock().unwrap().mask.lock().unwrap().enabled);
    }
}
