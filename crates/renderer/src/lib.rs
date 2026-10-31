//! A platform-neutral wgpu renderer for G-code toolpaths.
//!
//! Callers create a wgpu [`wgpu::Instance`] and a surface, then hand them to
//! [`Renderer::new`]. The renderer parses G-code through the core query API,
//! flattens the resulting motion records into line segments, and draws them
//! with an instanced wgpu pipeline. Browser bindings live in the separate
//! `gcode-viewer-wasm` crate.

mod camera;
mod load;
mod renderer;
mod shader_contract;
mod tessellate;
mod tube;
mod visibility;

pub use renderer::{PathStats, Renderer, RendererError, RendererOptions};
