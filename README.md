# RasterFit for OpenCADStudio

Native RasterFit plugin for **OpenCADStudio 2026.44, plugin API v14**.

## Install the ready-made plugin

The ready-made **Linux x86_64** package is in `release/com.rasterfit/` and contains:

- `librf_ocs_plugin.so`
- `plugin.toml`

From the repository root:

```bash
mkdir -p ~/.config/OpenCADStudio/plugins
cp -r release/com.rasterfit ~/.config/OpenCADStudio/plugins/
```

Restart OpenCADStudio after copying. The **RasterFit** ribbon tab contains the plugin tools.

## Source

Plugin sources and SVG icons are in `crates/rf-ocs-plugin/`. This plugin-only repository does not contain the `rf-core` workspace required to rebuild the sources; use the ready-made package above for installation.
