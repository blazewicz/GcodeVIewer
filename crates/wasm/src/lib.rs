//! WebAssembly bindings: a wgpu renderer for G-code toolpaths.
//!
//! The crate parses G-code through the core query API, flattens the resulting
//! motion records into line segments, and draws them on a canvas with an
//! instanced wgpu pipeline. Only the wasm32 target builds the renderer; the
//! tessellation and camera math are pure and covered by host tests.

#[cfg(any(test, target_arch = "wasm32"))]
mod camera;
#[cfg(any(test, target_arch = "wasm32"))]
mod tessellate;
#[cfg(any(test, target_arch = "wasm32"))]
mod tube;
#[cfg(any(test, target_arch = "wasm32"))]
mod visibility;

#[cfg(target_arch = "wasm32")]
mod renderer;
#[cfg(target_arch = "wasm32")]
mod shader_contract;

#[cfg(target_arch = "wasm32")]
pub use renderer::{PathStats, Renderer};
