// Instanced path rendering. Each instance is one 3D segment: non-extruding
// moves expand to a quad in screen space, while extruding moves become round
// tubes whose physical radius comes from the camera uniform. Tube ends that
// border a move get a domed cap that stops exactly at the endpoint, so separate
// moves read as separate rods.

import package::cap::cap_vertex;
import package::color::segment_color;
import package::contract::{uniforms, CapInput, VertexInput};
import package::line::{line_vertex, LineOutput};
import package::tube::{tube_vertex, TubeOutput};

@vertex
fn vs_line(input: VertexInput, @builtin(vertex_index) index: u32) -> LineOutput {
    return line_vertex(input, index);
}

@vertex
fn vs_tube(input: VertexInput, @builtin(vertex_index) index: u32) -> TubeOutput {
    return tube_vertex(input, index);
}

@vertex
fn vs_cap(input: CapInput, @builtin(vertex_index) index: u32) -> TubeOutput {
    return cap_vertex(input, index);
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

    let color = segment_color(input.kind, input.feed);

    // Camera-relative diffuse light: the lit side follows the view, so the
    // cross-section reads as round from every orbit angle.
    let normal = normalize(input.normal);
    let light = normalize(uniforms.light_dir);
    let shade = 0.45 + 0.55 * max(dot(normal, light), 0.0);

    return vec4<f32>(color.rgb * shade, color.a);
}
