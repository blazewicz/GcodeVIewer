# GcodeViewer

Browser-based viewer with limited editing functionality for G-code.

Core and renderer built with Rust using wgpu; the browser binding is compiled to WASM.
UI built with TypeScript, managed with vite.

Currently supported G-code flavors:
* [Marlin Firmware](https://marlinfw.org/docs/gcode/)

## Project structure

```
.github/         GitHub Actions CI config
crates/gcode/    G-code interpreter
crates/native/   Native window viewer (winit host)
crates/renderer/ Platform-neutral wgpu renderer, shaders, camera and tessellation
crates/wasm/     Thin WebAssembly binding around the renderer
ui/              Browser UI
zzz_artifacts/   Local artifacts - never committed to the repo
AGENTS.md        This file
Cargo.toml       Rust workspace config file
Justfile         Just commands definitions
mise.toml        Tool versions
README.md        User facing readme file
```

## Commands

Tasks live in the `Justfile`; run `just` to list them. Prefer these over ad-hoc
commands so local and CI runs stay in sync.

```sh
# Rust code checks
just check        # format, lint and test Rust
just fmt          # format Rust code
just clippy       # lint Rust code
just test         # run Rust tests

# Native viewer
just view FILE    # open a G-code file in the native viewer window

# WASM and UI specific commands
just wasm         # build the WASM binding package
just wasm-check   # lint the renderer and WASM binding for wasm32
just ui           # run the UI dev server

# Commands used in CI runs
just ci           # every ci-* recipe below, mirroring .github/workflows/test.yml
just ci-lint      # format + native clippy; no Python runtime needed
just ci-core      # core library tests
just ci-renderer  # native renderer lint and host-side tests
just ci-native    # native viewer host-side tests
just ci-wasm      # wasm32 clippy for the renderer and binding
```

## Writing code and docs

Write files as standalone artifacts without leaving references to the conversation they
originated from. If some idea was discussed and rejected it must be omitted. Never leave
comments like "We do not do X.", unless X is otherwise implied by the context of the
file. Always assume that the files you write will be viewed by someone who did not
partake in the process of developing them.

Write clean and idiomatic code in any language you use. Use modern syntax and tools,
introduce de-factor standard 3rd party libraries freely (e.x. tokio, rayon, clap, vite).
Test code in every layer, create integration tests on the boundaries where different
components interact with each other. Always use linters and code formatters on generated
code.
