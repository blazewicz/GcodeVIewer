struct Uniforms {
    view_proj: mat4x4<f32>,
    viewport: vec2f,
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
    // Unit vector from the scene toward the eye, used to shade print paths.
    light_dir: vec3f,
    // Radius of an extruding move in millimetres.
    extrusion_radius: f32,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct PathInput {
    @location(0) start: vec3f,
    @location(1) end: vec3f,
    @location(2) kind: u32,
    // Bit 0: the segment starts its move and is capped there; bit 1: it ends
    // one. Interior joints of a flattened curve carry neither.
    @location(3) flags: u32,
    // Active feedrate in mm/min, or NO_FEED when unknown.
    @location(4) feed: f32,
    // Highest Z of the whole move; the Z limit hides moves whole.
    @location(5) move_z_max: f32,
};

struct PrintPathCapInput {
    // The move endpoint the dome must reach.
    @location(0) tip: vec3f,
    // Unit direction from the path into the cap.
    @location(1) axis: vec3f,
    // Longest axial extent the dome may occupy, in millimetres.
    @location(2) room: f32,
    @location(3) kind: u32,
    // Active feedrate in mm/min, or NO_FEED when unknown.
    @location(4) feed: f32,
    // Highest Z of the whole move; the Z limit hides moves whole.
    @location(5) move_z_max: f32,
};
