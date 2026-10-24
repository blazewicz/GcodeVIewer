fn main() {
    let shader = wesl::Compiler::default()
        .compile("src/shader/main.wesl")
        .inspect_err(|error| eprintln!("{error}"))
        .expect("shader compilation failed");

    shader.emit_rerun_if_changed();
    shader.write_artifact("shader");
}
