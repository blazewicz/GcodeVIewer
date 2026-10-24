//! Host-side validation of the renderer shader.
//!
//! WGSL errors would otherwise surface only in the browser, so the shader is
//! parsed and validated with naga on the host.

#[test]
fn the_renderer_shader_parses_and_validates() {
    let source = include_str!("../src/shader.wgsl");

    let module = naga::front::wgsl::parse_str(source).expect("shader must parse");

    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("shader must validate");
}
