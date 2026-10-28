//! WebAssembly bindings for the G-code toolpath renderer.
//!
//! This crate is only the browser-facing surface: it creates the wgpu instance
//! and canvas surface, forwards calls to the platform-neutral renderer, and
//! converts numbers and errors for JavaScript. The actual rendering lives in
//! `gcode-viewer-renderer`.

#![cfg(target_arch = "wasm32")]

use gcode_viewer_renderer::{
    PathStats as SharedPathStats, Renderer as SharedRenderer, RendererOptions,
};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

/// Summary of the last [`Renderer::load_gcode`] call.
#[wasm_bindgen]
pub struct PathStats {
    inner: SharedPathStats,
}

#[wasm_bindgen]
impl PathStats {
    /// Number of motion records parsed from the file.
    #[wasm_bindgen(getter)]
    pub fn moves(&self) -> usize {
        self.inner.moves
    }

    /// Number of drawable segments after flattening.
    #[wasm_bindgen(getter)]
    pub fn segments(&self) -> usize {
        self.inner.segments
    }

    /// Lowest Z of any located move, in millimetres.
    #[wasm_bindgen(getter)]
    pub fn z_min(&self) -> f64 {
        self.inner.z_min
    }

    /// Highest Z of any located move, in millimetres.
    #[wasm_bindgen(getter)]
    pub fn z_max(&self) -> f64 {
        self.inner.z_max
    }

    /// Lowest known feedrate, in mm/min. Zero when the file carries none.
    #[wasm_bindgen(getter)]
    pub fn feed_min(&self) -> f64 {
        self.inner.feed_min
    }

    /// Highest known feedrate, in mm/min. Zero when the file carries none.
    #[wasm_bindgen(getter)]
    pub fn feed_max(&self) -> f64 {
        self.inner.feed_max
    }

    /// Whether any move carries a feedrate, so the feed legend has a range.
    #[wasm_bindgen(getter)]
    pub fn has_feed(&self) -> bool {
        self.inner.has_feed
    }
}

impl From<SharedPathStats> for PathStats {
    fn from(inner: SharedPathStats) -> Self {
        Self { inner }
    }
}

/// A renderer attached to a canvas.
#[wasm_bindgen]
pub struct Renderer {
    canvas: HtmlCanvasElement,
    inner: SharedRenderer,
}

#[wasm_bindgen]
impl Renderer {
    /// Creates a renderer attached to `canvas`. The canvas backing size must
    /// already be set in device pixels.
    pub async fn create(canvas: HtmlCanvasElement) -> Result<Renderer, JsValue> {
        console_error_panic_hook::set_once();

        let width = canvas.width().max(1);
        let height = canvas.height().max(1);

        let instance = wgpu::util::new_instance_with_webgpu_detection(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        })
        .await;

        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(js_error)?;

        let inner = SharedRenderer::new(
            &instance,
            surface,
            RendererOptions {
                width,
                height,
                limits: wgpu::Limits::downlevel_webgl2_defaults(),
            },
        )
        .await
        .map_err(js_error)?;

        Ok(Renderer { canvas, inner })
    }

    /// Replaces the rendered toolpath with `text` and frames it.
    pub fn load_gcode(&mut self, text: &str) -> PathStats {
        self.inner.load_gcode(text).into()
    }

    /// Draws one frame and presents it.
    pub fn render(&mut self) -> Result<(), JsValue> {
        self.inner.render().map_err(js_error)
    }

    /// Whether the view or toolpath changed since the last presented frame.
    /// The host can skip [`Renderer::render`] until this reports `true`.
    pub fn needs_redraw(&self) -> bool {
        self.inner.needs_redraw()
    }

    /// Reconfigures the surface after the canvas backing size changed.
    pub fn resize(&mut self) {
        self.inner
            .resize(self.canvas.width().max(1), self.canvas.height().max(1));
    }

    /// Frames the whole toolpath.
    pub fn fit(&mut self) {
        self.inner.fit();
    }

    /// Drags the view by a pointer delta in device pixels.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.inner.pan(dx as f32, dy as f32);
    }

    /// Scales the view around a device-pixel position.
    pub fn zoom(&mut self, factor: f64, x: f64, y: f64) {
        self.inner.zoom(factor as f32, x as f32, y as f32);
    }

    /// Orbits the view around the toolpath. Yaw and pitch deltas are in
    /// radians.
    pub fn orbit(&mut self, delta_yaw: f64, delta_pitch: f64) {
        self.inner.orbit(delta_yaw as f32, delta_pitch as f32);
    }

    /// Sets the line width in device pixels.
    pub fn set_line_width(&mut self, width: f64) {
        self.inner.set_line_width(width as f32);
    }

    /// Sets the cross-section diameter of extruding moves in millimetres. The
    /// diameter is physical, so print lines keep their size relative to the
    /// toolpath while zooming.
    pub fn set_extrusion_diameter(&mut self, diameter: f64) {
        self.inner.set_extrusion_diameter(diameter as f32);
    }

    /// Hides every move whose highest Z exceeds `z` millimetres. A move is
    /// drawn whole or not at all, so the boundary never cuts a tube or line in
    /// half.
    pub fn set_z_max(&mut self, z: f64) {
        self.inner.set_z_max(z as f32);
    }

    /// Shows or hides every segment of `kind`, where `kind` is the palette
    /// index: rapid, feed, arc, spline. Out-of-range indices are ignored.
    pub fn set_kind_visible(&mut self, kind: u32, visible: bool) {
        self.inner.set_kind_visible(kind, visible);
    }

    /// Colours segments by their active feedrate when `by_feed` is true,
    /// mapping the configured feedrate range onto a colour scale; otherwise
    /// colours them by move kind. Segments without a feedrate stay grey.
    pub fn set_color_by_feed(&mut self, by_feed: bool) {
        self.inner.set_color_by_feed(by_feed);
    }

    /// Narrows or widens the feedrate colour scale to `min..=max` mm/min, so
    /// outliers do not flatten the interesting part of the range. Non-finite or
    /// inverted bounds are ignored. Loading a file resets the scale to that
    /// file's feedrate range.
    pub fn set_feed_range(&mut self, min: f64, max: f64) {
        self.inner.set_feed_range(min as f32, max as f32);
    }
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
