// 2D compressible Euler solver: finite-volume MUSCL (minmod, primitive
// variables) reconstruction with HLLC fluxes and SSP-RK2 time stepping.
// This is the "Compressible (Euler)" solver mode — real gas dynamics with
// shocks, expansion fans and choked nozzles (inviscid: no boundary
// layers).
//
// Nondimensionalization: rho_inf = 1, a_inf (freestream sound speed) = 1,
// so p_inf = 1/gamma and the inlet velocity is the Mach number.
//
// Buffer conventions shared with the LBM path so dye advection, tracer
// particles and the renderer work unchanged:
// - `velocity` holds the per-STEP displacement in cells (u * dt),
// - `density` holds 1 + PRESSURE_RENDER_SCALE * (p - p_inf), so the
//   renderer's (density - 1) gauge-pressure mapping stays meaningful.
//
// NOTE: keep EulerParams in sync with sim.rs (EulerParamsRaw).

struct EulerParams {
    width: u32,
    height: u32,
    gamma: f32,           // ratio of specific heats (1.4)
    mach: f32,            // inlet Mach number (u_inf = mach * a_inf)
    dt: f32,              // CFL-limited time step (nondimensional)
    blend: f32,           // RK: dst = blend*base + (1-blend)*(src + dt*L(src))
    sponge_width: f32,    // absorbing-layer thickness in cells (0 = off)
    sponge_strength: f32, // absorbing-layer blend strength
    free_u: vec2f,        // freestream velocity ((mach, 0) in tunnel mode)
    time: f32,            // steps elapsed (drives fan gusts)
    write_render: f32,    // > 0.5 on the final RK stage: write vel/density
    wrap: u32,            // periodic wrap: bit 0 = x axis, bit 1 = y axis
    _pad2: u32,
    _pad3: u32,
    _pad4: u32,
};

const CELL_FLUID: u32 = 0u;
const CELL_WALL: u32 = 1u;
const CELL_INLET: u32 = 2u;
const CELL_OUTLET: u32 = 3u;

const RHO_FLOOR: f32 = 1e-3;
const P_FLOOR: f32 = 1e-4;
const PRESSURE_RENDER_SCALE: f32 = 0.1;

// Pipeline-specialization switch: 0 compiles the wrap logic out
// entirely (the default pipeline every non-periodic scene runs — zero
// added cost, measured); 1 enables the P.wrap uniform branches. The
// engine binds the variant by EdgeBcs::wrap_bits (sim.rs).
override WRAP_ENABLED: u32 = 0u;

@group(0) @binding(0) var<uniform> P: EulerParams;
// Conserved state (rho, rho*u, rho*v, E) per cell.
@group(0) @binding(1) var<storage, read> u_src: array<vec4f>;
@group(0) @binding(2) var<storage, read> u_base: array<vec4f>;
@group(0) @binding(3) var<storage, read_write> u_dst: array<vec4f>;
@group(0) @binding(4) var<storage, read> cell_type: array<u32>;
// Fan physics per cell: xy = direction * speed multiplier,
// z = gustiness (0..1), w = gust phase.
@group(0) @binding(5) var<storage, read> fan_dir: array<vec4f>;
@group(0) @binding(6) var<storage, read_write> velocity: array<vec2f>;
@group(0) @binding(7) var<storage, read_write> density: array<f32>;

// --- State conversions (primitive w = (rho, u, v, p)) ----------------

fn prim_of(U: vec4f) -> vec4f {
    let rho = max(U.x, RHO_FLOOR);
    let u = U.yz / rho;
    let p = max((P.gamma - 1.0) * (U.w - 0.5 * rho * dot(u, u)), P_FLOOR);
    return vec4f(rho, u.x, u.y, p);
}

fn cons_of(w: vec4f) -> vec4f {
    let e = w.w / (P.gamma - 1.0) + 0.5 * w.x * (w.y * w.y + w.z * w.z);
    return vec4f(w.x, w.x * w.y, w.x * w.z, e);
}

fn free_prim() -> vec4f {
    return vec4f(1.0, P.free_u.x, P.free_u.y, 1.0 / P.gamma);
}

// Fan inlet state: the stored vector's magnitude is a per-fan multiplier
// on the inlet Mach number; gustiness adds the same coherent wander as
// the LBM path (frequencies are exact multiples of 2*pi/65536 per step,
// so the CPU-side time wrap is phase-continuous).
fn inlet_prim(idx: u32) -> vec4f {
    let f4 = fan_dir[idx];
    var dir = f4.xy;
    let turb = f4.z;
    if (turb > 0.0) {
        let ph = f4.w * 6.2831853;
        let t = P.time;
        let ang = turb
            * (0.55 * sin(0.04103400 * t + ph)
                + 0.30 * sin(0.01265534 * t + 2.7 * ph + 1.7));
        let ca = cos(ang);
        let sa = sin(ang);
        dir = vec2f(dir.x * ca - dir.y * sa, dir.x * sa + dir.y * ca);
        let mag = 1.0 + turb
            * (0.30 * sin(0.02732404 * t + 1.9 * ph + 0.6)
                + 0.15 * sin(0.00910801 * t + 0.8 * ph + 3.9));
        dir *= max(mag, 0.0);
    }
    var u = dir * P.mach;
    let sp = length(u);
    if (sp > 8.0) { u *= 8.0 / sp; }
    return vec4f(1.0, u.x, u.y, 1.0 / P.gamma);
}

// --- Loads (zero-gradient at the domain edges via clamping; a periodic
// axis wraps the index around instead) ---------------------------------

// Stencil offsets stay within +/-2 cells, so one period of correction
// (v + n) covers every off-domain coordinate the kernel produces.
fn load_x(x: i32) -> i32 {
    let w = i32(P.width);
    if (WRAP_ENABLED != 0u && (P.wrap & 1u) != 0u) { return (x + w) % w; }
    return clamp(x, 0, w - 1);
}

fn load_y(y: i32) -> i32 {
    let h = i32(P.height);
    if (WRAP_ENABLED != 0u && (P.wrap & 2u) != 0u) { return (y + h) % h; }
    return clamp(y, 0, h - 1);
}

fn cell_at(x: i32, y: i32) -> u32 {
    return cell_type[u32(load_y(y)) * P.width + u32(load_x(x))];
}

fn prim_at(x: i32, y: i32) -> vec4f {
    let idx = u32(load_y(y)) * P.width + u32(load_x(x));
    let ct = cell_type[idx];
    if (ct == CELL_INLET) { return inlet_prim(idx); }
    // Walls are handled by the caller via mirroring; fluid and outlet
    // cells report their evolving state.
    return prim_of(u_src[idx]);
}

// --- Numerics --------------------------------------------------------

fn minmod4(a: vec4f, b: vec4f) -> vec4f {
    let keep = (a * b) > vec4f(0.0);
    let m = sign(a) * min(abs(a), abs(b));
    return select(vec4f(0.0), m, keep);
}

// Physical flux in the normal direction for a normal-first primitive
// state w = (rho, u_n, u_t, p).
fn flux_of(w: vec4f) -> vec4f {
    let e = w.w / (P.gamma - 1.0) + 0.5 * w.x * (w.y * w.y + w.z * w.z);
    return vec4f(
        w.x * w.y,
        w.x * w.y * w.y + w.w,
        w.x * w.y * w.z,
        w.y * (e + w.w),
    );
}

// HLLC approximate Riemann solver (normal-first primitive states).
fn hllc(wl: vec4f, wr: vec4f) -> vec4f {
    let g = P.gamma;
    let al = sqrt(g * wl.w / wl.x);
    let ar = sqrt(g * wr.w / wr.x);
    let sl = min(wl.y - al, wr.y - ar);
    let sr = max(wl.y + al, wr.y + ar);
    if (sl >= 0.0) { return flux_of(wl); }
    if (sr <= 0.0) { return flux_of(wr); }
    // ml < 0 < mr, so the contact-speed denominator never vanishes.
    let ml = wl.x * (sl - wl.y);
    let mr = wr.x * (sr - wr.y);
    let sm = (wr.w - wl.w + wl.y * ml - wr.y * mr) / (ml - mr);
    if (sm >= 0.0) {
        let ul4 = cons_of(wl);
        let c = ml / (sl - sm);
        let ustar = vec4f(
            c,
            c * sm,
            c * wl.z,
            c * (ul4.w / wl.x + (sm - wl.y) * (sm + wl.w / ml)),
        );
        return flux_of(wl) + sl * (ustar - ul4);
    }
    let ur4 = cons_of(wr);
    let c = mr / (sr - sm);
    let ustar = vec4f(
        c,
        c * sm,
        c * wr.z,
        c * (ur4.w / wr.x + (sm - wr.y) * (sm + wr.w / mr)),
    );
    return flux_of(wr) + sr * (ustar - ur4);
}

// Flux through the face between stencil cells 1 and 2 of (w0, w1, w2, w3)
// (normal-first primitives, s* = the cell is solid). Solid neighbours
// become slip-wall mirror ghosts; reconstruction drops to first order
// beside walls.
fn face_flux(
    w0: vec4f, w1in: vec4f, w2in: vec4f, w3: vec4f,
    s0: bool, s1: bool, s2: bool, s3: bool,
) -> vec4f {
    if (s1 && s2) { return vec4f(0.0); }
    var w1 = w1in;
    var w2 = w2in;
    var slope1 = vec4f(0.0);
    var slope2 = vec4f(0.0);
    if (s1) {
        w1 = vec4f(w2.x, -w2.y, w2.z, w2.w);
    } else if (s2) {
        w2 = vec4f(w1.x, -w1.y, w1.z, w1.w);
    } else {
        if (!s0) { slope1 = minmod4(w1 - w0, w2 - w1); }
        if (!s3) { slope2 = minmod4(w2 - w1, w3 - w2); }
    }
    let wl = w1 + 0.5 * slope1;
    let wr = w2 - 0.5 * slope2;
    return hllc(
        vec4f(max(wl.x, RHO_FLOOR), wl.y, wl.z, max(wl.w, P_FLOOR)),
        vec4f(max(wr.x, RHO_FLOOR), wr.y, wr.z, max(wr.w, P_FLOOR)),
    );
}

// Swap the velocity components: maps an (x-normal) primitive/flux to the
// y-normal frame and back (it is its own inverse).
fn swz(w: vec4f) -> vec4f {
    return vec4f(w.x, w.z, w.y, w.w);
}

// A wrapped axis has no edges, so it contributes no sponge distance
// (a sponged periodic edge would damp the flow crossing the seam).
fn sponge_factor(gx: u32, gy: u32) -> f32 {
    if (P.sponge_width <= 0.5) { return 0.0; }
    var dedge = 4294967295u;
    if (WRAP_ENABLED == 0u || (P.wrap & 1u) == 0u) { dedge = min(dedge, min(gx, P.width - 1u - gx)); }
    if (WRAP_ENABLED == 0u || (P.wrap & 2u) == 0u) { dedge = min(dedge, min(gy, P.height - 1u - gy)); }
    if (f32(dedge) >= P.sponge_width) { return 0.0; }
    let t = 1.0 - f32(dedge) / P.sponge_width;
    return P.sponge_strength * t * t;
}

@compute @workgroup_size(8, 8)
fn euler_step(@builtin(global_invocation_id) gid: vec3u) {
    let W = P.width;
    let H = P.height;
    if (gid.x >= W || gid.y >= H) { return; }
    let idx = gid.y * W + gid.x;
    let ct = cell_type[idx];
    let wr_render = P.write_render > 0.5;

    if (ct == CELL_WALL) {
        u_dst[idx] = cons_of(free_prim());
        if (wr_render) {
            velocity[idx] = vec2f(0.0);
            density[idx] = 1.0;
        }
        return;
    }
    if (ct == CELL_INLET) {
        let w = inlet_prim(idx);
        u_dst[idx] = cons_of(w);
        if (wr_render) {
            velocity[idx] = w.yz * P.dt;
            density[idx] = 1.0;
        }
        return;
    }
    if (ct == CELL_OUTLET) {
        // Zero-gradient-ish: relax to the average of the non-solid
        // neighbours so flow leaves without reflecting.
        var acc = vec4f(0.0);
        var cnt = 0.0;
        let x = i32(gid.x);
        let y = i32(gid.y);
        for (var k = 0; k < 4; k++) {
            var nx = x;
            var ny = y;
            if (k == 0) { nx = x + 1; }
            if (k == 1) { nx = x - 1; }
            if (k == 2) { ny = y + 1; }
            if (k == 3) { ny = y - 1; }
            if (WRAP_ENABLED != 0u && (P.wrap & 1u) != 0u) { nx = (nx + i32(W)) % i32(W); }
            if (WRAP_ENABLED != 0u && (P.wrap & 2u) != 0u) { ny = (ny + i32(H)) % i32(H); }
            if (nx < 0 || nx >= i32(W) || ny < 0 || ny >= i32(H)) { continue; }
            let nidx = u32(ny) * W + u32(nx);
            let nct = cell_type[nidx];
            if (nct == CELL_FLUID || nct == CELL_INLET) {
                acc += u_src[nidx];
                cnt += 1.0;
            }
        }
        var dst = cons_of(free_prim());
        if (cnt > 0.0) { dst = acc / cnt; }
        u_dst[idx] = dst;
        if (wr_render) {
            let w = prim_of(dst);
            velocity[idx] = w.yz * P.dt;
            density[idx] = 1.0;
        }
        return;
    }

    let x = i32(gid.x);
    let y = i32(gid.y);

    // Gather the 5-point stencils in x and y (primitive states + solid
    // flags). Domain edges clamp (zero-gradient); a periodic axis wraps.
    var wx: array<vec4f, 5>;
    var sx: array<bool, 5>;
    var wy: array<vec4f, 5>;
    var sy: array<bool, 5>;
    for (var k = -2; k <= 2; k++) {
        wx[k + 2] = prim_at(x + k, y);
        sx[k + 2] = cell_at(x + k, y) == CELL_WALL;
        wy[k + 2] = prim_at(x, y + k);
        sy[k + 2] = cell_at(x, y + k) == CELL_WALL;
    }

    // Faces: left (between x-1 and x), right (x and x+1), and the same in
    // y — every cell computes all four (redundant with neighbours but
    // keeps the kernel single-pass).
    let fxl = face_flux(wx[0], wx[1], wx[2], wx[3], sx[0], sx[1], sx[2], sx[3]);
    let fxr = face_flux(wx[1], wx[2], wx[3], wx[4], sx[1], sx[2], sx[3], sx[4]);
    let fyb = face_flux(
        swz(wy[0]), swz(wy[1]), swz(wy[2]), swz(wy[3]),
        sy[0], sy[1], sy[2], sy[3],
    );
    let fyt = face_flux(
        swz(wy[1]), swz(wy[2]), swz(wy[3]), swz(wy[4]),
        sy[1], sy[2], sy[3], sy[4],
    );

    let div = (fxr - fxl) + swz(fyt - fyb);
    let res = u_src[idx] - P.dt * div;
    var dst = P.blend * u_base[idx] + (1.0 - P.blend) * res;

    // Absorbing sponge toward the freestream near the domain edges.
    let sf = sponge_factor(gid.x, gid.y);
    if (sf > 0.0) {
        dst = mix(dst, cons_of(free_prim()), sf);
    }

    // Blow-up guard on the RAW conserved state — prim_of's floors would
    // mask exactly the bad values we need to catch (negative density or
    // internal energy). Every check is a direct comparison, so a NaN in
    // any component fails it and the cell quietly reinitialises.
    let m2 = dot(dst.yz, dst.yz);
    let e_int = dst.w - 0.5 * m2 / max(dst.x, RHO_FLOOR);
    let ok = dst.x > RHO_FLOOR && dst.x < 60.0
        && e_int > P_FLOOR / (P.gamma - 1.0)
        && e_int < 1.0e4
        && m2 < 144.0 * dst.x * dst.x; // |u| < 12
    if (!ok) {
        dst = cons_of(free_prim());
    }

    u_dst[idx] = dst;
    if (wr_render) {
        let w = prim_of(dst);
        velocity[idx] = w.yz * P.dt;
        density[idx] = 1.0 + PRESSURE_RENDER_SCALE * (w.w - 1.0 / P.gamma);
    }
}

// Reinitialise the written state buffer (and the render fields) to the
// freestream. Dispatched once per state buffer via the rotation bind
// groups.
@compute @workgroup_size(8, 8)
fn euler_reset(@builtin(global_invocation_id) gid: vec3u) {
    let W = P.width;
    let H = P.height;
    if (gid.x >= W || gid.y >= H) { return; }
    let idx = gid.y * W + gid.x;
    u_dst[idx] = cons_of(free_prim());
    velocity[idx] = P.free_u * P.dt;
    density[idx] = 1.0;
}
