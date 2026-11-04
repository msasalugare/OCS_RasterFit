use std::collections::HashMap;

use ocs_plugin_api::dialog::{DialogControl, DialogDef, DialogResponse, DialogValue};
use ocs_plugin_api::host::{CommandStep, HostApi, InteractiveCommand};

use crate::analysis::{export_report, ReportFormat};
use crate::fit::FitOptions;
use crate::state::{lock_state, SharedState};

pub(crate) fn start_fit_options(
    host: &mut dyn HostApi,
    state: &SharedState,
) -> Result<String, String> {
    let options = lock_state(state)?.fit_options;
    host.start_interactive(Box::new(FitOptionsCommand {
        state: state.clone(),
        options,
    }));
    Ok("RasterFit fit options opened".to_string())
}

pub(crate) fn start_brush_size(
    host: &mut dyn HostApi,
    state: &SharedState,
) -> Result<String, String> {
    let radius = lock_state(state)?
        .mask
        .lock()
        .map_err(|_| "mask state is unavailable".to_string())?
        .brush_radius;
    host.start_interactive(Box::new(MaskBrushCommand {
        state: state.clone(),
        radius,
    }));
    Ok("Choose RasterFit mask brush radius".to_string())
}

pub(crate) fn start_report_export(
    host: &mut dyn HostApi,
    state: &SharedState,
) -> Result<String, String> {
    if lock_state(state)?.last_fit.is_none() {
        return Err("run fit before exporting a report".to_string());
    }
    host.start_interactive(Box::new(ReportExportCommand {
        state: state.clone(),
    }));
    Ok("Choose RasterFit report output".to_string())
}

struct MaskBrushCommand {
    state: SharedState,
    radius: f64,
}

impl InteractiveCommand for MaskBrushCommand {
    fn prompt(&self) -> String {
        "RasterFit mask brush size".to_string()
    }

    fn on_point(&mut self, _pt: [f64; 3]) -> CommandStep {
        CommandStep::Cancel
    }

    fn modal_dialog(&self) -> Option<DialogDef> {
        Some(
            DialogDef::new("RasterFit mask brush size")
                .submit_label("Apply")
                .with_control(DialogControl::Number {
                    id: "radius".to_string(),
                    label: "Brush radius (raster px)".to_string(),
                    default: self.radius,
                    min: Some(1.0),
                    max: Some(80.0),
                    step: Some(1.0),
                })
                .width(380.0),
        )
    }

    fn on_dialog_response(&mut self, response: DialogResponse) -> CommandStep {
        let DialogResponse::Submitted(values) = response else {
            return CommandStep::Done;
        };
        let Some(DialogValue::Number(radius)) = values.get("radius") else {
            return CommandStep::Cancel;
        };
        if !radius.is_finite() || !(1.0..=80.0).contains(radius) {
            return CommandStep::Cancel;
        }
        let Ok(state) = lock_state(&self.state) else {
            return CommandStep::Cancel;
        };
        let Ok(mut mask) = state.mask.lock() else {
            return CommandStep::Cancel;
        };
        mask.brush_radius = *radius;
        CommandStep::Dispatch("RF_MASK_BRUSH_APPLY".to_string())
    }
}

struct FitOptionsCommand {
    state: SharedState,
    options: FitOptions,
}

impl InteractiveCommand for FitOptionsCommand {
    fn prompt(&self) -> String {
        "RasterFit fit options".to_string()
    }

    fn on_point(&mut self, _pt: [f64; 3]) -> CommandStep {
        CommandStep::Cancel
    }

    fn modal_dialog(&self) -> Option<DialogDef> {
        let o = self.options;
        Some(
            DialogDef::new("RasterFit fit options")
                .submit_label("Apply")
                .with_control(checkbox(
                    "fit_geometry",
                    "Fit geometry (constrained)",
                    o.fit_geometry,
                ))
                .with_control(checkbox("fit_rotation", "Fit rotation", o.fit_rotation))
                .with_control(checkbox(
                    "fit_translation",
                    "Fit translation",
                    o.fit_translation,
                ))
                .with_control(checkbox("fit_scale", "Fit background scale", o.fit_scale))
                .with_control(checkbox(
                    "include_construction",
                    "Include construction geometry",
                    o.include_construction,
                ))
                .with_control(number(
                    "capture_radius",
                    "Capture radius (px)",
                    o.capture_radius,
                    0.001,
                ))
                .with_control(number(
                    "robust_scale",
                    "Robust scale (px, 0 disables)",
                    o.robust_scale,
                    0.0,
                ))
                .with_control(number(
                    "tangent_weight",
                    "Tangent weight",
                    o.tangent_weight,
                    0.0,
                ))
                .width(460.0),
        )
    }

    fn on_dialog_response(&mut self, response: DialogResponse) -> CommandStep {
        let DialogResponse::Submitted(values) = response else {
            return CommandStep::Done;
        };
        let Ok(mut options) = options_from_values(values) else {
            return CommandStep::Cancel;
        };
        options.capture_radius = options.capture_radius.max(0.001);
        options.robust_scale = options.robust_scale.max(0.0);
        options.tangent_weight = options.tangent_weight.max(0.0);
        if let Ok(mut state) = lock_state(&self.state) {
            state.fit_options = options;
        }
        CommandStep::Done
    }
}

struct ReportExportCommand {
    state: SharedState,
}

impl InteractiveCommand for ReportExportCommand {
    fn prompt(&self) -> String {
        "RasterFit report export".to_string()
    }

    fn on_point(&mut self, _pt: [f64; 3]) -> CommandStep {
        CommandStep::Cancel
    }

    fn modal_dialog(&self) -> Option<DialogDef> {
        Some(
            DialogDef::new("Export RasterFit report")
                .submit_label("Export")
                .with_control(DialogControl::Choice {
                    id: "format".to_string(),
                    label: "Format".to_string(),
                    options: vec!["JSON".to_string(), "CSV".to_string()],
                    default_index: 0,
                })
                .with_control(DialogControl::FilePicker {
                    id: "path".to_string(),
                    label: "Output file".to_string(),
                    default_path: "rasterfit-report.json".to_string(),
                    filter_name: "RasterFit report".to_string(),
                    extensions: vec!["json".to_string(), "csv".to_string()],
                    save: true,
                })
                .width(460.0),
        )
    }

    fn on_dialog_response(&mut self, response: DialogResponse) -> CommandStep {
        let DialogResponse::Submitted(values) = response else {
            return CommandStep::Done;
        };
        let Some(DialogValue::FilePath(path)) = values.get("path") else {
            return CommandStep::Cancel;
        };
        let format = match values.get("format") {
            Some(DialogValue::Choice { index: 1, .. }) => ReportFormat::Csv,
            _ => ReportFormat::Json,
        };
        match export_report(&self.state, path, format) {
            Ok(_) => CommandStep::Done,
            Err(_) => CommandStep::Cancel,
        }
    }
}

fn checkbox(id: &str, label: &str, default: bool) -> DialogControl {
    DialogControl::Checkbox {
        id: id.to_string(),
        label: label.to_string(),
        default,
    }
}

fn number(id: &str, label: &str, default: f64, min: f64) -> DialogControl {
    DialogControl::Number {
        id: id.to_string(),
        label: label.to_string(),
        default,
        min: Some(min),
        max: None,
        step: None,
    }
}

fn options_from_values(values: HashMap<String, DialogValue>) -> Result<FitOptions, String> {
    let options = FitOptions {
        fit_geometry: bool_value(&values, "fit_geometry")?,
        fit_rotation: bool_value(&values, "fit_rotation")?,
        fit_translation: bool_value(&values, "fit_translation")?,
        fit_scale: bool_value(&values, "fit_scale")?,
        include_construction: bool_value(&values, "include_construction")?,
        capture_radius: number_value(&values, "capture_radius")?,
        robust_scale: number_value(&values, "robust_scale")?,
        tangent_weight: number_value(&values, "tangent_weight")?,
    };
    if [
        options.capture_radius,
        options.robust_scale,
        options.tangent_weight,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        Ok(options)
    } else {
        Err("fit options must be finite".to_string())
    }
}

fn bool_value(values: &HashMap<String, DialogValue>, id: &str) -> Result<bool, String> {
    match values.get(id) {
        Some(DialogValue::Bool(value)) => Ok(*value),
        _ => Err(format!("missing fit option {id}")),
    }
}

fn number_value(values: &HashMap<String, DialogValue>, id: &str) -> Result<f64, String> {
    match values.get(id) {
        Some(DialogValue::Number(value)) => Ok(*value),
        _ => Err(format!("missing fit option {id}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_size_dialog_validates_and_applies_shared_radius() {
        let state = SharedState::default();
        let mut command = MaskBrushCommand {
            state: state.clone(),
            radius: 6.0,
        };
        assert_eq!(command.modal_dialog().unwrap().rows.len(), 1);
        let values = HashMap::from([("radius".to_string(), DialogValue::Number(14.0))]);
        assert!(matches!(
            command.on_dialog_response(DialogResponse::Submitted(values)),
            CommandStep::Dispatch(command) if command == "RF_MASK_BRUSH_APPLY"
        ));
        assert_eq!(
            lock_state(&state)
                .unwrap()
                .mask
                .lock()
                .unwrap()
                .brush_radius,
            14.0
        );

        let values = HashMap::from([("radius".to_string(), DialogValue::Number(0.0))]);
        assert!(matches!(
            command.on_dialog_response(DialogResponse::Submitted(values)),
            CommandStep::Cancel
        ));
        assert_eq!(
            lock_state(&state)
                .unwrap()
                .mask
                .lock()
                .unwrap()
                .brush_radius,
            14.0
        );
    }

    #[test]
    fn options_dialog_exposes_all_fit_controls() {
        let state = SharedState::default();
        let command = FitOptionsCommand {
            state,
            options: FitOptions::default(),
        };
        let dialog = command.modal_dialog().unwrap();
        assert_eq!(dialog.rows.len(), 8);
    }

    #[test]
    fn submitted_options_update_native_state() {
        let state = SharedState::default();
        let values = HashMap::from([
            ("fit_geometry".to_string(), DialogValue::Bool(true)),
            ("fit_rotation".to_string(), DialogValue::Bool(false)),
            ("fit_translation".to_string(), DialogValue::Bool(true)),
            ("fit_scale".to_string(), DialogValue::Bool(true)),
            ("include_construction".to_string(), DialogValue::Bool(false)),
            ("capture_radius".to_string(), DialogValue::Number(20.0)),
            ("robust_scale".to_string(), DialogValue::Number(2.0)),
            ("tangent_weight".to_string(), DialogValue::Number(0.2)),
        ]);
        let mut command = FitOptionsCommand {
            state: state.clone(),
            options: FitOptions::default(),
        };
        assert!(matches!(
            command.on_dialog_response(DialogResponse::Submitted(values)),
            CommandStep::Done
        ));
        let options = lock_state(&state).unwrap().fit_options;
        assert!(options.fit_geometry);
        assert!(options.fit_scale);
        assert!(!options.fit_rotation);
        assert_eq!(options.capture_radius, 20.0);
        assert_eq!(options.tangent_weight, 0.2);
    }
}
