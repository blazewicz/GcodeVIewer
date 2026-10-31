---
name: wesl
description: Use when writing, refactoring, reasoning about or reviewing shader files, WGSL files, WESL files. This skill will tell you all that you need to known about writing WESL (.wesl) files and loading them in Rust code.
---

# WESL essentials

WESL (WebGPU Extended Shading Language) is a strict superset of WGSL that adds a
module system (`import`), conditional compilation (`@if`), and shader packages.
Anything valid in WGSL is valid WESL, and the output of a WESL linker is plain
WGSL that any WebGPU implementation (browser, wgpu, naga) consumes unchanged.
The linker performs compile-time assembly; it does not change runtime semantics.

Facts to anchor on:

- Current edition: `2026_pre`.
- File extension: `.wesl`. `.wgsl` files are also accepted, and linkers fall back
  to a `.wgsl` file when the matching `.wesl` file is missing, so a tree can be
  migrated file by file and can mix both extensions.
- Rust implementation: `wesl` (the compiler, 0.6.x, MSRV 1.97.1, edition 2024),
  `wesl-core` (tiny, dependency-free crate with the `include_wesl!` macro and
  package types), and `wesl-cli` (standalone linker binary).
- JavaScript implementation: `wesl` npm package plus bundler plugins in
  `wesl-plugin` (Vite, Webpack, Rollup, esbuild, rspack, ...).
- WESL extensions are active proposals for future WGSL. Adopting WESL now is
  cheap because it is a superset and expensive preprocessors can be deleted.
- Everything still gets validated and compiled by naga/wgpu/browsers exactly as
  before. WESL is a front-end, not a replacement.

## WESL vs WGSL at a glance

| Capability | WGSL | WESL |
| --- | --- | --- |
| Single-file shader | yes | yes (strict superset) |
| Multi-file shader | string concatenation, custom preprocessors | `import`, module graph visible to tools |
| Shader variants | host string interpolation | `@if` / `@elif` / `@else` feature flags |
| Host-injected values | string templating | `constants::` (experimental) |
| Distribution | copy-paste | packages on npm and crates.io |
| Tooling | no module awareness | wgsl-analyzer, wgsl-test, wgsl-studio, wgsl-play, wgsl-edit |

## Language extensions

### Imports

```wgsl
// Single item from a local module.
import package::colors::chartreuse;

// Multiple items, nested groups allowed.
import package::animals::{ bird, mammals::{ dog, cat } };

// External package installed via npm or cargo.
import random_wgsl::pcg_2u_3f;

// A whole parent module.
import super::util;
```

Special path prefixes:

| Prefix | Meaning |
| --- | --- |
| `package::` | absolute path from the package root |
| `super::` | one level up the module hierarchy; can be stacked |
| `constants::` | host-injected constants provided at link time |

Rules and behavior:

- All `import` statements must appear before any declarations in a file.
- Wildcard imports (`import package::utils::*;`) are specified but not
  implemented in shipped linkers; import items by name.
- Imports are lazy: importing a symbol does not include it; it must be
  referenced transitively from the root module.
- Transitive imports are automatic. If A imports B and B uses C, A does not
  need to import C.
- Modules can be referenced inline without an import:
  `bevy_ui::quad(...)`, `super::util::foo()`.
- An import path may end at a module (`import super::util;`) or at a
  declaration inside a module (`import package::utils::helper::rgb2hsv;`).
- External package names are sanitized into module prefixes: hyphens become
  underscores, `@` is dropped, `/` becomes `__`
  (`@foobar/shader-utils` → `foobar__shader_utils::`).

### Conditional compilation

```wgsl
@if(DEBUG)
color = chartreuse;

struct Ray {
  position: vec4f,
  direction: vec4f,
  @if(debug_mode && raytracing_enabled)
  ray_steps: u32,
}

@if(A) {
  ...
} @elif(B) {
  ...
} @else {
  ...
}
```

- Boolean operators: `&&`, `||`, `!`; literals `true`/`false`; feature flag names.
- Conditions are supplied by the host at link time. Unused branches are stripped
  from the output.
- This replaces most string interpolation used to build shader variants, e.g.
  optional bindings, struct fields, and declarations.

### Host-injected constants (`constants::`)

`constants::` is an experimental mechanism for values injected by the host at
link time. Each constant becomes a WGSL `const` declaration in the output, so it
can appear where only const-expressions are allowed, including attributes:

```wgsl
@group(0) @binding(constants::slot) var<uniform> u: f32;

fn size() -> f32 { return constants::size; }
```

In Rust, values come from `CompileOptions::constants`; in JavaScript they are
passed to `link({ ..., constants: { NumLights: 7 } })`. Numbers are emitted
directly; strings are emitted verbatim as WGSL expressions.

### Visibility (specified, not implemented)

The spec defines `public`, package (default), and `private` visibility levels,
plus re-exports. Shipped linkers do not accept these keywords yet. Do not emit
`public`/`private` or `public import`; unmarked declarations are visible
throughout the package, which is what current linkers implement.

### Packages

Shader libraries are distributed as packages: npm packages for JS consumers and
crates for Rust consumers. Package contents are plain WESL modules plus a
manifest. Consumers add dependencies in `build.rs` (Rust) or let the JS linker
autodetect packages from `node_modules`.

## Module paths and files

The `root` directory is the base. Files map to module paths with `::` as the
separator:

```
shaders/lighting.wesl        → package::lighting
shaders/utils/helper.wesl    → package::utils::helper
shaders/sun-atmo.wesl        → package::sun_atmo
```

- Hyphens and `@` are not valid WESL identifiers; file-name hyphens become
  underscores in the module path.
- The final segment of an import path names a declaration; the segments before
  it name the module file.
- The package root module is the file `package.wesl` in the root directory.

## Rust integration

### Build-time linking (recommended for wgpu)

Add the compiler as a build dependency and a tiny support crate as a runtime
dependency for the include macro:

```toml
[build-dependencies]
wesl = "0.6"

[dependencies]
wesl-core = "0.1"
```

Create `build.rs`:

```rust
fn main() {
    let shader = wesl::Compiler::default()
        .compile("src/shader.wesl")
        .inspect_err(|error| eprintln!("{error}"))
        .expect("shader compilation failed");

    shader.emit_rerun_if_changed();
    shader.write_artifact("shader");
}
```

- `emit_rerun_if_changed()` emits `cargo:rerun-if-changed` for every module
  involved in the compilation, so the script reruns only when a shader changes.
- `write_artifact("shader")` writes `$OUT_DIR/shader.wgsl`. The name must not
  contain path separators or an extension; `write_artifact` panics otherwise.
- The compiler crate is host-only (build dependency), so the wasm target never
  compiles it.

Embed the artifact in the renderer:

```rust
let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("shader.wesl"),
    source: wgpu::ShaderSource::Wgsl(wesl_core::include_wesl!("shader").into()),
});
```

`wesl_core::include_wesl!("shader")` is a convenience macro that expands to:

```rust
include_str!(concat!(env!("OUT_DIR"), "/", "shader", ".wgsl"))
```

`wgpu::include_wgsl!` cannot be used for the artifact because it takes a literal
path relative to the source file, not an `OUT_DIR` path.

Build-time linking means the shipped binary/wasm contains only the compiled WGSL
string; no WESL compiler code is linked in.

### Path and module selection

`Compiler::compile(path)` accepts a `wesl.toml` file, a directory, or a shader
file. The main module (the one exposing entry points, bindings, and overrides)
is chosen as follows:

| `path` points to | Package root | Main module |
| --- | --- | --- |
| `wesl.toml` | `root` field | `package.wesl` in root |
| a directory | that directory | `package.wesl` in root |
| a `.wesl`/`.wgsl` file | its parent directory | the file itself |

`.wgsl` extensions are supported, but `.wesl` takes priority. To pick a
different main module, use `Compiler::compile_module(root_dir, &module_path)`
with a `wesl::syntax::ModulePath`, for example `"package::shader".parse()`.

### Runtime linking

The same `Compiler` API works at runtime; `CompileResult` implements `Display`
and `to_string()` yields the linked WGSL. `CompileResult::eval`/`exec` (behind
the `eval` feature) can execute shader code on the CPU for early validation.
Linking is fast enough for interactive use.

### `CompileOptions`

Fields worth knowing:

| Field | Effect |
| --- | --- |
| `imports` | toggle WESL imports; disabling removes import statements silently |
| `condcomp` | toggle `@if` conditional compilation |
| `features` | feature flags used by `@if` |
| `constants` | values for `constants::` |
| `dependencies` | external WESL packages (cargo) |
| `keep` | list of main-module declarations to keep; `None` keeps only entry points and their dependencies |
| `keep_main` | keep all declarations of the main module |
| `mangle_main` | mangle names in the main module (off by default) |
| `mangler` | mangling scheme |
| `sourcemap` | emit a source map for debugging |
| `lower` | early evaluation of const-expressions (experimental, needs `eval`) |

Cargo features of the `wesl` crate (all opt-in): `generics` (experimental
type-generators and generic functions), `package` (build shader libraries),
`eval` (CPU evaluation, `@const`), `naga-ext` (Naga/WGPU extensions), `serde`.

### CLI

```sh
cargo install wesl-cli
wesl compile path/to/shader.wesl > generated.wgsl
```

Useful for inspecting linked output and for build systems without Rust code.

## JavaScript integration (context)

For projects that build shaders through a bundler, `wesl-plugin` offers two
import modes:

```ts
import wgsl from "./shaders/app.wesl?static";          // linked at build time, plain WGSL string
import appWesl from "./shaders/app.wesl?link";         // runtime linking descriptor
```

- `?static` bakes conditions at build time (`"./app.wesl MOBILE=true ?static"`)
  and adds no runtime dependency.
- `?link` requires the `wesl` runtime and supports changing conditions at
  runtime: `link({ ...appWesl, conditions: { DEBUG: false } })`, then
  `linked.dest` for the WGSL string or `linked.createShaderModule(device, {})`.
- Without a bundler, `link({ weslSrc: { "main.wesl": source }, rootModuleName, conditions, constants })`
  works in browsers, workers, Node.js, and Deno.

## `wesl.toml`

Optional for applications; tooling works without it using defaults. When
present, it is placed at the project root. All fields are optional except
`edition`:

```toml
edition = "2026_pre"
package-manager = "cargo"           # or "npm"; inferred when omitted
root = "./src"                      # base directory for module resolution
include = ["src/**/*.wesl"]         # shader files to process
exclude = []                        # globs to ignore
```

Shader library dependencies are not listed in `wesl.toml`; the format for them
is still changing and is not implemented. Rust projects declare packages in
`build.rs` (`CompileOptions::dependencies`); JS projects get them autodetected
from `node_modules`.

## Linker behavior and gotchas

- Dead code elimination is on: by default only entry points and their transitive
  dependencies are kept. `CompileOptions::keep` and `keep_main` override this.
- Name mangling: the root module's declarations keep their names by default.
  The Rust linker mangles all non-root-module declarations
  (`package_utils_helper` instead of `helper`); the JS linker mangles only
  conflicting names. Entry point names are stable.
- The linker re-emits WGSL from its syntax tree. Formatting is normalized and
  comments are dropped, so do not parse the linked output for exact source text.
  Tests that pin source-level constants should read the `.wesl` source; tests
  that validate semantics should parse the linked artifact.
- Imports must appear before declarations.
- `enable` directives are not inherited: if any module uses `enable f16;`, the
  root module must declare it too.
- Module-scope `const_assert` is only included in the output if something from
  that module is referenced.
- `super::` climbs one level; `package::` is absolute from the root. Most code
  uses `package::`.
- Incremental migration works: linkers fall back to `.wgsl` when a `.wesl` file
  is missing, so a project can mix extensions during a transition.

## Ecosystem tools

| Tool | What it does |
| --- | --- |
| wgsl-analyzer | language server with go-to-definition and diagnostics across the module graph (forthcoming) |
| wgsl-test | GPU shader tests with `@test`, CLI runner and Vitest integration |
| wgsl-studio | VS Code extension with test runner and shader preview |
| wgsl-play | web component for live WESL rendering with built-in uniforms (`env::u`) |
| wgsl-edit | CodeMirror web component for editing/linting WESL in web apps |
| lezer-wesl | Lezer grammar for WESL syntax highlighting |
