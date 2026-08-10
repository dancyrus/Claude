//! Parse and validate every WGSL shader with naga (the same front end
//! wgpu uses at runtime), so `cargo test` catches shader errors without
//! needing a GPU.

fn validate(name: &str, src: &str) {
    let module = naga::front::wgsl::parse_str(src)
        .unwrap_or_else(|e| panic!("{name}: WGSL parse error:\n{e}"));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: WGSL validation error:\n{e:?}"));
}

#[test]
fn lbm_shader_is_valid() {
    validate("lbm.wgsl", include_str!("../src/shaders/lbm.wgsl"));
}

#[test]
fn dye_shader_is_valid() {
    validate("dye.wgsl", include_str!("../src/shaders/dye.wgsl"));
}

#[test]
fn particles_shader_is_valid() {
    validate("particles.wgsl", include_str!("../src/shaders/particles.wgsl"));
}

#[test]
fn render_shader_is_valid() {
    validate("render.wgsl", include_str!("../src/shaders/render.wgsl"));
}

#[test]
fn euler_shader_is_valid() {
    validate("euler.wgsl", include_str!("../src/shaders/euler.wgsl"));
}
