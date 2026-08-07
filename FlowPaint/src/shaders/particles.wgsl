// Tracer-particle update: advect through the LBM velocity field, respawn
// on death, on leaving the domain, or on landing inside a wall.
//
// Particle layout (vec4f): x, y = position in grid cells; z = age in
// lattice steps; w = lifetime in lattice steps (0 = uninitialised).
//
// NOTE: keep PartParams in sync with sim.rs (PartParamsRaw).

struct PartParams {
    width: u32,
    height: u32,
    count: u32,
    frame: u32,   // frame counter, salts the respawn hash
    dt: f32,      // lattice steps advanced this frame
    _pad0: f32,
    spawn_min: vec2u, // respawn window (visible area + upstream band)
    spawn_max: vec2u,
    _pad1: f32,
    _pad2: f32,
};

const CELL_WALL: u32 = 1u;

@group(0) @binding(0) var<uniform> PP: PartParams;
@group(0) @binding(1) var<storage, read_write> particles: array<vec4f>;
@group(0) @binding(2) var<storage, read> velocity: array<vec2f>;
@group(0) @binding(3) var<storage, read> cell_type: array<u32>;

fn pcg(v_in: u32) -> u32 {
    var v = v_in * 747796405u + 2891336453u;
    let word = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand01(seed: u32) -> f32 {
    return f32(pcg(seed) & 0x00FFFFFFu) / 16777216.0;
}

fn sample_vel(p_in: vec2f) -> vec2f {
    let W = i32(PP.width);
    let H = i32(PP.height);
    let p = clamp(p_in - 0.5, vec2f(0.0), vec2f(f32(W - 1), f32(H - 1)));
    let x0 = i32(p.x);
    let y0 = i32(p.y);
    let x1 = min(x0 + 1, W - 1);
    let y1 = min(y0 + 1, H - 1);
    let tx = p.x - f32(x0);
    let ty = p.y - f32(y0);
    let v00 = velocity[y0 * W + x0];
    let v10 = velocity[y0 * W + x1];
    let v01 = velocity[y1 * W + x0];
    let v11 = velocity[y1 * W + x1];
    return mix(mix(v00, v10, tx), mix(v01, v11, tx), ty);
}

@compute @workgroup_size(256)
fn update(@builtin(global_invocation_id) gid: vec3u) {
    let i = gid.x;
    if (i >= PP.count) { return; }
    var p = particles[i];

    let W = PP.width;
    let H = PP.height;

    var respawn = p.w <= 0.0 || p.z > p.w;
    if (p.x < 0.0 || p.y < 0.0 || p.x >= f32(W) || p.y >= f32(H)) {
        respawn = true;
    } else {
        let cx = clamp(u32(p.x), 0u, W - 1u);
        let cy = clamp(u32(p.y), 0u, H - 1u);
        if (cell_type[cy * W + cx] == CELL_WALL) { respawn = true; }
    }

    if (respawn) {
        let s = i * 0x9E3779B9u + PP.frame * 0x85EBCA6Bu;
        let span = vec2f(PP.spawn_max - PP.spawn_min);
        p.x = f32(PP.spawn_min.x) + rand01(s) * span.x;
        p.y = f32(PP.spawn_min.y) + rand01(s ^ 0xB5297A4Du) * span.y;
        p.z = 0.0;
        p.w = 200.0 + 600.0 * rand01(s ^ 0x68E31DA4u);
    } else {
        let v = sample_vel(p.xy);
        p.x += v.x * PP.dt;
        p.y += v.y * PP.dt;
        p.z += PP.dt;
    }
    particles[i] = p;
}
