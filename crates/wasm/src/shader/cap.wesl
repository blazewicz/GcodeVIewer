// Domed caps that close the ends of a tube segment.
//
// A cap is a hemisphere truncated so it reaches exactly to the move endpoint: latitude
// bands from the equator up to the truncation, plus a tip fan that fills the flat face
// when the segment is shorter than the radius. The renderer draws
// `CAP_RINGS * TUBE_SIDES * 6 + TUBE_SIDES * 3` vertices per cap.

import package::color::segment_color;
import package::contract::{uniforms, CapInput};
import package::tube::{TUBE_SIDES, TubeOutput};

// Latitude bands of a hemispherical cap, from the equator to the tip.
const CAP_RINGS: u32 = 4u;

fn cap_vertex(input: CapInput, index: u32) -> TubeOutput {
    let radius = uniforms.extrusion_radius;
    let inset = min(radius, input.room);
    let center = input.tip - inset * input.axis;
    let max_phi = asin(clamp(inset / radius, 0.0, 1.0));

    // A perpendicular pair spanning the cross-section. The reference flips near
    // the Z axis so the cross product never degenerates.
    let reference = select(
        vec3f(0.0, 0.0, 1.0),
        vec3f(1.0, 0.0, 0.0),
        abs(input.axis.z) > 0.999,
    );
    let u = normalize(cross(reference, input.axis));
    let w = cross(input.axis, u);
    let step = 6.28318530718 / f32(TUBE_SIDES);
    let bands = CAP_RINGS * TUBE_SIDES * 6u;

    var normal: vec3f;
    var world: vec3f;

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

    let color = segment_color(input.kind, input.feed);

    var out: TubeOutput;
    out.position = uniforms.view_proj * vec4f(world, 1.0);
    out.kind = input.kind;
    out.move_z_max = input.move_z_max;
    out.normal = normal;
    out.color = color;
    return out;
}
