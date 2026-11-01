# GcodeViewer task runner. Run `just --list` to see all recipes.

# Show available recipes.
default:
    @just --list

# Format Rust sources.
fmt:
    cargo fmt

# Verify formatting without writing; used in CI.
fmt-check:
    cargo fmt --check

# Lint every crate.
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run the Rust test suite.
test:
    cargo test

# Format, lint and test Rust.
check: fmt clippy test

# Format check and native lint; no Python runtime needed (PyO3 uses its abi3 ABI).
ci-lint:
    cargo fmt --check
    cargo clippy --locked --workspace --all-targets -- -D warnings

# G-code interpreter tests.
ci-gcode:
    cargo test --locked -p gcode-viewer-gcode

# Renderer crate: native lint plus host-side tests.
ci-renderer:
    cargo clippy --locked -p gcode-viewer-renderer --all-targets -- -D warnings
    cargo test --locked -p gcode-viewer-renderer

# Native viewer: host-side tests.
ci-native:
    cargo test --locked -p gcode-viewer-native

# WebAssembly bindings: wasm32 lint for the wrapper and renderer.
ci-wasm:
    cargo clippy --locked -p gcode-viewer-renderer --target wasm32-unknown-unknown --all-targets -- -D warnings
    cargo clippy --locked -p gcode-viewer-wasm --target wasm32-unknown-unknown --all-targets -- -D warnings

# UI bundle: WASM plus Vite build with the SEO metadata checks.
ci-ui: ui-build

# Full local mirror of .github/workflows/test.yml.
ci: ci-lint ci-gcode ci-renderer ci-native ci-wasm ci-ui

# Install the wasm32 target needed by the renderer checks.
wasm-target:
    rustup target add wasm32-unknown-unknown

# Lint the renderer and WASM bindings against the wasm32 target.
wasm-check:
    cargo clippy -p gcode-viewer-renderer --target wasm32-unknown-unknown --all-targets
    cargo clippy -p gcode-viewer-wasm --target wasm32-unknown-unknown --all-targets

# Build the WASM binding package for the Vite demo.
wasm:
    wasm-pack build crates/wasm --profile wasm-release --target web --out-dir ../../ui/pkg

# Build the renderer and run the Vite demo at http://localhost:5173.
ui: wasm
    npm --prefix ui install
    npm --prefix ui run dev

# Build the WASM binding and the Vite bundle into ui/dist.
ui-bundle: wasm
    npm --prefix ui ci
    npm --prefix ui run build

# Build the deployable UI bundle (WASM + static site) and verify its SIMD
# instructions and SEO metadata.
ui-build: ui-bundle
    npm --prefix ui run check:wasm-simd
    npm --prefix ui run check:seo

# Verify the built UI bundle's SEO metadata (ui-build runs this too).
ui-seo-check:
    npm --prefix ui run check:seo

# Verify the built wasm binary uses SIMD instructions (ui-build runs this too).
ui-simd-check:
    npm --prefix ui run check:wasm-simd

# Regenerate og.png and the PNG icons from the built UI; downloads Chromium on first run.
og-image: ui-bundle
    npm --prefix ui install --no-save playwright
    npm --prefix ui exec -- playwright install chromium
    npm --prefix ui run og-image
    npm --prefix ui run check:seo

# Open a G-code file in the native viewer window.
view file:
    cargo run --release -p gcode-viewer-native -- {{file}}
