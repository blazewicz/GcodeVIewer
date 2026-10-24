# GcodeViewer

Browser-based viewer with limited editing functionality for G-code.

## Web renderer (WASM + wgpu)

`crates/wasm` builds a WebAssembly binding that parses G-code through the core
query API (`motions`) and draws the toolpath on a canvas with wgpu. Rapid, feed,
arc, and spline segments are colour-coded by kind, and `set_color_by_feed`
switches to a colour scale over the feedrate range of the loaded file (tunable
with `set_feed_range` to keep outliers from flattening the scale), leaving moves
without a known feedrate grey. Pan, zoom, orbit, and fit are exposed to the host
page. Extruding moves are drawn as round tubes with a
physical diameter, configurable through `set_extrusion_diameter` (0.2 mm by
default), domed at each move boundary so separate moves stay visually distinct;
rapids and non-extruding moves stay thin lines. The renderer takes a canvas
element and leaves the frame loop to the caller, so a React component only
needs a ref and a `requestAnimationFrame` loop.

```sh
mise install        # provides wasm-pack and node
just wasm-target    # once: rustup target add wasm32-unknown-unknown
just demo           # builds crates/wasm and serves the Vite demo
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

`just wasm-check` lints the crate for the wasm32 target, and `just wasm` builds
the package into `ui/pkg` for the Vite app.

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
