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
    time: f32,
    wrap: u32,   // periodic wrap: bit 0 = x axis, bit 1 = y axis
};

const CELL_WALL: u32 = 1u;

// Pipeline-specialization switch: 0 compiles the wrap logic out
// entirely (the default pipeline every non-periodic scene runs — zero
// added cost, measured); 1 enables the P.wrap uniform branches. The
// engine binds the variant by EdgeBcs::wrap_bits (sim.rs).
override WRAP_ENABLED: u32 = 0u;

@group(0) @binding(0) var<uniform> P: SimParams;
@group(0) @binding(1) var<storage, read> dye_in: array<vec4f>;
@group(0) @binding(2) var<storage, read_write> dye_out: array<vec4f>;
@group(0) @binding(3) var<storage, read> velocity: array<vec2f>;
@group(0) @binding(4) var<storage, read> cell_type: array<u32>;
@group(0) @binding(5) var<storage, read> dye_src: array<vec4f>;

// Wrap a position onto a periodic axis so backtraced samples that cross
// the seam re-enter from the other side (non-wrapped axes are untouched).
fn wrap_pos(p: vec2f) -> vec2f {
    var q = p;
    let fw = f32(P.width);
    let fh = f32(P.height);
    if (WRAP_ENABLED != 0u && (P.wrap & 1u) != 0u) { q.x = q.x - fw * floor(q.x / fw); }
    if (WRAP_ENABLED != 0u && (P.wrap & 2u) != 0u) { q.y = q.y - fh * floor(q.y / fh); }
    return q;
}

fn sample_dye(p_in: vec2f) -> vec4f {
    let W = i32(P.width);
    let H = i32(P.height);
    let p = p_in - 0.5;
    // Per axis: bilinear taps wrap across a periodic seam, clamp at an
    // ordinary edge (the pre-periodic behaviour).
    var x0: i32; var x1: i32; var tx: f32;
    if (WRAP_ENABLED != 0u && (P.wrap & 1u) != 0u) {
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
    if (WRAP_ENABLED != 0u && (P.wrap & 2u) != 0u) {
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

    // Backtrace in <= 1-cell substeps so a fast flow (compressible mode,
    // or many sub-steps per frame) can't pull dye straight through a
    // wall: the walk stops at the last free position before a solid.
    let disp = velocity[idx] * P.dye_dt;
    let n = max(1u, min(24u, u32(ceil(length(disp)))));
    let step = disp / f32(n);
    var pos = vec2f(gid.xy) + 0.5;
    for (var k = 0u; k < n; k++) {
        let cand = wrap_pos(pos - step);
        let bx = clamp(i32(floor(cand.x)), 0, i32(W) - 1);
        let by = clamp(i32(floor(cand.y)), 0, i32(H) - 1);
        if (cell_type[u32(by) * W + u32(bx)] == CELL_WALL) {
            break;
        }
        pos = cand;
    }
    var d = sample_dye(pos) * P.dye_decay;

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
