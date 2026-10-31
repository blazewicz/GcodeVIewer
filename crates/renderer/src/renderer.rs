//! The platform-neutral wgpu renderer.
//!
//! A [`Renderer`] owns the surface, device, and pipelines for one canvas or
//! window. G-code text is parsed through the core query API, flattened into
//! segments, uploaded to instance buffers, and drawn: non-extruding moves as
//! screen-space expanded quads, extruding moves as round tubes with conical
//! caps and start sockets. A load can be fed in chunks through
//! [`Renderer::begin_load`], [`Renderer::feed_load`], and
//! [`Renderer::end_load`], so the host never has to hold the whole file. The
//! caller draws a frame with [`Renderer::render`] when
//! [`Renderer::needs_redraw`] reports that the image changed, and forwards
//! pointer and resize events through the camera methods, which keeps the
//! renderer independent of the host's event model.

use bytemuck::Pod;
use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::load::Loader;
use crate::shader_contract::{PathInstance, Uniforms};
use crate::tessellate::Segment;
use crate::tube;
use crate::visibility::KindMask;

/// Device-pixel padding kept around the toolpath when fitting the view.
const FIT_MARGIN: f32 = 40.0;
const DEFAULT_LINE_WIDTH: f32 = 1.6;
/// Cross-section diameter of an extruding move in millimetres.
const DEFAULT_EXTRUSION_DIAMETER: f32 = 0.2;
/// Staged instances that trigger an upload. Keeps the host-side staging
/// buffer small while leaving the draw loop with few vertex buffers.
const INSTANCES_PER_BATCH: usize = 1 << 18;
/// Depth format chosen for WebGL2 compatibility; nothing samples it.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.968,
    g: 0.972,
    b: 0.980,
    a: 1.0,
};

/// Initial surface size and device limits for a [`Renderer`].
#[derive(Clone, Debug)]
pub struct RendererOptions {
    /// Surface width in device pixels.
    pub width: u32,
    /// Surface height in device pixels.
    pub height: u32,
    /// Device limits to request. The caller chooses these because they depend
    /// on the target; `wgpu::Limits::default()` is a portable baseline.
    pub limits: wgpu::Limits,
}

/// Errors that can occur while creating a renderer.
#[derive(Debug, thiserror::Error)]
pub enum RendererError {
    /// No graphics adapter compatible with the surface was found.
    #[error("no suitable graphics adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    /// The adapter could not provide a device and queue.
    #[error("failed to create graphics device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
}

/// Summary of the last completed load.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathStats {
    /// Number of motion records parsed from the file.
    pub moves: usize,
    /// Number of drawable segments after flattening.
    pub segments: usize,
    /// Lowest Z of any located move, in millimetres.
    pub z_min: f64,
    /// Highest Z of any located move, in millimetres.
    pub z_max: f64,
    /// Lowest known feedrate, in mm/min. Zero when the file carries none.
    pub feed_min: f64,
    /// Highest known feedrate, in mm/min. Zero when the file carries none.
    pub feed_max: f64,
    /// Whether any move carries a feedrate, so the feed legend has a range.
    pub has_feed: bool,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_view: wgpu::TextureView,
    travel_path_pipeline: wgpu::RenderPipeline,
    print_path_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    travel_path_batches: Vec<PathBatch>,
    print_path_batches: Vec<PathBatch>,
    /// Instances of the current load not yet uploaded.
    travel_staging: Vec<PathInstance>,
    print_staging: Vec<PathInstance>,
    /// The load in progress, if any.
    load: Option<Loader>,
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
    bounds: Option<(Vec3, Vec3)>,
    moves: usize,
    segments: usize,
    /// Whether the image changed since the last presented frame.
    needs_redraw: bool,
}

impl Renderer {
    /// Creates a renderer for `surface`, whose backing size must already be set
    /// in device pixels.
    pub async fn new(
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        options: RendererOptions,
    ) -> Result<Renderer, RendererError> {
        let width = options.width.max(1);
        let height = options.height.max(1);

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await?;

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("gcode-viewer"),
                required_features: wgpu::Features::empty(),
                required_limits: options.limits.using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                ..Default::default()
            })
            .await?;

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
        camera.set_viewport(Vec2::new(width as f32, height as f32));

        Ok(Renderer {
            surface,
            device,
            queue,
            config,
            depth_view,
            travel_path_pipeline,
            print_path_pipeline,
            bind_group,
            camera_buffer,
            travel_path_batches: Vec::new(),
            print_path_batches: Vec::new(),
            travel_staging: Vec::new(),
            print_staging: Vec::new(),
            load: None,
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
            needs_redraw: true,
        })
    }

    /// Replaces the rendered toolpath with `text` and frames it. Hosts that
    /// cannot hand over the whole file at once use the
    /// [begin](Renderer::begin_load), [feed](Renderer::feed_load), and
    /// [end](Renderer::end_load) sequence instead.
    pub fn load_gcode(&mut self, text: &str) -> PathStats {
        self.begin_load();
        self.feed_load(text);
        self.end_load()
            .expect("a load begun by begin_load always completes")
    }

    /// Starts a chunked load, replacing any load already in progress.
    ///
    /// The instance buffers of the previous toolpath are released right away,
    /// but no redraw is requested: the last presented image stays on screen
    /// until [`Renderer::end_load`] uploads the new toolpath.
    pub fn begin_load(&mut self) {
        self.load = Some(Loader::new());
        self.travel_path_batches.clear();
        self.print_path_batches.clear();
        self.travel_staging.clear();
        self.print_staging.clear();
        self.moves = 0;
        self.segments = 0;
        self.bounds = None;
        self.feed_min = 0.0;
        self.feed_max = 1.0;
    }

    /// Feeds the next chunk of G-code text into the active load, flattening
    /// and staging its segments for the GPU. Returns `false` when no load is
    /// active, leaving `text` unconsumed.
    pub fn feed_load(&mut self, text: &str) -> bool {
        let Some(load) = self.load.as_mut() else {
            return false;
        };

        let device = &self.device;
        let (travel_staging, print_staging) = (&mut self.travel_staging, &mut self.print_staging);
        let (travel_batches, print_batches) =
            (&mut self.travel_path_batches, &mut self.print_path_batches);

        for &segment in load.feed(text) {
            stage(segment, travel_staging, print_staging);

            // A chunk of long arcs can flatten to far more segments than the
            // staging limit, so the check runs per segment, not per chunk.
            if travel_staging.len() >= INSTANCES_PER_BATCH {
                flush(device, "travel_paths", travel_staging, travel_batches);
            }
            if print_staging.len() >= INSTANCES_PER_BATCH {
                flush(device, "print_paths", print_staging, print_batches);
            }
        }

        true
    }

    /// Ends the active load: uploads the remaining staged instances, frames
    /// the toolpath, and reports the load summary. Returns `None` when no load
    /// is active.
    pub fn end_load(&mut self) -> Option<PathStats> {
        let mut load = self.load.take()?;

        {
            let (travel_staging, print_staging) =
                (&mut self.travel_staging, &mut self.print_staging);

            for &segment in load.finish() {
                stage(segment, travel_staging, print_staging);
            }
        }

        flush(
            &self.device,
            "travel_paths",
            &mut self.travel_staging,
            &mut self.travel_path_batches,
        );
        flush(
            &self.device,
            "print_paths",
            &mut self.print_staging,
            &mut self.print_path_batches,
        );

        let stats = load.stats();
        self.moves = stats.moves;
        self.segments = stats.segments;
        self.bounds = stats.bounds;
        self.z_max = self.bounds.map(|(_, max)| max.z).unwrap_or(f32::INFINITY);
        (self.feed_min, self.feed_max) = stats.feed_range.unwrap_or((0.0, 1.0));
        self.needs_redraw = true;

        self.fit();

        let (z_min, z_max) = self
            .bounds
            .map(|(min, max)| (min.z as f64, max.z as f64))
            .unwrap_or((0.0, 0.0));
        let (feed_min, feed_max, has_feed) = stats
            .feed_range
            .map(|(min, max)| (min as f64, max as f64, true))
            .unwrap_or((0.0, 0.0, false));

        Some(PathStats {
            moves: self.moves,
            segments: self.segments,
            z_min,
            z_max,
            feed_min,
            feed_max,
            has_feed,
        })
    }

    /// Whether the view or toolpath changed since the last presented frame.
    /// Redrawing an unchanged image is pure waste, so the caller can skip
    /// [`Renderer::render`] until this reports `true`.
    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    /// Draws one frame and presents it.
    pub fn render(&mut self) -> Result<(), RendererError> {
        // A stale swap chain is reconfigured and retried once; the retry keeps
        // the caller from seeing an empty frame right after a resize.
        let mut retried = false;
        let frame = loop {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => break frame,
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost
                    if !retried =>
                {
                    retried = true;
                    self.recreate_swapchain();
                }
                wgpu::CurrentSurfaceTexture::Outdated
                | wgpu::CurrentSurfaceTexture::Lost
                | wgpu::CurrentSurfaceTexture::Timeout
                | wgpu::CurrentSurfaceTexture::Occluded
                | wgpu::CurrentSurfaceTexture::Validation => return Ok(()),
            }
        };

        self.write_camera();

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

            if !self.travel_path_batches.is_empty() {
                pass.set_pipeline(&self.travel_path_pipeline);

                for batch in &self.travel_path_batches {
                    pass.set_vertex_buffer(0, batch.buffer.slice(..));
                    pass.draw(0..6, 0..batch.count);
                }
            }

            if !self.print_path_batches.is_empty() {
                pass.set_pipeline(&self.print_path_pipeline);

                for batch in &self.print_path_batches {
                    pass.set_vertex_buffer(0, batch.buffer.slice(..));
                    pass.draw(0..tube::PRINT_PATH_VERTICES, 0..batch.count);
                }
            }
        }

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        // Only clear the flag once the frame is on screen: every early return
        // above leaves it set so the caller retries the draw.
        self.needs_redraw = false;

        Ok(())
    }

    /// Reconfigures the surface after the backing size changed. The size is in
    /// device pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);

        if width == self.config.width && height == self.config.height {
            return;
        }

        self.needs_redraw = true;
        self.config.width = width;
        self.config.height = height;
        self.recreate_swapchain();
        self.camera
            .set_viewport(Vec2::new(width as f32, height as f32));
    }

    /// Frames the whole toolpath.
    pub fn fit(&mut self) {
        if let Some((min, max)) = self.bounds {
            self.needs_redraw = true;
            self.camera.fit(min, max, FIT_MARGIN);
        }
    }

    /// Drags the view by a pointer delta in device pixels.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.needs_redraw = true;
        self.camera.pan(Vec2::new(dx, dy));
    }

    /// Scales the view around a device-pixel position.
    pub fn zoom(&mut self, factor: f32, x: f32, y: f32) {
        self.needs_redraw = true;
        self.camera.zoom_at(factor, Vec2::new(x, y));
    }

    /// Orbits the view around the toolpath. Yaw and pitch deltas are in
    /// radians.
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.needs_redraw = true;
        self.camera.orbit(delta_yaw, delta_pitch);
    }

    /// Sets the line width in device pixels.
    pub fn set_line_width(&mut self, width: f32) {
        self.line_width = width.clamp(0.5, 8.0);
        self.needs_redraw = true;
    }

    /// Sets the cross-section diameter of extruding moves in millimetres. The
    /// diameter is physical, so print lines keep their size relative to the
    /// toolpath while zooming.
    pub fn set_extrusion_diameter(&mut self, diameter: f32) {
        self.extrusion_radius = diameter.clamp(0.01, 10.0) * 0.5;
        self.needs_redraw = true;
    }

    /// Hides every move whose highest Z exceeds `z` millimetres. A move is
    /// drawn whole or not at all, so the boundary never cuts a tube or line in
    /// half.
    pub fn set_z_max(&mut self, z: f32) {
        self.z_max = z;
        self.needs_redraw = true;
    }

    /// Shows or hides every segment of `kind`, where `kind` is the palette
    /// index: rapid, feed, arc, spline. Out-of-range indices are ignored.
    pub fn set_kind_visible(&mut self, kind: u32, visible: bool) {
        self.kind_mask.set(kind, visible);
        self.needs_redraw = true;
    }

    /// Colours segments by their active feedrate when `by_feed` is true,
    /// mapping the configured feedrate range onto a colour scale; otherwise
    /// colours them by move kind. Segments without a feedrate stay grey.
    pub fn set_color_by_feed(&mut self, by_feed: bool) {
        self.color_by_feed = by_feed;
        self.needs_redraw = true;
    }

    /// Narrows or widens the feedrate colour scale to `min..=max` mm/min, so
    /// outliers do not flatten the interesting part of the range. Non-finite or
    /// inverted bounds are ignored. Loading a file resets the scale to that
    /// file's feedrate range.
    pub fn set_feed_range(&mut self, min: f32, max: f32) {
        if !min.is_finite() || !max.is_finite() || max <= min {
            return;
        }
        self.feed_min = min;
        self.feed_max = max;
        self.needs_redraw = true;
    }

    fn write_camera(&self) {
        let uniforms = Uniforms {
            view_proj: self.camera.view_projection().to_cols_array(),
            viewport: [self.config.width as f32, self.config.height as f32],
            line_width: self.line_width,
            z_max: self.z_max,
            kind_mask: self.kind_mask.bits(),
            color_mode: u32::from(self.color_by_feed),
            feed_min: self.feed_min,
            feed_max: self.feed_max,
            light_dir: self.camera.eye_direction().to_array(),
            extrusion_radius: self.extrusion_radius,
        };

        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniforms));
    }

    fn recreate_swapchain(&mut self) {
        self.surface.configure(&self.device, &self.config);
        self.depth_view = create_depth_view(&self.device, &self.config);
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

/// One uploaded vertex buffer and the number of instances it holds.
struct PathBatch {
    buffer: wgpu::Buffer,
    count: u32,
}

/// Files one flattened segment under the pipeline that draws it.
fn stage(segment: Segment, travel: &mut Vec<PathInstance>, print: &mut Vec<PathInstance>) {
    let instance = PathInstance::from_segment(segment);

    if segment.extruding {
        print.push(instance);
    } else {
        travel.push(instance);
    }
}

/// Uploads the staged instances as one buffer, leaving the staging vector
/// empty. Does nothing when there is nothing staged.
fn flush(
    device: &wgpu::Device,
    label: &str,
    staging: &mut Vec<PathInstance>,
    batches: &mut Vec<PathBatch>,
) {
    if staging.is_empty() {
        return;
    }

    let instances = std::mem::take(staging);
    let count = instances.len().min(u32::MAX as usize) as u32;

    batches.push(PathBatch {
        buffer: upload(device, label, &instances),
        count,
    });
}

/// Uploads one instance per element into a fresh vertex buffer.
fn upload<T: Pod>(device: &wgpu::Device, label: &str, instances: &[T]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(instances),
        usage: wgpu::BufferUsages::VERTEX,
    })
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
