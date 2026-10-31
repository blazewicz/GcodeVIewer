# GcodeViewer

Browser-based viewer with limited editing functionality for G-code.

## Renderer (wgpu + WASM)

`crates/renderer` parses G-code through the core query API (`motions`) and draws
the toolpath with wgpu. It is platform-neutral: callers provide a wgpu instance
and surface, so the same renderer code compiles for native targets as well as
the browser. `crates/wasm` is a thin `wasm-bindgen` wrapper that creates the
canvas surface and forwards calls to the renderer.

Rapid, feed, arc, and spline segments are colour-coded by kind, and
`set_color_by_feed` switches to a colour scale over the feedrate range of the
loaded file (tunable with `set_feed_range` to keep outliers from flattening the
scale), leaving moves without a known feedrate grey. Pan, zoom, orbit, and fit
are exposed to the host page. Extruding moves are drawn as round tubes with a
physical diameter, configurable through `set_extrusion_diameter` (0.2 mm by
default). Each segment ends in a conical cap and starts with a matching inset
socket; pieces of one move overlap, while separate moves meet cone-to-rim so
they stay visually distinct. Rapids and non-extruding moves stay thin lines. The
WASM binding takes a canvas element and leaves the frame loop to the caller, so
a React component only needs a ref and a `requestAnimationFrame` loop.

```sh
mise install        # provides wasm-pack and node
just wasm-target    # once: rustup target add wasm32-unknown-unknown
just ui             # builds crates/wasm and serves the Vite demo
```

```ts
import init, { Renderer } from "gcode-viewer-wasm";

await init();
const renderer = await Renderer.create(canvas);
renderer.load_gcode(text);

function frame() {
  renderer.render();
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
```

`just wasm-check` lints the renderer and binding for the wasm32 target, and
`just wasm` builds the package into `ui/pkg` for the Vite app.

## Native viewer

`crates/native` is a tiny native host: `gcode-viewer <FILE>` opens a window and
renders the toolpath with the same platform-neutral renderer the browser uses.
There is no chrome around the image, and files stream in chunks instead of
being held in memory.

```sh
just view path/to/file.gcode
# or
cargo run --release -p gcode-viewer-native -- path/to/file.gcode
```

Large files are parsed with a debug build far more slowly than with a release
build, so prefer `--release` (what `just view` uses) for real toolpaths.

Left-drag pans, middle-drag or Ctrl+drag orbits, and the wheel zooms under the
cursor. `F` fits the toolpath, `Up`/`Down` move the Z clip that hides everything
above it (`Shift` for a fine step), `Z` resets the clip, and `Esc` quits. The
window title shows the cut-off while the clip is active.

Without a hardware graphics adapter the viewer falls back to software
rasterization and says so at startup; every segment is then drawn on the CPU,
so large toolpaths render slowly.

On WSL2 the viewer selects Mesa's D3D12 driver, which reaches the Windows GPU
through `/dev/dxg`; without that, Mesa's fallback can end at llvmpipe even on a
machine with a GPU. Setting `GALLIUM_DRIVER` or `MESA_LOADER_DRIVER_OVERRIDE`
yourself overrides the choice.

## Development

```sh
uv sync
uv run pytest -q
uv run maturin develop --release
cargo test
cargo clippy --workspace --all-targets -- -D warnings
uv run python scripts/benchmark.py
uv run python scripts/benchmark.py --mode splines
```

Firmware semantics were checked against Marlin 2.1.2.5:
[G5 handler](https://github.com/MarlinFirmware/Marlin/blob/2.1.2.5/Marlin/src/gcode/motion/G5.cpp),
[Bézier planner](https://github.com/MarlinFirmware/Marlin/blob/2.1.2.5/Marlin/src/module/planner_bezier.cpp).
G5 always emits explicit I/J offsets from the start and P/Q offsets from the
end. In particular, its extrusion parameter is not arc length.
