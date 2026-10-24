// Instanced path rendering. Each instance is one 3D segment: non-extruding
// moves expand to a quad in screen space, while extruding moves become round
// tubes whose physical radius comes from the camera uniform. Tube ends that
// border a move get a domed cap that stops exactly at the endpoint, so separate
// moves read as separate rods.

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

// Tessellation of a tube: radial sides for the lateral surface, latitude bands
// and a tip fan for each domed cap. The renderer draws `TUBE_SIDES * 6` lateral
// vertices per segment and `CAP_RINGS * TUBE_SIDES * 6 + TUBE_SIDES * 3` per
// cap.
const TUBE_SIDES: u32 = 8u;
const CAP_RINGS: u32 = 4u;

struct TubeOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) kind: u32,
    // Highest Z of the whole move, so the Z limit hides or shows the move as
    // a unit instead of cutting it at the boundary.
    @location(1) @interpolate(flat) move_z_max: f32,
    @location(2) normal: vec3<f32>,
    @location(3) @interpolate(flat) feed: f32,
};

@vertex
fn vs_tube(input: VertexInput, @builtin(vertex_index) index: u32) -> TubeOutput {
    let segment = input.end - input.start;
    let length = max(length(segment), 1e-6);
    let axis = segment / length;

    // A domed end pulls the lateral surface back from the endpoint by the
    // dome's depth. When both ends are capped, each dome may use at most half
    // the segment so the two never overlap.
    let start_capped = (input.flags & 1u) != 0u;
    let end_capped = (input.flags & 2u) != 0u;
    let room = select(length, length * 0.5, start_capped && end_capped);
    let inset = min(uniforms.extrusion_radius, room);
    let inner_start = select(input.start, input.start + inset * axis, start_capped);
    let inner_end = select(input.end, input.end - inset * axis, end_capped);

    // A perpendicular pair spanning the cross-section. The reference flips near
    // the Z axis so the cross product never degenerates.
    let reference = select(
        vec3<f32>(0.0, 0.0, 1.0),
        vec3<f32>(1.0, 0.0, 0.0),
        abs(axis.z) > 0.999,
    );
    let u = normalize(cross(reference, axis));
    let w = cross(axis, u);
    let step = 6.28318530718 / f32(TUBE_SIDES);

    // Lateral surface: two triangles per side. Corner order is
    // (inner_start,0) (inner_start,1) (inner_end,1) (inner_start,0)
    // (inner_end,1) (inner_end,0).
    let side = index / 6u;
    let corner = index % 6u;
    let at_end = corner == 2u || corner == 4u || corner == 5u;
    let at_second_angle = corner == 1u || corner == 2u || corner == 4u;
    let angle = f32(side + select(0u, 1u, at_second_angle)) * step;

    let center = select(inner_start, inner_end, at_end);
    let normal = cos(angle) * u + sin(angle) * w;
    let world = center + uniforms.extrusion_radius * normal;

    var out: TubeOutput;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);
    out.kind = input.kind;
    out.move_z_max = input.move_z_max;
    out.normal = normal;
    out.feed = input.feed;
    return out;
}

struct CapInput {
    // The move endpoint the dome must reach.
    @location(0) tip: vec3<f32>,
    // Unit direction from the tube into the dome.
    @location(1) axis: vec3<f32>,
    // Longest axial extent the dome may occupy, in millimetres.
    @location(2) room: f32,
    @location(3) kind: u32,
    // Active feedrate in mm/min, or NO_FEED when unknown.
    @location(4) feed: f32,
    // Highest Z of the whole move; the Z limit hides moves whole.
    @location(5) move_z_max: f32,
};

@vertex
fn vs_cap(input: CapInput, @builtin(vertex_index) index: u32) -> TubeOutput {
    let radius = uniforms.extrusion_radius;
    let inset = min(radius, input.room);
    let center = input.tip - inset * input.axis;
    let max_phi = asin(clamp(inset / radius, 0.0, 1.0));

    // A perpendicular pair spanning the cross-section. The reference flips near
    // the Z axis so the cross product never degenerates.
    let reference = select(
        vec3<f32>(0.0, 0.0, 1.0),
        vec3<f32>(1.0, 0.0, 0.0),
        abs(input.axis.z) > 0.999,
    );
    let u = normalize(cross(reference, input.axis));
    let w = cross(input.axis, u);
    let step = 6.28318530718 / f32(TUBE_SIDES);
    let bands = CAP_RINGS * TUBE_SIDES * 6u;

    var normal: vec3<f32>;
    var world: vec3<f32>;

    if index < bands {
        // Latitude bands from the equator, where surface and normal match the
        // tube, up to `max_phi`; a full hemisphere collapses onto the tip.
        let band = index / (TUBE_SIDES * 6u);
        let corner = index % (TUBE_SIDES * 6u);
        let side = corner / 6u;
        let c = corner % 6u;
        let at_outer = c == 2u || c == 4u || c == 5u;
        let at_second_angle = c == 1u || c == 2u || c == 4u;
        let phi = f32(band + select(0u, 1u, at_outer)) * (max_phi / f32(CAP_RINGS));
        let angle = f32(side + select(0u, 1u, at_second_angle)) * step;
        let radial = cos(angle) * u + sin(angle) * w;

        normal = cos(phi) * radial + sin(phi) * input.axis;
        world = center + radius * normal;
    } else {
        // Tip fan, filling the flat face of a dome truncated because the
        // segment is shorter than the radius.
        let fan = index - bands;
        let triangle = fan / 3u;
        let corner = fan % 3u;
        let angle = (f32(triangle + corner) - 1.0) * step;
        let radial = cos(angle) * u + sin(angle) * w;
        let rim = cos(max_phi) * radial + sin(max_phi) * input.axis;

        normal = select(rim, input.axis, corner == 0u);
        world = select(center + radius * rim, input.tip, corner == 0u);
    }

    var out: TubeOutput;
    out.position = uniforms.view_proj * vec4<f32>(world, 1.0);
    out.kind = input.kind;
    out.move_z_max = input.move_z_max;
    out.normal = normal;
    out.feed = input.feed;
    return out;
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
