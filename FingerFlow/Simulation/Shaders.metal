//
//  Shaders.metal
//  FingerFlow
//
//  A D2Q9 lattice-Boltzmann fluid solver plus dye advection and a
//  field-visualisation kernel. Everything runs as Metal compute.
//
//  Distribution functions are stored SoA: f[i * cellCount + cellIndex],
//  i in 0..8. The solver uses the "pull" streaming scheme with half-way
//  bounce-back at walls, equilibrium inlets (fans) and pressure outlets.
//

#include <metal_stdlib>
#include "SimTypes.h"
using namespace metal;

// D2Q9 lattice velocities, their opposites, and weights.
constant int2 E[9] = {
    int2( 0,  0),
    int2( 1,  0), int2( 0,  1), int2(-1,  0), int2( 0, -1),
    int2( 1,  1), int2(-1,  1), int2(-1, -1), int2( 1, -1)
};
constant int OPP[9] = { 0, 3, 4, 1, 2, 7, 8, 5, 6 };
constant float WT[9] = {
    4.0f / 9.0f,
    1.0f / 9.0f,  1.0f / 9.0f,  1.0f / 9.0f,  1.0f / 9.0f,
    1.0f / 36.0f, 1.0f / 36.0f, 1.0f / 36.0f, 1.0f / 36.0f
};

// Maximum lattice speed before the Mach guard clamps velocity.
constant float MAX_LATTICE_SPEED = 0.3f;

static inline float equilibrium(int i, float rho, float2 u, float usq) {
    float eu = dot(float2(E[i]), u);
    return WT[i] * rho * (1.0f + 3.0f * eu + 4.5f * eu * eu - 1.5f * usq);
}

kernel void collideStream(
    device const float*  fIn      [[buffer(0)]],
    device float*        fOut     [[buffer(1)]],
    device const uchar*  cellType [[buffer(2)]],
    device const float2* fanDir   [[buffer(3)]],
    device float2*       velocity [[buffer(4)]],
    device float*        density  [[buffer(5)]],
    constant SimParams&  P        [[buffer(6)]],
    uint2 gid [[thread_position_in_grid]])
{
    const int W = P.width;
    const int H = P.height;
    if (int(gid.x) >= W || int(gid.y) >= H) return;

    const int n   = W * H;
    const int idx = int(gid.y) * W + int(gid.x);
    const uchar ct = cellType[idx];

    if (ct == CELL_WALL) {
        // Wall populations are never read by neighbours (they bounce back
        // off their own cell), so just keep the wall finite and quiet.
        velocity[idx] = float2(0.0f);
        density[idx]  = 1.0f;
        for (int i = 0; i < 9; i++) fOut[i * n + idx] = WT[i];
        return;
    }

    // --- Streaming (pull) ---------------------------------------------
    // f[i] arrives from the cell one lattice vector upstream. A wall
    // upstream reflects our own opposite population (half-way bounce-back).
    // Off-domain neighbours copy the local value (zero-gradient open edge).
    float f[9];
    for (int i = 0; i < 9; i++) {
        const int sx = int(gid.x) - E[i].x;
        const int sy = int(gid.y) - E[i].y;
        if (sx < 0 || sx >= W || sy < 0 || sy >= H) {
            f[i] = fIn[i * n + idx];
        } else {
            const int sidx = sy * W + sx;
            if (cellType[sidx] == CELL_WALL) {
                f[i] = fIn[OPP[i] * n + idx];
            } else {
                f[i] = fIn[i * n + sidx];
            }
        }
    }

    // --- Boundary cells ------------------------------------------------
    if (ct == CELL_INLET) {
        // Fan: force equilibrium at the painted direction and speed.
        const float2 u = fanDir[idx] * P.inletSpeed;
        const float usq = dot(u, u);
        for (int i = 0; i < 9; i++) fOut[i * n + idx] = equilibrium(i, 1.0f, u, usq);
        velocity[idx] = u;
        density[idx]  = 1.0f;
        return;
    }

    float rho = 0.0f;
    float2 mom = float2(0.0f);
    for (int i = 0; i < 9; i++) {
        rho += f[i];
        mom += float2(E[i]) * f[i];
    }

    if (ct == CELL_OUTLET) {
        // Pressure outlet: reference density, keep the local velocity so
        // flow can leave the domain without reflecting.
        float2 u = mom; // rho is pinned to 1
        const float sp = length(u);
        if (sp > MAX_LATTICE_SPEED) u *= MAX_LATTICE_SPEED / sp;
        const float usq = dot(u, u);
        for (int i = 0; i < 9; i++) fOut[i * n + idx] = equilibrium(i, 1.0f, u, usq);
        velocity[idx] = u;
        density[idx]  = 1.0f;
        return;
    }

    // --- Blow-up guard ---------------------------------------------------
    // If the cell diverged (NaN or wild density), quietly reinitialise it.
    if (!isfinite(rho) || rho < 0.1f || rho > 5.0f) {
        for (int i = 0; i < 9; i++) fOut[i * n + idx] = WT[i];
        velocity[idx] = float2(0.0f);
        density[idx]  = 1.0f;
        return;
    }

    // --- Collision (BGK) -------------------------------------------------
    float2 u = mom / rho;
    const float sp = length(u);
    if (sp > MAX_LATTICE_SPEED) u *= MAX_LATTICE_SPEED / sp;

    const float usq = dot(u, u);
    for (int i = 0; i < 9; i++) {
        const float fe = equilibrium(i, rho, u, usq);
        fOut[i * n + idx] = f[i] + P.omega * (fe - f[i]);
    }
    velocity[idx] = u;
    density[idx]  = rho;
}

// ---------------------------------------------------------------------------
// Bilinear samplers over the cell-centred grids.
// ---------------------------------------------------------------------------

static float4 sampleDye(device const float4* dye, float2 p, int W, int H) {
    p = clamp(p - 0.5f, float2(0.0f), float2(float(W - 1), float(H - 1)));
    const int x0 = int(p.x), y0 = int(p.y);
    const int x1 = min(x0 + 1, W - 1), y1 = min(y0 + 1, H - 1);
    const float tx = p.x - float(x0), ty = p.y - float(y0);
    const float4 d00 = dye[y0 * W + x0], d10 = dye[y0 * W + x1];
    const float4 d01 = dye[y1 * W + x0], d11 = dye[y1 * W + x1];
    return mix(mix(d00, d10, tx), mix(d01, d11, tx), ty);
}

static float2 sampleVel(device const float2* vel, float2 p, int W, int H) {
    p = clamp(p - 0.5f, float2(0.0f), float2(float(W - 1), float(H - 1)));
    const int x0 = int(p.x), y0 = int(p.y);
    const int x1 = min(x0 + 1, W - 1), y1 = min(y0 + 1, H - 1);
    const float tx = p.x - float(x0), ty = p.y - float(y0);
    const float2 v00 = vel[y0 * W + x0], v10 = vel[y0 * W + x1];
    const float2 v01 = vel[y1 * W + x0], v11 = vel[y1 * W + x1];
    return mix(mix(v00, v10, tx), mix(v01, v11, tx), ty);
}

// ---------------------------------------------------------------------------
// Passive dye: semi-Lagrangian advection through the LBM velocity field,
// with persistent painted sources.
// ---------------------------------------------------------------------------

kernel void advectDye(
    device const float4* dyeIn    [[buffer(0)]],
    device float4*       dyeOut   [[buffer(1)]],
    device const float2* velocity [[buffer(2)]],
    device const uchar*  cellType [[buffer(3)]],
    device const float4* dyeSrc   [[buffer(4)]],
    constant SimParams&  P        [[buffer(5)]],
    uint2 gid [[thread_position_in_grid]])
{
    const int W = P.width;
    const int H = P.height;
    if (int(gid.x) >= W || int(gid.y) >= H) return;
    const int idx = int(gid.y) * W + int(gid.x);

    if (cellType[idx] == CELL_WALL) {
        dyeOut[idx] = float4(0.0f);
        return;
    }

    const float2 pos = float2(gid) + 0.5f - velocity[idx] * P.dyeDt;

    // Don't pull dye through a solid: if the backtraced point lands inside
    // a wall cell, keep the local dye instead of sampling across it.
    const int bx = clamp(int(floor(pos.x)), 0, W - 1);
    const int by = clamp(int(floor(pos.y)), 0, H - 1);
    float4 d;
    if (cellType[by * W + bx] == CELL_WALL) {
        d = dyeIn[idx] * P.dyeDecay;
    } else {
        d = sampleDye(dyeIn, pos, W, H) * P.dyeDecay;
    }

    const float4 src = dyeSrc[idx];
    if (src.a > 0.0f) {
        d.rgb = max(d.rgb, src.rgb * src.a);
    }
    dyeOut[idx] = clamp(d, 0.0f, 1.0f);
}

// ---------------------------------------------------------------------------
// Visualisation: writes straight into the drawable texture.
// ---------------------------------------------------------------------------

static float3 infernoMap(float t) {
    t = clamp(t, 0.0f, 1.0f) * 4.0f;
    const float3 c0 = float3(0.001f, 0.000f, 0.014f);
    const float3 c1 = float3(0.341f, 0.062f, 0.429f);
    const float3 c2 = float3(0.730f, 0.216f, 0.330f);
    const float3 c3 = float3(0.973f, 0.555f, 0.035f);
    const float3 c4 = float3(0.988f, 0.998f, 0.645f);
    if (t < 1.0f) return mix(c0, c1, t);
    if (t < 2.0f) return mix(c1, c2, t - 1.0f);
    if (t < 3.0f) return mix(c2, c3, t - 2.0f);
    return mix(c3, c4, t - 3.0f);
}

// Diverging blue-white-red map; t in [-1, 1].
static float3 coolwarmMap(float t) {
    t = clamp(t, -1.0f, 1.0f);
    const float3 cold  = float3(0.230f, 0.299f, 0.754f);
    const float3 white = float3(0.940f, 0.930f, 0.920f);
    const float3 warm  = float3(0.706f, 0.016f, 0.150f);
    return (t < 0.0f) ? mix(white, cold, -t) : mix(white, warm, t);
}

kernel void renderField(
    texture2d<half, access::write> outTex [[texture(0)]],
    device const float2* velocity [[buffer(0)]],
    device const float*  density  [[buffer(1)]],
    device const float4* dye      [[buffer(2)]],
    device const uchar*  cellType [[buffer(3)]],
    constant SimParams&  P        [[buffer(4)]],
    uint2 gid [[thread_position_in_grid]])
{
    if (gid.x >= outTex.get_width() || gid.y >= outTex.get_height()) return;

    const int W = P.width;
    const int H = P.height;
    // Per-axis pixel-to-grid mapping so display stays aligned with painting
    // even when grid and drawable aspect ratios drift slightly.
    const float2 scale = float2(float(W) / float(outTex.get_width()),
                                float(H) / float(outTex.get_height()));
    const float2 g = (float2(gid) + 0.5f) * scale;
    const int cx = clamp(int(g.x), 0, W - 1);
    const int cy = clamp(int(g.y), 0, H - 1);
    const int idx = cy * W + cx;
    const uchar ct = cellType[idx];

    float3 col;
    if (ct == CELL_WALL) {
        // Solid body: light fill with a brighter rim against the fluid.
        bool rim = false;
        const int nx[4] = { cx + 1, cx - 1, cx, cx };
        const int ny[4] = { cy, cy, cy + 1, cy - 1 };
        for (int k = 0; k < 4; k++) {
            if (nx[k] < 0 || nx[k] >= W || ny[k] < 0 || ny[k] >= H) continue;
            if (cellType[ny[k] * W + nx[k]] != CELL_WALL) { rim = true; break; }
        }
        col = rim ? float3(0.93f, 0.94f, 0.97f) : float3(0.58f, 0.62f, 0.70f);
    } else {
        switch (P.renderMode) {
            case RENDER_SPEED: {
                const float s = length(sampleVel(velocity, g, W, H));
                col = infernoMap(s / max(P.inletSpeed * 1.6f, 1e-3f));
                break;
            }
            case RENDER_VORTICITY: {
                const float2 vr = sampleVel(velocity, g + float2(1.0f, 0.0f), W, H);
                const float2 vl = sampleVel(velocity, g - float2(1.0f, 0.0f), W, H);
                const float2 vu = sampleVel(velocity, g + float2(0.0f, 1.0f), W, H);
                const float2 vd = sampleVel(velocity, g - float2(0.0f, 1.0f), W, H);
                const float curl = 0.5f * ((vr.y - vl.y) - (vu.x - vd.x));
                col = coolwarmMap(curl * (4.0f / max(P.inletSpeed, 0.02f)));
                break;
            }
            case RENDER_PRESSURE: {
                const float p = density[idx] - 1.0f;
                col = coolwarmMap(p * 25.0f);
                break;
            }
            default: { // RENDER_DYE
                const float3 bg = float3(0.030f, 0.040f, 0.070f);
                const float3 d = sampleDye(dye, g, W, H).rgb;
                col = 1.0f - (1.0f - bg) * (1.0f - d); // screen blend
                break;
            }
        }
        // Tint the placed boundary-condition cells so they stay visible.
        if (ct == CELL_INLET)  col = mix(col, float3(0.20f, 0.95f, 0.90f), 0.45f);
        if (ct == CELL_OUTLET) col = mix(col, float3(0.55f, 0.25f, 0.85f), 0.45f);
    }

    outTex.write(half4(half3(col), 1.0h), gid);
}
