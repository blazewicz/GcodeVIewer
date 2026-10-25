// Tube geometry for extruding moves.
//
// Each segment expands `vertex_index` into a round tube whose physical radius comes
// from the camera uniform. Capped ends are pulled back so the domes from `package::cap`
// fit exactly.

import package::color::segment_color;
import package::contract::{uniforms, VertexInput};

// Radial sides of a tube. The renderer draws `TUBE_SIDES * 6` lateral vertices
// per segment.
const TUBE_SIDES: u32 = 8u;

struct TubeOutput {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) color: vec4f,
    @location(1) normal: vec3f,
};

fn tube_vertex(input: VertexInput, index: u32) -> TubeOutput {
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
        vec3f(0.0, 0.0, 1.0),
        vec3f(1.0, 0.0, 0.0),
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

    let color = segment_color(input.kind, input.feed);

    var out: TubeOutput;
    out.position = uniforms.view_proj * vec4f(world, 1.0);
    out.color = color;
    out.normal = normal;
    return out;
}
