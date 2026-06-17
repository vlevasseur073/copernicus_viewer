# Copernicus Viewer

A Rust GUI application to explore and visualize [EOPF](https://cpm.pages.eopf.copernicus.eu/eopf-cpm/main/PSFD/index.html) Zarr products.

## Features

- Open EOPF Zarr stores (`.zarr` directories or `.zarr.zip` archives)
- Browse the product hierarchy (groups and variables) in a tree view
- Inspect metadata with an xarray-inspired representation (DataTree / Group / DataArray with attributes)
- **Product attributes** tree for root metadata (nested STAC / EOPF attributes, foldable like the hierarchy)
- **Statistics and data preview** tables in the inspector for loaded array subsets
- **Async plot loading** with progress bar for large arrays
- **Geo-referenced plotting**: coordinate arrays, CRS, and axis extent labels on heatmaps
- **CF flag variables** with `flag_meanings` and `flag_values` / `flag_masks` (bitmask plots per flag)
- **Coverage map** in the inspector — adaptive Plate Carrée view (zooms to regional tiles, global view for wide footprints) with Natural Earth coastlines

## Requirements

- Rust 1.75+
- OpenGL support for the GUI (uses the **Glow** OpenGL backend)

### WSL2 / Linux system packages

On **WSLg** (default on recent WSL2), the app uses Wayland automatically — no extra packages needed.

If you use **X11 forwarding** to an external X server (VcXsrv, X410, …), install:

```bash
sudo apt install libxkbcommon-x11-0 libgl1-mesa-dri
```

### WSL2 graphics

On WSL2 the app automatically selects **Mesa llvmpipe software rendering**, which is much more stable than ZINK/EGL when resizing windows over X11.

You can override this:

```bash
# Force software (default on WSL)
COPERNICUS_VIEWER_GL=software cargo run

# Try native GPU (WSLg only — may crash on X11 forwarding)
COPERNICUS_VIEWER_GL=hardware cargo run
```

If problems persist, also try:

```bash
LIBGL_ALWAYS_SOFTWARE=1 cargo run
```

## Build & Run

```bash
# Optional: generate a local sample product for testing
cargo run --example create_sample_zarr

cargo run
```

Use **File → Open Zarr…** to load an EOPF product. You can also pass a path on the command line:

```bash
cargo run -- /path/to/product.zarr
```

## EOPF Zarr structure

EOPF products follow the standard Zarr hierarchy described in [PSFD §4.4](https://cpm.pages.eopf.copernicus.eu/eopf-cpm/main/PSFD/4-storage-formats.html#zarr-representation-of-eopf-data-products):

- Root `.zattrs` — product-level metadata (STAC attributes)
- `.zmetadata` — consolidated metadata
- Group directories (`measurements`, `quality`, `conditions`, …) with `.zgroup`
- Variable leaf directories with `.zarray`, `.zattrs`, and chunk files

Sample data is available from the [EOPF Sentinel Zarr Samples Service](https://zarr.eopf.copernicus.eu/).

Coastline data: [Natural Earth 110m land](https://www.naturalearthdata.com/) (public domain).

## License

MIT OR Apache-2.0
