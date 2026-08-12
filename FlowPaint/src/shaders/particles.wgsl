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
    wrap: u32,    // periodic wrap: bit 0 = x axis, bit 1 = y axis
    spawn_min: vec2u, // respawn window (visible area + upstream band)
    spawn_max: vec2u,
    _pad1: f32,
    _pad2: f32,
};

const CELL_WALL: u32 = 1u;

// Pipeline-specialization switch: 0 compiles the wrap logic out
// entirely (the default pipeline every non-periodic scene runs — zero
// added cost, measured); 1 enables the PP.wrap uniform branches. The
// engine binds the variant by EdgeBcs::wrap_bits (sim.rs).
override WRAP_ENABLED: u32 = 0u;

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

// Wrap a position onto a periodic axis; a tracer crossing the seam
// re-enters from the other side instead of respawning.
fn wrap_pos(p: vec2f) -> vec2f {
    var q = p;
    let fw = f32(PP.width);
    let fh = f32(PP.height);
    if (WRAP_ENABLED != 0u && (PP.wrap & 1u) != 0u) { q.x = q.x - fw * floor(q.x / fw); }
    if (WRAP_ENABLED != 0u && (PP.wrap & 2u) != 0u) { q.y = q.y - fh * floor(q.y / fh); }
    return q;
}

fn sample_vel(p_in: vec2f) -> vec2f {
    let W = i32(PP.width);
    let H = i32(PP.height);
    let p = p_in - 0.5;
    // Per axis: bilinear taps wrap across a periodic seam, clamp at an
    // ordinary edge (the pre-periodic behaviour).
    var x0: i32; var x1: i32; var tx: f32;
    if (WRAP_ENABLED != 0u && (PP.wrap & 1u) != 0u) {
        let fx = floor(p.x);
        tx = p.x - fx;
        x0 = ((i32(fx) % W) + W) % W;
        x1 = (x0 + 1) % W;
    } else {
        let cx = clamp(p.x, 0.0, f32(W - 1));
        x0 = i32(cx);
        x1 = min(x0 + 1, W - 1);
        tx = cx - f32(x0);
    }
    var y0: i32; var y1: i32; var ty: f32;
    if (WRAP_ENABLED != 0u && (PP.wrap & 2u) != 0u) {
        let fy = floor(p.y);
        ty = p.y - fy;
        y0 = ((i32(fy) % H) + H) % H;
        y1 = (y0 + 1) % H;
    } else {
        let cy = clamp(p.y, 0.0, f32(H - 1));
        y0 = i32(cy);
        y1 = min(y0 + 1, H - 1);
        ty = cy - f32(y0);
    }
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
        // Advect in <= 1-cell substeps so fast (compressible-mode) flow
        // can't carry a tracer straight through a thin wall — it stops at
        // the surface and the next frame's respawn check recycles it.
        let v0 = sample_vel(p.xy) * PP.dt;
        let n = max(1u, min(24u, u32(ceil(length(v0)))));
        let sub_dt = PP.dt / f32(n);
        var pos = p.xy;
        for (var k = 0u; k < n; k++) {
            let v = sample_vel(pos);
            let cand = wrap_pos(pos + v * sub_dt);
            if (cand.x < 0.0 || cand.y < 0.0
                || cand.x >= f32(W) || cand.y >= f32(H)) {
                pos = cand; // off-domain: respawn logic handles it
                break;
            }
            let cx = clamp(u32(cand.x), 0u, W - 1u);
            let cy = clamp(u32(cand.y), 0u, H - 1u);
            pos = cand;
            if (cell_type[cy * W + cx] == CELL_WALL) {
                break;
            }
        }
        p.x = pos.x;
        p.y = pos.y;
        p.z += PP.dt;
    }
    particles[i] = p;
}
