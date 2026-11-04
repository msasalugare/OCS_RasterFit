#![allow(clippy::not_unsafe_ptr_arg_deref)]

mod analysis;
mod cad;
mod calibration;
mod commands;
mod constraints;
mod dialogs;
mod drawing;
mod fit;
mod image_tools;
mod mask;
mod native;
mod overlay;
mod panel;
mod pdf;
mod preprocess;
mod preview;
mod raster;
mod ribbon;
mod state;
mod workspace;

use ocs_plugin_api::export_plugin;
use ocs_plugin_api::host::{BuiltinPlugin, HostApi, HostNotification};
use ocs_plugin_api::manifest::{ApiVersion, PluginManifest};
use ocs_plugin_api::ribbon::CadModule;

use ribbon::RasterFitModule;
use state::SharedState;

static MANIFEST: PluginManifest = PluginManifest {
    id: "com.rasterfit",
    name: "RasterFit",
    version: env!("CARGO_PKG_VERSION"),
    description: "Fit parametric sketches to calibrated raster drawings",
    api_version: ApiVersion::CURRENT,
    ribbon_order: 60,
    xdata_apps: &["RASTERFIT"],
    command_prefixes: &[
        "RF_OPEN",
        "RF_OPEN_PDF",
        "RF_PAGE_PREV",
        "RF_PAGE_NEXT",
        "RF_IMAGE_ADJUST",
        "RF_IMAGE_APPLY",
        "RF_LINE",
        "RF_POLYLINE",
        "RF_CALIBRATE",
        "RF_CALIBRATE_LENGTH",
        "RF_CALIBRATE_APPLY",
        "RF_LEVEL",
        "RF_LEVEL_APPLY",
        "RF_ORIGIN",
        "RF_ORIGIN_APPLY",
        "RF_ORIGIN_RESET",
        "RF_GUIDES",
        "RF_MASK_SYNC",
        "RF_MASK_BRUSH",
        "RF_MASK_BRUSH_APPLY",
        "RF_MASK_INCLUDE",
        "RF_MASK_EXCLUDE",
        "RF_MASK_RECT",
        "RF_MASK_POLYGON",
        "RF_MASK_ERASE",
        "RF_MASK_FROM_SELECTION",
        "RF_MASK_CLEAR",
        "RF_CONSTRAINT_COINCIDENT",
        "RF_CONSTRAINT_TANGENT",
        "RF_MEASURE_DISTANCE",
        "RF_MEASURE_ANGLE",
        "RF_HATCH",
        "RF_FIT_PREVIEW",
        "RF_FIT",
        "RF_FIT_OPTIONS",
        "RF_HEATMAP",
        "RF_EXPORT_REPORT",
        "RF_STATUS",
    ],
};

#[derive(Default)]
struct RasterFitPlugin {
    state: SharedState,
}

impl BuiltinPlugin for RasterFitPlugin {
    fn manifest(&self) -> &'static PluginManifest {
        &MANIFEST
    }

    fn ribbon(&self) -> Box<dyn CadModule> {
        Box::new(RasterFitModule)
    }

    fn dispatch(&self, host: &mut dyn HostApi, cmd: &str) -> bool {
        commands::dispatch(host, &self.state, cmd)
    }

    fn on_load(&mut self, host: &mut dyn HostApi) {
        if let Err(error) = commands::restore_workspace(host, &self.state) {
            host.push_error(&format!("RasterFit workspace restore failed: {error}"));
        }
        panel::register(host, &self.state);
    }

    fn on_notification(&mut self, _command_id: Option<u64>, notification: HostNotification) {
        panel::on_notification(&self.state, &notification);
    }
}

export_plugin!(RasterFitPlugin::default());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_toml_exposes_every_command_to_the_host_parser() {
        let manifest = include_str!("../plugin.toml");
        let command_line = manifest
            .lines()
            .find(|line| line.trim_start().starts_with("command_prefixes"))
            .expect("command_prefixes must be declared on one line for the host parser");
        for command in MANIFEST.command_prefixes {
            assert!(
                command_line.contains(&format!("\"{command}\"")),
                "missing {command} from plugin.toml command_prefixes"
            );
        }
        assert!(manifest.contains("api_version = 14"));
        assert!(manifest.contains("xdata_apps = [\"RASTERFIT\"]"));
    }
}
