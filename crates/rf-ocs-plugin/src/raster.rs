use rf_core::geometry::Point;

#[derive(Debug, Clone)]
pub(crate) struct RasterData {
    pub gray: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct RasterTransform {
    pub sx: f64,
    pub sy: f64,
    pub theta: f64,
    #[serde(default)]
    pub origin: [f64; 2],
}

impl Default for RasterTransform {
    fn default() -> Self {
        Self {
            sx: 1.0,
            sy: 1.0,
            theta: 0.0,
            origin: [0.0, 0.0],
        }
    }
}

pub(crate) fn decode_raster(bytes: &[u8]) -> Result<RasterData, String> {
    let image = image::load_from_memory(bytes)
        .map_err(|error| format!("failed to decode raster image: {error}"))?;
    let gray = image.to_luma8();
    let width = usize::try_from(gray.width())
        .map_err(|_| "decoded raster width does not fit in usize".to_string())?;
    let height = usize::try_from(gray.height())
        .map_err(|_| "decoded raster height does not fit in usize".to_string())?;

    if width == 0 || height == 0 {
        return Err("decoded raster image must have nonzero dimensions".to_string());
    }

    let expected_len = width
        .checked_mul(height)
        .ok_or_else(|| "decoded raster dimensions overflow usize".to_string())?;
    let gray = gray.into_raw();
    if gray.len() != expected_len {
        return Err(format!(
            "decoded raster buffer has {} bytes, expected {expected_len}",
            gray.len()
        ));
    }

    Ok(RasterData {
        gray,
        width,
        height,
    })
}

pub(crate) fn image_vectors(transform: RasterTransform) -> ([f64; 3], [f64; 3]) {
    let (sin, cos) = transform.theta.sin_cos();
    (
        [transform.sx * cos, transform.sx * sin, 0.0],
        [-transform.sy * sin, transform.sy * cos, 0.0],
    )
}

pub(crate) fn raster_to_world(transform: RasterTransform, height: usize, point: Point) -> [f64; 3] {
    let (u_vector, v_vector) = image_vectors(transform);
    let vertical = height as f64 - point.y;
    [
        transform.origin[0] + u_vector[0] * point.x + v_vector[0] * vertical,
        transform.origin[1] + u_vector[1] * point.x + v_vector[1] * vertical,
        0.0,
    ]
}

pub(crate) fn world_to_raster(
    transform: RasterTransform,
    height: usize,
    world: [f64; 3],
) -> Option<Point> {
    if !transform.sx.is_finite()
        || !transform.sy.is_finite()
        || !transform.theta.is_finite()
        || transform
            .origin
            .iter()
            .any(|coordinate| !coordinate.is_finite())
        || world.iter().any(|coordinate| !coordinate.is_finite())
    {
        return None;
    }

    let (u_vector, v_vector) = image_vectors(transform);
    let (a, b) = (u_vector[0], v_vector[0]);
    let (c, d) = (u_vector[1], v_vector[1]);
    let determinant = a * d - b * c;
    let norm = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
    if !determinant.is_finite()
        || !norm.is_finite()
        || norm == 0.0
        || determinant.abs() <= f64::EPSILON * norm * norm
    {
        return None;
    }

    let x = world[0] - transform.origin[0];
    let y = world[1] - transform.origin[1];
    let u = (d * x - b * y) / determinant;
    let vertical = (-c * x + a * y) / determinant;
    let point = Point::new(u, height as f64 - vertical);
    if point.x.is_finite() && point.y.is_finite() {
        Some(point)
    } else {
        None
    }
}

pub(crate) fn is_isotropic(transform: RasterTransform) -> bool {
    if !transform.sx.is_finite() || !transform.sy.is_finite() {
        return false;
    }

    let sx = transform.sx.abs();
    let sy = transform.sy.abs();
    let scale = sx.max(sy);
    (sx - sy).abs() <= 1e-9 * scale
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_point_near(actual: Point, expected: Point) {
        assert!(
            (actual.x - expected.x).abs() < 1e-9,
            "x differs: actual {}, expected {}",
            actual.x,
            expected.x
        );
        assert!(
            (actual.y - expected.y).abs() < 1e-9,
            "y differs: actual {}, expected {}",
            actual.y,
            expected.y
        );
    }

    #[test]
    fn default_transform_uses_bottom_left_insertion() {
        let transform = RasterTransform::default();
        assert_eq!(transform.sx, 1.0);
        assert_eq!(transform.sy, 1.0);
        assert_eq!(transform.theta, 0.0);

        let (u, v) = image_vectors(transform);
        assert_eq!(u, [1.0, 0.0, 0.0]);
        assert_eq!(v, [0.0, 1.0, 0.0]);
        assert_eq!(
            raster_to_world(transform, 4, Point::new(0.0, 4.0)),
            [0.0, 0.0, 0.0]
        );
        assert_eq!(
            raster_to_world(transform, 4, Point::new(0.0, 0.0)),
            [0.0, 4.0, 0.0]
        );
        assert!(is_isotropic(transform));
    }

    #[test]
    fn scaled_rotated_translated_transform_roundtrips_multiple_points() {
        let transform = RasterTransform {
            sx: 2.5,
            sy: 0.75,
            theta: 0.63,
            origin: [20.0, -11.0],
        };
        let points = [
            Point::new(0.0, 0.0),
            Point::new(0.0, 17.0),
            Point::new(12.0, 17.0),
            Point::new(3.25, 8.5),
            Point::new(-2.0, 21.0),
        ];

        for point in points {
            let world = raster_to_world(transform, 17, point);
            let raster = world_to_raster(transform, 17, world).unwrap();
            assert_point_near(raster, point);
        }
        assert_eq!(
            raster_to_world(transform, 17, Point::new(0.0, 17.0)),
            [20.0, -11.0, 0.0]
        );
    }

    #[test]
    fn degenerate_or_nonfinite_transform_has_no_inverse() {
        for transform in [
            RasterTransform {
                sx: 0.0,
                sy: 1.0,
                theta: 0.0,
                origin: [0.0, 0.0],
            },
            RasterTransform {
                sx: 1.0,
                sy: 0.0,
                theta: 0.0,
                origin: [0.0, 0.0],
            },
            RasterTransform {
                sx: f64::NAN,
                sy: 1.0,
                theta: 0.0,
                origin: [0.0, 0.0],
            },
            RasterTransform {
                sx: 1.0,
                sy: 1.0,
                theta: f64::INFINITY,
                origin: [0.0, 0.0],
            },
            RasterTransform {
                sx: 1.0,
                sy: 1.0,
                theta: 0.0,
                origin: [f64::NAN, 0.0],
            },
        ] {
            assert_eq!(world_to_raster(transform, 10, [1.0, 2.0, 0.0]), None);
        }

        assert_eq!(
            world_to_raster(RasterTransform::default(), 10, [f64::NAN, 2.0, 0.0]),
            None
        );
    }

    #[test]
    fn isotropy_uses_scale_magnitudes_with_relative_tolerance() {
        assert!(is_isotropic(RasterTransform {
            sx: -2.0,
            sy: 2.0 * (1.0 + 1e-10),
            theta: 0.3,
            origin: [0.0, 0.0],
        }));
        assert!(!is_isotropic(RasterTransform {
            sx: 2.0,
            sy: 2.1,
            theta: 0.0,
            origin: [0.0, 0.0],
        }));
        assert!(!is_isotropic(RasterTransform {
            sx: f64::NAN,
            sy: 1.0,
            theta: 0.0,
            origin: [0.0, 0.0],
        }));
    }

    #[test]
    fn decode_rejects_invalid_bytes() {
        let error = decode_raster(b"not an image").unwrap_err();
        assert!(error.contains("failed to decode raster image"));
    }

    #[test]
    fn decode_converts_pixels_to_luma8() {
        let image = image::GrayImage::from_raw(2, 2, vec![0, 64, 128, 255]).unwrap();
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();

        let raster = decode_raster(bytes.get_ref()).unwrap();
        assert_eq!(raster.width, 2);
        assert_eq!(raster.height, 2);
        assert_eq!(raster.gray, vec![0, 64, 128, 255]);
    }
}
