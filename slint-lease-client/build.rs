// Compile the UI markup (ui/main.slint) to Rust at build time. slint-build
// must come from the same fork rev as the `slint` runtime dependency (see
// Cargo.toml), so the generated code matches the compiled runtime.
fn main() {
    slint_build::compile("ui/main.slint").expect("compiling ui/main.slint");
}
