# GcodeWelder task runner. Run `just --list` to see all recipes.

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
ci-core:
    cargo test --locked -p gcode-viewer-gcode

# Renderer crate: native lint plus host-side tests.
ci-renderer:
    cargo clippy --locked -p gcode-viewer-renderer --all-targets -- -D warnings
    cargo test --locked -p gcode-viewer-renderer

# WebAssembly bindings: wasm32 lint for the wrapper and renderer.
ci-wasm:
    cargo clippy --locked -p gcode-viewer-renderer --target wasm32-unknown-unknown --all-targets -- -D warnings
    cargo clippy --locked -p gcode-viewer-wasm --target wasm32-unknown-unknown --all-targets -- -D warnings

# Full local mirror of .github/workflows/test.yml.
ci: ci-lint ci-core ci-renderer ci-wasm

# Install the wasm32 target needed by the renderer checks.
wasm-target:
    rustup target add wasm32-unknown-unknown

# Lint the renderer and WASM bindings against the wasm32 target.
wasm-check:
    cargo clippy -p gcode-viewer-renderer --target wasm32-unknown-unknown --all-targets
    cargo clippy -p gcode-viewer-wasm --target wasm32-unknown-unknown --all-targets

# Build the WASM binding package for the Vite demo.
wasm:
    wasm-pack build crates/wasm --target web --out-dir ../../ui/pkg

# Build the renderer and run the Vite demo at http://localhost:5173.
ui: wasm
    npm --prefix ui install
    npm --prefix ui run dev
