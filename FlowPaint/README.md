# FlowPaint V2 — CFD you can finger paint, for your desktop

A CAD sketchpad, if the canvas were a wind tunnel. FlowPaint is a
native desktop program (Windows / Linux / macOS) that runs a real
computational-fluid-dynamics solver — a D2Q9 lattice-Boltzmann method in
GPU compute shaders — underneath a persistent, object-based sketching
interface: everything you draw stays a live, selectable, editable
object while the fluid reacts to it in real time.

On an RTX 3080 it happily pushes the **Ultra** grid (2560 × 1280 —
3.3 million cells) with dozens of solver sub-steps per frame plus up to
**2 million tracer particles** drawn additively on top.

## The object model

FlowPaint V2 is built around a **persistent vector sketch model**, not a
pixel canvas. Lines, polylines, rectangles, ellipses, pencil strokes
and generated parts are all *objects*: the solver grid is continuously
re-projected from the model (damage-region rasterization), so nothing
is ever "flattened" or destructively committed.

That means, at any time — including mid-simulation — you can:

- **Select** any object (S, topmost under the cursor) and drag it
  through the flow; the fluid reacts live.
- **Drag its vertices/corners** to reshape it (lines, polylines and
  pencil strokes expose every vertex; rectangles and ellipses expose
  their corners, honoring rotation).
- **Retune its physics**: change its material (Wall / Fan / Smoke /
  Drain), thickness, filled/outline, per-fan speed multiplier,
  gustiness and blow direction, smoke color — undoably, from the
  Object panel.
- **Rotate, scale, duplicate (Ctrl+D), nudge (arrows), delete** it.
- **Undo/redo** (Ctrl+Z / Ctrl+Y) every add, edit, move and delete as
  clean per-object steps — panel slider tweaks coalesce into single
  undo entries.

Scene files (`.flow`) store the vector objects plus physics settings,
so they're resolution-independent: load a scene at any grid size and
it re-rasterizes crisply.

## Sketch tools

- **Line (L), Rectangle (R), Ellipse (E)** — rubber-band drawing with
  CAD constraints: **Shift** angle-snaps lines (snap angle settable to
  any increment, presets 5° / 15° / 22.5° / 30° / 45° / 90°) and makes
  squares/circles; **Alt** draws from the centre. Outlines offset to a
  settable thickness, or filled via a toggle.
- **Polyline (P)** — click vertices with a live rubber segment,
  right-click/Enter finishes, click the first vertex to close the
  polygon, Esc cancels.
- **Pencil (B)** — freehand strokes are simplified (Ramer–Douglas–
  Peucker) into clean polylines whose vertices stay draggable.
- **Sketch aids**: optional snap-to-grid with adjustable spacing (drawn
  faintly while a draw tool is armed), live dimension readouts in real
  units (length + angle, width × height, diameters), cursor cell
  coordinates in the status bar.
- **Materials**: Wall (no-slip solid), Fan (inlet — blows along the
  shape for lines/polylines, settable direction for filled shapes, with
  per-fan **speed multiplier** and **gustiness** that makes the jet
  meander and pulse), Smoke (colored dye emitter), Drain (pressure
  outlet). Defaults for new objects sit in the side panel; selected
  objects edit their own copies.

## Two solvers

**Incompressible (LBM)** — the default: a D2Q9 lattice-Boltzmann
method. Viscous, low Mach; smoke, wakes, vortex streets, Reynolds
number control.

**Compressible (Euler)** — one click in the Physics panel switches to a
finite-volume compressible Euler solver (MUSCL reconstruction with a
minmod limiter + HLLC fluxes, SSP-RK2 time stepping): **real gas
dynamics**. Set the inlet Mach number (0.3–3) and you get bow shocks
ahead of blunt bodies, expansion fans, shock diamonds — and rocket
nozzles that genuinely choke at the throat and accelerate supersonic
through the bell, instead of a scaled jet. It's inviscid (no boundary
layers), runs on the same grid, same objects, same views; walls are
slip walls, and the same off-screen margin + sponge absorb outgoing
waves. The legend switches to compressible-flow numbers (sound speed,
Mach, gauge pressure in Pa).

**Fluid presets** (Physics panel): one click sets the regime — Still
air, Gentle breeze, Wind tunnel (air), Storm (high Re), Water flume,
Glycerin/syrup (creeping flow), and a stylized Supersonic tunnel
(maximum speed and Reynolds number within the incompressible solver).
Each preset carries the fluid's real kinematic viscosity, density and
sound speed.

**Real units everywhere**: the canvas maps to a physical domain
(settable width, default 1 m), which anchors every readout — cell size,
time step, speeds in m/s, pressures in Pa. A **legend panel** on the
right shows the important flow numbers (fluid properties, Δx, Δt, inlet
speed, Reynolds number, dynamic pressure, sim rate and elapsed sim
time) plus a labeled color scale for the current view. The Pressure
view shows **gauge pressure** (relative to ambient). Sliders and
dimension readouts are annotated with physical units. The wind tunnel
initializes to a uniform freestream, so there is no impulsive-start
transient (flow arriving from both ends).

**Generators** (side panel) — both insert as objects you can move,
rotate, scale and retune like anything else:
- **Airfoil** — full NACA 4-digit generator (camber, camber position,
  thickness, chord, angle of attack) with presets: NACA 0012, 2412
  (Cessna 172), 4412, 0015, 6412, Clark Y, 0006.
- **Rocket nozzle** — de Laval (converging-diverging) generator with
  conical or parabolic-bell contours, wall thickness, and an optional
  fan across the chamber so it self-propels. Presets use width ratios
  and real exhaust velocities from actual engines: V-2, F-1, Merlin 1D,
  RS-25, Raptor, RL10-B2 — the chamber fan auto-scales so the throat
  jet runs as close to the engine's real ejecta speed as the solver's
  Mach cap allows, and stays adjustable after placement from the
  Object panel. (The solver is incompressible, so you get the geometry
  and a jet — not real choked compressible flow.)

**Simulation:**
- **Extended domain**: the simulation runs on a larger grid than you
  see — a configurable off-screen margin (None / +25 % / +50 % / +100 %
  per side, default +50 %) pushes the domain boundaries away from the
  visible canvas, and an absorbing sponge layer at the far edges damps
  outgoing pressure waves so boundary reflections don't contaminate
  what you're watching.
- Wind-tunnel mode (left → right) with automatic smoke streaklines
  entering from the far upstream edge.
- Scene presets in the side panel — cylinder (von Kármán vortex
  street), NACA 0012 airfoil, venturi, backward-facing step, pinball
  field — built from ordinary editable objects, so you can grab the
  cylinder and drag it around.
- Live controls: flow speed, viscosity (Reynolds number readout),
  solver sub-steps per frame, smoke persistence, pause (Space) / reset,
  plus advanced controls for display gain, smoke brightness, edge
  damping, and particle size/brightness.
- Visible resolutions from 960 × 480 to 2560 × 1280 (the simulated grid
  is bigger by the margin); on switch the vector model rescales and
  re-rasterizes crisply.
- Tracer particles are **off by default** — enable them from the View
  section of the side panel.
- The menu bar carries only rarely used operations (file open/save,
  PNG export, grid resolution, domain margin, help); everything you
  touch while playing lives in the side panel.

**Views:** Smoke, Speed (inferno), Vorticity, Pressure (diverging), with
optional highlight tints on fans/drains, plus the particle layer.

**Files:** save/open scenes (`.flow`, vector format), export the
current view as PNG. The status bar shows object count, grid size,
MLUPS (million lattice-cell updates per second) and the Reynolds
estimate.

## Download

Prebuilt binaries are produced by GitHub Actions for every push that
touches `FlowPaint/` — grab the artifact for your OS from the **Actions**
tab (or from **Releases** for tagged versions):

- `FlowPaint-V2-windows-x64` — Windows 10/11, runs on DX12/Vulkan
- `FlowPaint-V2-linux-x64` — X11/Wayland, Vulkan
- `FlowPaint-V2-macos-arm64` — Metal

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
| `src/shaders/lbm.wgsl` | D2Q9 lattice-Boltzmann: BGK collision + pull streaming in one kernel; half-way bounce-back walls, equilibrium inlets, pressure outlets, divergence guard; freestream reset kernel. |
| `src/shaders/euler.wgsl` | Compressible Euler: MUSCL (minmod, primitive variables) + HLLC finite-volume fluxes, SSP-RK2, slip-wall mirror ghosts, gusty inlets, absorbing sponge, positivity floors with freestream self-healing. |
| `src/shaders/dye.wgsl` | Semi-Lagrangian advection of colored dye with wall-aware backtracing. |
| `src/shaders/particles.wgsl` | Tracer particle advection with hash-based respawning. |
| `src/shaders/render.wgsl` | Letterboxed field visualisation (colormaps, wall rims, boundary tints) and the additive particle overlay. |
| `src/model.rs` | The persistent sketch model: vector objects (line/poly/rect/ellipse/stamp) with materials and fan physics, per-object undo/redo, and the damage-region rasterizer that projects the model onto the solver grid. |
| `src/geometry.rs` | The solver-grid layers (cell types, fan physics, dye sources) and dirty-region bookkeeping. |
| `src/sim.rs` | wgpu engine: buffers, pipelines, ping-pong bind groups, dirty-region uploads, frame encoding, PNG export. |
| `src/app.rs` | The egui shell: gesture-based sketch tools, object/defaults panels, legend, menus, scene files. |
| `src/generators.rs` | Parametric NACA airfoil and de Laval nozzle rasterizers (stamp payloads). |

The LBM solver runs in lattice units with `tau = 3 nu + 0.5`, a Mach
guard at 0.3 lattice speed, and a self-healing divergence check — the
same numerics as the FingerFlow iOS app in this repository, scaled up
~30x in cell count. The Euler solver is nondimensionalized on the
freestream sound speed (ρ∞ = 1, a∞ = 1, γ = 1.4) with a CFL-limited
time step, and shares the LBM path's velocity/pressure render buffers
so every view, the dye advection and the tracer particles work
identically in both modes.

## License

MIT.
