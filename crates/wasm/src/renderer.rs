//! The wasm-facing wgpu renderer.
//!
//! A [`Renderer`] owns the surface, device, and pipelines for one canvas.
//! G-code text is parsed through the core query API, flattened into segments,
//! uploaded to instance buffers, and drawn: non-extruding moves as screen-space
//! expanded quads, extruding moves as round tubes with conical caps and start
//! sockets. The caller drives frames with [`Renderer::render`] and forwards
//! pointer and resize events through the camera methods, which keeps the
//! renderer independent of the page's event model.

use bytemuck::Pod;
use gcode_viewer::MotionRecord;
use glam::{DVec2, DVec3};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::shader_contract::{PathInstance, Uniforms};
use crate::tessellate;
use crate::tube;
use crate::visibility::KindMask;

/// Device-pixel padding kept around the toolpath when fitting the view.
const FIT_MARGIN: f64 = 40.0;
const DEFAULT_LINE_WIDTH: f32 = 1.6;
/// Cross-section diameter of an extruding move in millimetres.
const DEFAULT_EXTRUSION_DIAMETER: f32 = 0.2;
/// Depth format chosen for WebGL2 compatibility; nothing samples it.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.968,
    g: 0.972,
    b: 0.980,
    a: 1.0,
};

/// Summary of the last [`Renderer::load_gcode`] call.
#[wasm_bindgen]
pub struct PathStats {
    moves: usize,
    segments: usize,
    z_min: f64,
    z_max: f64,
    feed_min: f64,
    feed_max: f64,
    has_feed: bool,
}

#[wasm_bindgen]
impl PathStats {
    /// Number of motion records parsed from the file.
    #[wasm_bindgen(getter)]
    pub fn moves(&self) -> usize {
        self.moves
    }

    /// Number of drawable segments after flattening.
    #[wasm_bindgen(getter)]
    pub fn segments(&self) -> usize {
        self.segments
    }

    /// Lowest Z of any located move, in millimetres.
    #[wasm_bindgen(getter)]
    pub fn z_min(&self) -> f64 {
        self.z_min
    }

    /// Highest Z of any located move, in millimetres.
    #[wasm_bindgen(getter)]
    pub fn z_max(&self) -> f64 {
        self.z_max
    }

    /// Lowest known feedrate, in mm/min. Zero when the file carries none.
    #[wasm_bindgen(getter)]
    pub fn feed_min(&self) -> f64 {
        self.feed_min
    }

    /// Highest known feedrate, in mm/min. Zero when the file carries none.
    #[wasm_bindgen(getter)]
    pub fn feed_max(&self) -> f64 {
        self.feed_max
    }

    /// Whether any move carries a feedrate, so the feed legend has a range.
    #[wasm_bindgen(getter)]
    pub fn has_feed(&self) -> bool {
        self.has_feed
    }
}

#[wasm_bindgen]
pub struct Renderer {
    canvas: HtmlCanvasElement,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    travel_path_pipeline: wgpu::RenderPipeline,
    print_path_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    travel_path_buffer: Option<wgpu::Buffer>,
    travel_path_count: u32,
    print_path_buffer: Option<wgpu::Buffer>,
    print_path_count: u32,
    camera: Camera,
    line_width: f32,
    extrusion_radius: f32,
    z_max: f32,
    kind_mask: KindMask,
    /// Colour segments by feedrate instead of move kind.
    color_by_feed: bool,
    /// Bounds of the feedrate colour scale, in mm/min.
    feed_min: f32,
    feed_max: f32,
    bounds: Option<(DVec3, DVec3)>,
    moves: usize,
    segments: usize,
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

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await
            .map_err(js_error)?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("gcode-viewer"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                ..Default::default()
            })
            .await
            .map_err(js_error)?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: Vec::new(),
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        let depth_view = create_depth_view(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader/main.wesl"),
            source: wgpu::ShaderSource::Wgsl(wesl_core::include_wesl!("shader").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<Uniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("paths"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let travel_path_pipeline = create_path_pipeline(
            &device,
            &pipeline_layout,
            &shader,
            format,
            PathPipeline {
                label: "travel_paths",
                vertex_layout: PathInstance::layout(),
                vertex_entry: "vs_travel_path",
                fragment_entry: "fs_travel_path",
            },
        );
        let print_path_pipeline = create_path_pipeline(
            &device,
            &pipeline_layout,
            &shader,
            format,
            PathPipeline {
                label: "print_paths",
                vertex_layout: PathInstance::layout(),
                vertex_entry: "vs_print_path",
                fragment_entry: "fs_print_path",
            },
        );

        let mut camera = Camera::new();
        camera.set_viewport(DVec2::new(width as f64, height as f64));

        Ok(Renderer {
            canvas,
            surface,
            device,
            queue,
            config,
            depth_view,
            travel_path_pipeline,
            print_path_pipeline,
            bind_group,
            camera_buffer,
            travel_path_buffer: None,
            travel_path_count: 0,
            print_path_buffer: None,
            print_path_count: 0,
            camera,
            line_width: DEFAULT_LINE_WIDTH,
            extrusion_radius: DEFAULT_EXTRUSION_DIAMETER * 0.5,
            z_max: f32::INFINITY,
            kind_mask: KindMask::DEFAULT,
            color_by_feed: false,
            feed_min: 0.0,
            feed_max: 1.0,
            bounds: None,
            moves: 0,
            segments: 0,
        })
    }

    /// Replaces the rendered toolpath with `text` and frames it.
    pub fn load_gcode(&mut self, text: &str) -> PathStats {
        let records = gcode_viewer::motions(text);
        let segments = tessellate::segments(&records);
        let feed_range = tessellate::feed_range(&segments);

        self.moves = records.len();
        self.segments = segments.len();
        self.bounds = bounds_of(&records);
        self.z_max = self
            .bounds
            .map(|(_, max)| max.z as f32)
            .unwrap_or(f32::INFINITY);
        (self.feed_min, self.feed_max) = feed_range.unwrap_or((0.0, 1.0));

        let mut travel_paths = Vec::new();
        let mut print_paths = Vec::new();

        for segment in segments {
            let path_instance = PathInstance::from_segment(segment);

            if segment.extruding {
                print_paths.push(path_instance);
            } else {
                travel_paths.push(path_instance);
            }
        }

        self.travel_path_count = travel_paths.len().min(u32::MAX as usize) as u32;
        self.print_path_count = print_paths.len().min(u32::MAX as usize) as u32;
        self.travel_path_buffer = upload(&self.device, "travel_paths", travel_paths);
        self.print_path_buffer = upload(&self.device, "print_paths", print_paths);

        self.fit();

        let (z_min, z_max) = self
            .bounds
            .map(|(min, max)| (min.z, max.z))
            .unwrap_or((0.0, 0.0));
        let (feed_min, feed_max, has_feed) = feed_range
            .map(|(min, max)| (min as f64, max as f64, true))
            .unwrap_or((0.0, 0.0, false));

        PathStats {
            moves: self.moves,
            segments: self.segments,
            z_min,
            z_max,
            feed_min,
            feed_max,
            has_feed,
        }
    }

    /// Draws one frame and presents it.
    pub fn render(&mut self) -> Result<(), JsValue> {
        self.write_camera();

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                self.depth_view = create_depth_view(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return Ok(()),
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("paths"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BACKGROUND),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_bind_group(0, &self.bind_group, &[]);

            if let Some(buffer) = &self.travel_path_buffer {
                pass.set_pipeline(&self.travel_path_pipeline);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..6, 0..self.travel_path_count);
            }

            if let Some(buffer) = &self.print_path_buffer {
                pass.set_pipeline(&self.print_path_pipeline);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..tube::PRINT_PATH_VERTICES, 0..self.print_path_count);
            }
        }

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);

        Ok(())
    }

    /// Reconfigures the surface after the canvas backing size changed.
    pub fn resize(&mut self) {
        let width = self.canvas.width().max(1);
        let height = self.canvas.height().max(1);

        if width == self.config.width && height == self.config.height {
            return;
        }

        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, &self.config);
        self.camera
            .set_viewport(DVec2::new(width as f64, height as f64));
    }

    /// Frames the whole toolpath.
    pub fn fit(&mut self) {
        if let Some((min, max)) = self.bounds {
            self.camera.fit(min, max, FIT_MARGIN);
        }
    }

    /// Drags the view by a pointer delta in device pixels.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.camera.pan(DVec2::new(dx, dy));
    }

    /// Scales the view around a device-pixel position.
    pub fn zoom(&mut self, factor: f64, x: f64, y: f64) {
        self.camera.zoom_at(factor, DVec2::new(x, y));
    }

    /// Orbits the view around the toolpath. Yaw and pitch deltas are in
    /// radians.
    pub fn orbit(&mut self, delta_yaw: f64, delta_pitch: f64) {
        self.camera.orbit(delta_yaw, delta_pitch);
    }

    /// Sets the line width in device pixels.
    pub fn set_line_width(&mut self, width: f64) {
        self.line_width = (width as f32).clamp(0.5, 8.0);
    }

    /// Sets the cross-section diameter of extruding moves in millimetres. The
    /// diameter is physical, so print lines keep their size relative to the
    /// toolpath while zooming.
    pub fn set_extrusion_diameter(&mut self, diameter: f64) {
        self.extrusion_radius = (diameter as f32).clamp(0.01, 10.0) * 0.5;
    }

    /// Hides every move whose highest Z exceeds `z` millimetres. A move is
    /// drawn whole or not at all, so the boundary never cuts a tube or line in
    /// half.
    pub fn set_z_max(&mut self, z: f64) {
        self.z_max = z as f32;
    }

    /// Shows or hides every segment of `kind`, where `kind` is the palette
    /// index: rapid, feed, arc, spline. Out-of-range indices are ignored.
    pub fn set_kind_visible(&mut self, kind: u32, visible: bool) {
        self.kind_mask.set(kind, visible);
    }

    /// Colours segments by their active feedrate when `by_feed` is true,
    /// mapping the configured feedrate range onto a colour scale; otherwise
    /// colours them by move kind. Segments without a feedrate stay grey.
    pub fn set_color_by_feed(&mut self, by_feed: bool) {
        self.color_by_feed = by_feed;
    }

    /// Narrows or widens the feedrate colour scale to `min..=max` mm/min, so
    /// outliers do not flatten the interesting part of the range. Non-finite or
    /// inverted bounds are ignored. Loading a file resets the scale to that
    /// file's feedrate range.
    pub fn set_feed_range(&mut self, min: f64, max: f64) {
        if !min.is_finite() || !max.is_finite() || max <= min {
            return;
        }
        self.feed_min = min as f32;
        self.feed_max = max as f32;
    }

    fn write_camera(&self) {
        let uniforms = Uniforms {
            view_proj: self.camera.view_projection().as_mat4().to_cols_array(),
            viewport: [self.config.width as f32, self.config.height as f32],
            line_width: self.line_width,
            z_max: self.z_max,
            kind_mask: self.kind_mask.bits(),
            color_mode: u32::from(self.color_by_feed),
            feed_min: self.feed_min,
            feed_max: self.feed_max,
            light_dir: self.camera.eye_direction().as_vec3().to_array(),
            extrusion_radius: self.extrusion_radius,
        };

        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}

/// The vertex layout and shader entry points of one path pipeline.
struct PathPipeline<'a> {
    label: &'a str,
    vertex_layout: wgpu::VertexBufferLayout<'static>,
    vertex_entry: &'a str,
    fragment_entry: &'a str,
}

/// Creates one of the instanced path pipelines. All share the camera uniform
/// and the depth state; the vertex layout and shader entry points differ.
fn create_path_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    spec: PathPipeline<'_>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(spec.vertex_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(spec.vertex_layout)],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(spec.fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Uploads one instance per element, or `None` when there is nothing to draw.
fn upload<T: Pod>(device: &wgpu::Device, label: &str, instances: Vec<T>) -> Option<wgpu::Buffer> {
    (!instances.is_empty()).then(|| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        })
    })
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

/// A depth attachment matching the surface size. Depth ordering keeps far
/// toolpath layers from drawing over near ones when the view is orbited.
fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// The XYZ extent of every record with a known endpoint.
fn bounds_of(records: &[MotionRecord]) -> Option<(DVec3, DVec3)> {
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    let mut found = false;

    for point in records.iter().flat_map(|r| [r.start, r.end]).flatten() {
        let point = DVec3::from(point);
        min = min.min(point);
        max = max.max(point);
        found = true;
    }

    found.then_some((min, max))
}
