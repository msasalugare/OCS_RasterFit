use std::path::PathBuf;

use ocs_plugin_api::host::{Handle, HostApi, XRecordEntry, XRecordValue};
use rf_core::calibration::CalibrationSegment;
use rf_core::fitter::FitterResult;
use rf_core::mask::MaskStroke;
use serde::{Deserialize, Serialize};

use crate::raster::RasterTransform;
use crate::state::{lock_state, PluginState, SharedState};

const DICTIONARY_PATH: &[&str] = &["RASTERFIT"];
const RECORD_NAME: &str = "WORKSPACE";
const SCHEMA_VERSION: u32 = 1;
const CHUNK_SIZE: usize = 240;

fn default_mask_brush_radius() -> f64 {
    6.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SourceKind {
    Image,
    Pdf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WorkspaceSnapshot {
    pub version: u32,
    pub source_kind: SourceKind,
    pub source_path: PathBuf,
    pub raster_path: PathBuf,
    pub raster_handle: u64,
    pub page_index: usize,
    pub dpi: f32,
    pub transform: RasterTransform,
    pub calibrated: bool,
    pub segments: Vec<CalibrationSegment>,
    pub mask_strokes: Vec<MaskStroke>,
    pub mask_enabled: bool,
    #[serde(default = "default_mask_brush_radius")]
    pub mask_brush_radius: f64,
    #[serde(default)]
    pub mask_overlay_handles: Vec<u64>,
    #[serde(default)]
    pub guides_enabled: bool,
    #[serde(default)]
    pub guide_handles: Vec<u64>,
    pub preprocess: crate::preprocess::PreprocessSettings,
    pub fit_options: crate::fit::FitOptions,
    pub last_fit: Option<FitterResult>,
    pub last_origins: Vec<u64>,
}

impl WorkspaceSnapshot {
    pub fn capture(state: &PluginState) -> Option<Self> {
        let source_path = state.source_path.clone()?;
        let raster_path = state.raster_path.clone()?;
        let mask = state.mask.lock().ok()?;
        Some(Self {
            version: SCHEMA_VERSION,
            source_kind: if state.pdf.is_some() {
                SourceKind::Pdf
            } else {
                SourceKind::Image
            },
            source_path,
            raster_path,
            raster_handle: state.raster_handle?.value(),
            page_index: state.page_index,
            dpi: 150.0,
            transform: state.transform,
            calibrated: state.calibrated,
            segments: state.segments.clone(),
            mask_strokes: mask.strokes.clone(),
            mask_enabled: mask.enabled,
            mask_brush_radius: mask.brush_radius,
            mask_overlay_handles: state
                .mask_overlay_handles
                .iter()
                .map(|handle| handle.value())
                .collect(),
            guides_enabled: state.guides_enabled,
            guide_handles: state
                .guide_handles
                .iter()
                .map(|handle| handle.value())
                .collect(),
            preprocess: state.preprocess,
            fit_options: state.fit_options,
            last_fit: state.last_fit.clone(),
            last_origins: state
                .last_origins
                .iter()
                .map(|handle| handle.value())
                .collect(),
        })
    }
}

pub(crate) fn save_workspace(host: &mut dyn HostApi, state: &SharedState) -> Result<(), String> {
    let snapshot = {
        let state = lock_state(state)?;
        WorkspaceSnapshot::capture(&state)
    };
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    let entries = snapshot.to_entries()?;
    if host.read_xrecord(DICTIONARY_PATH, RECORD_NAME).is_ok() {
        host.update_xrecord(DICTIONARY_PATH, RECORD_NAME, entries)?;
    } else {
        host.create_xrecord(DICTIONARY_PATH, RECORD_NAME, entries)?;
    }
    Ok(())
}

pub(crate) fn load_workspace(host: &mut dyn HostApi) -> Result<WorkspaceSnapshot, String> {
    let entries = host.read_xrecord(DICTIONARY_PATH, RECORD_NAME)?;
    WorkspaceSnapshot::from_entries(&entries)
}

impl WorkspaceSnapshot {
    fn to_entries(&self) -> Result<Vec<XRecordEntry>, String> {
        if self.version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported RasterFit workspace version {}",
                self.version
            ));
        }
        let json = serde_json::to_vec(self)
            .map_err(|error| format!("could not serialize RasterFit workspace: {error}"))?;
        Ok(json
            .chunks(CHUNK_SIZE)
            .enumerate()
            .map(|(index, chunk)| {
                XRecordEntry::new(chunk_code(index), XRecordValue::Chunk(chunk.to_vec()))
            })
            .collect())
    }

    fn from_entries(entries: &[XRecordEntry]) -> Result<Self, String> {
        let mut entries = entries.to_vec();
        entries.sort_by_key(|entry| entry.code);
        let mut json = Vec::new();
        for entry in entries {
            match entry.value {
                XRecordValue::Chunk(chunk) => json.extend_from_slice(&chunk),
                _ => return Err("RasterFit workspace contains a non-chunk entry".to_string()),
            }
        }
        let snapshot: Self = serde_json::from_slice(&json)
            .map_err(|error| format!("could not decode RasterFit workspace: {error}"))?;
        if snapshot.version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported RasterFit workspace version {}",
                snapshot.version
            ));
        }
        Ok(snapshot)
    }
}

fn chunk_code(index: usize) -> i32 {
    let mut code = index + 1;
    if code >= 5 {
        code += 1;
    }
    if code >= 105 {
        code += 1;
    }
    code as i32
}

pub(crate) fn raster_handle(snapshot: &WorkspaceSnapshot) -> Handle {
    Handle::new(snapshot.raster_handle)
}

#[cfg(test)]
mod tests {
    use rf_core::calibration::SegmentOrientation;
    use rf_core::geometry::Point;
    use rf_core::mask::StrokeKind;

    use super::*;

    fn snapshot() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            version: SCHEMA_VERSION,
            source_kind: SourceKind::Pdf,
            source_path: PathBuf::from("drawing.pdf"),
            raster_path: PathBuf::from("page-2.png"),
            raster_handle: 42,
            page_index: 1,
            dpi: 150.0,
            transform: RasterTransform {
                sx: 2.0,
                sy: 2.0,
                theta: 0.25,
                origin: [0.0, 0.0],
            },
            calibrated: true,
            segments: vec![CalibrationSegment {
                a: Point::new(1.0, 2.0),
                b: Point::new(3.0, 4.0),
                length: 100.0,
                orientation: SegmentOrientation::Horizontal,
            }],
            mask_strokes: vec![MaskStroke {
                mode: rf_core::mask::StrokeMode::Include,
                kind: StrokeKind::Rect,
                radius: 0.0,
                points: vec![Point::new(0.0, 0.0), Point::new(10.0, 10.0)],
            }],
            mask_enabled: true,
            mask_brush_radius: 18.0,
            mask_overlay_handles: vec![43, 44],
            guides_enabled: true,
            guide_handles: vec![45, 46],
            preprocess: crate::preprocess::PreprocessSettings::default(),
            fit_options: crate::fit::FitOptions::default(),
            last_fit: None,
            last_origins: vec![42],
        }
    }

    #[test]
    fn workspace_roundtrips_through_chunked_xrecord_entries() {
        let snapshot = snapshot();
        let entries = snapshot.to_entries().unwrap();
        assert!(entries.len() > 1);
        assert!(entries
            .iter()
            .all(|entry| matches!(entry.value, XRecordValue::Chunk(_))));
        assert!(entries
            .iter()
            .all(|entry| entry.code != 5 && entry.code != 105));
        assert!(entries.windows(2).all(|pair| pair[0].code < pair[1].code));
        let restored = WorkspaceSnapshot::from_entries(&entries).unwrap();
        assert_eq!(restored.source_kind, SourceKind::Pdf);
        assert_eq!(restored.page_index, 1);
        assert_eq!(restored.raster_handle, 42);
        assert_eq!(restored.transform.sx, 2.0);
        assert_eq!(restored.segments.len(), 1);
        assert_eq!(restored.mask_strokes.len(), 1);
        assert!(restored.mask_enabled);
        assert_eq!(restored.mask_brush_radius, 18.0);
        assert_eq!(restored.mask_overlay_handles, vec![43, 44]);
        assert!(restored.guides_enabled);
        assert_eq!(restored.guide_handles, vec![45, 46]);
        assert_eq!(raster_handle(&restored), Handle::new(42));
    }

    #[test]
    fn old_workspace_uses_compatible_overlay_defaults() {
        let mut value = serde_json::to_value(snapshot()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("mask_brush_radius");
        object.remove("mask_overlay_handles");
        object.remove("guides_enabled");
        object.remove("guide_handles");
        let bytes = serde_json::to_vec(&value).unwrap();
        let entries = bytes
            .chunks(CHUNK_SIZE)
            .enumerate()
            .map(|(index, chunk)| {
                XRecordEntry::new(chunk_code(index), XRecordValue::Chunk(chunk.to_vec()))
            })
            .collect::<Vec<_>>();
        let restored = WorkspaceSnapshot::from_entries(&entries).unwrap();
        assert_eq!(restored.mask_brush_radius, 6.0);
        assert!(restored.mask_overlay_handles.is_empty());
        assert!(!restored.guides_enabled);
        assert!(restored.guide_handles.is_empty());
    }

    #[test]
    fn rejects_unsupported_workspace_version() {
        let mut snapshot = snapshot();
        snapshot.version = 99;
        assert!(snapshot.to_entries().is_err());
    }
}
