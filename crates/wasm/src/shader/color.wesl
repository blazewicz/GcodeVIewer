import package::contract::uniforms;

// Palette indices match tessellate::Kind: rapid, feed, arc, spline.
const PALETTE = array<vec3f, 4>(
    vec3f(0.690, 0.769, 0.839),
    vec3f(0.227, 0.267, 0.345),
    vec3f(0.871, 0.306, 0.220),
    vec3f(0.141, 0.588, 0.361),
);

// Feedrate scale: viridis, slow to fast. The stops match the demo legend, so
// the rendered colours and the UI gradient agree.
const FEED_0 = vec3f(0.267, 0.004, 0.329);
const FEED_1 = vec3f(0.231, 0.322, 0.545);
const FEED_2 = vec3f(0.129, 0.569, 0.549);
const FEED_3 = vec3f(0.369, 0.788, 0.384);
const FEED_4 = vec3f(0.992, 0.906, 0.145);

// Segments without a known feedrate; the renderer passes this sentinel.
const NO_FEED: f32 = -1.0;
const UNKNOWN_FEED_COLOR = vec3f(0.541, 0.573, 0.620);

fn feed_color(feed: f32) -> vec3f {
    if feed == NO_FEED {
        return UNKNOWN_FEED_COLOR;
    }

    // The span is floored so a collapsed range does not divide by zero;
    // feeds outside the range clamp to the ends of the scale.
    let span = max(uniforms.feed_max - uniforms.feed_min, 1e-6);
    let t = (feed - uniforms.feed_min) / span;
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

fn segment_color(kind: u32, feed: f32) -> vec4f {
    var color: vec3f;
    if uniforms.color_mode == 1u {
        color = feed_color(feed);
    } else {
        color = PALETTE[kind];
    }

    return vec4f(color, 1.0);
}
