// Instanced path rendering. Each instance is one 3D segment: non-extruding
// moves expand to a quad in screen space, while extruding moves become round
// tubes whose physical radius comes from the camera uniform. Tube ends that
// border a move get a domed cap that stops exactly at the endpoint, so separate
// moves read as separate rods.
//
// This is the root module: it defines the shared uniforms and the pipeline
// entry points. The tube and cap vertex geometry lives in `package::tube` and
// `package::cap`.

import package::cap::{cap_vertex, CapInput};
import package::tube::{tube_vertex, TubeOutput};

struct Uniforms {
    view_proj: mat4x4<f32>,
    viewport: vec2<f32>,
    line_width: f32,
    // Moves whose highest Z exceeds this are hidden whole.
    z_max: f32,
    // Bit `kind` set means that kind is visible.
    kind_mask: u32,
    // 0 colours by move kind, 1 by feedrate.
    color_mode: u32,
    // Feedrates mapped onto the ends of the colour scale, in mm/min.
    feed_min: f32,
    feed_max: f32,
    // Unit vector from the scene toward the eye, used to shade tubes.
    light_dir: vec3<f32>,
    // Radius of an extruding move in millimetres.
    extrusion_radius: f32,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexInput {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) kind: u32,
    // Bit 0: the segment starts its move and is capped there; bit 1: it ends
    // one. Interior joints of a flattened curve carry neither.
    @location(3) flags: u32,
    // Active feedrate in mm/min, or NO_FEED when unknown.
    @location(4) feed: f32,
    // Highest Z of the whole move; the Z limit hides moves whole.
    @location(5) move_z_max: f32,
};

struct LineOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) kind: u32,
    // Highest Z of the whole move, so the Z limit hides or shows the move as
    // a unit instead of cutting it at the boundary.
    @location(1) @interpolate(flat) move_z_max: f32,
    @location(2) @interpolate(flat) feed: f32,
};

@vertex
fn vs_line(input: VertexInput, @builtin(vertex_index) index: u32) -> LineOutput {
    let clip_start = uniforms.view_proj * vec4<f32>(input.start, 1.0);
    let clip_end = uniforms.view_proj * vec4<f32>(input.end, 1.0);

    // Pixel positions relative to the viewport center.
    let screen_start = clip_start.xy / clip_start.w * uniforms.viewport * 0.5;
    let screen_end = clip_end.xy / clip_end.w * uniforms.viewport * 0.5;

    var direction = screen_end - screen_start;
    let length = length(direction);
    if length < 0.0001 {
        direction = vec2<f32>(1.0, 0.0);
    } else {
        direction = direction / length;
    }

    let normal = vec2<f32>(-direction.y, direction.x) * (uniforms.line_width * 0.5 + 0.5);

    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );

    let corner = corners[index];
    let end_side = corner.x > 0.0;
    let base_screen = select(screen_start, screen_end, end_side);
    let base_clip = select(clip_start, clip_end, end_side);
    let screen = base_screen + normal * corner.y;

    // Rebuild the clip position from the expanded pixel position, keeping the
    // base endpoint's depth so the depth test still orders segments.
    let ndc = screen / (uniforms.viewport * 0.5);

    var out: LineOutput;
    out.position = vec4<f32>(ndc * base_clip.w, base_clip.z, base_clip.w);
    out.kind = input.kind;
    out.move_z_max = input.move_z_max;
    out.feed = input.feed;
    return out;
}

// The tube and cap entry points declare the pipeline interface; the geometry
// itself lives in `package::tube` and `package::cap`.
@vertex
fn vs_tube(input: VertexInput, @builtin(vertex_index) index: u32) -> TubeOutput {
    return tube_vertex(input, index);
}

@vertex
fn vs_cap(input: CapInput, @builtin(vertex_index) index: u32) -> TubeOutput {
    return cap_vertex(input, index);
}

// Palette indices match tessellate::Kind: rapid, feed, arc, spline.
const PALETTE = array<vec4<f32>, 4>(
    vec4<f32>(0.690, 0.769, 0.839, 1.0),
    vec4<f32>(0.227, 0.267, 0.345, 1.0),
    vec4<f32>(0.871, 0.306, 0.220, 1.0),
    vec4<f32>(0.141, 0.588, 0.361, 1.0),
);

// Feedrate scale: viridis, slow to fast. The stops match the demo legend, so
// the rendered colours and the UI gradient agree.
const FEED_0 = vec3<f32>(0.267, 0.004, 0.329);
const FEED_1 = vec3<f32>(0.231, 0.322, 0.545);
const FEED_2 = vec3<f32>(0.129, 0.569, 0.549);
const FEED_3 = vec3<f32>(0.369, 0.788, 0.384);
const FEED_4 = vec3<f32>(0.992, 0.906, 0.145);

// Segments without a known feedrate; the renderer passes this sentinel.
const NO_FEED: f32 = -1.0;
const UNKNOWN_FEED_COLOR = vec3<f32>(0.541, 0.573, 0.620);

fn feed_color(t: f32) -> vec3<f32> {
    let x = clamp(t, 0.0, 1.0) * 4.0;
    if x < 1.0 {
        return mix(FEED_0, FEED_1, x);
    }
    if x < 2.0 {
        return mix(FEED_1, FEED_2, x - 1.0);
    }
    if x < 3.0 {
        return mix(FEED_2, FEED_3, x - 2.0);
    }
    return mix(FEED_3, FEED_4, x - 3.0);
}

fn segment_color(kind: u32, feed: f32) -> vec4<f32> {
    if uniforms.color_mode == 1u {
        if feed == NO_FEED {
            return vec4<f32>(UNKNOWN_FEED_COLOR, 1.0);
        }
        // The span is floored so a collapsed range does not divide by zero;
        // feeds outside the range clamp to the ends of the scale.
        let span = max(uniforms.feed_max - uniforms.feed_min, 1e-6);
        return vec4<f32>(feed_color((feed - uniforms.feed_min) / span), 1.0);
    }
    return PALETTE[kind];
}

fn hidden(kind: u32, move_z_max: f32) -> bool {
    return move_z_max > uniforms.z_max || (uniforms.kind_mask & (1u << kind)) == 0u;
}

@fragment
fn fs_line(input: LineOutput) -> @location(0) vec4<f32> {
    if hidden(input.kind, input.move_z_max) {
        discard;
    }
    return segment_color(input.kind, input.feed);
}

@fragment
fn fs_tube(input: TubeOutput) -> @location(0) vec4<f32> {
    if hidden(input.kind, input.move_z_max) {
        discard;
    }

    // Camera-relative diffuse light: the lit side follows the view, so the
    // cross-section reads as round from every orbit angle.
    let normal = normalize(input.normal);
    let light = normalize(uniforms.light_dir);
    let shade = 0.45 + 0.55 * max(dot(normal, light), 0.0);
    let color = segment_color(input.kind, input.feed);
    return vec4<f32>(color.rgb * shade, color.a);
}
