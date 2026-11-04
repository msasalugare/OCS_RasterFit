use ocs_plugin_api::ribbon::{CadModule, IconKind, ModuleEvent, RibbonGroup, RibbonItem, ToolDef};

const OPEN_ICON: &[u8] = include_bytes!("../assets/icons/open.svg");
const OPEN_PDF_ICON: &[u8] = include_bytes!("../assets/icons/open_pdf.svg");
const PAGE_PREV_ICON: &[u8] = include_bytes!("../assets/icons/page_prev.svg");
const PAGE_NEXT_ICON: &[u8] = include_bytes!("../assets/icons/page_next.svg");
const IMAGE_ADJUST_ICON: &[u8] = include_bytes!("../assets/icons/image_adjust.svg");
const SNAP_LINE_ICON: &[u8] = include_bytes!("../assets/icons/snap_line.svg");
const SNAP_POLYLINE_ICON: &[u8] = include_bytes!("../assets/icons/snap_polyline.svg");
const CALIBRATE_ICON: &[u8] = include_bytes!("../assets/icons/calibrate.svg");
const LEVEL_ICON: &[u8] = include_bytes!("../assets/icons/level.svg");
const ORIGIN_ICON: &[u8] = include_bytes!("../assets/icons/origin.svg");
const GUIDES_ICON: &[u8] = include_bytes!("../assets/icons/guides.svg");
const BRUSH_SIZE_ICON: &[u8] = include_bytes!("../assets/icons/brush_size.svg");
const MASK_INCLUDE_ICON: &[u8] = include_bytes!("../assets/icons/mask_include.svg");
const MASK_EXCLUDE_ICON: &[u8] = include_bytes!("../assets/icons/mask_exclude.svg");
const MASK_RECT_ICON: &[u8] = include_bytes!("../assets/icons/mask_rect.svg");
const MASK_POLYGON_ICON: &[u8] = include_bytes!("../assets/icons/mask_polygon.svg");
const MASK_ERASE_ICON: &[u8] = include_bytes!("../assets/icons/mask_erase.svg");
const MASK_FROM_SELECTION_ICON: &[u8] = include_bytes!("../assets/icons/mask_from_selection.svg");
const MASK_CLEAR_ICON: &[u8] = include_bytes!("../assets/icons/mask_clear.svg");
const CONSTRAINT_COINCIDENT_ICON: &[u8] =
    include_bytes!("../assets/icons/constraint_coincident.svg");
const CONSTRAINT_TANGENT_ICON: &[u8] = include_bytes!("../assets/icons/constraint_tangent.svg");
const FIT_ICON: &[u8] = include_bytes!("../assets/icons/fit.svg");
const FIT_PREVIEW_ICON: &[u8] = include_bytes!("../assets/icons/fit_preview.svg");
const FIT_OPTIONS_ICON: &[u8] = include_bytes!("../assets/icons/fit_options.svg");
const HEATMAP_ICON: &[u8] = include_bytes!("../assets/icons/heatmap.svg");
const EXPORT_REPORT_ICON: &[u8] = include_bytes!("../assets/icons/export_report.svg");
const STATUS_ICON: &[u8] = include_bytes!("../assets/icons/status.svg");
const MEASURE_DISTANCE_ICON: &[u8] = include_bytes!("../assets/icons/measure_distance.svg");
const MEASURE_ANGLE_ICON: &[u8] = include_bytes!("../assets/icons/measure_angle.svg");
const HATCH_ICON: &[u8] = include_bytes!("../assets/icons/hatch.svg");

pub(crate) struct RasterFitModule;

fn command(id: &'static str, label: &'static str, icon: &'static [u8]) -> ToolDef {
    ToolDef {
        id,
        label,
        icon: IconKind::Svg(icon),
        event: ModuleEvent::Command(id.to_string()),
    }
}

impl CadModule for RasterFitModule {
    fn id(&self) -> &'static str {
        "com.rasterfit"
    }

    fn title(&self) -> &'static str {
        "RasterFit"
    }

    fn ribbon_groups(&self) -> &[RibbonGroup] {
        static GROUPS: std::sync::OnceLock<Vec<RibbonGroup>> = std::sync::OnceLock::new();
        GROUPS.get_or_init(|| {
            vec![
                RibbonGroup {
                    title: "Raster",
                    tools: vec![
                        RibbonItem::LargeTool(ToolDef {
                            id: "RF_OPEN",
                            label: "Open raster",
                            icon: IconKind::Svg(OPEN_ICON),
                            event: ModuleEvent::PluginFileDialog {
                                command: "RF_OPEN".to_string(),
                                title: "Open raster image".to_string(),
                                filter_name: "Raster images".to_string(),
                                extensions: vec![
                                    "png".to_string(),
                                    "jpg".to_string(),
                                    "jpeg".to_string(),
                                    "webp".to_string(),
                                ],
                            },
                        }),
                        RibbonItem::LargeTool(ToolDef {
                            id: "RF_OPEN_PDF",
                            label: "Open PDF",
                            icon: IconKind::Svg(OPEN_PDF_ICON),
                            event: ModuleEvent::PluginFileDialog {
                                command: "RF_OPEN_PDF".to_string(),
                                title: "Open PDF drawing".to_string(),
                                filter_name: "PDF documents".to_string(),
                                extensions: vec!["pdf".to_string()],
                            },
                        }),
                        RibbonItem::ToolGrid {
                            columns: vec![
                                vec![command("RF_PAGE_PREV", "Previous page", PAGE_PREV_ICON)],
                                vec![command("RF_PAGE_NEXT", "Next page", PAGE_NEXT_ICON)],
                            ],
                        },
                        RibbonItem::LargeTool(command(
                            "RF_IMAGE_ADJUST",
                            "Image settings",
                            IMAGE_ADJUST_ICON,
                        )),
                    ],
                },
                RibbonGroup {
                    title: "Draw",
                    tools: vec![
                        RibbonItem::LargeTool(command("RF_LINE", "Snap line", SNAP_LINE_ICON)),
                        RibbonItem::LargeTool(command(
                            "RF_POLYLINE",
                            "Snap polyline",
                            SNAP_POLYLINE_ICON,
                        )),
                    ],
                },
                RibbonGroup {
                    title: "Calibrate",
                    tools: vec![
                        RibbonItem::LargeTool(command("RF_CALIBRATE", "Calibrate", CALIBRATE_ICON)),
                        RibbonItem::LargeTool(command("RF_LEVEL", "Level", LEVEL_ICON)),
                        RibbonItem::LargeTool(command("RF_ORIGIN", "Set origin", ORIGIN_ICON)),
                        RibbonItem::LargeTool(command("RF_GUIDES", "Guides", GUIDES_ICON)),
                    ],
                },
                RibbonGroup {
                    title: "Mask",
                    tools: vec![
                        RibbonItem::LargeTool(command(
                            "RF_MASK_INCLUDE",
                            "Include",
                            MASK_INCLUDE_ICON,
                        )),
                        RibbonItem::LargeTool(command(
                            "RF_MASK_EXCLUDE",
                            "Exclude",
                            MASK_EXCLUDE_ICON,
                        )),
                        RibbonItem::ToolGrid {
                            columns: vec![
                                vec![
                                    command("RF_MASK_RECT", "Rectangle", MASK_RECT_ICON),
                                    command("RF_MASK_POLYGON", "Polygon", MASK_POLYGON_ICON),
                                ],
                                vec![
                                    command("RF_MASK_ERASE", "Erase mask", MASK_ERASE_ICON),
                                    command(
                                        "RF_MASK_FROM_SELECTION",
                                        "From selection",
                                        MASK_FROM_SELECTION_ICON,
                                    ),
                                ],
                                vec![command("RF_MASK_CLEAR", "Clear mask", MASK_CLEAR_ICON)],
                                vec![command("RF_MASK_BRUSH", "Brush size", BRUSH_SIZE_ICON)],
                            ],
                        },
                    ],
                },
                RibbonGroup {
                    title: "Constraints",
                    tools: vec![
                        RibbonItem::LargeTool(command(
                            "RF_CONSTRAINT_COINCIDENT",
                            "Coincident",
                            CONSTRAINT_COINCIDENT_ICON,
                        )),
                        RibbonItem::LargeTool(command(
                            "RF_CONSTRAINT_TANGENT",
                            "Tangent",
                            CONSTRAINT_TANGENT_ICON,
                        )),
                    ],
                },
                RibbonGroup {
                    title: "Analyze",
                    tools: vec![
                        RibbonItem::LargeTool(command("RF_FIT", "Fit", FIT_ICON)),
                        RibbonItem::LargeTool(command(
                            "RF_FIT_PREVIEW",
                            "Preview fit",
                            FIT_PREVIEW_ICON,
                        )),
                        RibbonItem::ToolGrid {
                            columns: vec![
                                vec![
                                    command("RF_FIT_OPTIONS", "Fit options", FIT_OPTIONS_ICON),
                                    command("RF_HEATMAP", "Heatmap", HEATMAP_ICON),
                                ],
                                vec![
                                    command(
                                        "RF_EXPORT_REPORT",
                                        "Export report",
                                        EXPORT_REPORT_ICON,
                                    ),
                                    command("RF_STATUS", "Status", STATUS_ICON),
                                ],
                                vec![
                                    command(
                                        "RF_MEASURE_DISTANCE",
                                        "Distance",
                                        MEASURE_DISTANCE_ICON,
                                    ),
                                    command("RF_MEASURE_ANGLE", "Angle", MEASURE_ANGLE_ICON),
                                    command("RF_HATCH", "Hatch", HATCH_ICON),
                                ],
                            ],
                        },
                    ],
                },
            ]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMANDS: [&str; 30] = [
        "RF_OPEN",
        "RF_OPEN_PDF",
        "RF_PAGE_PREV",
        "RF_PAGE_NEXT",
        "RF_IMAGE_ADJUST",
        "RF_LINE",
        "RF_POLYLINE",
        "RF_CALIBRATE",
        "RF_LEVEL",
        "RF_ORIGIN",
        "RF_GUIDES",
        "RF_MASK_BRUSH",
        "RF_MASK_INCLUDE",
        "RF_MASK_EXCLUDE",
        "RF_MASK_RECT",
        "RF_MASK_POLYGON",
        "RF_MASK_ERASE",
        "RF_MASK_FROM_SELECTION",
        "RF_MASK_CLEAR",
        "RF_CONSTRAINT_COINCIDENT",
        "RF_CONSTRAINT_TANGENT",
        "RF_MEASURE_DISTANCE",
        "RF_MEASURE_ANGLE",
        "RF_HATCH",
        "RF_FIT_PREVIEW",
        "RF_FIT",
        "RF_FIT_OPTIONS",
        "RF_HEATMAP",
        "RF_EXPORT_REPORT",
        "RF_STATUS",
    ];

    const SVG_ASSETS: [(&str, &[u8]); 30] = [
        ("open.svg", OPEN_ICON),
        ("open_pdf.svg", OPEN_PDF_ICON),
        ("page_prev.svg", PAGE_PREV_ICON),
        ("page_next.svg", PAGE_NEXT_ICON),
        ("image_adjust.svg", IMAGE_ADJUST_ICON),
        ("snap_line.svg", SNAP_LINE_ICON),
        ("snap_polyline.svg", SNAP_POLYLINE_ICON),
        ("calibrate.svg", CALIBRATE_ICON),
        ("level.svg", LEVEL_ICON),
        ("origin.svg", ORIGIN_ICON),
        ("guides.svg", GUIDES_ICON),
        ("brush_size.svg", BRUSH_SIZE_ICON),
        ("mask_include.svg", MASK_INCLUDE_ICON),
        ("mask_exclude.svg", MASK_EXCLUDE_ICON),
        ("mask_rect.svg", MASK_RECT_ICON),
        ("mask_polygon.svg", MASK_POLYGON_ICON),
        ("mask_erase.svg", MASK_ERASE_ICON),
        ("mask_from_selection.svg", MASK_FROM_SELECTION_ICON),
        ("mask_clear.svg", MASK_CLEAR_ICON),
        ("constraint_coincident.svg", CONSTRAINT_COINCIDENT_ICON),
        ("constraint_tangent.svg", CONSTRAINT_TANGENT_ICON),
        ("measure_distance.svg", MEASURE_DISTANCE_ICON),
        ("measure_angle.svg", MEASURE_ANGLE_ICON),
        ("hatch.svg", HATCH_ICON),
        ("fit_preview.svg", FIT_PREVIEW_ICON),
        ("fit.svg", FIT_ICON),
        ("fit_options.svg", FIT_OPTIONS_ICON),
        ("heatmap.svg", HEATMAP_ICON),
        ("export_report.svg", EXPORT_REPORT_ICON),
        ("status.svg", STATUS_ICON),
    ];

    fn tools(module: &RasterFitModule) -> Vec<&ToolDef> {
        let mut tools = Vec::new();
        for item in module.ribbon_groups().iter().flat_map(|group| &group.tools) {
            match item {
                RibbonItem::Tool(tool)
                | RibbonItem::LabeledTool(tool)
                | RibbonItem::LargeTool(tool) => tools.push(tool),
                RibbonItem::ToolGrid { columns } => {
                    for column in columns {
                        tools.extend(column);
                    }
                }
                _ => {}
            }
        }
        tools
    }

    #[test]
    fn svg_assets_have_24_viewbox_and_no_current_color() {
        for (name, bytes) in SVG_ASSETS {
            let svg = std::str::from_utf8(bytes).expect("SVG assets must be UTF-8");
            assert!(
                svg.contains(r#"viewBox="0 0 24 24""#),
                "{name} must have a 24x24 viewBox"
            );
            assert!(
                !svg.contains("currentColor"),
                "{name} must not use currentColor"
            );
        }
    }

    #[test]
    fn every_command_has_an_svg_tool_with_a_matching_event() {
        let tools = tools(&RasterFitModule);
        assert_eq!(tools.len(), COMMANDS.len());

        for id in COMMANDS {
            let tool = tools
                .iter()
                .find(|tool| tool.id == id)
                .unwrap_or_else(|| panic!("missing ToolDef for {id}"));
            assert!(matches!(tool.icon, IconKind::Svg(_)), "{id} must use SVG");
            match &tool.event {
                ModuleEvent::Command(command) => assert_eq!(command, id),
                ModuleEvent::PluginFileDialog { command, .. } => {
                    assert!(matches!(id, "RF_OPEN" | "RF_OPEN_PDF"));
                    assert_eq!(command, id);
                }
                _ => panic!("unexpected event for {id}"),
            }
        }
    }
}
