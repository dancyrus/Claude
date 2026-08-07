// Passive dye: semi-Lagrangian advection through the LBM velocity field,
// with persistent painted sources. Runs once per frame with dt equal to
// the number of lattice steps advanced.
//
// NOTE: keep SimParams in sync with sim.rs (SimParamsRaw) and lbm.wgsl.

struct SimParams {
    width: u32,
    height: u32,
    omega: f32,
    inlet_speed: f32,
    dye_dt: f32,
    dye_decay: f32,
    sponge_width: f32,
    sponge_strength: f32,
    free_u: vec2f,
    _pad0: f32,
    _pad1: f32,
};

const CELL_WALL: u32 = 1u;

@group(0) @binding(0) var<uniform> P: SimParams;
@group(0) @binding(1) var<storage, read> dye_in: array<vec4f>;
@group(0) @binding(2) var<storage, read_write> dye_out: array<vec4f>;
@group(0) @binding(3) var<storage, read> velocity: array<vec2f>;
@group(0) @binding(4) var<storage, read> cell_type: array<u32>;
@group(0) @binding(5) var<storage, read> dye_src: array<vec4f>;

fn sample_dye(p_in: vec2f) -> vec4f {
    let W = i32(P.width);
    let H = i32(P.height);
    let p = clamp(p_in - 0.5, vec2f(0.0), vec2f(f32(W - 1), f32(H - 1)));
    let x0 = i32(p.x);
    let y0 = i32(p.y);
    let x1 = min(x0 + 1, W - 1);
    let y1 = min(y0 + 1, H - 1);
    let tx = p.x - f32(x0);
    let ty = p.y - f32(y0);
    let d00 = dye_in[y0 * W + x0];
    let d10 = dye_in[y0 * W + x1];
    let d01 = dye_in[y1 * W + x0];
    let d11 = dye_in[y1 * W + x1];
    return mix(mix(d00, d10, tx), mix(d01, d11, tx), ty);
}

@compute @workgroup_size(8, 8)
fn advect(@builtin(global_invocation_id) gid: vec3u) {
    let W = P.width;
    let H = P.height;
    if (gid.x >= W || gid.y >= H) { return; }
    let idx = gid.y * W + gid.x;

    if (cell_type[idx] == CELL_WALL) {
        dye_out[idx] = vec4f(0.0);
        return;
    }

    let pos = vec2f(gid.xy) + 0.5 - velocity[idx] * P.dye_dt;

    // Don't pull dye through a solid: if the backtraced point lands inside
    // a wall cell, keep the local dye instead of sampling across it.
    let bx = clamp(i32(floor(pos.x)), 0, i32(W) - 1);
    let by = clamp(i32(floor(pos.y)), 0, i32(H) - 1);
    var d: vec4f;
    if (cell_type[u32(by) * W + u32(bx)] == CELL_WALL) {
        d = dye_in[idx] * P.dye_decay;
    } else {
        d = sample_dye(pos) * P.dye_decay;
    }

    let src = dye_src[idx];
    if (src.a > 0.0) {
        d = vec4f(max(d.rgb, src.rgb * src.a), d.a);
    }
    dye_out[idx] = clamp(d, vec4f(0.0), vec4f(1.0));
}

@compute @workgroup_size(8, 8)
fn clear_dye(@builtin(global_invocation_id) gid: vec3u) {
    let W = P.width;
    let H = P.height;
    if (gid.x >= W || gid.y >= H) { return; }
    dye_out[gid.y * W + gid.x] = vec4f(0.0);
}
