//! Tube geometry parameters shared by the renderer and the shader.
//!
//! The tube vertex shaders decode `vertex_index` into geometry, so the draw
//! counts in the renderer must match the shader's tessellation constants. They
//! live together here, and a host test pins them to the shader sources.

/// Radial sides of a tube.
pub(crate) const SIDES: u32 = 8;
/// Latitude bands of a hemispherical cap, from the equator to the tip.
pub(crate) const CAP_RINGS: u32 = 4;
/// Vertices per lateral instance: two triangles per side.
pub(crate) const LATERAL_VERTICES: u32 = SIDES * 6;
/// Vertices per cap instance: `CAP_RINGS` bands plus a tip fan that closes the
/// dome when it is truncated by a segment shorter than the radius.
pub(crate) const CAP_VERTICES: u32 = CAP_RINGS * SIDES * 6 + SIDES * 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_tessellation_matches_the_draw_counts() {
        let sides = shader_const(include_str!("shader/print_path.wesl"), "TUBE_SIDES");
        let cap_rings = shader_const(include_str!("shader/print_path_cap.wesl"), "CAP_RINGS");

        assert_eq!(sides, SIDES);
        assert_eq!(cap_rings, CAP_RINGS);
        assert_eq!(LATERAL_VERTICES, sides * 6);
        assert_eq!(CAP_VERTICES, cap_rings * sides * 6 + sides * 3);
    }

    /// Reads `const NAME: u32 = VALUE;` from a shader source.
    fn shader_const(source: &str, name: &str) -> u32 {
        source
            .lines()
            .find_map(|line| {
                let value = line
                    .strip_prefix("const ")?
                    .strip_prefix(name)?
                    .strip_prefix(": u32 = ")?
                    .trim_end_matches(';');
                value.trim_end_matches('u').parse().ok()
            })
            .unwrap_or_else(|| panic!("const {name} not found in the shader source"))
    }
}
