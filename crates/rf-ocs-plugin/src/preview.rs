use ocs_plugin_api::host::{CommandStep, EntityType, HostApi, InteractiveCommand, PreviewWire};

use crate::cad::sketch_to_entities;
use crate::fit::fit_native_sketch;
use crate::mask::mask_params;
use crate::native::selected_sketch;
use crate::state::{lock_state, SharedState};

const PREVIEW_COLOR: [f32; 4] = [0.3, 0.8, 1.0, 1.0];

pub(crate) fn start_fit_preview(
    host: &mut dyn HostApi,
    state: &SharedState,
) -> Result<String, String> {
    let (raster, transform, options) = {
        let state = lock_state(state)?;
        if !state.calibrated {
            return Err("calibrate the raster before previewing fit".to_string());
        }
        (
            state
                .raster
                .clone()
                .ok_or_else(|| "open a raster before previewing fit".to_string())?,
            state.transform,
            state.fit_options,
        )
    };
    let mut native = selected_sketch(host, transform, raster.height)?
        .ok_or_else(|| "select native geometry before previewing fit".to_string())?;
    let native_constraints = crate::constraints::read_native_constraints(host.document(), &native)?;
    native_constraints.apply_to_sketch(&mut native.sketch);
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
    let wires = preview_wires(sketch_to_entities(
        &output.sketch,
        transform,
        raster.height,
    )?);
    if wires.is_empty() {
        return Err("fit preview has no visible geometry".to_string());
    }
    lock_state(state)?.pending_fit_selection = native
        .originals
        .iter()
        .map(|entity| entity.common().handle)
        .collect();
    host.start_interactive(Box::new(FitPreviewCommand { wires }));
    Ok("RasterFit preview: Enter closes preview; run Fit to apply".to_string())
}

struct FitPreviewCommand {
    wires: Vec<PreviewWire>,
}

impl InteractiveCommand for FitPreviewCommand {
    fn prompt(&self) -> String {
        "RasterFit fit preview: Enter closes; run Fit to apply, Esc cancels".to_string()
    }

    fn on_point(&mut self, _pt: [f64; 3]) -> CommandStep {
        CommandStep::Done
    }

    fn on_enter(&mut self) -> CommandStep {
        CommandStep::Done
    }

    fn on_cursor_move(&mut self, _pt: [f64; 3]) -> Vec<PreviewWire> {
        self.wires.clone()
    }
}

fn preview_wires(entities: Vec<crate::cad::CadEntity>) -> Vec<PreviewWire> {
    entities
        .into_iter()
        .map(|output| match output.entity {
            EntityType::Line(line) => {
                PreviewWire::with_color(vec![point(line.start), point(line.end)], PREVIEW_COLOR)
            }
            EntityType::Circle(circle) => PreviewWire::Circle {
                center: point(circle.center),
                radius: circle.radius,
                color: Some(PREVIEW_COLOR),
            },
            EntityType::Arc(arc) => PreviewWire::Arc {
                center: point(arc.center),
                radius: arc.radius,
                start_angle_rad: arc.start_angle,
                end_angle_rad: arc.end_angle,
                color: Some(PREVIEW_COLOR),
            },
            _ => PreviewWire::new(Vec::new()),
        })
        .filter(|wire| match wire {
            PreviewWire::Polyline { points, .. } => points.len() >= 2,
            PreviewWire::Circle { radius, .. } | PreviewWire::Arc { radius, .. } => {
                radius.is_finite() && *radius > 0.0
            }
        })
        .collect()
}

fn point(point: ocs_plugin_api::host::acadrust::types::Vector3) -> [f64; 3] {
    [point.x, point.y, point.z]
}

#[cfg(test)]
mod tests {
    use ocs_plugin_api::host::acadrust::entities::{Arc, Circle, Line};

    use super::*;

    #[test]
    fn native_fit_output_becomes_nonwriting_preview_wires() {
        let wires = preview_wires(vec![
            crate::cad::CadEntity {
                entity: EntityType::Line(Line::from_coords(0.0, 0.0, 0.0, 10.0, 0.0, 0.0)),
                kind: "LINE",
                id: 1,
            },
            crate::cad::CadEntity {
                entity: EntityType::Circle(Circle::from_coords(0.0, 0.0, 0.0, 5.0)),
                kind: "CIRCLE",
                id: 2,
            },
            crate::cad::CadEntity {
                entity: EntityType::Arc(Arc::from_coords(0.0, 0.0, 0.0, 5.0, 0.0, 1.0)),
                kind: "ARC",
                id: 3,
            },
        ]);
        assert_eq!(wires.len(), 3);
        assert!(matches!(wires[0], PreviewWire::Polyline { .. }));
        assert!(matches!(wires[1], PreviewWire::Circle { .. }));
        assert!(matches!(wires[2], PreviewWire::Arc { .. }));
    }
}
