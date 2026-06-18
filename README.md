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
- **Product comparison** (**Tools → Comparison**) — compare a reference and a new product (structure, variable data, CF flags) with user-defined thresholds and a pass/fail report; same logic via the `compare_products` example

## Requirements

- Rust 1.75+
- GPU support for the GUI (**wgpu** on Linux desktop, macOS, and Windows; **Glow**/OpenGL on WSL2 — see [WSL2 graphics](#wsl2-graphics))

### WSL2 / Linux system packages

On **WSLg** (default on recent WSL2), the app uses Wayland automatically — no extra packages needed.

If you use **X11 forwarding** to an external X server (VcXsrv, X410, …), install:

```bash
sudo apt install libxkbcommon-x11-0 libgl1-mesa-dri
```

Runtime for the GTK file dialog (optional build): `libgtk-3-0`.

**Opening products:** **File → Open Zarr…** opens an in-app browser for `.zarr` directories and `.zarr.zip` archives — click or double-click a product, or paste a path and press **Open**. Use **System picker…** inside that dialog for the native file chooser; it automatically uses a folder picker for `.zarr` paths and a file picker for `.zip` paths. You can open several products at once; each appears as a top-level entry in the **Hierarchy** panel. Close one with **✕** next to its name or **File → Close product**. Opening a product reads hierarchy metadata only; array values are loaded when you select a variable to plot.

If the native dialog is empty or opens twice on WSL, rebuild with the GTK backend:

```bash
sudo apt install libgtk-3-dev
cargo build --no-default-features --features dialog-gtk
cargo run --no-default-features --features dialog-gtk
```

### WSL2 graphics

The app picks a renderer from your WSL environment (override with `COPERNICUS_VIEWER_GL`):

| Environment | Default | Backend |
|-------------|---------|---------|
| WSLg (Wayland) | GPU OpenGL | Glow |
| WSL + X11 forwarding | Mesa llvmpipe (software) | Glow |
| Linux desktop / macOS / Windows | GPU | wgpu |

On **WSLg** (`WAYLAND_DISPLAY` set), the default is **Glow with GPU acceleration**. Mesa llvmpipe is not compatible with WSLg/Wayland.

On **WSL over X11** (VcXsrv, X410, …), the default is **Glow with Mesa llvmpipe** software rendering, which is much more stable than ZINK/EGL when resizing windows. Vsync is disabled on this path; the app sets `LIBGL_ALWAYS_SOFTWARE=1` and related Mesa env vars automatically.

```bash
# Force Mesa llvmpipe + Glow (X11 only; on WSLg falls back to GPU Glow with a warning)
COPERNICUS_VIEWER_GL=software cargo run

# GPU: Glow on WSL, wgpu elsewhere
COPERNICUS_VIEWER_GL=hardware cargo run

# Force wgpu everywhere (expert — may fail on WSL with EGL/ZINK errors)
COPERNICUS_VIEWER_GL=wgpu cargo run

# Explicit auto-detection (same as unset)
COPERNICUS_VIEWER_GL=auto cargo run
```

`native` is an alias for `wgpu`.

## Install

### Rust (crates.io)

```bash
cargo install copernicus_viewer
copernicus_viewer
```

Requires a Rust toolchain and GPU support (same as [Requirements](#requirements)).

### Prebuilt binaries (GitHub Releases)

Download the archive for your platform from [GitHub Releases](https://github.com/vlevasseur073/copernicus_viewer/releases), extract it, and run:

```bash
# Linux / macOS
./copernicus_viewer /path/to/product.zarr

# Windows
copernicus_viewer.exe C:\path\to\product.zarr
```

Verify downloads with the `SHA256SUMS.txt` file attached to each release.

## Build & Run

```bash
# Optional: generate a local sample product for testing
cargo run --example create_sample_zarr

# Compare two products from the CLI (reference vs new)
cargo run --example compare_products -- reference.zarr new.zarr

cargo run
```

Use **File → Open Zarr…** to load additional EOPF products. Pass one or more paths on the command line:

```bash
cargo run -- /path/to/product_a.zarr /path/to/product_b.zarr
```

## Releasing (maintainers)

1. Bump `version` in `Cargo.toml` and commit.
2. Create and push an annotated tag: `git tag -a v0.1.0 -m "v0.1.0" && git push origin v0.1.0`
3. Ensure the repository secret `CARGO_REGISTRY_TOKEN` is set ([crates.io token](https://crates.io/settings/tokens)).

The [release workflow](.github/workflows/release.yml) runs tests, builds binaries for Linux, Windows, and macOS (x86_64 + arm64), attaches them to a GitHub Release, and publishes to crates.io (requires the `CARGO_REGISTRY_TOKEN` repository secret).

For the **first** crates.io publish, create an API token at [crates.io/settings/tokens](https://crates.io/settings/tokens), add it as a GitHub secret, then either push a tag or run locally:

```bash
cargo publish
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

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
