use crate::raster::RasterData;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PreprocessSettings {
    pub brightness: f64,
    pub contrast: f64,
    pub sharpness: f64,
    pub invert: bool,
    pub opacity: f64,
}

impl Default for PreprocessSettings {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 0.0,
            sharpness: 0.0,
            invert: false,
            opacity: 100.0,
        }
    }
}

pub(crate) fn apply(
    original: &RasterData,
    settings: PreprocessSettings,
) -> Result<RasterData, String> {
    for value in [
        settings.brightness,
        settings.contrast,
        settings.sharpness,
        settings.opacity,
    ] {
        if !value.is_finite() {
            return Err("image settings must be finite".to_string());
        }
    }
    if settings.sharpness < 0.0 || settings.opacity < 0.0 || settings.opacity > 100.0 {
        return Err("sharpness must be nonnegative and opacity must be 0..100".to_string());
    }

    let contrast = (100.0 + settings.contrast).max(0.0) / 100.0;
    let mut gray = original
        .gray
        .iter()
        .map(|pixel| {
            let value = (*pixel as f64 - 128.0) * contrast + 128.0 + settings.brightness;
            let value = if settings.invert {
                255.0 - value
            } else {
                value
            };
            value.round().clamp(0.0, 255.0) as u8
        })
        .collect::<Vec<_>>();

    if settings.sharpness > 0.0 {
        let image =
            image::GrayImage::from_raw(original.width as u32, original.height as u32, gray.clone())
                .ok_or_else(|| "could not construct image for sharpness".to_string())?;
        let blurred = image::imageops::blur(&image, 1.0);
        for (index, pixel) in gray.iter_mut().enumerate() {
            let x = (index % original.width) as u32;
            let y = (index / original.width) as u32;
            let value = image.get_pixel(x, y).0[0] as f64;
            let blur = blurred.get_pixel(x, y).0[0] as f64;
            *pixel = (value + settings.sharpness * (value - blur))
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }

    Ok(RasterData {
        gray,
        width: original.width,
        height: original.height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster() -> RasterData {
        RasterData {
            gray: vec![0, 64, 128, 255],
            width: 2,
            height: 2,
        }
    }

    #[test]
    fn brightness_contrast_and_invert_change_gray_pixels() {
        let adjusted = apply(
            &raster(),
            PreprocessSettings {
                brightness: 10.0,
                contrast: 20.0,
                invert: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(adjusted.gray, vec![255, 194, 117, 0]);
    }

    #[test]
    fn sharpness_preserves_dimensions_and_buffer_length() {
        let adjusted = apply(
            &raster(),
            PreprocessSettings {
                sharpness: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((adjusted.width, adjusted.height), (2, 2));
        assert_eq!(adjusted.gray.len(), 4);
    }

    #[test]
    fn invalid_settings_are_rejected() {
        assert!(apply(
            &raster(),
            PreprocessSettings {
                sharpness: -1.0,
                ..Default::default()
            }
        )
        .is_err());
        assert!(apply(
            &raster(),
            PreprocessSettings {
                opacity: 101.0,
                ..Default::default()
            }
        )
        .is_err());
    }
}
