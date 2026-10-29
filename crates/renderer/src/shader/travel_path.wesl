// Geometry for paths of travel moves.

import package::color::segment_color;
import package::contract::{uniforms, PathInput};

const CORNERS = array<vec2f, 6>(
    vec2f(-1.0, -1.0),
    vec2f(1.0, -1.0),
    vec2f(-1.0, 1.0),
    vec2f(-1.0, 1.0),
    vec2f(1.0, -1.0),
    vec2f(1.0, 1.0),
);

struct TravelPathOutput {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) color: vec4f,
};

@must_use
fn travel_path_vertex(input: PathInput, index: u32) -> TravelPathOutput {
    let clip_start = uniforms.view_proj * vec4f(input.start, 1.0);
    let clip_end = uniforms.view_proj * vec4f(input.end, 1.0);

    // Pixel positions relative to the viewport center.
    let screen_start = clip_start.xy / clip_start.w * uniforms.viewport * 0.5;
    let screen_end = clip_end.xy / clip_end.w * uniforms.viewport * 0.5;

    var direction = screen_end - screen_start;
    let length = length(direction);
    if length < 0.0001 {
        direction = vec2f(1.0, 0.0);
    } else {
        direction = direction / length;
    }

    let normal = vec2f(-direction.y, direction.x) * (uniforms.line_width * 0.5 + 0.5);

    let corner = CORNERS[index];
    let end_side = corner.x > 0.0;
    let base_screen = select(screen_start, screen_end, end_side);
    let base_clip = select(clip_start, clip_end, end_side);
    let screen = base_screen + normal * corner.y;

    // Rebuild the clip position from the expanded pixel position, keeping the
    // base endpoint's depth so the depth test still orders segments.
    let ndc = screen / (uniforms.viewport * 0.5);

    let color = segment_color(input.kind, input.feed);

    var out: TravelPathOutput;
    out.position = vec4f(ndc * base_clip.w, base_clip.z, base_clip.w);
    out.color = color;
    return out;
}
