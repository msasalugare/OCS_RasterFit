use rf_core::fitter::{fit, fit_masked, FitterParams, FitterResult};
use rf_core::mask::{MaskParams, MaskStroke};
use rf_core::ridge::{detect_ridges, RidgeParams};
use rf_core::sketch::{Constraint, Sketch};

use crate::raster::RasterData;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct FitOptions {
    pub fit_geometry: bool,
    pub fit_rotation: bool,
    pub fit_translation: bool,
    pub fit_scale: bool,
    pub capture_radius: f64,
    pub robust_scale: f64,
    pub tangent_weight: f64,
    pub include_construction: bool,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            fit_geometry: false,
            fit_rotation: true,
            fit_translation: true,
            fit_scale: false,
            capture_radius: 12.0,
            robust_scale: 1.5,
            tangent_weight: 0.1,
            include_construction: false,
        }
    }
}

impl FitOptions {
    fn params(self) -> FitterParams {
        FitterParams {
            max_iterations: 40,
            capture_radius: self.capture_radius,
            robust_scale: self.robust_scale,
            tangent_weight: self.tangent_weight,
            fit_geometry: self.fit_geometry,
            fit_rotation: self.fit_rotation,
            fit_translation: self.fit_translation,
            fit_scale: self.fit_scale,
            include_construction: self.include_construction,
            ..FitterParams::default()
        }
    }
}

pub(crate) struct FitOutput {
    pub sketch: Sketch,
    pub result: FitterResult,
}

pub(crate) fn fit_native_sketch(
    sketch: &Sketch,
    raster: &RasterData,
    mask: Option<(&[MaskStroke], MaskParams)>,
    options: FitOptions,
) -> Result<FitOutput, String> {
    if sketch.points.is_empty() {
        return Err("select native CAD geometry before fitting".to_string());
    }
    let ridges = detect_ridges(
        &raster.gray,
        raster.width,
        raster.height,
        &RidgeParams::default(),
    );
    if ridges.is_empty() {
        return Err("no raster centerlines were detected; check the image contrast".to_string());
    }

    let params = options.params();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Some((strokes, mask_params)) = mask {
            fit_masked(
                sketch,
                &ridges,
                strokes,
                &mask_params,
                raster.width,
                raster.height,
                &[0.0, 0.0, 0.0],
                &params,
            )
        } else {
            fit(sketch, &ridges, &[0.0, 0.0, 0.0], &params)
        }
    }))
    .map_err(|_| {
        "selected geometry or mask is invalid and fitting could not continue".to_string()
    })?;
    let fitted = apply_native_fit_result(sketch, &result)?;
    Ok(FitOutput {
        sketch: fitted,
        result,
    })
}

fn apply_native_fit_result(sketch: &Sketch, result: &FitterResult) -> Result<Sketch, String> {
    if result.params.len() != sketch.parameter_count() {
        return Err(format!(
            "fitter returned {} parameters, expected {}",
            result.params.len(),
            sketch.parameter_count()
        ));
    }
    if result.params.iter().any(|value| !value.is_finite())
        || result.transform.iter().any(|value| !value.is_finite())
    {
        return Err("fitter returned non-finite transform parameters".to_string());
    }

    if !result.scale.is_finite() || result.scale <= 0.0 {
        return Err("fitter returned an invalid background scale".to_string());
    }
    let [theta, tx, ty] = result.transform;
    let (sin, cos) = theta.sin_cos();
    let scale = result.scale;
    let mut mapped = result.params.clone();
    for (index, point) in sketch.points.iter().enumerate() {
        let fixed = sketch
            .constraints
            .iter()
            .find_map(|constraint| match constraint {
                Constraint::FixedPoint(id, x, y) if *id == point.id => Some((*x, *y)),
                _ => None,
            });
        if let Some((x, y)) = fixed {
            mapped[2 * index] = x;
            mapped[2 * index + 1] = y;
        } else {
            let x = (result.params[2 * index] - tx) / scale;
            let y = (result.params[2 * index + 1] - ty) / scale;
            mapped[2 * index] = cos * x + sin * y;
            mapped[2 * index + 1] = -sin * x + cos * y;
        }
    }

    let circle_base = 2 * sketch.points.len();
    for index in 0..sketch.circles.len() {
        mapped[circle_base + index] /= scale;
    }
    let arc_base = circle_base + sketch.circles.len();
    for index in 0..sketch.arcs.len() {
        mapped[arc_base + 3 * index] /= scale;
        mapped[arc_base + 3 * index + 1] -= theta;
        mapped[arc_base + 3 * index + 2] -= theta;
    }

    let mut fitted = sketch.clone();
    fitted.apply_params(&mapped);
    Ok(fitted)
}

pub(crate) fn fit_status(result: &FitterResult) -> String {
    let coverage = if result.total_samples == 0 {
        0.0
    } else {
        100.0 * result.matched_samples as f64 / result.total_samples as f64
    };
    format!(
        "Fit: RMS {:.3} px, coverage {coverage:.1}%, iterations {}, converged {}",
        result.rms_px, result.iterations, result.converged
    )
}

#[cfg(test)]
mod tests {
    use rf_core::sketch::{ArcId, LineId, PointId, SketchArc, SketchLine, SketchPoint};

    use super::*;

    fn result(params: Vec<f64>) -> FitterResult {
        FitterResult {
            params,
            transform: [std::f64::consts::FRAC_PI_2, 10.0, 20.0],
            scale: 1.0,
            rms_px: 0.0,
            max_constraint: 0.0,
            iterations: 1,
            converged: true,
            gauge_warning: false,
            residuals: Vec::new(),
            total_samples: 10,
            matched_samples: 8,
            inlier_ratio: 1.0,
            uncovered_ratio: 0.2,
            mask_too_narrow: false,
            transform_stddev: [0.0; 3],
            primitive_stats: Vec::new(),
        }
    }

    #[test]
    fn applies_inverse_rigid_transform_to_native_geometry() {
        let sketch = Sketch {
            points: vec![SketchPoint {
                id: PointId(1),
                x: 3.0,
                y: 4.0,
            }],
            arcs: vec![SketchArc {
                id: ArcId(1),
                center: PointId(1),
                radius: 5.0,
                start: 0.2,
                end: 1.0,
                construction: false,
            }],
            ..Sketch::default()
        };
        let fitted =
            apply_native_fit_result(&sketch, &result(vec![3.0, 4.0, 5.0, 0.2, 1.0])).unwrap();
        assert!((fitted.points[0].x + 16.0).abs() < 1e-12);
        assert!((fitted.points[0].y - 7.0).abs() < 1e-12);
        assert!((fitted.arcs[0].start - (0.2 - std::f64::consts::FRAC_PI_2)).abs() < 1e-12);
        assert!((fitted.arcs[0].end - (1.0 - std::f64::consts::FRAC_PI_2)).abs() < 1e-12);
    }

    #[test]
    fn rejects_invalid_parameter_count() {
        let sketch = Sketch::default();
        let error = apply_native_fit_result(&sketch, &result(vec![1.0])).unwrap_err();
        assert!(error.contains("expected 0"));
    }

    #[test]
    fn native_fit_preserves_the_original_shape() {
        let mut gray = vec![255; 64 * 64];
        for x in 8..56 {
            gray[32 * 64 + x] = 0;
        }
        let raster = RasterData {
            gray,
            width: 64,
            height: 64,
        };
        let sketch = Sketch {
            points: vec![
                SketchPoint {
                    id: PointId(1),
                    x: 10.0,
                    y: 30.0,
                },
                SketchPoint {
                    id: PointId(2),
                    x: 50.0,
                    y: 30.0,
                },
            ],
            lines: vec![SketchLine {
                id: LineId(1),
                a: PointId(1),
                b: PointId(2),
                construction: false,
            }],
            ..Sketch::default()
        };

        let output = fit_native_sketch(&sketch, &raster, None, FitOptions::default()).unwrap();
        let a = output.sketch.points[0];
        let b = output.sketch.points[1];
        let length = (b.x - a.x).hypot(b.y - a.y);
        assert!(output.result.converged);
        assert!((length - 40.0).abs() < 1e-9);
        assert!((a.y - 32.5).abs() < 0.5);
        assert!((b.y - 32.5).abs() < 0.5);
    }

    #[test]
    fn formats_coverage_from_actual_sample_counts() {
        let output = fit_status(&result(Vec::new()));
        assert!(output.contains("coverage 80.0%"));
        assert!(output.contains("converged true"));
    }
}
