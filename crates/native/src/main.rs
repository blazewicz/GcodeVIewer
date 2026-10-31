//! A minimal native window viewer for G-code files.
//!
//! `gcode-viewer <FILE>` opens a window and draws the toolpath of `FILE` with
//! the platform-neutral renderer. There is no chrome around the image: the
//! camera is driven entirely by mouse and keyboard, mirroring the controls of
//! the browser demo.

mod load;

use std::ffi::OsString;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use gcode_viewer_renderer::{PathStats, Renderer, RendererOptions};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

/// The binary name the user typed, used in messages and the usage text.
const BIN: &str = env!("CARGO_BIN_NAME");
/// Initial window size in logical pixels.
const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 800.0);
/// Fraction of the toolpath height a coarse Z-clip press covers.
const Z_CLIP_COARSE_FRACTION: f32 = 0.05;
/// Fine Z-clip step in millimetres, roughly a print layer.
const Z_CLIP_FINE_STEP: f32 = 0.1;
/// Device pixels one wheel line is worth; a browser reports about 100 px per
/// wheel notch.
const WHEEL_LINE_PIXELS: f64 = 100.0;
/// Largest wheel travel honoured per event, in device pixels. A Wayland
/// compositor may report a notch as 120 discrete steps, which without the cap
/// would zoom the view away in a single event.
const WHEEL_MAX_PIXELS: f64 = 300.0;
/// Zoom per device pixel of wheel travel, matching the browser demo.
const WHEEL_ZOOM_PER_PIXEL: f64 = 0.001;
/// Orbit radians per device pixel of drag, matching the browser demo.
const ORBIT_RADIANS_PER_PIXEL: f32 = 0.01;

fn main() -> ExitCode {
    env_logger::init();

    let path = match parse_args(std::env::args_os().skip(1)) {
        Cli::Help => {
            println!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Cli::Error(message) => {
            eprintln!("{BIN}: {message}\n\n{}", usage());
            return ExitCode::from(2);
        }
        Cli::View(path) => path,
    };

    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{BIN}: cannot open {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
    };

    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            eprintln!("{BIN}: cannot create the event loop: {error}");
            return ExitCode::FAILURE;
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new(path, file);

    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("{BIN}: {error}");
        return ExitCode::FAILURE;
    }

    if let Some(fatal) = app.fatal {
        eprintln!("{BIN}: {fatal}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// The `--help` text and the tail of every command line error.
fn usage() -> String {
    format!(
        "usage: {BIN} <FILE>

Renders the toolpath of <FILE> in a window.

Mouse: left-drag pans, middle-drag or Ctrl+drag orbits, the wheel zooms.
Keys:  F fits the toolpath, Up/Down move the Z clip (Shift for a fine step),
       Z resets it, Esc quits."
    )
}

/// One command line invocation.
#[derive(Debug, PartialEq)]
enum Cli {
    /// Print the usage text and exit.
    Help,
    /// Open this file.
    View(PathBuf),
    /// Report the message and the usage text, then fail.
    Error(String),
}

/// Interprets the arguments after the program name.
fn parse_args(args: impl IntoIterator<Item = OsString>) -> Cli {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Cli::Error("missing file argument".to_owned());
    };

    if first == "-h" || first == "--help" {
        return Cli::Help;
    }

    match args.next() {
        None => Cli::View(PathBuf::from(first)),
        Some(extra) => Cli::Error(format!("unexpected argument {}", extra.to_string_lossy())),
    }
}

/// Whether a mouse drag pans or orbits the camera.
#[derive(Clone, Copy, Debug)]
enum Drag {
    Pan,
    Orbit,
}

/// The Z clip: every move whose highest Z exceeds `current` is hidden.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ZClip {
    /// Lowest Z of the toolpath, in millimetres.
    min: f32,
    /// Highest Z of the toolpath, in millimetres.
    max: f32,
    /// Current cut-off; equal to `max` while nothing is hidden.
    current: f32,
}

impl ZClip {
    fn new(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            current: max,
        }
    }

    /// Moves the cut-off by `step` millimetres, clamped to the toolpath.
    /// Returns whether the cut-off changed.
    fn nudge(&mut self, step: f32) -> bool {
        let next = (self.current + step).clamp(self.min, self.max);

        if next == self.current {
            return false;
        }

        self.current = next;
        true
    }

    /// Restores the cut-off to the full height. Returns whether it changed.
    fn reset(&mut self) -> bool {
        let changed = self.current != self.max;
        self.current = self.max;
        changed
    }
}

/// The window title: the file name, plus the Z cut-off while it is active.
fn title(path: &Path, z_clip: Option<ZClip>) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );

    match z_clip {
        Some(z_clip) if z_clip.current < z_clip.max => {
            format!("{name} — z ≤ {:.2} mm", z_clip.current)
        }
        _ => name,
    }
}

/// Points Mesa at its D3D12 driver on WSL.
///
/// With no DRM device, Mesa resolves GL through its software fallback, which
/// on Wayland prefers Zink first. Zink only finds lavapipe there, fails, and
/// the fallback ends at llvmpipe even though the D3D12 driver can reach the
/// Windows GPU through `/dev/dxg`. Choosing the driver up front bypasses the
/// Zink preference; an explicit choice by the user is respected.
fn prefer_d3d12_on_wsl() {
    if !Path::new("/dev/dxg").exists() {
        return;
    }

    for variable in [
        "GALLIUM_DRIVER",
        "MESA_LOADER_DRIVER_OVERRIDE",
        "LIBGL_ALWAYS_SOFTWARE",
    ] {
        if std::env::var_os(variable).is_some() {
            return;
        }
    }

    // SAFETY: no other thread exists yet, and the GL stack reads the variable
    // while the instance is created below.
    unsafe { std::env::set_var("GALLIUM_DRIVER", "d3d12") };
}

/// Reports when the machine has no hardware adapter and rendering falls back
/// to software rasterization, which turns large toolpaths into a slideshow.
fn warn_if_software_only(instance: &wgpu::Instance) {
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));

    let hardware = adapters
        .iter()
        .find(|adapter| adapter.get_info().device_type != wgpu::DeviceType::Cpu);

    if hardware.is_none() {
        let name = adapters
            .first()
            .map_or("an unknown adapter".to_owned(), |adapter| {
                adapter.get_info().name
            });
        eprintln!("{BIN}: no hardware graphics adapter; rendering on {name} in software");
    }
}

struct App {
    path: PathBuf,
    /// The open file, consumed by the first `resumed`.
    file: Option<File>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// Last cursor position in device pixels.
    cursor: PhysicalPosition<f64>,
    /// Whether `cursor` is a fresh in-window position.
    cursor_known: bool,
    drag: Option<Drag>,
    modifiers: ModifiersState,
    z_clip: Option<ZClip>,
    /// Whether the current surface stall was already reported.
    stall_reported: bool,
    /// Set when a fatal error ends the loop; reported by `main`.
    fatal: Option<String>,
}

impl App {
    fn new(path: PathBuf, file: File) -> Self {
        Self {
            path,
            file: Some(file),
            window: None,
            renderer: None,
            cursor: PhysicalPosition::new(0.0, 0.0),
            cursor_known: false,
            drag: None,
            modifiers: ModifiersState::default(),
            z_clip: None,
            stall_reported: false,
            fatal: None,
        }
    }

    /// Creates the window, the renderer, and the initial view.
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(title(&self.path, self.z_clip))
                        .with_inner_size(WINDOW_SIZE),
                )
                .map_err(|error| format!("cannot create a window: {error}"))?,
        );

        let size = window.inner_size();
        prefer_d3d12_on_wsl();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        warn_if_software_only(&instance);
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| format!("cannot create a rendering surface: {error}"))?;

        let mut renderer = pollster::block_on(Renderer::new(
            &instance,
            surface,
            RendererOptions {
                width: size.width,
                height: size.height,
                limits: wgpu::Limits::default(),
            },
        ))
        .map_err(|error| format!("cannot create the renderer: {error}"))?;

        let stats = self.load(&mut renderer)?;
        println!(
            "{}: {} moves, {} segments",
            self.path.display(),
            stats.moves,
            stats.segments
        );

        self.z_clip = Some(ZClip::new(stats.z_min as f32, stats.z_max as f32));
        self.cursor =
            PhysicalPosition::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
        self.window = Some(window);
        self.renderer = Some(renderer);
        self.request_redraw();

        Ok(())
    }

    /// Streams the file into `renderer` and frames the result.
    fn load(&mut self, renderer: &mut Renderer) -> Result<PathStats, String> {
        let file = self.file.take().ok_or("the file was loaded twice")?;

        renderer.begin_load();
        let read = load::stream(BufReader::new(file), |chunk| {
            assert!(renderer.feed_load(chunk), "a load is in progress");
        });
        let stats = renderer.end_load().expect("a load was begun above");
        read.map_err(|error| format!("cannot read {}: {error}", self.path.display()))?;

        Ok(stats)
    }

    /// Draws the frame the compositor asked for. The system also issues
    /// `RedrawRequested` when the window contents are invalidated (uncovered,
    /// restored), so the frame is drawn unconditionally. Redraws are requested
    /// on state changes only; a surface that refuses a frame is retried on the
    /// next event rather than in a loop.
    fn draw(&mut self) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };

        if let Err(error) = renderer.render() {
            eprintln!("{BIN}: cannot draw a frame: {error}");
        }

        if renderer.needs_redraw() {
            // The surface refused the frame (stale swap chain, occlusion,
            // timeout). The image stays as it was until the next event, and
            // one report per stall keeps a misbehaving compositor visible.
            if !self.stall_reported {
                self.stall_reported = true;
                eprintln!("{BIN}: the surface refused a frame; the image may be stale");
            }
        } else {
            self.stall_reported = false;
        }
    }

    /// Asks the window for a frame after the image changed.
    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn pointer_moved(&mut self, position: PhysicalPosition<f64>) {
        let previous = std::mem::replace(&mut self.cursor, position);
        let was_known = std::mem::replace(&mut self.cursor_known, true);

        if !was_known {
            return;
        }

        let Some(renderer) = &mut self.renderer else {
            return;
        };

        let dx = (position.x - previous.x) as f32;
        let dy = (position.y - previous.y) as f32;

        match self.drag {
            Some(Drag::Pan) => renderer.pan(dx, dy),
            Some(Drag::Orbit) => {
                renderer.orbit(-dx * ORBIT_RADIANS_PER_PIXEL, dy * ORBIT_RADIANS_PER_PIXEL);
            }
            None => return,
        }

        self.request_redraw();
    }

    fn button(&mut self, state: ElementState, button: MouseButton) {
        if !matches!(button, MouseButton::Left | MouseButton::Middle) {
            return;
        }

        let orbit = self.modifiers.control_key() || self.modifiers.super_key();

        self.drag = match state {
            ElementState::Pressed if button == MouseButton::Middle || orbit => Some(Drag::Orbit),
            ElementState::Pressed => Some(Drag::Pan),
            _ => None,
        };
    }

    fn wheel(&mut self, delta: MouseScrollDelta) {
        let pixels = match delta {
            MouseScrollDelta::LineDelta(_, lines) => f64::from(lines) * WHEEL_LINE_PIXELS,
            MouseScrollDelta::PixelDelta(position) => position.y,
        }
        .clamp(-WHEEL_MAX_PIXELS, WHEEL_MAX_PIXELS);

        let Some(renderer) = &mut self.renderer else {
            return;
        };

        let factor = (-pixels * WHEEL_ZOOM_PER_PIXEL).exp() as f32;
        renderer.zoom(factor, self.cursor.x as f32, self.cursor.y as f32);

        self.request_redraw();
    }

    fn key(&mut self, event_loop: &ActiveEventLoop, event: KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => event_loop.exit(),
            Key::Named(NamedKey::ArrowUp) => self.nudge_z_clip(self.z_step()),
            Key::Named(NamedKey::ArrowDown) => self.nudge_z_clip(-self.z_step()),
            Key::Character(key) if key.eq_ignore_ascii_case("f") => self.fit(),
            Key::Character(key) if key.eq_ignore_ascii_case("z") => self.reset_z_clip(),
            _ => {}
        }
    }

    /// Millimetres one Z-clip press covers.
    fn z_step(&self) -> f32 {
        if self.modifiers.shift_key() {
            return Z_CLIP_FINE_STEP;
        }

        self.z_clip.map_or(0.0, |z_clip| {
            (z_clip.max - z_clip.min) * Z_CLIP_COARSE_FRACTION
        })
    }

    /// Moves the Z clip by `step` millimetres; positive shows more.
    fn nudge_z_clip(&mut self, step: f32) {
        let Some(z_clip) = &mut self.z_clip else {
            return;
        };

        if !z_clip.nudge(step) {
            return;
        }
        let current = z_clip.current;

        self.apply_z_clip(current);
    }

    fn reset_z_clip(&mut self) {
        let Some(z_clip) = &mut self.z_clip else {
            return;
        };

        if !z_clip.reset() {
            return;
        }
        let current = z_clip.current;

        self.apply_z_clip(current);
    }

    /// Hands the cut-off to the renderer and reflects it in the title.
    fn apply_z_clip(&mut self, current: f32) {
        if let Some(renderer) = &mut self.renderer {
            renderer.set_z_max(current);
        }

        if let Some(window) = &self.window {
            window.set_title(&title(&self.path, self.z_clip));
        }

        self.request_redraw();
    }

    fn fit(&mut self) {
        if let Some(renderer) = &mut self.renderer {
            renderer.fit();
            self.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        if let Err(error) = self.create(event_loop) {
            self.fatal = Some(error);
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => self.pointer_moved(position),
            WindowEvent::CursorLeft { .. } => self.cursor_known = false,
            WindowEvent::MouseInput { state, button, .. } => self.button(state, button),
            WindowEvent::MouseWheel { delta, .. } => self.wheel(delta),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(false) => self.drag = None,
            WindowEvent::KeyboardInput { event, .. } => self.key(event_loop, event),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// `parse_args` over a literal argument list.
    fn parse(items: &[&str]) -> Cli {
        parse_args(items.iter().map(OsString::from))
    }

    #[test]
    fn a_single_path_is_a_view_request() {
        assert_eq!(
            parse(&["part.gcode"]),
            Cli::View(PathBuf::from("part.gcode"))
        );
    }

    #[rstest]
    #[case::short(&["-h"])]
    #[case::long(&["--help"])]
    fn help_flags_report_help(#[case] arguments: &[&str]) {
        assert_eq!(parse(arguments), Cli::Help);
    }

    #[test]
    fn a_missing_path_is_an_error() {
        assert!(matches!(parse(&[]), Cli::Error(_)));
    }

    #[test]
    fn extra_arguments_are_an_error() {
        assert!(matches!(parse(&["a.gcode", "b.gcode"]), Cli::Error(_)));
    }

    #[test]
    fn the_z_clip_starts_at_the_full_height() {
        assert_eq!(ZClip::new(0.0, 10.0).current, 10.0);
    }

    #[rstest]
    #[case::one_step(-5.0, 5.0)]
    #[case::clamps_at_the_bottom(-20.0, 0.0)]
    #[case::stays_at_the_top(20.0, 10.0)]
    fn nudging_clamps_to_the_toolpath(#[case] step: f32, #[case] expected: f32) {
        let mut z_clip = ZClip::new(0.0, 10.0);

        z_clip.nudge(step);

        assert_eq!(z_clip.current, expected);
    }

    #[test]
    fn resetting_restores_the_full_height() {
        let mut z_clip = ZClip::new(0.0, 10.0);
        z_clip.nudge(-3.0);

        assert!(z_clip.reset());

        assert_eq!(z_clip.current, 10.0);
        assert!(!z_clip.reset(), "already at the full height");
    }

    #[test]
    fn the_title_shows_the_file_name_and_the_z_cut_off() {
        let path = Path::new("/tmp/part.gcode");

        assert_eq!(title(path, None), "part.gcode");
        assert_eq!(title(path, Some(ZClip::new(0.0, 10.0))), "part.gcode");
        assert_eq!(
            title(
                path,
                Some(ZClip {
                    min: 0.0,
                    max: 10.0,
                    current: 4.25,
                }),
            ),
            "part.gcode — z ≤ 4.25 mm"
        );
    }
}
