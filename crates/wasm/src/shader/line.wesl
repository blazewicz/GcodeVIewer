// Line geometry for non-extruding moves.

import package::color::segment_color;
import package::contract::{uniforms, VertexInput};

struct LineOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) kind: u32,
    // Highest Z of the whole move, so the Z limit hides or shows the move as
    // a unit instead of cutting it at the boundary.
    @location(1) @interpolate(flat) move_z_max: f32,
    @location(2) @interpolate(flat) color: vec4<f32>,
};

fn line_vertex(input: VertexInput, index: u32) -> LineOutput {
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

    let color = segment_color(input.kind, input.feed);

    var out: LineOutput;
    out.position = vec4<f32>(ndc * base_clip.w, base_clip.z, base_clip.w);
    out.kind = input.kind;
    out.move_z_max = input.move_z_max;
    out.color = color;
    return out;
}
