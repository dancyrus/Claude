// Field visualisation (fullscreen triangle, letterboxed into the canvas
// viewport) plus the tracer-particle overlay (instanced quads, additive).
//
// Coordinate conventions: the render pass viewport is set to the canvas
// rect; @builtin(position) in the fragment shader is in absolute
// framebuffer pixels, and RenderParams carries the letterbox mapping in
// the same absolute pixels.
//
// NOTE: keep RenderParams in sync with sim.rs (RenderParamsRaw).

struct RenderParams {
    width: u32,        // full grid cells in x (including margin)
    height: u32,       // full grid cells in y
    mode: u32,         // 0 dye, 1 speed, 2 vorticity, 3 pressure
    flags: u32,        // bit 0: draw boundary tints; bit 1: swap the
                       // view's colormap (speed -> coolwarm, vorticity/
                       // pressure -> inferno). The legend mirrors both
                       // maps CPU-side (app.rs stop tables).
    vp_origin: vec2f,  // canvas viewport origin, framebuffer px
    vp_size: vec2f,    // canvas viewport size, framebuffer px
    lb_origin: vec2f,  // letterboxed visible-window origin, framebuffer px
    px_per_cell: f32,
    inlet_speed: f32,
    vis_origin: vec2u, // visible window offset into the full grid (margin)
    vis_size: vec2u,   // visible window size in cells
    display_gain: f32, // user gain on speed/vorticity/pressure mapping
    smoke_gain: f32,   // user gain on smoke brightness
    particle_size: f32,        // half-size of a particle quad, px
    particle_brightness: f32,  // peak particle alpha
};

const CELL_FLUID: u32 = 0u;
const CELL_WALL: u32 = 1u;
const CELL_INLET: u32 = 2u;
const CELL_OUTLET: u32 = 3u;

const MODE_DYE: u32 = 0u;
const MODE_SPEED: u32 = 1u;
const MODE_VORTICITY: u32 = 2u;
const MODE_PRESSURE: u32 = 3u;

@group(0) @binding(0) var<uniform> R: RenderParams;
@group(0) @binding(1) var<storage, read> velocity: array<vec2f>;
@group(0) @binding(2) var<storage, read> density: array<f32>;
@group(0) @binding(3) var<storage, read> dye: array<vec4f>;
@group(0) @binding(4) var<storage, read> cell_type: array<u32>;

fn sample_vel(p_in: vec2f) -> vec2f {
    let W = i32(R.width);
    let H = i32(R.height);
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

fn sample_dye_rgb(p_in: vec2f) -> vec3f {
    let W = i32(R.width);
    let H = i32(R.height);
    let p = clamp(p_in - 0.5, vec2f(0.0), vec2f(f32(W - 1), f32(H - 1)));
    let x0 = i32(p.x);
    let y0 = i32(p.y);
    let x1 = min(x0 + 1, W - 1);
    let y1 = min(y0 + 1, H - 1);
    let tx = p.x - f32(x0);
    let ty = p.y - f32(y0);
    let d00 = dye[y0 * W + x0];
    let d10 = dye[y0 * W + x1];
    let d01 = dye[y1 * W + x0];
    let d11 = dye[y1 * W + x1];
    return mix(mix(d00, d10, tx), mix(d01, d11, tx), ty).rgb;
}

fn inferno_map(t_in: f32) -> vec3f {
    let t = clamp(t_in, 0.0, 1.0) * 4.0;
    let c0 = vec3f(0.001, 0.000, 0.014);
    let c1 = vec3f(0.341, 0.062, 0.429);
    let c2 = vec3f(0.730, 0.216, 0.330);
    let c3 = vec3f(0.973, 0.555, 0.035);
    let c4 = vec3f(0.988, 0.998, 0.645);
    if (t < 1.0) { return mix(c0, c1, t); }
    if (t < 2.0) { return mix(c1, c2, t - 1.0); }
    if (t < 3.0) { return mix(c2, c3, t - 2.0); }
    return mix(c3, c4, t - 3.0);
}

// Diverging blue-white-red map; t in [-1, 1].
fn coolwarm_map(t_in: f32) -> vec3f {
    let t = clamp(t_in, -1.0, 1.0);
    let cold = vec3f(0.230, 0.299, 0.754);
    let white = vec3f(0.940, 0.930, 0.920);
    let warm = vec3f(0.706, 0.016, 0.150);
    if (t < 0.0) { return mix(white, cold, -t); }
    return mix(white, warm, t);
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
    // One oversized triangle covering the viewport.
    let xy = vec2f(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4f(xy * 4.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_field(@builtin(position) frag: vec4f) -> @location(0) vec4f {
    let W = i32(R.width);
    let H = i32(R.height);
    // Map the pixel into the VISIBLE window, then offset into the full
    // grid (the margin ring around the window is simulated but not shown).
    let gv = (frag.xy - R.lb_origin) / R.px_per_cell;
    if (gv.x < 0.0 || gv.y < 0.0
        || gv.x >= f32(R.vis_size.x) || gv.y >= f32(R.vis_size.y)) {
        return vec4f(0.09, 0.10, 0.12, 1.0);
    }
    let g = gv + vec2f(R.vis_origin);

    let cx = clamp(i32(g.x), 0, W - 1);
    let cy = clamp(i32(g.y), 0, H - 1);
    let idx = cy * W + cx;
    let ct = cell_type[idx];

    var col: vec3f;
    if (ct == CELL_WALL) {
        // Solid body: light fill with a brighter rim against the fluid.
        var rim = false;
        for (var k = 0; k < 4; k++) {
            var nx = cx;
            var ny = cy;
            if (k == 0) { nx = cx + 1; }
            if (k == 1) { nx = cx - 1; }
            if (k == 2) { ny = cy + 1; }
            if (k == 3) { ny = cy - 1; }
            if (nx < 0 || nx >= W || ny < 0 || ny >= H) { continue; }
            if (cell_type[ny * W + nx] != CELL_WALL) { rim = true; }
        }
        if (rim) {
            col = vec3f(0.93, 0.94, 0.97);
        } else {
            col = vec3f(0.58, 0.62, 0.70);
        }
    } else {
        switch R.mode {
            case MODE_SPEED: {
                let s = length(sample_vel(g));
                let t = s * R.display_gain / max(R.inlet_speed * 1.6, 1e-3);
                if ((R.flags & 2u) != 0u) {
                    // Sequential data on the diverging map: 0..1 spans
                    // the full blue-white-red ramp.
                    col = coolwarm_map(t * 2.0 - 1.0);
                } else {
                    col = inferno_map(t);
                }
            }
            case MODE_VORTICITY: {
                let vr = sample_vel(g + vec2f(1.0, 0.0));
                let vl = sample_vel(g - vec2f(1.0, 0.0));
                let vu = sample_vel(g + vec2f(0.0, 1.0));
                let vd = sample_vel(g - vec2f(0.0, 1.0));
                let curl = 0.5 * ((vr.y - vl.y) - (vu.x - vd.x));
                let t = curl * R.display_gain * (4.0 / max(R.inlet_speed, 0.02));
                if ((R.flags & 2u) != 0u) {
                    // Diverging data on the sequential map: -1..1 spans
                    // the inferno ramp end to end.
                    col = inferno_map(t * 0.5 + 0.5);
                } else {
                    col = coolwarm_map(t);
                }
            }
            case MODE_PRESSURE: {
                let p = density[idx] - 1.0;
                let t = p * R.display_gain * 25.0;
                if ((R.flags & 2u) != 0u) {
                    col = inferno_map(t * 0.5 + 0.5);
                } else {
                    col = coolwarm_map(t);
                }
            }
            default: { // MODE_DYE
                let bg = vec3f(0.030, 0.040, 0.070);
                let d = clamp(sample_dye_rgb(g) * R.smoke_gain, vec3f(0.0), vec3f(1.0));
                col = 1.0 - (1.0 - bg) * (1.0 - d); // screen blend
            }
        }
        if ((R.flags & 1u) != 0u) {
            // Tint placed boundary-condition cells so they stay visible.
            if (ct == CELL_INLET) { col = mix(col, vec3f(0.20, 0.95, 0.90), 0.45); }
            if (ct == CELL_OUTLET) { col = mix(col, vec3f(0.55, 0.25, 0.85), 0.45); }
        }
    }

    return vec4f(col, 1.0);
}

// --- Tracer particles ------------------------------------------------

@group(1) @binding(0) var<storage, read> particles: array<vec4f>;

struct ParticleVsOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
    @location(1) alpha: f32,
};

var<private> QUAD: array<vec2f, 6> = array<vec2f, 6>(
    vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
    vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0),
);

@vertex
fn vs_particles(@builtin(vertex_index) vi: u32) -> ParticleVsOut {
    let pi = vi / 6u;
    let corner = QUAD[vi % 6u];
    let p = particles[pi];

    var out: ParticleVsOut;

    // Cull particles that live in the off-screen margin ring: the letterbox
    // bars are inside the scissor rect, so they must not be drawn there.
    let pv = p.xy - vec2f(R.vis_origin);
    if (pv.x < 0.0 || pv.y < 0.0
        || pv.x >= f32(R.vis_size.x) || pv.y >= f32(R.vis_size.y)) {
        out.pos = vec4f(-10.0, -10.0, 0.0, 1.0);
        out.uv = corner;
        out.alpha = 0.0;
        return out;
    }

    let W = i32(R.width);
    let H = i32(R.height);
    let cx = clamp(i32(p.x), 0, W - 1);
    let cy = clamp(i32(p.y), 0, H - 1);
    let speed = length(velocity[cy * W + cx]);

    // Fade in over the first part of life, out over the last.
    let life = max(p.w, 1.0);
    let t = clamp(p.z / life, 0.0, 1.0);
    let envelope = smoothstep(0.0, 0.08, t) * (1.0 - smoothstep(0.75, 1.0, t));
    let alpha = envelope
        * clamp(speed / max(R.inlet_speed, 0.02), 0.06, 1.0)
        * R.particle_brightness;

    let px = R.lb_origin + pv * R.px_per_cell + corner * R.particle_size;
    let ndc = (px - R.vp_origin) / R.vp_size * 2.0 - 1.0;

    out.pos = vec4f(ndc.x, -ndc.y, 0.0, 1.0);
    out.uv = corner;
    out.alpha = alpha;
    return out;
}

@fragment
fn fs_particles(in: ParticleVsOut) -> @location(0) vec4f {
    let d = length(in.uv);
    let a = in.alpha * (1.0 - smoothstep(0.3, 1.0, d));
    // Additive blend (ONE, ONE): output premultiplied light.
    return vec4f(vec3f(0.85, 0.92, 1.0) * a, a);
}
