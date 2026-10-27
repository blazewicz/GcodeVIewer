// Geometry for paths of print moves.

import package::color::segment_color;
import package::contract::{uniforms, PathInput};

// Radial sides of a tube. The renderer draws `TUBE_SIDES * 12` lateral,
// socket, and cone vertices per segment.
const TUBE_SIDES: u32 = 8u;
const TAU: f32 = 6.28318530718;
const TUBE_SIDE_STEP: f32 = TAU / f32(TUBE_SIDES);
// Axial clearance between a cone tip and the apex of the socket it enters,
// as a fraction of the cone height. The radial clearance is the same fraction
// of the tube radius.
const CONE_CLEARANCE: f32 = 0.1;

const LATERAL_VERTICES: u32 = TUBE_SIDES * 6u;
const SOCKET_VERTICES: u32 = TUBE_SIDES * 3u;

struct PrintPathOutput {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) color: vec4f,
    @location(1) normal: vec3f,
};

struct CrossSection {
    u: vec3f,
    w: vec3f,
};

struct ConeVertex {
    world: vec3f,
    normal: vec3f,
};

// A perpendicular pair spanning the cross-section. The reference flips near
// the Z axis so the cross product never degenerates.
fn cross_section(axis: vec3f) -> CrossSection {
    let reference = select(
        vec3f(0.0, 0.0, 1.0),
        vec3f(1.0, 0.0, 0.0),
        abs(axis.z) > 0.999,
    );
    let u = normalize(cross(reference, axis));
    return CrossSection(u, cross(axis, u));
}

// One vertex of a cone whose base circle is at `base` with radius `radius`
// and whose apex is `height` along `axis`. `inward` flips the normals for a
// socket carved into the tube.
fn cone_vertex(
    base: vec3f,
    axis: vec3f,
    section: CrossSection,
    radius: f32,
    height: f32,
    local: u32,
    inward: bool,
) -> ConeVertex {
    let triangle = local / 3u;
    let corner = local % 3u;
    let angle = f32(triangle + select(0u, 1u, corner == 1u)) * TUBE_SIDE_STEP;
    let radial = cos(angle) * section.u + sin(angle) * section.w;

    let sign = select(1.0, -1.0, inward);
    let at_apex = corner == 2u;
    let world = select(base + radius * radial, base + height * axis, at_apex);
    let normal = select(
        normalize(sign * (height * radial + radius * axis)),
        sign * axis,
        at_apex,
    );

    return ConeVertex(world, normal);
}

fn print_path_vertex(input: PathInput, index: u32) -> PrintPathOutput {
    let segment = input.end - input.start;
    let length = max(length(segment), 1e-6);
    let axis = segment / length;
    let radius = uniforms.extrusion_radius;

    // A cone never reaches deeper than its segment. Interior cones may poke
    // into the following segment's socket; move-final cones stay within their
    // own segment so the next move's rim starts where the cone ends.
    let height = min(radius, length);
    let clearance = height * CONE_CLEARANCE;
    let end_capped = (input.flags & 2u) != 0u;
    let lateral_end = select(
        input.end - clearance * axis,
        input.end - height * axis,
        end_capped,
    );

    let section = cross_section(axis);

    var world: vec3f;
    var normal: vec3f;

    if index < LATERAL_VERTICES {
        // Lateral surface: two triangles per side. Corner order is
        // (start,0) (start,1) (end,1) (start,0) (end,1) (end,0).
        let side = index / 6u;
        let corner = index % 6u;
        let at_end = corner == 2u || corner == 4u || corner == 5u;
        let at_second_angle = corner == 1u || corner == 2u || corner == 4u;
        let angle = f32(side + select(0u, 1u, at_second_angle)) * TUBE_SIDE_STEP;

        let center = select(input.start, lateral_end, at_end);
        normal = cos(angle) * section.u + sin(angle) * section.w;
        world = center + radius * normal;
    } else if index < LATERAL_VERTICES + SOCKET_VERTICES {
        // Conical cavity opening at the segment start.
        let socket = cone_vertex(
            input.start,
            axis,
            section,
            radius,
            height,
            index - LATERAL_VERTICES,
            true,
        );
        world = socket.world;
        normal = socket.normal;
    } else {
        // Conical cap closing the tube. Interior caps share their base with
        // the lateral surface and reach past the joint into the next socket.
        let cap = cone_vertex(
            lateral_end,
            axis,
            section,
            radius,
            height,
            index - LATERAL_VERTICES - SOCKET_VERTICES,
            false,
        );
        world = cap.world;
        normal = cap.normal;
    }

    let color = segment_color(input.kind, input.feed);

    var out: PrintPathOutput;
    out.position = uniforms.view_proj * vec4f(world, 1.0);
    out.color = color;
    out.normal = normal;
    return out;
}
