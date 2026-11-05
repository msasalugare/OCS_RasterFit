use ocs_plugin_api::host::acadrust::entities::LwPolyline;
use ocs_plugin_api::host::acadrust::types::Vector3;
use ocs_plugin_api::host::{EntityType, ExtendedDataRecord, Handle, HostApi, XDataValue};
use rf_core::fitter::FitterResult;
use rf_core::sketch::{
    ArcId, CircleId, LineId, PointId, Sketch, SketchArc, SketchCircle, SketchLine, SketchPoint,
};

use crate::cad::CadEntity;
use crate::raster::{is_isotropic, raster_to_world, world_to_raster, RasterTransform};

const ANISOTROPIC_CURVE_ERROR: &str =
    "anisotropic calibration cannot fit circles or arcs exactly; use isotropic calibration or lines only";

#[derive(Debug)]
pub(crate) struct NativeSelection {
    pub sketch: Sketch,
    pub originals: Vec<EntityType>,
    pub primitive_origins: Vec<Handle>,
}

pub(crate) fn selected_sketch(
    host: &mut dyn HostApi,
    transform: RasterTransform,
    raster_height: usize,
) -> Result<Option<NativeSelection>, String> {
    let handles = host.selection();
    if handles.is_empty() {
        return Ok(None);
    }
    let entities = host.entities_batch(&handles)?;
    let mut originals = Vec::with_capacity(entities.len());
    for (handle, entity) in handles.iter().zip(entities) {
        let entity = entity.ok_or_else(|| format!("selected entity {handle} no longer exists"))?;
        if matches!(entity, EntityType::RasterImage(_))
            || matches!(
                entity.common().layer.as_str(),
                crate::overlay::MASK_LAYER | crate::overlay::GUIDES_LAYER
            )
        {
            continue;
        }
        originals.push(entity);
    }
    sketch_from_entities(originals, transform, raster_height)
}

pub(crate) fn sketch_from_entities(
    originals: Vec<EntityType>,
    transform: RasterTransform,
    raster_height: usize,
) -> Result<Option<NativeSelection>, String> {
    if originals.is_empty() {
        return Ok(None);
    }
    if !is_isotropic(transform)
        && originals
            .iter()
            .any(|entity| matches!(entity, EntityType::Circle(_) | EntityType::Arc(_)))
    {
        return Err(ANISOTROPIC_CURVE_ERROR.to_string());
    }

    let mut sketch = Sketch::default();
    let mut line_origins = Vec::new();
    let mut circle_origins = Vec::new();
    let mut arc_origins = Vec::new();
    for entity in &originals {
        let handle = entity.common().handle;
        match entity {
            EntityType::Line(line) => {
                ensure_planar(line.start, "line")?;
                ensure_planar(line.end, "line")?;
                let a = push_point(&mut sketch, line.start, transform, raster_height, "line")?;
                let b = push_point(&mut sketch, line.end, transform, raster_height, "line")?;
                push_line(&mut sketch, a, b);
                line_origins.push(handle);
            }
            EntityType::Circle(circle) => {
                ensure_planar(circle.center, "circle")?;
                ensure_xy_normal(circle.normal, "circle")?;
                ensure_radius(circle.radius, "circle")?;
                let center = push_point(
                    &mut sketch,
                    circle.center,
                    transform,
                    raster_height,
                    "circle",
                )?;
                sketch.circles.push(SketchCircle {
                    id: CircleId(sketch.circles.len() as u32 + 1),
                    center,
                    radius: circle.radius / transform.sx,
                    construction: false,
                });
                circle_origins.push(handle);
            }
            EntityType::Arc(arc) => {
                push_arc(
                    &mut sketch,
                    arc.center,
                    arc.normal,
                    arc.radius,
                    arc.start_angle,
                    arc.end_angle,
                    transform,
                    raster_height,
                    "arc",
                )?;
                arc_origins.push(handle);
            }
            EntityType::LwPolyline(polyline) => {
                push_lwpolyline(
                    &mut sketch,
                    polyline,
                    handle,
                    transform,
                    raster_height,
                    &mut line_origins,
                    &mut arc_origins,
                )?;
            }
            _ => {
                return Err(
                    "native fitting supports only LINE, LWPOLYLINE, CIRCLE and ARC selection; clear the selection to use a loaded RasterFit sketch"
                        .to_string(),
                )
            }
        }
    }
    let primitive_origins = line_origins
        .into_iter()
        .chain(circle_origins)
        .chain(arc_origins)
        .collect();
    Ok(Some(NativeSelection {
        sketch,
        originals,
        primitive_origins,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_fit_entities(
    host: &mut dyn HostApi,
    originals: Vec<EntityType>,
    outputs: Vec<CadEntity>,
    primitive_origins: &[Handle],
    fit: &FitterResult,
    transform: RasterTransform,
    raster_height: usize,
    fit_geometry: bool,
) -> Result<usize, String> {
    if originals.is_empty() {
        return Err("fitted geometry does not match the selected CAD entities".to_string());
    }
    if fit_geometry && outputs.len() != primitive_origins.len() {
        return Err("fitted primitive origins do not match the fit output".to_string());
    }
    let mut handles = Vec::with_capacity(originals.len());
    let mut entities = Vec::with_capacity(originals.len());
    let mut records = Vec::with_capacity(originals.len());
    let mut used = vec![false; outputs.len()];
    for (index, original) in originals.into_iter().enumerate() {
        let handle: Handle = original.common().handle;
        let entity = if fit_geometry {
            fit_entity(&original, &outputs, primitive_origins, &mut used)?
        } else {
            rigid_entity(&original, fit, transform, raster_height)?
        };
        entities.push(entity);
        let mut record = ExtendedDataRecord::new("RASTERFIT");
        record.add_value(XDataValue::String("RASTERFIT-CLASS:FIT".to_string()));
        record.add_value(XDataValue::String(
            if matches!(original, EntityType::LwPolyline(_)) {
                "LWPOLYLINE"
            } else {
                outputs
                    .get(index)
                    .map(|output| output.kind)
                    .unwrap_or_else(|| entity_kind(&original))
            }
            .to_string(),
        ));
        record.add_value(XDataValue::String(
            outputs
                .get(index)
                .map(|output| output.id.to_string())
                .unwrap_or_else(|| (index + 1).to_string()),
        ));
        record.add_value(XDataValue::Real(fit.rms_px));
        record.add_value(XDataValue::Real(1.0 - fit.uncovered_ratio.clamp(0.0, 1.0)));
        record.add_value(XDataValue::Integer16(i16::from(fit.converged)));
        handles.push(handle);
        records.push(record);
    }
    if fit_geometry && used.iter().any(|used| !used) {
        return Err(
            "fitted geometry has primitives that do not belong to the selection".to_string(),
        );
    }

    host.begin_undo_group("RasterFit fit selection");
    let result = (|| {
        host.update_entities_transaction("RasterFit fit", entities)
            .map_err(|error| format!("could not update selected entities: {error}"))?;
        for (handle, record) in handles.iter().copied().zip(records) {
            if !host.write_record(handle, record) {
                return Err(format!("could not write fit metadata to entity {handle}"));
            }
        }
        Ok(())
    })();
    host.end_undo_group();
    result?;

    host.bump_geometry();
    host.set_dirty();
    if let Err(error) = host.set_selection(&handles) {
        host.push_error(&format!(
            "RasterFit entities were updated, but selection failed: {error}"
        ));
    }
    Ok(handles.len())
}

fn fit_entity(
    original: &EntityType,
    outputs: &[CadEntity],
    origins: &[Handle],
    used: &mut [bool],
) -> Result<EntityType, String> {
    if let EntityType::LwPolyline(polyline) = original {
        return fit_polyline(polyline, outputs, origins, used);
    }
    let index = take_fit_output(
        original.common().handle,
        entity_kind(original),
        outputs,
        origins,
        used,
    )?;
    Ok(with_original_common(
        original,
        outputs[index].entity.clone(),
    ))
}

fn fit_polyline(
    original: &LwPolyline,
    outputs: &[CadEntity],
    origins: &[Handle],
    used: &mut [bool],
) -> Result<EntityType, String> {
    let handle = original.common.handle;
    let vertex_count = polyline_vertex_count(original);
    let segment_count = if original.is_closed {
        vertex_count
    } else {
        vertex_count - 1
    };
    let mut polyline = original.clone();
    polyline.vertices.clear();
    for index in 0..segment_count {
        let original_bulge = original.vertices[index].bulge;
        let kind = if original_bulge == 0.0 { "LINE" } else { "ARC" };
        let output_index = take_fit_output(handle, kind, outputs, origins, used)?;
        let (start, end, bulge) =
            fit_segment_geometry(&outputs[output_index].entity, original_bulge, index + 1)?;
        if index == 0 {
            let mut start_vertex = original.vertices[index];
            start_vertex.location.x = start.x;
            start_vertex.location.y = start.y;
            start_vertex.bulge = bulge;
            polyline.vertices.push(start_vertex);
        } else {
            polyline.vertices[index].bulge = bulge;
        }
        if !original.is_closed || index + 1 < vertex_count {
            let next = (index + 1) % vertex_count;
            let mut end_vertex = original.vertices[next];
            end_vertex.location.x = end.x;
            end_vertex.location.y = end.y;
            polyline.vertices.push(end_vertex);
        }
    }
    Ok(EntityType::LwPolyline(polyline))
}

fn polyline_vertex_count(polyline: &LwPolyline) -> usize {
    if polyline.is_closed
        && polyline.vertices.len() > 1
        && polyline.vertices.first().map(|vertex| vertex.location)
            == polyline.vertices.last().map(|vertex| vertex.location)
    {
        polyline.vertices.len() - 1
    } else {
        polyline.vertices.len()
    }
}

fn fit_segment_geometry(
    output: &EntityType,
    original_bulge: f64,
    segment: usize,
) -> Result<(Vector3, Vector3, f64), String> {
    match output {
        EntityType::Line(line) => Ok((line.start, line.end, 0.0)),
        EntityType::Arc(arc) => {
            let reversed = original_bulge < 0.0;
            let start_angle = if reversed {
                arc.end_angle
            } else {
                arc.start_angle
            };
            let end_angle = if reversed {
                arc.start_angle
            } else {
                arc.end_angle
            };
            let sweep = if reversed {
                -(start_angle - end_angle).rem_euclid(std::f64::consts::TAU)
            } else {
                (end_angle - start_angle).rem_euclid(std::f64::consts::TAU)
            };
            let bulge = (sweep / 4.0).tan();
            if !bulge.is_finite() {
                return Err(format!(
                    "fitted LWPOLYLINE segment {segment} has invalid bulge geometry"
                ));
            }
            Ok((
                arc_point(arc.center, arc.radius, start_angle),
                arc_point(arc.center, arc.radius, end_angle),
                bulge,
            ))
        }
        _ => Err(format!(
            "fitted LWPOLYLINE segment {segment} has an unexpected entity type"
        )),
    }
}

fn arc_point(center: Vector3, radius: f64, angle: f64) -> Vector3 {
    Vector3::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
        0.0,
    )
}

fn take_fit_output(
    handle: Handle,
    kind: &str,
    outputs: &[CadEntity],
    origins: &[Handle],
    used: &mut [bool],
) -> Result<usize, String> {
    let index = (0..outputs.len())
        .find(|index| {
            !used[*index]
                && origins[*index] == handle
                && entity_kind(&outputs[*index].entity) == kind
        })
        .ok_or_else(|| format!("fitted {kind} primitive for entity {handle} is missing"))?;
    used[index] = true;
    Ok(index)
}

fn entity_kind(entity: &EntityType) -> &'static str {
    match entity {
        EntityType::Line(_) => "LINE",
        EntityType::Circle(_) => "CIRCLE",
        EntityType::Arc(_) => "ARC",
        EntityType::LwPolyline(_) => "LWPOLYLINE",
        _ => "UNKNOWN",
    }
}

fn rigid_entity(
    original: &EntityType,
    fit: &FitterResult,
    transform: RasterTransform,
    raster_height: usize,
) -> Result<EntityType, String> {
    let mut entity = original.clone();
    let scale = fit.scale;
    if !scale.is_finite() || scale <= 0.0 {
        return Err("fit background scale is invalid".to_string());
    }
    match &mut entity {
        EntityType::Line(line) => {
            line.start = rigid_point(line.start, fit, transform, raster_height)?;
            line.end = rigid_point(line.end, fit, transform, raster_height)?;
        }
        EntityType::Circle(circle) => {
            circle.center = rigid_point(circle.center, fit, transform, raster_height)?;
            circle.radius /= scale;
        }
        EntityType::Arc(arc) => {
            arc.center = rigid_point(arc.center, fit, transform, raster_height)?;
            arc.radius /= scale;
            arc.start_angle -= fit.transform[0];
            arc.end_angle -= fit.transform[0];
        }
        EntityType::LwPolyline(polyline) => {
            for vertex in &mut polyline.vertices {
                let world = Vector3::new(vertex.location.x, vertex.location.y, 0.0);
                let world = rigid_point(world, fit, transform, raster_height)?;
                vertex.location.x = world.x;
                vertex.location.y = world.y;
            }
        }
        _ => return Err("unsupported native entity in rigid fit output".to_string()),
    }
    Ok(entity)
}

fn rigid_point(
    world: Vector3,
    fit: &FitterResult,
    transform: RasterTransform,
    raster_height: usize,
) -> Result<Vector3, String> {
    let point = world_to_raster(transform, raster_height, [world.x, world.y, world.z])
        .ok_or_else(|| "could not convert native fit point to raster".to_string())?;
    let [theta, tx, ty] = fit.transform;
    let x = (point.x - tx) / fit.scale;
    let y = (point.y - ty) / fit.scale;
    let (sin, cos) = theta.sin_cos();
    let raster = Point2 {
        x: cos * x + sin * y,
        y: -sin * x + cos * y,
    };
    let world = raster_to_world(
        transform,
        raster_height,
        rf_core::geometry::Point::new(raster.x, raster.y),
    );
    if world.iter().all(|coordinate| coordinate.is_finite()) {
        Ok(Vector3::new(world[0], world[1], 0.0))
    } else {
        Err("native fit point is non-finite".to_string())
    }
}

struct Point2 {
    x: f64,
    y: f64,
}

fn with_original_common(original: &EntityType, mut output: EntityType) -> EntityType {
    *output.common_mut() = original.common().clone();
    output
}

fn push_line(sketch: &mut Sketch, a: PointId, b: PointId) {
    sketch.lines.push(SketchLine {
        id: LineId(sketch.lines.len() as u32 + 1),
        a,
        b,
        construction: false,
    });
}

#[allow(clippy::too_many_arguments)]
fn push_arc(
    sketch: &mut Sketch,
    center: Vector3,
    normal: Vector3,
    radius: f64,
    world_start: f64,
    world_end: f64,
    transform: RasterTransform,
    raster_height: usize,
    kind: &str,
) -> Result<(), String> {
    ensure_planar(center, kind)?;
    ensure_xy_normal(normal, kind)?;
    ensure_radius(radius, kind)?;
    let center = push_point(sketch, center, transform, raster_height, kind)?;
    sketch.arcs.push(SketchArc {
        id: ArcId(sketch.arcs.len() as u32 + 1),
        center,
        radius: radius / transform.sx,
        start: transform.theta - world_end,
        end: transform.theta - world_start,
        construction: false,
    });
    Ok(())
}

fn push_lwpolyline(
    sketch: &mut Sketch,
    polyline: &LwPolyline,
    handle: Handle,
    transform: RasterTransform,
    raster_height: usize,
    line_origins: &mut Vec<Handle>,
    arc_origins: &mut Vec<Handle>,
) -> Result<(), String> {
    ensure_xy_normal(polyline.normal, "LWPOLYLINE")?;
    if !polyline.elevation.is_finite() || polyline.elevation.abs() > 1e-9 {
        return Err("native LWPOLYLINE elevation is not supported; use XY elevation zero".into());
    }
    if !polyline.thickness.is_finite() || polyline.thickness.abs() > 1e-9 {
        return Err("native LWPOLYLINE thickness is not supported".into());
    }
    if polyline.constant_width != 0.0 {
        return Err("native LWPOLYLINE constant width is not supported".into());
    }
    for (index, vertex) in polyline.vertices.iter().enumerate() {
        if !vertex.location.x.is_finite() || !vertex.location.y.is_finite() {
            return Err(format!(
                "native LWPOLYLINE vertex {} has non-finite coordinates",
                index + 1
            ));
        }
        if !vertex.bulge.is_finite() {
            return Err(format!(
                "native LWPOLYLINE vertex {} has an invalid bulge",
                index + 1
            ));
        }
        if vertex.start_width != 0.0 || vertex.end_width != 0.0 {
            return Err(format!(
                "native LWPOLYLINE vertex {} widths are not supported",
                index + 1
            ));
        }
    }

    let vertex_count = if polyline.is_closed
        && polyline.vertices.len() > 1
        && polyline.vertices.first().map(|vertex| vertex.location)
            == polyline.vertices.last().map(|vertex| vertex.location)
    {
        polyline.vertices.len() - 1
    } else {
        polyline.vertices.len()
    };
    if vertex_count < 2 {
        return Err("native LWPOLYLINE needs at least two distinct vertices".into());
    }

    let mut points = Vec::with_capacity(vertex_count);
    for vertex in &polyline.vertices[..vertex_count] {
        points.push(push_point(
            sketch,
            Vector3::new(vertex.location.x, vertex.location.y, 0.0),
            transform,
            raster_height,
            "LWPOLYLINE",
        )?);
    }
    let segment_count = if polyline.is_closed {
        vertex_count
    } else {
        vertex_count - 1
    };
    for index in 0..segment_count {
        let next = (index + 1) % vertex_count;
        let start = polyline.vertices[index].location;
        let end = polyline.vertices[next].location;
        let bulge = polyline.vertices[index].bulge;
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let chord = dx.hypot(dy);
        if !chord.is_finite() || chord == 0.0 {
            return Err(format!(
                "native LWPOLYLINE segment {} has coincident or invalid endpoints",
                index + 1
            ));
        }
        if bulge == 0.0 {
            push_line(sketch, points[index], points[next]);
            line_origins.push(handle);
            continue;
        }
        if !is_isotropic(transform) {
            return Err(ANISOTROPIC_CURVE_ERROR.to_string());
        }
        let (center, radius, world_start, world_end) =
            bulge_arc(start.x, start.y, end.x, end.y, bulge).ok_or_else(|| {
                format!(
                    "native LWPOLYLINE segment {} bulge produces invalid arc geometry",
                    index + 1
                )
            })?;
        let kind = format!("LWPOLYLINE segment {} arc", index + 1);
        push_arc(
            sketch,
            center,
            polyline.normal,
            radius,
            world_start,
            world_end,
            transform,
            raster_height,
            &kind,
        )?;
        arc_origins.push(handle);
    }
    Ok(())
}

fn bulge_arc(
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    bulge: f64,
) -> Option<(Vector3, f64, f64, f64)> {
    if !bulge.is_finite() || bulge == 0.0 {
        return None;
    }
    let dx = end_x - start_x;
    let dy = end_y - start_y;
    let chord = dx.hypot(dy);
    if !chord.is_finite() || chord == 0.0 {
        return None;
    }
    let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
    let center_offset = chord * (1.0 - bulge * bulge) / (4.0 * bulge);
    let center_x = (start_x + end_x) / 2.0 - dy / chord * center_offset;
    let center_y = (start_y + end_y) / 2.0 + dx / chord * center_offset;
    if !center_x.is_finite() || !center_y.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return None;
    }
    let start_angle = (start_y - center_y).atan2(start_x - center_x);
    let end_angle = (end_y - center_y).atan2(end_x - center_x);
    let (world_start, world_end) = if bulge > 0.0 {
        (start_angle, end_angle)
    } else {
        (end_angle, start_angle)
    };
    Some((
        Vector3::new(center_x, center_y, 0.0),
        radius,
        world_start,
        world_end,
    ))
}

fn push_point(
    sketch: &mut Sketch,
    world: Vector3,
    transform: RasterTransform,
    raster_height: usize,
    kind: &str,
) -> Result<PointId, String> {
    ensure_planar(world, kind)?;
    let point = world_to_raster(transform, raster_height, [world.x, world.y, world.z])
        .ok_or_else(|| "could not convert selected CAD point to raster coordinates".to_string())?;
    let id = PointId(sketch.points.len() as u32 + 1);
    sketch.points.push(SketchPoint {
        id,
        x: point.x,
        y: point.y,
    });
    Ok(id)
}

fn ensure_planar(point: Vector3, kind: &str) -> Result<(), String> {
    if point.x.is_finite() && point.y.is_finite() && point.z.is_finite() && point.z.abs() <= 1e-9 {
        Ok(())
    } else {
        Err(format!("native {kind} must lie in the XY plane"))
    }
}

fn ensure_xy_normal(normal: Vector3, kind: &str) -> Result<(), String> {
    if normal.x.abs() <= 1e-9 && normal.y.abs() <= 1e-9 && normal.z > 0.0 {
        Ok(())
    } else {
        Err(format!(
            "native {kind} must use the positive XY-plane normal"
        ))
    }
}

fn ensure_radius(radius: f64, kind: &str) -> Result<(), String> {
    if radius.is_finite() && radius > 0.0 {
        Ok(())
    } else {
        Err(format!("native {kind} has an invalid radius"))
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::f64::consts::{PI, TAU};

    use ocs_plugin_api::host::acadrust::entities::{Arc, Circle, Line, LwVertex, Point};
    use ocs_plugin_api::host::{CadDocumentReader, Document, DocumentReader, InteractiveCommand};

    use super::*;
    use crate::raster::raster_to_world;

    fn polyline(vertices: &[(f64, f64, f64)], closed: bool) -> LwPolyline {
        let mut polyline = LwPolyline::new();
        polyline.vertices = vertices
            .iter()
            .map(|&(x, y, bulge)| {
                let mut vertex = LwVertex::from_coords(x, y);
                vertex.bulge = bulge;
                vertex
            })
            .collect();
        polyline.is_closed = closed;
        polyline.common.handle = Handle::new(10);
        polyline
    }

    fn polyline_sketch(polyline: LwPolyline) -> Sketch {
        sketch_from_entities(
            vec![EntityType::LwPolyline(polyline)],
            RasterTransform::default(),
            100,
        )
        .unwrap()
        .unwrap()
        .sketch
    }

    fn polyline_error(polyline: LwPolyline) -> String {
        sketch_from_entities(
            vec![EntityType::LwPolyline(polyline)],
            RasterTransform::default(),
            100,
        )
        .unwrap_err()
    }

    fn assert_near(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() <= 1e-8,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_angle_near(actual: f64, expected: f64) {
        let delta = (actual - expected + PI).rem_euclid(TAU) - PI;
        assert_near(delta, 0.0);
    }

    #[test]
    fn converts_native_geometry_to_raster_sketch() {
        let transform = RasterTransform {
            sx: 2.0,
            sy: 2.0,
            theta: 0.0,
            origin: [0.0, 0.0],
        };
        let mut line = Line::from_coords(0.0, 0.0, 0.0, 40.0, 0.0, 0.0);
        line.common.handle = Handle::new(1);
        let mut circle = Circle::from_coords(20.0, 50.0, 0.0, 10.0);
        circle.common.handle = Handle::new(2);
        let mut arc = Arc::from_coords(20.0, 50.0, 0.0, 10.0, 0.0, PI / 2.0);
        arc.common.handle = Handle::new(3);
        let selection = sketch_from_entities(
            vec![
                EntityType::Line(line),
                EntityType::Circle(circle),
                EntityType::Arc(arc),
            ],
            transform,
            100,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            selection.primitive_origins,
            vec![Handle::new(1), Handle::new(2), Handle::new(3)]
        );
        let sketch = selection.sketch;
        assert_eq!(sketch.points.len(), 4);
        assert_eq!((sketch.points[0].x, sketch.points[0].y), (0.0, 100.0));
        assert_eq!((sketch.points[1].x, sketch.points[1].y), (20.0, 100.0));
        assert_eq!(sketch.circles[0].radius, 5.0);
        assert_eq!(sketch.arcs[0].radius, 5.0);
        assert_eq!(sketch.arcs[0].start, -PI / 2.0);
        assert_eq!(sketch.arcs[0].end, 0.0);
    }

    #[test]
    fn polyline_lines_share_vertex_points_and_map_origins_by_primitive_kind() {
        let polyline = polyline(
            &[(0.0, 0.0, 1.0), (10.0, 0.0, 0.0), (10.0, 10.0, 0.0)],
            false,
        );
        let mut arc = Arc::from_coords(20.0, 50.0, 0.0, 10.0, 0.0, PI / 2.0);
        arc.common.handle = Handle::new(30);
        let mut circle = Circle::from_coords(20.0, 50.0, 0.0, 10.0);
        circle.common.handle = Handle::new(20);
        let mut line = Line::from_coords(0.0, 0.0, 0.0, 40.0, 0.0, 0.0);
        line.common.handle = Handle::new(40);
        let selection = sketch_from_entities(
            vec![
                EntityType::Arc(arc),
                EntityType::LwPolyline(polyline),
                EntityType::Circle(circle),
                EntityType::Line(line),
            ],
            RasterTransform::default(),
            100,
        )
        .unwrap()
        .unwrap();
        assert_eq!(selection.sketch.lines.len(), 2);
        assert_eq!(selection.sketch.arcs.len(), 2);
        assert_eq!(selection.sketch.circles.len(), 1);
        assert_eq!(selection.sketch.points.len(), 8);
        assert_eq!(
            selection.primitive_origins,
            vec![
                Handle::new(10),
                Handle::new(40),
                Handle::new(20),
                Handle::new(30),
                Handle::new(10),
            ]
        );
    }

    #[test]
    fn straight_polyline_segments_share_each_connected_point_id() {
        let sketch = polyline_sketch(polyline(
            &[(0.0, 0.0, 0.0), (10.0, 0.0, 0.0), (10.0, 10.0, 0.0)],
            false,
        ));
        assert_eq!(sketch.points.len(), 3);
        assert_eq!(sketch.lines.len(), 2);
        assert_eq!(sketch.lines[0].a, PointId(1));
        assert_eq!(sketch.lines[0].b, PointId(2));
        assert_eq!(sketch.lines[1].a, PointId(2));
        assert_eq!(sketch.lines[1].b, PointId(3));
    }

    #[test]
    fn closed_polyline_reuses_first_point_and_ignores_duplicate_closing_vertex() {
        let ordinary = sketch_from_entities(
            vec![EntityType::LwPolyline(polyline(
                &[(0.0, 0.0, 0.0), (10.0, 0.0, 0.0), (10.0, 10.0, 0.0)],
                true,
            ))],
            RasterTransform::default(),
            100,
        )
        .unwrap()
        .unwrap();
        let duplicated = sketch_from_entities(
            vec![EntityType::LwPolyline(polyline(
                &[
                    (0.0, 0.0, 0.0),
                    (10.0, 0.0, 0.0),
                    (10.0, 10.0, 0.0),
                    (0.0, 0.0, 1.0),
                ],
                true,
            ))],
            RasterTransform::default(),
            100,
        )
        .unwrap()
        .unwrap();
        assert_eq!(ordinary.sketch, duplicated.sketch);
        assert_eq!(ordinary.primitive_origins, duplicated.primitive_origins);
        assert_eq!(ordinary.sketch.points.len(), 3);
        assert_eq!(ordinary.sketch.lines.len(), 3);
        assert!(ordinary.sketch.arcs.is_empty());
        for index in 0..3 {
            assert_eq!(
                ordinary.sketch.lines[index].b,
                ordinary.sketch.lines[(index + 1) % 3].a
            );
        }
    }

    #[test]
    fn bulge_conversion_preserves_center_radius_endpoints_and_signed_sweep() {
        let transform = RasterTransform {
            sx: 2.0,
            sy: 2.0,
            theta: 0.3,
            origin: [0.0, 0.0],
        };
        let start: (f64, f64) = (1.0, 2.0);
        let end: (f64, f64) = (5.0, 7.0);
        let chord = (end.0 - start.0).hypot(end.1 - start.1);
        for bulge in [-2.0, -1.0, -0.5, 0.5, 1.0, 2.0] {
            let selection = sketch_from_entities(
                vec![EntityType::LwPolyline(polyline(
                    &[(start.0, start.1, bulge), (end.0, end.1, 0.0)],
                    false,
                ))],
                transform,
                100,
            )
            .unwrap()
            .unwrap();
            assert!(selection.sketch.lines.is_empty());
            assert_eq!(selection.sketch.arcs.len(), 1);
            let arc = selection.sketch.arcs[0];
            let center = selection
                .sketch
                .points
                .iter()
                .find(|point| point.id == arc.center)
                .unwrap();
            let center = raster_to_world(
                transform,
                100,
                rf_core::geometry::Point::new(center.x, center.y),
            );
            let radius = arc.radius * transform.sx;
            let expected_radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
            assert_near(radius, expected_radius);
            assert_near((start.0 - center[0]).hypot(start.1 - center[1]), radius);
            assert_near((end.0 - center[0]).hypot(end.1 - center[1]), radius);

            let world_start = transform.theta - arc.end;
            let world_end = transform.theta - arc.start;
            let directed_start = if bulge > 0.0 { start } else { end };
            let directed_end = if bulge > 0.0 { end } else { start };
            assert_angle_near(
                world_start,
                (directed_start.1 - center[1]).atan2(directed_start.0 - center[0]),
            );
            assert_angle_near(
                world_end,
                (directed_end.1 - center[1]).atan2(directed_end.0 - center[0]),
            );
            assert_near(
                (world_end - world_start).rem_euclid(TAU),
                (4.0 * bulge.atan()).abs(),
            );
        }
    }

    #[test]
    fn rejects_unsupported_lwpolyline_features_clearly() {
        let mut non_xy = polyline(&[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0)], false);
        non_xy.normal = Vector3::new(1.0, 0.0, 1.0);
        assert!(polyline_error(non_xy).contains("positive XY-plane normal"));

        let mut elevated = polyline(&[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0)], false);
        elevated.elevation = 2.0;
        assert!(polyline_error(elevated).contains("elevation"));

        let mut thick = polyline(&[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0)], false);
        thick.thickness = 2.0;
        assert!(polyline_error(thick).contains("thickness"));

        let mut constant_width = polyline(&[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0)], false);
        constant_width.constant_width = 0.5;
        assert!(polyline_error(constant_width).contains("constant width"));

        let mut vertex_width = polyline(&[(0.0, 0.0, 0.0), (1.0, 0.0, 0.0)], false);
        vertex_width.vertices[0].end_width = 0.5;
        assert!(polyline_error(vertex_width).contains("widths"));
    }

    #[test]
    fn rejects_invalid_lwpolyline_values_clearly() {
        for bulge in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let error = polyline_error(polyline(&[(0.0, 0.0, bulge), (1.0, 0.0, 0.0)], false));
            assert!(error.contains("invalid bulge"), "unexpected error: {error}");
        }

        let error = polyline_error(polyline(&[(0.0, 0.0, f64::MAX), (1.0, 0.0, 0.0)], false));
        assert!(error.contains("invalid arc geometry"));

        let error = polyline_error(polyline(&[(0.0, 0.0, 1.0), (0.0, 0.0, 0.0)], false));
        assert!(error.contains("coincident"));

        let error = polyline_error(polyline(&[(0.0, 0.0, 0.0), (0.0, 0.0, 0.0)], true));
        assert!(error.contains("two distinct vertices"));

        let error = sketch_from_entities(
            vec![EntityType::LwPolyline(polyline(
                &[(0.0, 0.0, 1.0), (1.0, 0.0, 0.0)],
                false,
            ))],
            RasterTransform {
                sx: 1.0,
                sy: 2.0,
                theta: 0.0,
                origin: [0.0, 0.0],
            },
            100,
        )
        .unwrap_err();
        assert!(error.contains("anisotropic"));
    }

    #[test]
    fn rejects_unsupported_and_nonplanar_geometry() {
        let error = sketch_from_entities(
            vec![EntityType::Point(Point::from_coords(0.0, 0.0, 0.0))],
            RasterTransform::default(),
            10,
        )
        .unwrap_err();
        assert!(error.contains("only LINE, LWPOLYLINE, CIRCLE and ARC"));

        let error = sketch_from_entities(
            vec![EntityType::Line(Line::from_coords(
                0.0, 0.0, 1.0, 10.0, 0.0, 1.0,
            ))],
            RasterTransform::default(),
            10,
        )
        .unwrap_err();
        assert!(error.contains("XY plane"));
    }

    #[derive(Default)]
    struct TestHost {
        document: Document,
        transactions: Vec<(String, Vec<EntityType>)>,
        records: Vec<(Handle, ExtendedDataRecord)>,
        selection: Vec<Handle>,
        begin_undo_groups: usize,
        end_undo_groups: usize,
        sequential_updates: usize,
        geometry_bumps: usize,
        dirty: bool,
    }

    impl HostApi for TestHost {
        fn tab_index(&self) -> usize {
            0
        }

        fn document(&self) -> &Document {
            &self.document
        }

        fn document_mut(&mut self) -> &mut Document {
            &mut self.document
        }

        fn document_reader(&self) -> Box<dyn DocumentReader + '_> {
            Box::new(CadDocumentReader(&self.document))
        }

        fn add_entity(&mut self, _entity: EntityType) -> Handle {
            Handle::default()
        }

        fn update_entity(&mut self, _entity: EntityType) -> bool {
            self.sequential_updates += 1;
            false
        }

        fn update_entities_transaction(
            &mut self,
            label: &str,
            entities: Vec<EntityType>,
        ) -> Result<(), String> {
            self.transactions.push((label.to_string(), entities));
            Ok(())
        }

        fn bump_geometry(&mut self) {
            self.geometry_bumps += 1;
        }

        fn read_record(&self, _handle: Handle, _app_name: &str) -> Option<&ExtendedDataRecord> {
            None
        }

        fn write_record(&mut self, handle: Handle, record: ExtendedDataRecord) -> bool {
            self.records.push((handle, record));
            true
        }

        fn remove_record(&mut self, _handle: Handle, _app_name: &str) -> bool {
            false
        }

        fn push_undo(&mut self, _label: &str) {}

        fn begin_undo_group(&mut self, _label: &str) {
            self.begin_undo_groups += 1;
        }

        fn end_undo_group(&mut self) {
            self.end_undo_groups += 1;
        }

        fn set_dirty(&mut self) {
            self.dirty = true;
        }

        fn set_selection(&mut self, handles: &[Handle]) -> Result<(), String> {
            self.selection = handles.to_vec();
            Ok(())
        }

        fn push_info(&mut self, _msg: &str) {}
        fn push_output(&mut self, _msg: &str) {}
        fn push_error(&mut self, _msg: &str) {}
        fn start_interactive(&mut self, _command: Box<dyn InteractiveCommand>) {}

        fn plugin_state_any(&self, _plugin_id: &str) -> Option<&(dyn Any + Send + Sync)> {
            None
        }

        fn plugin_state_any_mut(
            &mut self,
            _plugin_id: &str,
        ) -> Option<&mut (dyn Any + Send + Sync)> {
            None
        }

        fn ensure_plugin_state_any(
            &mut self,
            _plugin_id: &'static str,
            _init: &mut dyn FnMut() -> Box<dyn Any + Send + Sync>,
        ) -> Option<&mut (dyn Any + Send + Sync)> {
            None
        }
    }

    #[test]
    fn update_uses_one_transaction_and_preserves_common_and_fit_xdata() {
        let mut original = Line::from_coords(1.0, 2.0, 0.0, 3.0, 4.0, 0.0);
        original.common.layer = "MECHANICAL".to_string();
        original.common.handle = Handle::new(42);
        original.common.owner_handle = Handle::new(99);
        let expected_common = original.common.clone();
        let original = EntityType::Line(original);
        let output = CadEntity {
            entity: EntityType::Line(Line::from_coords(5.0, 6.0, 0.0, 7.0, 8.0, 0.0)),
            kind: "LINE",
            id: 7,
        };
        let fit = FitterResult {
            params: Vec::new(),
            transform: [0.0; 3],
            scale: 1.0,
            rms_px: 0.25,
            max_constraint: 0.0,
            iterations: 1,
            converged: true,
            gauge_warning: false,
            residuals: Vec::new(),
            total_samples: 0,
            matched_samples: 0,
            inlier_ratio: 1.0,
            uncovered_ratio: 0.1,
            mask_too_narrow: false,
            transform_stddev: [0.0; 3],
            primitive_stats: Vec::new(),
        };
        let mut host = TestHost::default();
        assert_eq!(
            update_fit_entities(
                &mut host,
                vec![original],
                vec![output],
                &[Handle::new(42)],
                &fit,
                RasterTransform::default(),
                100,
                false,
            ),
            Ok(1)
        );

        assert_eq!(host.sequential_updates, 0);
        assert_eq!(host.begin_undo_groups, 1);
        assert_eq!(host.end_undo_groups, 1);
        assert_eq!(host.geometry_bumps, 1);
        assert!(host.dirty);
        assert_eq!(host.selection, vec![Handle::new(42)]);
        assert_eq!(host.transactions.len(), 1);
        assert_eq!(host.transactions[0].0, "RasterFit fit");
        let EntityType::Line(updated) = &host.transactions[0].1[0] else {
            panic!("expected line");
        };
        assert_eq!(updated.common, expected_common);
        assert_eq!(updated.start, Vector3::new(1.0, 2.0, 0.0));

        assert_eq!(host.records.len(), 1);
        assert_eq!(host.records[0].0, Handle::new(42));
        let record = &host.records[0].1;
        assert_eq!(record.application_name, "RASTERFIT");
        assert_eq!(
            record.values,
            vec![
                XDataValue::String("RASTERFIT-CLASS:FIT".to_string()),
                XDataValue::String("LINE".to_string()),
                XDataValue::String("7".to_string()),
                XDataValue::Real(0.25),
                XDataValue::Real(0.9),
                XDataValue::Integer16(1),
            ]
        );
    }

    #[test]
    fn constrained_polyline_is_rebuilt_from_mixed_line_and_arc_outputs() {
        let mut original = polyline(
            &[(0.0, 0.0, 0.0), (10.0, 0.0, 1.0), (10.0, 10.0, 0.0)],
            true,
        );
        original.common.layer = "MECHANICAL".to_string();
        let outputs = vec![
            CadEntity {
                entity: EntityType::Line(Line::from_coords(0.0, 0.0, 0.0, 10.0, 0.0, 0.0)),
                kind: "LINE",
                id: 1,
            },
            CadEntity {
                entity: EntityType::Line(Line::from_coords(15.0, 5.0, 0.0, 0.0, 0.0, 0.0)),
                kind: "LINE",
                id: 3,
            },
            CadEntity {
                entity: EntityType::Arc(Arc::from_center_radius_angles(
                    Vector3::new(10.0, 5.0, 0.0),
                    5.0,
                    -PI / 2.0,
                    0.0,
                )),
                kind: "ARC",
                id: 2,
            },
        ];
        let fit = FitterResult {
            params: Vec::new(),
            transform: [0.0; 3],
            scale: 1.0,
            rms_px: 0.0,
            max_constraint: 0.0,
            iterations: 1,
            converged: true,
            gauge_warning: false,
            residuals: Vec::new(),
            total_samples: 0,
            matched_samples: 0,
            inlier_ratio: 1.0,
            uncovered_ratio: 0.0,
            mask_too_narrow: false,
            transform_stddev: [0.0; 3],
            primitive_stats: Vec::new(),
        };
        let mut host = TestHost::default();
        assert_eq!(
            update_fit_entities(
                &mut host,
                vec![EntityType::LwPolyline(original)],
                outputs,
                &[Handle::new(10); 3],
                &fit,
                RasterTransform::default(),
                100,
                true,
            ),
            Ok(1)
        );
        let EntityType::LwPolyline(updated) = &host.transactions[0].1[0] else {
            panic!("polyline must remain LWPOLYLINE");
        };
        assert_eq!(updated.common.handle, Handle::new(10));
        assert_eq!(updated.common.layer, "MECHANICAL");
        assert!(updated.is_closed);
        assert_eq!(updated.vertices.len(), 3);
        assert_eq!(updated.vertices[0].bulge, 0.0);
        assert!((updated.vertices[1].bulge - (PI / 8.0).tan()).abs() < 1e-9);
        assert_eq!(updated.vertices[2].bulge, 0.0);
        assert_eq!(updated.vertices[2].location.x, 15.0);
        assert_eq!(updated.vertices[2].location.y, 5.0);
    }

    #[test]
    fn constrained_single_segment_polyline_does_not_change_entity_kind() {
        let original = polyline(&[(0.0, 0.0, 0.0), (10.0, 0.0, 0.0)], false);
        let output = CadEntity {
            entity: EntityType::Line(Line::from_coords(0.0, 0.0, 0.0, 10.0, 0.0, 0.0)),
            kind: "LINE",
            id: 1,
        };
        let mut used = [false];
        let entity = fit_polyline(
            &original,
            std::slice::from_ref(&output),
            &[Handle::new(10)],
            &mut used,
        )
        .unwrap();
        assert!(matches!(entity, EntityType::LwPolyline(_)));
        assert!(used[0]);
    }
}
