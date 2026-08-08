# FlowPaint — CFD you can finger paint, for your desktop

MS Paint, if the canvas were a wind tunnel. FlowPaint is a native
desktop program (Windows / Linux / macOS) that runs a real
computational-fluid-dynamics solver — a D2Q9 lattice-Boltzmann method in
GPU compute shaders — behind a familiar paint interface: tool palette,
shape tools, undo/redo, save/open, PNG export.

On an RTX 3080 it happily pushes the **Ultra** grid (2560 × 1280 —
3.3 million cells) with dozens of solver sub-steps per frame plus up to
**2 million tracer particles** drawn additively on top.

## Features

**Paint tools** (left palette, MS-Paint style):
- **Brush** (B) — freehand strokes; **Line** (L), **Rectangle** (R),
  **Ellipse** (E) — rubber-band previews, stamped on release;
  **Eraser** (X). Right-drag erases with any tool.
- **Polyline** (P) — CAD-style connected wall/fan runs: click vertices,
  Enter or right-click finishes (one undo entry), Esc cancels.
- **Select** (S) — marquee-select any drawn region, then **move it live
  through the flow**, rotate, scale, flip, copy/paste (Ctrl+C/V), nudge
  with arrows, Delete, Enter to apply, Esc to cancel. The selection
  stays physically present while you drag it, so the fluid reacts in
  real time. If it contains fans, sliders appear to retune their
  speed and gustiness in place.
- **CAD sketch aids**: live dimension readouts at the cursor (length,
  angle, width × height), cursor cell coordinates in the status bar,
  Shift = 45°-snapped lines / squares / circles, Alt = draw rectangles
  and ellipses from the centre, and an optional snap-to-grid with
  adjustable spacing.
- **Materials**: Wall (no-slip solid), Fan (inlet — blows along your
  stroke or line direction, with per-fan **speed multiplier** and
  **gustiness** that makes the jet meander and pulse like a real
  blustery inflow), Smoke (colored dye emitter, with color picker),
  Drain (pressure outlet).
- **Undo/redo** (Ctrl+Z / Ctrl+Y) with region snapshots.
- Brush size slider + `[` `]` keys; circle cursor preview.

**Fluid presets** (Physics panel): one click sets the regime — Still
air, Gentle breeze, Wind tunnel (air), Storm (high Re), Water flume,
Glycerin/syrup (creeping flow), and a stylized Supersonic tunnel
(maximum speed and Reynolds number — the solver is incompressible, so
no shocks).

**Generators** (side panel):
- **Airfoil** — full NACA 4-digit generator (camber, camber position,
  thickness, chord, angle of attack) with presets: NACA 0012, 2412
  (Cessna 172), 4412, 0015, 6412, Clark Y, 0006. Inserts as a floating
  selection you can position and rotate before committing.
- **Rocket nozzle** — de Laval (converging-diverging) generator with
  conical or parabolic-bell contours, wall thickness, and an optional
  fan across the chamber so it self-propels. Presets use width ratios
  derived from real engines: V-2, F-1, Merlin 1D, RS-25, Raptor,
  RL10-B2. (The solver is incompressible, so you get the geometry and a
  jet — not real choked compressible flow.)

**Simulation:**
- **Extended domain**: the simulation runs on a larger grid than you
  see — a configurable off-screen margin (None / +25 % / +50 % / +100 %
  per side, default +50 %) pushes the domain boundaries away from the
  visible canvas, and an absorbing sponge layer at the far edges damps
  outgoing pressure waves so boundary reflections don't contaminate
  what you're watching.
- Wind-tunnel mode (left → right) with automatic smoke streaklines
  entering from the far upstream edge.
- Scene presets in the side panel: cylinder (von Kármán vortex street),
  NACA 0012 airfoil, venturi nozzle, backward-facing step, pinball
  field.
- Live controls: flow speed, viscosity (Reynolds number readout),
  solver sub-steps per frame, smoke persistence, pause (Space) / reset,
  plus advanced controls for display gain, smoke brightness, edge
  damping, and particle size/brightness.
- Visible resolutions from 960 × 480 to 2560 × 1280 (the simulated grid
  is bigger by the margin), resampling your drawing on switch.
- Tracer particles are **off by default** — enable them from the View
  section of the side panel.
- The menu bar carries only rarely used operations (file open/save,
  PNG export, grid resolution, domain margin, help); everything you
  touch while playing lives in the side panel.

**Views:** Smoke, Speed (inferno), Vorticity, Pressure (diverging), with
optional highlight tints on fans/drains, plus the particle layer.

**Files:** save/open scenes (`.flow`), export the current view as PNG.
The status bar shows grid size, MLUPS (million lattice-cell updates per
second) and the Reynolds estimate.

## Download

Prebuilt binaries are produced by GitHub Actions for every push that
touches `FlowPaint/` — grab the artifact for your OS from the **Actions**
tab (or from **Releases** for tagged versions):

- `FlowPaint-windows-x64` — Windows 10/11, runs on DX12/Vulkan
- `FlowPaint-linux-x64` — X11/Wayland, Vulkan
- `FlowPaint-macos-arm64` — Metal

No installer, no runtime dependencies — it's a single executable.

## Building from source

Install Rust (https://rustup.rs), then:

```sh
cd FlowPaint
cargo run --release
```

`cargo test` validates all WGSL shaders with naga without needing a GPU.

## How it works

| File | Role |
| --- | --- |
| `src/shaders/lbm.wgsl` | D2Q9 lattice-Boltzmann: BGK collision + pull streaming in one kernel; half-way bounce-back walls, equilibrium inlets, pressure outlets, divergence guard; a reset kernel. |
| `src/shaders/dye.wgsl` | Semi-Lagrangian advection of colored dye with wall-aware backtracing. |
| `src/shaders/particles.wgsl` | Tracer particle advection with hash-based respawning. |
| `src/shaders/render.wgsl` | Letterboxed field visualisation (colormaps, wall rims, boundary tints) and the additive particle overlay. |
| `src/geometry.rs` | Canonical CPU-side scene: cell types, fan directions, dye sources; capsule/rect/ellipse rasterizers; presets; wind tunnel; undo regions. |
| `src/sim.rs` | wgpu engine: buffers, pipelines, ping-pong bind groups, dirty-region uploads, frame encoding, PNG export, scene files. |
| `src/app.rs` | The egui shell: menus, tool palette, canvas interaction, shape previews, status bar. |

The solver runs in lattice units with `tau = 3 nu + 0.5`, a Mach guard at
0.3 lattice speed, and a self-healing divergence check — the same
numerics as the FingerFlow iOS app in this repository, scaled up ~30x in
cell count.

## License

MIT.
