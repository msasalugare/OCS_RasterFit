use std::sync::{Arc, Mutex};

use ocs_plugin_api::host::{CommandStep, EntityType, InteractiveCommand, PreviewWire};
use rf_core::geometry::Point;
use rf_core::mask::{default_base_all, MaskParams, MaskStroke, StrokeKind, StrokeMode};

use crate::raster::{raster_to_world, world_to_raster, RasterTransform};

const INCLUDE_COLOR: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const EXCLUDE_COLOR: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaskMode {
    Include,
    Exclude,
    Rect,
    Polygon,
    Eraser,
}

#[derive(Debug, Clone)]
pub(crate) struct MaskState {
    pub strokes: Vec<MaskStroke>,
    pub enabled: bool,
    pub brush_radius: f64,
}

impl Default for MaskState {
    fn default() -> Self {
        Self {
            strokes: Vec::new(),
            enabled: false,
            brush_radius: 6.0,
        }
    }
}

pub(crate) type SharedMask = Arc<Mutex<MaskState>>;

pub(crate) struct MaskCommand {
    shared: SharedMask,
    mode: MaskMode,
    width: usize,
    height: usize,
    transform: RasterTransform,
    points: Vec<Point>,
}

impl MaskCommand {
    pub(crate) fn new(
        shared: SharedMask,
        mode: MaskMode,
        width: usize,
        height: usize,
        transform: RasterTransform,
    ) -> Self {
        Self {
            shared,
            mode,
            width,
            height,
            transform,
            points: Vec::new(),
        }
    }

    fn stroke_mode(&self) -> StrokeMode {
        match self.mode {
            MaskMode::Exclude | MaskMode::Eraser => StrokeMode::Exclude,
            _ => StrokeMode::Include,
        }
    }

    fn finish(&mut self, kind: StrokeKind) -> CommandStep {
        let Ok(mut state) = self.shared.lock() else {
            return CommandStep::Cancel;
        };
        let radius = if kind == StrokeKind::Brush {
            state.brush_radius
        } else {
            0.0
        };
        state.strokes.push(MaskStroke {
            mode: self.stroke_mode(),
            kind,
            radius,
            points: std::mem::take(&mut self.points),
        });
        state.enabled = true;
        CommandStep::Dispatch("RF_MASK_SYNC".to_string())
    }
}

impl InteractiveCommand for MaskCommand {
    fn prompt(&self) -> String {
        match self.mode {
            MaskMode::Include => "RasterFit mask: click brush points, Enter to finish".to_string(),
            MaskMode::Exclude => {
                "RasterFit mask: click exclude brush points, Enter to finish".to_string()
            }
            MaskMode::Eraser => "RasterFit mask: click eraser points, Enter to finish".to_string(),
            MaskMode::Rect if self.points.is_empty() => {
                "RasterFit mask: pick first rectangle corner".to_string()
            }
            MaskMode::Rect => "RasterFit mask: pick opposite rectangle corner".to_string(),
            MaskMode::Polygon => {
                "RasterFit mask: click polygon vertices, Enter to close".to_string()
            }
        }
    }

    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let Some(point) = world_to_raster(self.transform, self.height, pt) else {
            return CommandStep::Cancel;
        };
        if point.x < 0.0
            || point.y < 0.0
            || point.x > self.width as f64
            || point.y > self.height as f64
        {
            return CommandStep::NeedPoint;
        }

        self.points.push(point);
        if self.mode == MaskMode::Rect && self.points.len() == 2 {
            self.finish(StrokeKind::Rect)
        } else {
            CommandStep::NeedPoint
        }
    }

    fn on_enter(&mut self) -> CommandStep {
        match self.mode {
            MaskMode::Polygon if self.points.len() >= 3 => self.finish(StrokeKind::Polygon),
            MaskMode::Polygon | MaskMode::Rect => CommandStep::NeedPoint,
            _ if self.points.is_empty() => CommandStep::Cancel,
            _ => self.finish(StrokeKind::Brush),
        }
    }

    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        let Ok(state) = self.shared.lock() else {
            return Vec::new();
        };
        let mut wires = state
            .strokes
            .iter()
            .flat_map(|stroke| stroke_previews(stroke, self.transform, self.height))
            .collect::<Vec<_>>();
        let brush_radius = state.brush_radius;
        drop(state);

        let Some(cursor) = world_to_raster(self.transform, self.height, pt) else {
            return wires;
        };
        let current = match self.mode {
            MaskMode::Rect if self.points.is_empty() => return wires,
            MaskMode::Rect => MaskStroke {
                mode: self.stroke_mode(),
                kind: StrokeKind::Rect,
                radius: 0.0,
                points: vec![self.points[0], cursor],
            },
            MaskMode::Polygon if self.points.is_empty() => return wires,
            MaskMode::Polygon => MaskStroke {
                mode: self.stroke_mode(),
                kind: StrokeKind::Polygon,
                radius: 0.0,
                points: self
                    .points
                    .iter()
                    .copied()
                    .chain(std::iter::once(cursor))
                    .collect(),
            },
            _ => MaskStroke {
                mode: self.stroke_mode(),
                kind: StrokeKind::Brush,
                radius: brush_radius,
                points: self
                    .points
                    .iter()
                    .copied()
                    .chain(std::iter::once(cursor))
                    .collect(),
            },
        };
        wires.extend(stroke_previews(&current, self.transform, self.height));
        wires
    }
}

fn stroke_previews(
    stroke: &MaskStroke,
    transform: RasterTransform,
    height: usize,
) -> Vec<PreviewWire> {
    let color = match stroke.mode {
        StrokeMode::Include => INCLUDE_COLOR,
        StrokeMode::Exclude => EXCLUDE_COLOR,
    };
    match stroke.kind {
        StrokeKind::Brush => brush_previews(stroke, transform, height, color),
        StrokeKind::Rect if stroke.points.len() >= 2 => {
            let a = stroke.points[0];
            let b = stroke.points[1];
            vec![world_polyline(
                vec![a, Point::new(b.x, a.y), b, Point::new(a.x, b.y), a],
                transform,
                height,
                color,
            )]
        }
        StrokeKind::Polygon if stroke.points.len() >= 2 => {
            let mut points = stroke.points.clone();
            if points.len() >= 3 {
                points.push(points[0]);
            }
            vec![world_polyline(points, transform, height, color)]
        }
        _ => Vec::new(),
    }
}

pub(crate) fn stroke_fill_polygons(
    stroke: &MaskStroke,
    transform: RasterTransform,
    height: usize,
) -> Vec<Vec<[f64; 3]>> {
    let polygons = match stroke.kind {
        StrokeKind::Brush => brush_polygons(stroke),
        StrokeKind::Rect if stroke.points.len() >= 2 => {
            let a = stroke.points[0];
            let b = stroke.points[1];
            vec![vec![a, Point::new(b.x, a.y), b, Point::new(a.x, b.y), a]]
        }
        StrokeKind::Polygon if stroke.points.len() >= 3 => {
            let mut points = stroke.points.clone();
            points.push(points[0]);
            vec![points]
        }
        _ => Vec::new(),
    };
    polygons
        .into_iter()
        .map(|points| {
            let mut world = world_points(points, transform, height);
            if world.len() > 1 && world.first() == world.last() {
                world.pop();
            }
            world
        })
        .filter(|points| points.len() >= 3)
        .collect()
}

fn brush_previews(
    stroke: &MaskStroke,
    transform: RasterTransform,
    height: usize,
    color: [f32; 4],
) -> Vec<PreviewWire> {
    brush_polygons(stroke)
        .into_iter()
        .map(|points| world_polyline(points, transform, height, color))
        .collect()
}

fn brush_polygons(stroke: &MaskStroke) -> Vec<Vec<Point>> {
    if !stroke.radius.is_finite() || stroke.radius <= 0.0 {
        return Vec::new();
    }
    let mut polygons = Vec::new();
    for &center in &stroke.points {
        polygons.push(
            (0..=32)
                .map(|index| {
                    let angle = index as f64 * std::f64::consts::TAU / 32.0;
                    Point::new(
                        center.x + stroke.radius * angle.cos(),
                        center.y + stroke.radius * angle.sin(),
                    )
                })
                .collect(),
        );
    }
    for segment in stroke.points.windows(2) {
        let [a, b] = [segment[0], segment[1]];
        let delta = b - a;
        let length = (delta.x * delta.x + delta.y * delta.y).sqrt();
        if length <= f64::EPSILON {
            continue;
        }
        let normal = Point::new(
            -delta.y * stroke.radius / length,
            delta.x * stroke.radius / length,
        );
        polygons.push(vec![
            a + normal,
            b + normal,
            b - normal,
            a - normal,
            a + normal,
        ]);
    }
    polygons
}

fn world_polyline(
    points: Vec<Point>,
    transform: RasterTransform,
    height: usize,
    color: [f32; 4],
) -> PreviewWire {
    PreviewWire::with_color(world_points(points, transform, height), color)
}

fn world_points(points: Vec<Point>, transform: RasterTransform, height: usize) -> Vec<[f64; 3]> {
    points
        .into_iter()
        .map(|point| raster_to_world(transform, height, point))
        .collect()
}

pub(crate) fn mask_from_lines(
    entities: &[EntityType],
    transform: RasterTransform,
    height: usize,
) -> Result<Vec<MaskStroke>, String> {
    let mut strokes = Vec::new();
    for (index, entity) in entities.iter().enumerate() {
        match entity {
            EntityType::Line(line) => {
                let a = world_to_raster(
                    transform,
                    height,
                    [line.start.x, line.start.y, line.start.z],
                )
                .ok_or_else(|| {
                    format!("failed to convert native LINE entity {index} to raster coordinates")
                })?;
                let b = world_to_raster(transform, height, [line.end.x, line.end.y, line.end.z])
                    .ok_or_else(|| {
                        format!(
                            "failed to convert native LINE entity {index} to raster coordinates"
                        )
                    })?;
                strokes.push(MaskStroke {
                    mode: StrokeMode::Include,
                    kind: StrokeKind::Brush,
                    radius: 6.0,
                    points: vec![a, b],
                });
            }
            EntityType::RasterImage(_) => {}
            _ => {
                return Err(format!(
                    "mask from lines supports only native LINE entities; unsupported entity at index {index}"
                ));
            }
        }
    }
    Ok(strokes)
}

pub(crate) fn mask_params(state: &MaskState) -> MaskParams {
    MaskParams {
        enabled: state.enabled,
        base_all: default_base_all(&state.strokes),
        feather: 3.0,
    }
}

#[cfg(test)]
mod tests {
    use ocs_plugin_api::host::acadrust::entities::{Line, Point as CadPoint, RasterImage};
    use ocs_plugin_api::host::acadrust::types::Vector3;

    use super::*;

    fn shared() -> SharedMask {
        SharedMask::default()
    }

    fn command(shared: &SharedMask, mode: MaskMode) -> MaskCommand {
        MaskCommand::new(shared.clone(), mode, 100, 100, RasterTransform::default())
    }

    #[test]
    fn brush_modes_collect_points_and_use_the_expected_stroke_mode() {
        for (mode, expected) in [
            (MaskMode::Include, StrokeMode::Include),
            (MaskMode::Exclude, StrokeMode::Exclude),
            (MaskMode::Eraser, StrokeMode::Exclude),
        ] {
            let state = shared();
            let mut command = command(&state, mode);
            assert!(matches!(
                command.on_point([10.0, 90.0, 0.0]),
                CommandStep::NeedPoint
            ));
            assert!(matches!(
                command.on_point([20.0, 80.0, 0.0]),
                CommandStep::NeedPoint
            ));
            assert!(state.lock().unwrap().strokes.is_empty());
            assert!(
                matches!(command.on_enter(), CommandStep::Dispatch(command) if command == "RF_MASK_SYNC")
            );

            let state = state.lock().unwrap();
            assert_eq!(state.strokes.len(), 1);
            let stroke = &state.strokes[0];
            assert_eq!(stroke.mode, expected);
            assert_eq!(stroke.kind, StrokeKind::Brush);
            assert_eq!(stroke.radius, 6.0);
            assert_eq!(
                stroke.points,
                vec![Point::new(10.0, 10.0), Point::new(20.0, 20.0)]
            );
        }
    }

    #[test]
    fn changed_brush_radius_updates_cursor_and_finished_stroke() {
        let state = shared();
        state.lock().unwrap().brush_radius = 14.0;
        let mut command = command(&state, MaskMode::Include);
        let preview = command.on_cursor_move([20.0, 80.0, 0.0]);
        let PreviewWire::Polyline { points, .. } = &preview[0] else {
            panic!("expected brush outline");
        };
        assert_eq!(points[0], [34.0, 80.0, 0.0]);
        command.on_point([20.0, 80.0, 0.0]);
        assert!(
            matches!(command.on_enter(), CommandStep::Dispatch(command) if command == "RF_MASK_SYNC")
        );
        assert_eq!(state.lock().unwrap().strokes[0].radius, 14.0);
    }

    #[test]
    fn rect_finishes_on_the_second_point() {
        let state = shared();
        let mut command = command(&state, MaskMode::Rect);
        assert!(matches!(
            command.on_point([10.0, 90.0, 0.0]),
            CommandStep::NeedPoint
        ));
        assert!(state.lock().unwrap().strokes.is_empty());
        assert!(matches!(
            command.on_point([30.0, 60.0, 0.0]),
            CommandStep::Dispatch(command) if command == "RF_MASK_SYNC"
        ));

        let state = state.lock().unwrap();
        assert_eq!(state.strokes.len(), 1);
        assert_eq!(state.strokes[0].mode, StrokeMode::Include);
        assert_eq!(state.strokes[0].kind, StrokeKind::Rect);
        assert_eq!(state.strokes[0].radius, 0.0);
        assert_eq!(
            state.strokes[0].points,
            vec![Point::new(10.0, 10.0), Point::new(30.0, 40.0)]
        );
    }

    #[test]
    fn polygon_closes_only_with_at_least_three_points() {
        let state = shared();
        let mut command = command(&state, MaskMode::Polygon);
        command.on_point([10.0, 90.0, 0.0]);
        command.on_point([30.0, 90.0, 0.0]);
        assert!(matches!(command.on_enter(), CommandStep::NeedPoint));
        assert!(state.lock().unwrap().strokes.is_empty());

        command.on_point([20.0, 60.0, 0.0]);
        assert!(
            matches!(command.on_enter(), CommandStep::Dispatch(command) if command == "RF_MASK_SYNC")
        );
        let state = state.lock().unwrap();
        assert_eq!(state.strokes.len(), 1);
        assert_eq!(state.strokes[0].mode, StrokeMode::Include);
        assert_eq!(state.strokes[0].kind, StrokeKind::Polygon);
        assert_eq!(state.strokes[0].points.len(), 3);
    }

    #[test]
    fn points_outside_the_image_are_rejected() {
        let state = shared();
        let mut command = command(&state, MaskMode::Include);
        assert!(matches!(
            command.on_point([-1.0, 50.0, 0.0]),
            CommandStep::NeedPoint
        ));
        assert!(matches!(
            command.on_point([50.0, 101.0, 0.0]),
            CommandStep::NeedPoint
        ));
        assert!(state.lock().unwrap().strokes.is_empty());
        assert!(matches!(command.on_enter(), CommandStep::Cancel));
    }

    #[test]
    fn brush_cursor_is_visible_before_the_first_click() {
        let state = shared();
        let mut include = command(&state, MaskMode::Include);
        let preview = include.on_cursor_move([20.0, 80.0, 0.0]);
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].color(), Some(INCLUDE_COLOR));
        let PreviewWire::Polyline { points, .. } = &preview[0] else {
            panic!("expected brush outline");
        };
        assert_eq!(points.len(), 33);
        assert_eq!(points[0], [26.0, 80.0, 0.0]);
        assert_eq!(points[16], [14.0, 80.0, 0.0]);

        for mode in [MaskMode::Exclude, MaskMode::Eraser] {
            let mut exclude = command(&state, mode);
            assert_eq!(
                exclude.on_cursor_move([20.0, 80.0, 0.0])[0].color(),
                Some(EXCLUDE_COLOR)
            );
        }
    }

    #[test]
    fn committed_shapes_produce_translated_closed_fill_polygons() {
        let transform = RasterTransform {
            origin: [100.0, 200.0],
            ..RasterTransform::default()
        };
        let rect = MaskStroke {
            mode: StrokeMode::Include,
            kind: StrokeKind::Rect,
            radius: 0.0,
            points: vec![Point::new(10.0, 10.0), Point::new(30.0, 40.0)],
        };
        let polygons = stroke_fill_polygons(&rect, transform, 100);
        assert_eq!(polygons.len(), 1);
        assert_eq!(polygons[0].len(), 4);
        assert_eq!(polygons[0][0], [110.0, 290.0, 0.0]);

        let brush = MaskStroke {
            mode: StrokeMode::Include,
            kind: StrokeKind::Brush,
            radius: 6.0,
            points: vec![Point::new(10.0, 10.0), Point::new(30.0, 10.0)],
        };
        let polygons = stroke_fill_polygons(&brush, transform, 100);
        assert_eq!(polygons.len(), 3);
        assert!(polygons.iter().all(|polygon| polygon.len() >= 4));
    }

    #[test]
    fn brush_preview_shows_radius_and_segment_outline() {
        let state = shared();
        let mut include = command(&state, MaskMode::Include);
        include.on_point([10.0, 90.0, 0.0]);
        let preview = include.on_cursor_move([20.0, 80.0, 0.0]);
        assert_eq!(preview.len(), 3);
        assert!(preview
            .iter()
            .all(|wire| wire.color() == Some(INCLUDE_COLOR)));
        let PreviewWire::Polyline { points, .. } = &preview[2] else {
            panic!("expected segment outline");
        };
        assert_eq!(points.len(), 5);
        assert_eq!(points.first(), points.last());
    }

    #[test]
    fn editing_shows_existing_include_and_exclude_strokes() {
        let state = shared();
        state.lock().unwrap().strokes = vec![
            MaskStroke {
                mode: StrokeMode::Include,
                kind: StrokeKind::Rect,
                radius: 0.0,
                points: vec![Point::new(10.0, 10.0), Point::new(30.0, 40.0)],
            },
            MaskStroke {
                mode: StrokeMode::Exclude,
                kind: StrokeKind::Polygon,
                radius: 0.0,
                points: vec![
                    Point::new(50.0, 50.0),
                    Point::new(60.0, 50.0),
                    Point::new(55.0, 60.0),
                ],
            },
        ];
        let mut rect = command(&state, MaskMode::Rect);
        let preview = rect.on_cursor_move([80.0, 20.0, 0.0]);
        assert_eq!(preview.len(), 2);
        assert_eq!(preview[0].color(), Some(INCLUDE_COLOR));
        assert_eq!(preview[1].color(), Some(EXCLUDE_COLOR));
    }

    #[test]
    fn rect_preview_is_closed_and_aligned_to_raster_axes() {
        let state = shared();
        let mut command = command(&state, MaskMode::Rect);
        command.on_point([10.0, 90.0, 0.0]);
        let preview = command.on_cursor_move([30.0, 60.0, 0.0]);
        let PreviewWire::Polyline { points, color } = &preview[0] else {
            panic!("expected polyline preview");
        };
        assert_eq!(*color, Some(INCLUDE_COLOR));
        assert_eq!(
            points,
            &vec![
                [10.0, 90.0, 0.0],
                [30.0, 90.0, 0.0],
                [30.0, 60.0, 0.0],
                [10.0, 60.0, 0.0],
                [10.0, 90.0, 0.0],
            ]
        );
    }

    #[test]
    fn mask_from_lines_converts_lines_and_skips_raster_images() {
        let line = EntityType::Line(Line::from_coords(10.0, 90.0, 0.0, 30.0, 60.0, 0.0));
        let image =
            EntityType::RasterImage(RasterImage::new("raster.png", Vector3::ZERO, 100.0, 100.0));
        let strokes = mask_from_lines(&[line, image], RasterTransform::default(), 100).unwrap();
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].mode, StrokeMode::Include);
        assert_eq!(strokes[0].kind, StrokeKind::Brush);
        assert_eq!(strokes[0].radius, 6.0);
        assert_eq!(
            strokes[0].points,
            vec![Point::new(10.0, 10.0), Point::new(30.0, 40.0)]
        );
    }

    #[test]
    fn mask_from_lines_clearly_rejects_unsupported_entities() {
        let entities = vec![EntityType::Point(CadPoint::from_coords(0.0, 0.0, 0.0))];
        let error = mask_from_lines(&entities, RasterTransform::default(), 100).unwrap_err();
        assert!(error.contains("only native LINE"));
        assert!(error.contains("index 0"));
    }

    #[test]
    fn mask_params_follow_enabled_and_include_stroke_semantics() {
        let mut state = MaskState::default();
        assert!(!state.enabled);
        assert_eq!(state.brush_radius, 6.0);
        assert!(state.strokes.is_empty());

        let params = mask_params(&state);
        assert!(!params.enabled);
        assert!(params.base_all);
        assert_eq!(params.feather, 3.0);

        state.enabled = true;
        state.strokes.push(MaskStroke {
            mode: StrokeMode::Include,
            kind: StrokeKind::Brush,
            radius: 6.0,
            points: vec![Point::new(10.0, 10.0)],
        });
        let params = mask_params(&state);
        assert!(params.enabled);
        assert!(!params.base_all);
        assert_eq!(params.feather, 3.0);
    }
}
