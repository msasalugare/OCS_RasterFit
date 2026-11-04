use ocs_plugin_api::host::acadrust::entities::{Line, LwPolyline};
use ocs_plugin_api::host::acadrust::types::{Vector2, Vector3};
use ocs_plugin_api::host::{CommandStep, EntityType, HostApi, InteractiveCommand, PreviewWire};

use crate::raster::{raster_to_world, world_to_raster, RasterData, RasterTransform};
use crate::state::{lock_state, SharedState};

const SNAP_RADIUS_PX: f64 = 8.0;
const PREVIEW_COLOR: [f32; 4] = [0.3, 0.8, 1.0, 1.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrawKind {
    Line,
    Polyline,
}

pub(crate) fn start(
    host: &mut dyn HostApi,
    state: &SharedState,
    kind: DrawKind,
) -> Result<String, String> {
    let (raster, transform) = {
        let state = lock_state(state)?;
        (
            state
                .raster
                .clone()
                .ok_or_else(|| "open a raster before using raster snap".to_string())?,
            state.transform,
        )
    };
    host.start_interactive(Box::new(RasterDrawCommand {
        kind,
        width: raster.width,
        height: raster.height,
        raster,
        transform,
        points: Vec::new(),
    }));
    Ok(match kind {
        DrawKind::Line => "RasterFit snap line: pick the first point".to_string(),
        DrawKind::Polyline => "RasterFit snap polyline: pick points, Enter to finish".to_string(),
    })
}

struct RasterDrawCommand {
    kind: DrawKind,
    width: usize,
    height: usize,
    raster: RasterData,
    transform: RasterTransform,
    points: Vec<[f64; 3]>,
}

impl RasterDrawCommand {
    fn snapped_point(&self, pt: [f64; 3]) -> Option<[f64; 3]> {
        let point = world_to_raster(self.transform, self.height, pt)?;
        let point = rf_core::ridge::snap_to_ridge(
            &self.raster.gray,
            self.width,
            self.height,
            point.x,
            point.y,
            SNAP_RADIUS_PX,
        )
        .unwrap_or(point);
        if point.x < 0.0
            || point.y < 0.0
            || point.x > self.width as f64
            || point.y > self.height as f64
        {
            return None;
        }
        Some(raster_to_world(self.transform, self.height, point))
    }

    fn line_entity(&self) -> EntityType {
        EntityType::Line(Line::from_points(
            vector(self.points[0]),
            vector(self.points[1]),
        ))
    }

    fn polyline_entity(&self) -> EntityType {
        EntityType::LwPolyline(LwPolyline::from_points(
            self.points
                .iter()
                .map(|point| Vector2::new(point[0], point[1]))
                .collect(),
        ))
    }
}

impl InteractiveCommand for RasterDrawCommand {
    fn prompt(&self) -> String {
        match (self.kind, self.points.len()) {
            (DrawKind::Line, 0) => "RasterFit snap line: pick first point".to_string(),
            (DrawKind::Line, _) => "RasterFit snap line: pick second point".to_string(),
            (DrawKind::Polyline, 0) => "RasterFit snap polyline: pick first point".to_string(),
            (DrawKind::Polyline, _) => {
                "RasterFit snap polyline: pick next point, Enter to finish".to_string()
            }
        }
    }

    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let Some(point) = self.snapped_point(pt) else {
            return CommandStep::NeedPoint;
        };
        if self.points.last().is_some_and(|previous| {
            (previous[0] - point[0]).hypot(previous[1] - point[1]) <= f64::EPSILON
        }) {
            return CommandStep::NeedPoint;
        }
        self.points.push(point);
        if self.kind == DrawKind::Line && self.points.len() == 2 {
            CommandStep::CommitAndEnd(self.line_entity())
        } else {
            CommandStep::NeedPoint
        }
    }

    fn on_enter(&mut self) -> CommandStep {
        match self.kind {
            DrawKind::Polyline if self.points.len() >= 2 => {
                CommandStep::CommitAndEnd(self.polyline_entity())
            }
            DrawKind::Polyline if self.points.is_empty() => CommandStep::Cancel,
            _ => CommandStep::NeedPoint,
        }
    }

    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        let point = self.snapped_point(pt).unwrap_or(pt);
        let marker = self.transform.sx.abs().max(self.transform.sy.abs()) * 1.5;
        let mut wires = vec![
            PreviewWire::line(
                [point[0] - marker, point[1], 0.0],
                [point[0] + marker, point[1], 0.0],
                Some(PREVIEW_COLOR),
            ),
            PreviewWire::line(
                [point[0], point[1] - marker, 0.0],
                [point[0], point[1] + marker, 0.0],
                Some(PREVIEW_COLOR),
            ),
        ];
        if let Some(first) = self.points.first() {
            let mut points = self.points.clone();
            points.push(point);
            if self.kind == DrawKind::Line {
                points = vec![*first, point];
            }
            wires.push(PreviewWire::with_color(points, PREVIEW_COLOR));
        }
        wires
    }
}

fn vector(point: [f64; 3]) -> Vector3 {
    Vector3::new(point[0], point[1], 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(kind: DrawKind, gray: Vec<u8>) -> RasterDrawCommand {
        RasterDrawCommand {
            kind,
            width: 100,
            height: 100,
            raster: RasterData {
                gray,
                width: 100,
                height: 100,
            },
            transform: RasterTransform::default(),
            points: Vec::new(),
        }
    }

    fn horizontal_ridge() -> Vec<u8> {
        let mut gray = vec![255; 100 * 100];
        for x in 10..90 {
            gray[50 * 100 + x] = 0;
        }
        gray
    }

    #[test]
    fn line_commits_native_geometry_with_subpixel_ridge_snap() {
        let mut command = command(DrawKind::Line, horizontal_ridge());
        assert!(matches!(
            command.on_point([20.2, 49.0, 0.0]),
            CommandStep::NeedPoint
        ));
        let step = command.on_point([80.2, 49.2, 0.0]);
        let CommandStep::CommitAndEnd(EntityType::Line(line)) = step else {
            panic!("expected native LINE");
        };
        assert!((line.start.y - 50.0).abs() < 0.6);
        assert!((line.end.y - 50.0).abs() < 0.6);
    }

    #[test]
    fn polyline_enter_commits_all_snapped_vertices() {
        let mut command = command(DrawKind::Polyline, horizontal_ridge());
        for point in [[20.0, 49.0, 0.0], [50.0, 49.2, 0.0], [80.0, 49.1, 0.0]] {
            assert!(matches!(command.on_point(point), CommandStep::NeedPoint));
        }
        let CommandStep::CommitAndEnd(EntityType::LwPolyline(polyline)) = command.on_enter() else {
            panic!("expected native LWPOLYLINE");
        };
        assert_eq!(polyline.vertices.len(), 3);
        assert!(polyline
            .vertices
            .iter()
            .all(|vertex| (vertex.location.y - 50.0).abs() < 0.6));
    }

    #[test]
    fn preview_has_snap_marker_and_rubber_band() {
        let mut command = command(DrawKind::Line, horizontal_ridge());
        assert_eq!(command.on_cursor_move([20.0, 49.0, 0.0]).len(), 2);
        command.on_point([20.0, 49.0, 0.0]);
        assert_eq!(command.on_cursor_move([80.0, 49.0, 0.0]).len(), 3);
    }

    #[test]
    fn blank_raster_uses_clicked_point_and_rejects_outside_points() {
        let mut command = command(DrawKind::Line, vec![255; 100 * 100]);
        assert_eq!(
            command.snapped_point([12.0, 34.0, 0.0]),
            Some([12.0, 34.0, 0.0])
        );
        assert!(matches!(
            command.on_point([-1.0, 20.0, 0.0]),
            CommandStep::NeedPoint
        ));
        assert!(command.points.is_empty());
    }
}
