use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ocs_plugin_api::dialog::{DialogControl, DialogDef, DialogResponse, DialogValue};
use ocs_plugin_api::host::{CommandStep, HostApi, InteractiveCommand};

use crate::cad::add_raster;
use crate::preprocess::{apply as apply_preprocess, PreprocessSettings};
use crate::raster::RasterData;
use crate::state::{lock_state, SharedState};

pub(crate) fn start_image_adjustments(
    host: &mut dyn HostApi,
    state: &SharedState,
) -> Result<String, String> {
    let settings = lock_state(state)?.preprocess;
    host.start_interactive(Box::new(ImageAdjustCommand {
        state: state.clone(),
        settings,
    }));
    Ok("RasterFit image settings opened".to_string())
}

pub(crate) fn apply_image_adjustments(
    host: &mut dyn HostApi,
    state: &SharedState,
) -> Result<String, String> {
    let (original, settings, old_handle) = {
        let state = lock_state(state)?;
        let original = state
            .original_raster
            .clone()
            .ok_or_else(|| "open a raster before image adjustments".to_string())?;
        (original, state.preprocess, state.raster_handle)
    };
    let raster = apply_preprocess(&original, settings)?;
    let source = lock_state(state)?.source_path.clone();
    let path = write_processed_png(&raster, source.as_deref())?;
    let transform = lock_state(state)?.transform;

    host.begin_undo_group("RasterFit image adjustments");
    let result = (|| {
        if let Some(handle) = old_handle {
            if !host.remove_entity(handle) {
                return Err(format!("could not replace raster entity {handle}"));
            }
        }
        let handle = add_raster(host, &path, &raster, transform)?;
        if settings.opacity < 100.0 {
            let mut entities = host.entities_batch(&[handle])?;
            let Some(entity) = entities.pop().flatten() else {
                return Err("processed raster entity is unavailable".to_string());
            };
            let mut entity = entity;
            let ocs_plugin_api::host::EntityType::RasterImage(image) = &mut entity else {
                return Err("processed entity is not a raster image".to_string());
            };
            image.fade = (100.0 - settings.opacity).round().clamp(0.0, 100.0) as u8;
            if !host.update_entity(entity) {
                return Err("could not apply raster opacity".to_string());
            }
        }
        Ok(handle)
    })();
    host.end_undo_group();
    let handle = result?;

    let mut state = lock_state(state)?;
    state.raster = Some(raster);
    state.raster_handle = Some(handle);
    state.raster_path = Some(path);
    state.last_fit = None;
    state.last_origins.clear();
    Ok("RasterFit image adjustments applied".to_string())
}

struct ImageAdjustCommand {
    state: SharedState,
    settings: PreprocessSettings,
}

impl InteractiveCommand for ImageAdjustCommand {
    fn prompt(&self) -> String {
        "RasterFit image settings".to_string()
    }

    fn on_point(&mut self, _pt: [f64; 3]) -> CommandStep {
        CommandStep::Cancel
    }

    fn modal_dialog(&self) -> Option<DialogDef> {
        Some(
            DialogDef::new("RasterFit image settings")
                .submit_label("Apply")
                .with_control(number_control(
                    "brightness",
                    "Brightness",
                    self.settings.brightness,
                ))
                .with_control(number_control(
                    "contrast",
                    "Contrast (%)",
                    self.settings.contrast,
                ))
                .with_control(number_control(
                    "sharpness",
                    "Sharpness",
                    self.settings.sharpness,
                ))
                .with_control(number_control(
                    "opacity",
                    "Opacity (%)",
                    self.settings.opacity,
                ))
                .with_control(DialogControl::Checkbox {
                    id: "invert".to_string(),
                    label: "Invert".to_string(),
                    default: self.settings.invert,
                })
                .width(420.0),
        )
    }

    fn on_dialog_response(&mut self, response: DialogResponse) -> CommandStep {
        let DialogResponse::Submitted(values) = response else {
            return CommandStep::Done;
        };
        let Ok(settings) = settings_from_values(values) else {
            return CommandStep::Cancel;
        };
        if let Ok(mut state) = lock_state(&self.state) {
            state.preprocess = settings;
        }
        CommandStep::Dispatch("RF_IMAGE_APPLY".to_string())
    }
}

fn settings_from_values(
    values: HashMap<String, DialogValue>,
) -> Result<PreprocessSettings, String> {
    let settings = PreprocessSettings {
        brightness: number(&values, "brightness")?,
        contrast: number(&values, "contrast")?,
        sharpness: number(&values, "sharpness")?,
        opacity: number(&values, "opacity")?,
        invert: matches!(values.get("invert"), Some(DialogValue::Bool(true))),
    };
    if [
        settings.brightness,
        settings.contrast,
        settings.sharpness,
        settings.opacity,
    ]
    .iter()
    .all(|value| value.is_finite())
    {
        Ok(settings)
    } else {
        Err("image settings must be finite".to_string())
    }
}

fn number_control(id: &str, label: &str, default: f64) -> DialogControl {
    DialogControl::Number {
        id: id.to_string(),
        label: label.to_string(),
        default,
        min: None,
        max: None,
        step: None,
    }
}

fn number(values: &HashMap<String, DialogValue>, id: &str) -> Result<f64, String> {
    match values.get(id) {
        Some(DialogValue::Number(value)) => Ok(*value),
        _ => Err(format!("missing image setting {id}")),
    }
}

pub(crate) fn write_processed_png(
    raster: &RasterData,
    source: Option<&std::path::Path>,
) -> Result<PathBuf, String> {
    let image = image::GrayImage::from_raw(
        raster.width as u32,
        raster.height as u32,
        raster.gray.clone(),
    )
    .ok_or_else(|| "could not construct processed raster".to_string())?;
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|error| format!("could not encode processed raster: {error}"))?;
    let directory = std::env::temp_dir().join("rasterfit-ocs");
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "could not create raster cache '{}': {error}",
            directory.display()
        )
    })?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_nanos();
    let stem = source
        .and_then(|path| path.file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("raster");
    let path = directory.join(format!("{stem}-processed-{unique}.png"));
    std::fs::write(&path, bytes.into_inner()).map_err(|error| {
        format!(
            "could not write processed raster '{}': {error}",
            path.display()
        )
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submitted_settings_dispatch_native_apply() {
        let state = SharedState::default();
        let values = HashMap::from([
            ("brightness".to_string(), DialogValue::Number(10.0)),
            ("contrast".to_string(), DialogValue::Number(20.0)),
            ("sharpness".to_string(), DialogValue::Number(1.0)),
            ("opacity".to_string(), DialogValue::Number(80.0)),
            ("invert".to_string(), DialogValue::Bool(true)),
        ]);
        let mut command = ImageAdjustCommand {
            state: state.clone(),
            settings: PreprocessSettings::default(),
        };
        assert!(matches!(
            command.on_dialog_response(DialogResponse::Submitted(values)),
            CommandStep::Dispatch(command) if command == "RF_IMAGE_APPLY"
        ));
        let settings = lock_state(&state).unwrap().preprocess;
        assert_eq!(settings.brightness, 10.0);
        assert_eq!(settings.opacity, 80.0);
        assert!(settings.invert);
    }
}
