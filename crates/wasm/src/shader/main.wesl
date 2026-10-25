import package::contract::{uniforms, PathInput, PrintPathCapInput};
import package::print_path_cap::print_path_cap_vertex;
import package::print_path::{print_path_vertex, PrintPathOutput};
import package::travel_path::{travel_path_vertex, TravelPathOutput};

// Parked beyond the right clip plane. With x > w at every vertex the whole
// triangle fails clipping, so it never rasterizes.
const CULL_POSITION = vec4f(2.0, 2.0, 2.0, 1.0);

fn hidden(kind: u32, move_z_max: f32) -> bool {
    return move_z_max > uniforms.z_max || (uniforms.kind_mask & (1u << kind)) == 0u;
}

@vertex
fn vs_travel_path(input: PathInput, @builtin(vertex_index) index: u32) -> TravelPathOutput {
    if hidden(input.kind, input.move_z_max) {
        var out: TravelPathOutput;
        out.position = CULL_POSITION;
        return out;
    }

    return travel_path_vertex(input, index);
}

@vertex
fn vs_print_path(input: PathInput, @builtin(vertex_index) index: u32) -> PrintPathOutput {
    if hidden(input.kind, input.move_z_max) {
        var out: PrintPathOutput;
        out.position = CULL_POSITION;
        return out;
    }

    return print_path_vertex(input, index);
}

@vertex
fn vs_print_path_cap(input: PrintPathCapInput, @builtin(vertex_index) index: u32) -> PrintPathOutput {
    if hidden(input.kind, input.move_z_max) {
        var out: PrintPathOutput;
        out.position = CULL_POSITION;
        return out;
    }

    return print_path_cap_vertex(input, index);
}

@fragment
fn fs_travel_path(input: TravelPathOutput) -> @location(0) vec4f {
    return input.color;
}

@fragment
fn fs_print_path(input: PrintPathOutput) -> @location(0) vec4f {
    // Camera-relative diffuse light: the lit side follows the view, so the
    // cross-section reads as round from every orbit angle.
    let normal = normalize(input.normal);
    let light = normalize(uniforms.light_dir);
    let shade = 0.45 + 0.55 * max(dot(normal, light), 0.0);

    return vec4f(input.color.rgb * shade, input.color.a);
}
