use std::path::Path;

use ocs_plugin_api::host::acadrust::entities::{Arc, Circle, Line, RasterImage};
use ocs_plugin_api::host::acadrust::types::Vector3;
use ocs_plugin_api::host::{EntityType, ExtendedDataRecord, Handle, HostApi, XDataValue};
use rf_core::geometry::Point;
use rf_core::sketch::{PointId, Sketch};

use crate::raster::{image_vectors, is_isotropic, raster_to_world, RasterData, RasterTransform};

pub(crate) const RASTER_LAYER: &str = "RASTERFIT-RASTER";
pub(crate) const FIT_LAYER: &str = "RASTERFIT-FIT";
const XDATA_APP: &str = "RASTERFIT";

#[derive(Debug)]
pub(crate) struct CadEntity {
    pub entity: EntityType,
    pub kind: &'static str,
    pub id: u32,
}

pub(crate) fn raster_entity(
    path: &Path,
    raster: &RasterData,
    transform: RasterTransform,
) -> EntityType {
    let mut image = RasterImage::new(
        &path.to_string_lossy(),
        vector([transform.origin[0], transform.origin[1], 0.0]),
        raster.width as f64,
        raster.height as f64,
    );
    let (u, v) = image_vectors(transform);
    image.u_vector = vector(u);
    image.v_vector = vector(v);
    image.common.layer = RASTER_LAYER.to_string();
    EntityType::RasterImage(image)
}

pub(crate) fn add_raster(
    host: &mut dyn HostApi,
    path: &Path,
    raster: &RasterData,
    transform: RasterTransform,
) -> Result<Handle, String> {
    let handle = host.add_entity(raster_entity(path, raster, transform));
    let mut record = base_record("RASTER");
    record.add_value(XDataValue::String(path.to_string_lossy().into_owned()));
    record.add_value(XDataValue::String(format!(
        "{}x{}",
        raster.width, raster.height
    )));
    if !host.write_record(handle, record) {
        return Err("OpenCADStudio could not attach RasterFit raster metadata".to_string());
    }
    host.bump_geometry();
    host.set_dirty();
    Ok(handle)
}

pub(crate) fn update_raster_transform(
    host: &mut dyn HostApi,
    handle: Handle,
    transform: RasterTransform,
) -> Result<(), String> {
    let entities = host.entities_batch(&[handle])?;
    let Some(EntityType::RasterImage(mut image)) = entities.into_iter().next().flatten() else {
        return Err("the RasterFit raster entity no longer exists".to_string());
    };
    let (u, v) = image_vectors(transform);
    image.insertion_point = vector([transform.origin[0], transform.origin[1], 0.0]);
    image.u_vector = vector(u);
    image.v_vector = vector(v);
    if !host.update_entity(EntityType::RasterImage(image)) {
        return Err("OpenCADStudio could not update the RasterFit raster".to_string());
    }
    host.bump_geometry();
    host.set_dirty();
    Ok(())
}

pub(crate) fn sketch_to_entities(
    sketch: &Sketch,
    transform: RasterTransform,
    raster_height: usize,
) -> Result<Vec<CadEntity>, String> {
    if !is_isotropic(transform) && (!sketch.circles.is_empty() || !sketch.arcs.is_empty()) {
        return Err(
            "anisotropic calibration cannot represent circles or arcs exactly; use isotropic calibration or lines only"
                .to_string(),
        );
    }

    let mut entities = Vec::new();
    for line in &sketch.lines {
        if line.construction {
            continue;
        }
        let a = world_point(sketch, line.a, transform, raster_height)?;
        let b = world_point(sketch, line.b, transform, raster_height)?;
        if a == b {
            return Err(format!("line {} has coincident endpoints", line.id.0));
        }
        let mut entity = Line::from_points(vector(a), vector(b));
        entity.common.layer = FIT_LAYER.to_string();
        entities.push(CadEntity {
            entity: EntityType::Line(entity),
            kind: "LINE",
            id: line.id.0,
        });
    }

    for circle in &sketch.circles {
        if circle.construction {
            continue;
        }
        validate_radius(circle.id.0, circle.radius, "circle")?;
        let center = world_point(sketch, circle.center, transform, raster_height)?;
        let mut entity = Circle::from_center_radius(vector(center), circle.radius * transform.sx);
        entity.common.layer = FIT_LAYER.to_string();
        entities.push(CadEntity {
            entity: EntityType::Circle(entity),
            kind: "CIRCLE",
            id: circle.id.0,
        });
    }

    for arc in &sketch.arcs {
        if arc.construction {
            continue;
        }
        validate_radius(arc.id.0, arc.radius, "arc")?;
        let center = world_point(sketch, arc.center, transform, raster_height)?;
        let mut entity = Arc::from_center_radius_angles(
            vector(center),
            arc.radius * transform.sx,
            transform.theta - arc.end,
            transform.theta - arc.start,
        );
        entity.common.layer = FIT_LAYER.to_string();
        entities.push(CadEntity {
            entity: EntityType::Arc(entity),
            kind: "ARC",
            id: arc.id.0,
        });
    }
    Ok(entities)
}

fn base_record(class: &str) -> ExtendedDataRecord {
    let mut record = ExtendedDataRecord::new(XDATA_APP);
    record.add_value(XDataValue::String(format!("RASTERFIT-CLASS:{class}")));
    record
}

fn world_point(
    sketch: &Sketch,
    id: PointId,
    transform: RasterTransform,
    raster_height: usize,
) -> Result<[f64; 3], String> {
    let point = sketch
        .points
        .iter()
        .find(|point| point.id == id)
        .ok_or_else(|| format!("sketch references missing point {}", id.0))?;
    let world = raster_to_world(transform, raster_height, Point::new(point.x, point.y));
    if world.iter().all(|coordinate| coordinate.is_finite()) {
        Ok(world)
    } else {
        Err(format!("point {} has non-finite coordinates", id.0))
    }
}

fn validate_radius(id: u32, radius: f64, kind: &str) -> Result<(), String> {
    if radius.is_finite() && radius > 0.0 {
        Ok(())
    } else {
        Err(format!("{kind} {id} has an invalid radius"))
    }
}

fn vector(value: [f64; 3]) -> Vector3 {
    Vector3::new(value[0], value[1], value[2])
}

#[cfg(test)]
mod tests {
    use rf_core::sketch::{
        ArcId, CircleId, LineId, PointId, SketchArc, SketchCircle, SketchLine, SketchPoint,
    };

    use super::*;

    fn point(id: u32, x: f64, y: f64) -> SketchPoint {
        SketchPoint {
            id: PointId(id),
            x,
            y,
        }
    }

    #[test]
    fn converts_visible_geometry_and_skips_construction() {
        let sketch = Sketch {
            points: vec![
                point(1, 0.0, 100.0),
                point(2, 50.0, 100.0),
                point(3, 20.0, 50.0),
            ],
            lines: vec![
                SketchLine {
                    id: LineId(10),
                    a: PointId(1),
                    b: PointId(2),
                    construction: false,
                },
                SketchLine {
                    id: LineId(11),
                    a: PointId(2),
                    b: PointId(3),
                    construction: true,
                },
            ],
            circles: vec![SketchCircle {
                id: CircleId(12),
                center: PointId(3),
                radius: 5.0,
                construction: false,
            }],
            arcs: vec![SketchArc {
                id: ArcId(13),
                center: PointId(3),
                radius: 10.0,
                start: 0.0,
                end: std::f64::consts::FRAC_PI_2,
                construction: false,
            }],
            ..Sketch::default()
        };
        let transform = RasterTransform {
            sx: 2.0,
            sy: 2.0,
            theta: 0.0,
            origin: [0.0, 0.0],
        };
        let entities = sketch_to_entities(&sketch, transform, 100).unwrap();
        assert_eq!(entities.len(), 3);
        let EntityType::Line(line) = &entities[0].entity else {
            panic!("expected line");
        };
        assert_eq!(line.start, Vector3::new(0.0, 0.0, 0.0));
        assert_eq!(line.end, Vector3::new(100.0, 0.0, 0.0));
        assert_eq!(line.common.layer, FIT_LAYER);
        let EntityType::Arc(arc) = &entities[2].entity else {
            panic!("expected arc");
        };
        assert_eq!(arc.start_angle, -std::f64::consts::FRAC_PI_2);
        assert_eq!(arc.end_angle, 0.0);
        assert_eq!(arc.radius, 20.0);
    }

    #[test]
    fn rejects_inexact_curves_under_anisotropic_scale() {
        let sketch = Sketch {
            points: vec![point(1, 0.0, 0.0)],
            circles: vec![SketchCircle {
                id: CircleId(1),
                center: PointId(1),
                radius: 2.0,
                construction: false,
            }],
            ..Sketch::default()
        };
        let error = sketch_to_entities(
            &sketch,
            RasterTransform {
                sx: 1.0,
                sy: 2.0,
                theta: 0.0,
                origin: [0.0, 0.0],
            },
            10,
        )
        .unwrap_err();
        assert!(error.contains("anisotropic"));
    }

    #[test]
    fn raster_uses_calibrated_pixel_vectors() {
        let raster = RasterData {
            gray: vec![0; 4],
            width: 2,
            height: 2,
        };
        let transform = RasterTransform {
            sx: 2.0,
            sy: 2.0,
            theta: std::f64::consts::FRAC_PI_2,
            origin: [0.0, 0.0],
        };
        let EntityType::RasterImage(image) = raster_entity(Path::new("a.png"), &raster, transform)
        else {
            panic!("expected raster");
        };
        assert!(image.u_vector.x.abs() < 1e-12);
        assert!((image.u_vector.y - 2.0).abs() < 1e-12);
        assert!((image.v_vector.x + 2.0).abs() < 1e-12);
        assert!(image.v_vector.y.abs() < 1e-12);
    }
}
