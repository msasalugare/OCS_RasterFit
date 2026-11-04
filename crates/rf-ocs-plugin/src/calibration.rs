use rf_core::calibration::{
    isotropic_warnings, leave_one_out_isotropic, segment_residuals, solve_anisotropic,
    solve_isotropic, solve_rotation, CalibrationSegment,
};

use crate::raster::RasterTransform;

pub(crate) fn solve_calibration(
    segments: &[CalibrationSegment],
    previous: RasterTransform,
) -> Result<RasterTransform, String> {
    let scale = solve_isotropic(segments)
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .ok_or_else(|| {
            "calibration needs at least one non-degenerate segment with a positive length"
                .to_string()
        })?;
    let theta = solve_rotation(segments).unwrap_or(previous.theta);
    if !theta.is_finite() {
        return Err("calibration produced a non-finite rotation".to_string());
    }
    let has_horizontal = segments
        .iter()
        .any(|segment| segment.orientation == rf_core::calibration::SegmentOrientation::Horizontal);
    let has_vertical = segments
        .iter()
        .any(|segment| segment.orientation == rf_core::calibration::SegmentOrientation::Vertical);
    if has_horizontal && has_vertical {
        if let Some((sx, sy)) = solve_anisotropic(segments, theta) {
            return Ok(RasterTransform {
                sx,
                sy,
                theta,
                origin: previous.origin,
            });
        }
    }
    Ok(RasterTransform {
        sx: scale,
        sy: scale,
        theta,
        origin: previous.origin,
    })
}

pub(crate) fn level_transform(
    mut transform: RasterTransform,
    a: rf_core::geometry::Point,
    b: rf_core::geometry::Point,
) -> Result<RasterTransform, String> {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    if dx.hypot(dy) < 1.0 {
        return Err("leveling needs two distinct points at least one pixel apart".to_string());
    }
    if !transform.sx.is_finite()
        || !transform.sy.is_finite()
        || transform.sx <= 0.0
        || transform.sy <= 0.0
    {
        return Err("leveling needs a valid positive raster scale".to_string());
    }

    let alpha = dy.atan2(dx);
    let nearest_axis = (alpha / std::f64::consts::FRAC_PI_2).round() as i32;
    transform.theta = if nearest_axis.rem_euclid(2) == 1 {
        (-transform.sx * dx).atan2(transform.sy * dy)
    } else {
        (transform.sy * dy).atan2(transform.sx * dx)
    };
    Ok(transform)
}

pub(crate) fn diagnostics(
    segments: &[CalibrationSegment],
    transform: RasterTransform,
    width: usize,
    height: usize,
) -> String {
    if segments.is_empty() {
        return "calibration diagnostics: no segments".to_string();
    }
    let residuals = segment_residuals(segments, transform.theta, transform.sx, transform.sy);
    let max_pct = residuals
        .iter()
        .map(|residual| residual.residual_pct.abs())
        .fold(0.0, f64::max);
    let warnings = isotropic_warnings(segments, &residuals, (width as f64).hypot(height as f64));
    let loo = leave_one_out_isotropic(segments);
    let (min, max) = loo
        .iter()
        .filter(|value| value.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
            (min.min(*value), max.max(*value))
        });
    let loo = if min.is_finite() {
        format!("LOO {min:.6}..{max:.6} mm/px")
    } else {
        "LOO unavailable".to_string()
    };
    format!(
        "calibration diagnostics: max residual {max_pct:.3}%; {loo}; warnings: {}",
        if warnings.is_empty() {
            "none".to_string()
        } else {
            warnings
                .iter()
                .map(|warning| format!("{warning:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    )
}

#[cfg(test)]
mod tests {
    use rf_core::calibration::SegmentOrientation;
    use rf_core::geometry::Point;

    use super::*;

    fn segment(length: f64, orientation: SegmentOrientation) -> CalibrationSegment {
        CalibrationSegment {
            a: Point::new(0.0, 0.0),
            b: Point::new(100.0, 0.0),
            length,
            orientation,
        }
    }

    #[test]
    fn solves_scale_and_horizontal_rotation() {
        let transform = solve_calibration(
            &[segment(50.0, SegmentOrientation::Horizontal)],
            RasterTransform::default(),
        )
        .unwrap();
        assert_eq!(transform.sx, 0.5);
        assert_eq!(transform.sy, 0.5);
        assert_eq!(transform.theta, 0.0);
    }

    #[test]
    fn solves_anisotropic_scale_from_horizontal_and_vertical_segments() {
        let vertical = CalibrationSegment {
            a: Point::new(0.0, 0.0),
            b: Point::new(0.0, 200.0),
            length: 50.0,
            orientation: SegmentOrientation::Vertical,
        };
        let transform = solve_calibration(
            &[segment(50.0, SegmentOrientation::Horizontal), vertical],
            RasterTransform::default(),
        )
        .unwrap();
        assert!((transform.sx - 0.5).abs() < 1e-9);
        assert!((transform.sy - 0.25).abs() < 1e-9);
    }

    #[test]
    fn diagnostics_include_residuals_loo_and_warnings() {
        let transform = RasterTransform {
            sx: 0.5,
            sy: 0.5,
            theta: 0.0,
            origin: [0.0, 0.0],
        };
        let text = diagnostics(
            &[segment(50.0, SegmentOrientation::Free)],
            transform,
            100,
            100,
        );
        assert!(text.contains("max residual"));
        assert!(text.contains("LOO unavailable"));
        assert!(text.contains("TooFewSegments"));
    }

    #[test]
    fn free_segment_preserves_previous_rotation_and_origin() {
        let previous = RasterTransform {
            theta: 0.3,
            origin: [12.0, -8.0],
            ..RasterTransform::default()
        };
        let transform =
            solve_calibration(&[segment(25.0, SegmentOrientation::Free)], previous).unwrap();
        assert_eq!(transform.theta, 0.3);
        assert_eq!(transform.origin, [12.0, -8.0]);
    }

    #[test]
    fn level_aligns_with_nearest_horizontal_or_vertical_axis() {
        let initial = RasterTransform {
            sx: 2.0,
            sy: 2.0,
            theta: 0.4,
            origin: [0.0, 0.0],
        };
        let horizontal =
            level_transform(initial, Point::new(10.0, 20.0), Point::new(110.0, 30.0)).unwrap();
        assert!((horizontal.theta - 0.1f64.atan()).abs() < 1e-12);
        assert_eq!(horizontal.sx, 2.0);
        assert_eq!(horizontal.sy, 2.0);
        let a = crate::raster::raster_to_world(horizontal, 100, Point::new(10.0, 20.0));
        let b = crate::raster::raster_to_world(horizontal, 100, Point::new(110.0, 30.0));
        assert!((a[1] - b[1]).abs() < 1e-9);

        let vertical =
            level_transform(initial, Point::new(10.0, 20.0), Point::new(20.0, 120.0)).unwrap();
        assert!((vertical.theta + 0.1f64.atan()).abs() < 1e-12);
        let a = crate::raster::raster_to_world(vertical, 140, Point::new(10.0, 20.0));
        let b = crate::raster::raster_to_world(vertical, 140, Point::new(20.0, 120.0));
        assert!((a[0] - b[0]).abs() < 1e-9);
    }

    #[test]
    fn level_is_exact_with_anisotropic_scale() {
        let initial = RasterTransform {
            sx: 2.0,
            sy: 0.5,
            theta: 0.0,
            origin: [0.0, 0.0],
        };
        let horizontal =
            level_transform(initial, Point::new(0.0, 0.0), Point::new(100.0, 20.0)).unwrap();
        let a = crate::raster::raster_to_world(horizontal, 100, Point::new(0.0, 0.0));
        let b = crate::raster::raster_to_world(horizontal, 100, Point::new(100.0, 20.0));
        assert!((a[1] - b[1]).abs() < 1e-9);

        let vertical =
            level_transform(initial, Point::new(0.0, 0.0), Point::new(20.0, 100.0)).unwrap();
        let a = crate::raster::raster_to_world(vertical, 100, Point::new(0.0, 0.0));
        let b = crate::raster::raster_to_world(vertical, 100, Point::new(20.0, 100.0));
        assert!((a[0] - b[0]).abs() < 1e-9);
    }

    #[test]
    fn level_rejects_short_or_invalid_input() {
        let transform = RasterTransform::default();
        assert!(level_transform(transform, Point::new(1.0, 1.0), Point::new(1.5, 1.0)).is_err());
        assert!(level_transform(
            RasterTransform {
                sx: 0.0,
                sy: 1.0,
                theta: 0.0,
                origin: [0.0, 0.0],
            },
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
        )
        .is_err());
    }

    #[test]
    fn rejects_degenerate_segments() {
        let mut segment = segment(10.0, SegmentOrientation::Free);
        segment.b = segment.a;
        assert!(solve_calibration(&[segment], RasterTransform::default()).is_err());
    }
}
