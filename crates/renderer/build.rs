fn main() {
    let shader = wesl::Compiler::default()
        .compile("src/shader/main.wesl")
        .inspect_err(|error| eprintln!("{error}"))
        .expect("shader compilation failed");

    validate(&shader.to_string());

    shader.emit_rerun_if_changed();
    shader.write_artifact("shader");
}

fn validate(source: &str) {
    let module = naga::front::wgsl::parse_str(source)
        .inspect_err(|error| eprintln!("{error}"))
        .expect("linked shader must parse");

    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .inspect_err(|error| eprintln!("{error}"))
    .expect("linked shader must validate");
}
