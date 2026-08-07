// D2Q9 lattice-Boltzmann collide-and-stream (pull scheme), plus a reset
// kernel. Mirrors the layout used on the CPU side: distribution functions
// are SoA, f[i * cellCount + cellIndex], i in 0..8.
//
// NOTE: keep SimParams in sync with sim.rs (SimParamsRaw) and dye.wgsl.

struct SimParams {
    width: u32,
    height: u32,
    omega: f32,        // BGK relaxation rate 1/tau, tau = 3*nu + 0.5
    inlet_speed: f32,  // lattice speed at fan/inlet cells
    dye_dt: f32,       // lattice time advanced this frame (dye advection)
    dye_decay: f32,    // per-frame dye retention
    sponge_width: f32,    // absorbing-layer thickness in cells (0 = off)
    sponge_strength: f32, // absorbing-layer blend strength
    free_u: vec2f,        // freestream velocity the sponge relaxes toward
    _pad0: f32,
    _pad1: f32,
};

const CELL_FLUID: u32 = 0u;
const CELL_WALL: u32 = 1u;
const CELL_INLET: u32 = 2u;
const CELL_OUTLET: u32 = 3u;

const MAX_LATTICE_SPEED: f32 = 0.3;

@group(0) @binding(0) var<uniform> P: SimParams;
@group(0) @binding(1) var<storage, read> f_in: array<f32>;
@group(0) @binding(2) var<storage, read_write> f_out: array<f32>;
@group(0) @binding(3) var<storage, read> cell_type: array<u32>;
@group(0) @binding(4) var<storage, read> fan_dir: array<vec2f>;
@group(0) @binding(5) var<storage, read_write> velocity: array<vec2f>;
@group(0) @binding(6) var<storage, read_write> density: array<f32>;

// D2Q9 lattice vectors, opposites and weights. var<private> so they can be
// indexed dynamically.
var<private> E: array<vec2i, 9> = array<vec2i, 9>(
    vec2i(0, 0),
    vec2i(1, 0), vec2i(0, 1), vec2i(-1, 0), vec2i(0, -1),
    vec2i(1, 1), vec2i(-1, 1), vec2i(-1, -1), vec2i(1, -1),
);
var<private> OPP: array<u32, 9> = array<u32, 9>(0u, 3u, 4u, 1u, 2u, 7u, 8u, 5u, 6u);
var<private> WT: array<f32, 9> = array<f32, 9>(
    4.0 / 9.0,
    1.0 / 9.0, 1.0 / 9.0, 1.0 / 9.0, 1.0 / 9.0,
    1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0,
);

fn equilibrium(i: u32, rho: f32, u: vec2f, usq: f32) -> f32 {
    let eu = dot(vec2f(E[i]), u);
    return WT[i] * rho * (1.0 + 3.0 * eu + 4.5 * eu * eu - 1.5 * usq);
}

@compute @workgroup_size(8, 8)
fn collide(@builtin(global_invocation_id) gid: vec3u) {
    let W = P.width;
    let H = P.height;
    if (gid.x >= W || gid.y >= H) { return; }

    let n = W * H;
    let idx = gid.y * W + gid.x;
    let ct = cell_type[idx];

    if (ct == CELL_WALL) {
        // Wall populations are never read by neighbours (they bounce back
        // off their own cell); keep the wall finite and quiet.
        velocity[idx] = vec2f(0.0);
        density[idx] = 1.0;
        for (var i = 0u; i < 9u; i++) { f_out[i * n + idx] = WT[i]; }
        return;
    }

    // --- Streaming (pull) --------------------------------------------
    // f[i] arrives from the cell one lattice vector upstream. A wall
    // upstream reflects our own opposite population (half-way bounce-back).
    // Off-domain neighbours copy the local value (zero-gradient open edge).
    var f: array<f32, 9>;
    for (var i = 0u; i < 9u; i++) {
        let sx = i32(gid.x) - E[i].x;
        let sy = i32(gid.y) - E[i].y;
        if (sx < 0 || sx >= i32(W) || sy < 0 || sy >= i32(H)) {
            f[i] = f_in[i * n + idx];
        } else {
            let sidx = u32(sy) * W + u32(sx);
            if (cell_type[sidx] == CELL_WALL) {
                f[i] = f_in[OPP[i] * n + idx];
            } else {
                f[i] = f_in[i * n + sidx];
            }
        }
    }

    // --- Boundary cells ----------------------------------------------
    if (ct == CELL_INLET) {
        // Fan: force equilibrium at the painted direction and speed.
        let u = fan_dir[idx] * P.inlet_speed;
        let usq = dot(u, u);
        for (var i = 0u; i < 9u; i++) { f_out[i * n + idx] = equilibrium(i, 1.0, u, usq); }
        velocity[idx] = u;
        density[idx] = 1.0;
        return;
    }

    var rho = 0.0;
    var mom = vec2f(0.0);
    for (var i = 0u; i < 9u; i++) {
        rho += f[i];
        mom += vec2f(E[i]) * f[i];
    }

    if (ct == CELL_OUTLET) {
        // Pressure outlet: reference density, keep the local momentum so
        // flow can leave the domain without reflecting.
        var u = mom;
        let sp = length(u);
        if (sp > MAX_LATTICE_SPEED) { u *= MAX_LATTICE_SPEED / sp; }
        let usq = dot(u, u);
        for (var i = 0u; i < 9u; i++) { f_out[i * n + idx] = equilibrium(i, 1.0, u, usq); }
        velocity[idx] = u;
        density[idx] = 1.0;
        return;
    }

    // --- Blow-up guard: quietly reinitialise a diverged cell ----------
    if (!(rho >= 0.1 && rho <= 5.0)) {
        for (var i = 0u; i < 9u; i++) { f_out[i * n + idx] = WT[i]; }
        velocity[idx] = vec2f(0.0);
        density[idx] = 1.0;
        return;
    }

    // --- Collision (BGK) ---------------------------------------------
    var u = mom / rho;
    let sp = length(u);
    if (sp > MAX_LATTICE_SPEED) { u *= MAX_LATTICE_SPEED / sp; }

    // Absorbing sponge layer near the domain edges: blend post-collision
    // populations toward the freestream equilibrium with a quadratic ramp,
    // so outgoing pressure waves die instead of reflecting back into the
    // visible region.
    var sponge = 0.0;
    if (P.sponge_width > 0.5) {
        let dedge = f32(min(
            min(gid.x, W - 1u - gid.x),
            min(gid.y, H - 1u - gid.y),
        ));
        if (dedge < P.sponge_width) {
            let t = 1.0 - dedge / P.sponge_width;
            sponge = P.sponge_strength * t * t;
        }
    }
    let free_usq = dot(P.free_u, P.free_u);

    let usq = dot(u, u);
    for (var i = 0u; i < 9u; i++) {
        let fe = equilibrium(i, rho, u, usq);
        var fnew = f[i] + P.omega * (fe - f[i]);
        if (sponge > 0.0) {
            fnew = mix(fnew, equilibrium(i, 1.0, P.free_u, free_usq), sponge);
        }
        f_out[i * n + idx] = fnew;
    }
    velocity[idx] = u;
    density[idx] = rho;
}

// Reinitialise the written distribution buffer to rest equilibrium.
// Dispatched once per f buffer (with the two ping-pong bind groups).
@compute @workgroup_size(8, 8)
fn reset_rest(@builtin(global_invocation_id) gid: vec3u) {
    let W = P.width;
    let H = P.height;
    if (gid.x >= W || gid.y >= H) { return; }
    let n = W * H;
    let idx = gid.y * W + gid.x;
    for (var i = 0u; i < 9u; i++) { f_out[i * n + idx] = WT[i]; }
    velocity[idx] = vec2f(0.0);
    density[idx] = 1.0;
}
