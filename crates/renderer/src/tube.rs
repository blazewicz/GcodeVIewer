//! Tube geometry parameters shared by the renderer and the shader.
//!
//! The tube vertex shaders decode `vertex_index` into geometry, so the draw
//! counts in the renderer must match the shader's tessellation constants. They
//! live together here, and a host test pins them to the shader sources.

/// Radial sides of a tube and its cones.
pub(crate) const SIDES: u32 = 8;
/// Vertices per lateral instance: two triangles per side.
pub(crate) const LATERAL_VERTICES: u32 = SIDES * 6;
/// Vertices per start socket: one triangle per side.
pub(crate) const SOCKET_VERTICES: u32 = SIDES * 3;
/// Vertices per end cone: one triangle per side.
pub(crate) const CONE_VERTICES: u32 = SIDES * 3;
/// Vertices per print path instance: lateral tube, start socket, end cone.
pub(crate) const PRINT_PATH_VERTICES: u32 = LATERAL_VERTICES + SOCKET_VERTICES + CONE_VERTICES;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_tessellation_matches_the_draw_counts() {
        let sides = shader_const(include_str!("shader/print_path.wesl"), "TUBE_SIDES");

        assert_eq!(sides, SIDES);
        assert_eq!(LATERAL_VERTICES, sides * 6);
        assert_eq!(SOCKET_VERTICES, sides * 3);
        assert_eq!(CONE_VERTICES, sides * 3);
        assert_eq!(PRINT_PATH_VERTICES, sides * 12);
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
