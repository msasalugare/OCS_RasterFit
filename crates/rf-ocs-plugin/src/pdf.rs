use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings};

use crate::raster::RasterData;

const MAX_RASTER_PIXELS: f64 = 268_435_456.0;

pub(crate) struct PdfDocument {
    pdf: Pdf,
}

pub(crate) struct RenderedPage {
    pub raster: RasterData,
    pub png: Vec<u8>,
    pub dpi: f32,
}

pub(crate) fn load(path: &Path) -> Result<PdfDocument, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read PDF '{}': {error}", path.display()))?;
    let pdf = Pdf::new(Arc::new(bytes))
        .map_err(|error| format!("failed to parse PDF '{}': {error:?}", path.display()))?;
    Ok(PdfDocument { pdf })
}

impl PdfDocument {
    pub(crate) fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    pub(crate) fn render_page(&self, page_index: usize, dpi: f32) -> Result<RenderedPage, String> {
        if !dpi.is_finite() || !(1.0..=600.0).contains(&dpi) {
            return Err("PDF render DPI must be finite and between 1 and 600".to_string());
        }

        let page = self.pdf.pages().get(page_index).ok_or_else(|| {
            format!(
                "PDF page index {page_index} is out of range for {} pages",
                self.page_count()
            )
        })?;
        let (page_width, page_height) = page.render_dimensions();
        let scale = dpi / 72.0;
        let render_width = f64::from(page_width) * f64::from(scale);
        let render_height = f64::from(page_height) * f64::from(scale);
        if !render_width.is_finite()
            || !render_height.is_finite()
            || render_width <= 0.0
            || render_height <= 0.0
        {
            return Err("PDF page has invalid render dimensions".to_string());
        }
        if render_width > f64::from(u16::MAX) || render_height > f64::from(u16::MAX) {
            return Err("PDF page render dimensions exceed hayro limits".to_string());
        }
        let pixel_count = render_width * render_height;
        if !pixel_count.is_finite() || pixel_count > MAX_RASTER_PIXELS {
            return Err("PDF page exceeds the raster pixel limit".to_string());
        }

        let pixmap = hayro::render(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
                ..Default::default()
            },
        );
        let width = u32::from(pixmap.width());
        let height = u32::from(pixmap.height());
        if width == 0 || height == 0 {
            return Err("hayro produced an empty PDF page raster".to_string());
        }
        let raster_width = usize::try_from(width)
            .map_err(|_| "PDF raster width does not fit in usize".to_string())?;
        let raster_height = usize::try_from(height)
            .map_err(|_| "PDF raster height does not fit in usize".to_string())?;
        let expected_len = raster_width
            .checked_mul(raster_height)
            .ok_or_else(|| "PDF raster dimensions overflow usize".to_string())?;
        let rgba_len = expected_len
            .checked_mul(4)
            .ok_or_else(|| "PDF RGBA raster dimensions overflow usize".to_string())?;
        let rgba = pixmap.data_as_u8_slice().to_vec();
        if rgba.len() != rgba_len {
            return Err(format!(
                "hayro produced {} RGBA bytes, expected {rgba_len}",
                rgba.len()
            ));
        }

        let rgba = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| "failed to construct the PDF RGBA raster".to_string())?;
        let image = image::DynamicImage::ImageRgba8(rgba);
        let gray = image.to_luma8().into_raw();
        if gray.len() != expected_len {
            return Err(format!(
                "gray8 raster has {} bytes, expected {expected_len}",
                gray.len()
            ));
        }
        let mut png = Cursor::new(Vec::new());
        image
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|error| format!("failed to encode PDF page as PNG: {error}"))?;

        Ok(RenderedPage {
            raster: RasterData {
                gray,
                width: raster_width,
                height: raster_height,
            },
            png: png.into_inner(),
            dpi,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn minimal_pdf() -> Vec<u8> {
        let content = "0 0 0 rg\n12 12 48 48 re f\n";
        let objects = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << >> /Contents 4 0 R >>\nendobj\n".to_string(),
            format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
                content.len(),
                content
            ),
        ];
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for object in objects {
            offsets.push(pdf.len());
            pdf.push_str(&object);
        }
        let xref_offset = pdf.len();
        pdf.push_str("xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n"
        ));
        pdf.into_bytes()
    }

    #[test]
    fn minimal_pdf_reports_page_count_and_renders_gray8_and_png() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rf-ocs-plugin-pdf-{}-{unique}.pdf",
            std::process::id()
        ));
        std::fs::write(&path, minimal_pdf()).unwrap();

        let document = load(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(document.page_count(), 1);

        let rendered = document.render_page(0, 72.0).unwrap();
        assert_eq!(rendered.dpi, 72.0);
        assert_eq!((rendered.raster.width, rendered.raster.height), (72, 72));
        assert_eq!(rendered.raster.gray.len(), 72 * 72);
        assert!(rendered.raster.gray.contains(&0));
        assert!(rendered.raster.gray.contains(&255));

        assert!(rendered.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let decoded =
            image::load_from_memory_with_format(&rendered.png, image::ImageFormat::Png).unwrap();
        assert_eq!(decoded.color(), image::ColorType::Rgba8);
        assert_eq!((decoded.width(), decoded.height()), (72, 72));
        assert_eq!(decoded.to_luma8().into_raw(), rendered.raster.gray);

        assert!(document.render_page(1, 72.0).is_err());
        for dpi in [0.0, 0.99, 600.01, f32::NAN, f32::INFINITY] {
            assert!(document.render_page(0, dpi).is_err());
        }
    }
}
