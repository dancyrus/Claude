# CFD you can finger paint

Two apps, one idea: draw geometry, place boundary conditions like game
pieces, and watch a real lattice-Boltzmann fluid solver react in real
time.

| App | Platform | Where |
| --- | --- | --- |
| **FingerFlow** | iPhone / iPad (SwiftUI + Metal) | this page, below |
| **FlowPaint** | Windows / Linux / macOS desktop (Rust + wgpu) | [`FlowPaint/`](FlowPaint/) — MS-Paint-style shell, multi-million-cell grids, tracer particles, undo/redo, save/export |

---

# FingerFlow — a pocket wind tunnel for iPhone

Draw geometry with your finger, place boundary conditions like game
pieces, and watch a real computational-fluid-dynamics solver react in
real time. FingerFlow runs a GPU lattice-Boltzmann simulation (D2Q9)
in Metal compute shaders at interactive frame rates, so vortex streets,
jets and recirculation zones emerge from actual fluid physics — not a
canned animation.

## What you can do

- **Draw walls** with your finger; the flow immediately deflects around
  them (half-way bounce-back, i.e. no-slip boundaries).
- **Place fans (inlets)** — the stroke direction sets the blowing
  direction, so a swipe literally paints a jet.
- **Place drains (outlets)** — pressure outlets that let flow leave the
  domain anywhere you put them.
- **Blow smoke** — paint persistent colored dye emitters and watch
  streaklines thread through your design.
- **Wind-tunnel mode** — a full-edge inlet/outlet pair along the long
  axis of the screen (bottom-to-top in portrait, left-to-right in
  landscape) with automatic streaklines.
- **Visualisations** — smoke, speed (inferno colormap), vorticity and
  pressure (diverging colormaps).
- **Presets** — cylinder (watch the von Kármán vortex street), NACA 0012
  airfoil at angle of attack, venturi nozzle, backward-facing step, and
  a pinball field of cylinders.
- **Tune the physics** — flow speed, viscosity (an approximate Reynolds
  number is displayed), simulation sub-steps per frame, smoke fade,
  pause/resume, reset flow, clear all.

## Building

1. Open `FingerFlow.xcodeproj` in **Xcode 16 or newer** (the project
   uses file-system-synchronized groups).
2. Select your development team under *Signing & Capabilities* (any
   personal team works; there are no entitlements).
3. Build and run on an iPhone or iPad running **iOS 17+**. The
   simulator works too, but a real device is smoother.

There are no dependencies — plain SwiftUI + MetalKit.

## How it works

| Layer | File | Role |
| --- | --- | --- |
| Solver | `FingerFlow/Simulation/Shaders.metal` | D2Q9 lattice-Boltzmann: BGK collision + pull streaming in one kernel, half-way bounce-back walls, equilibrium inlets, pressure outlets, NaN self-healing. A second kernel advects colored dye semi-Lagrangian-style; a third renders the fields straight into the drawable. |
| Shared types | `FingerFlow/Simulation/SimTypes.h` | One header defines the parameter struct and cell-type constants for both Swift (bridging header) and Metal. |
| Engine | `FingerFlow/Simulation/FluidSimulation.swift` | Owns Metal buffers (shared storage), drives N solver steps per frame from the `MTKView` draw loop, and applies finger strokes, presets and the wind tunnel directly into the grids on the CPU. |
| Canvas | `FingerFlow/Views/SimulationCanvasView.swift` | `UIViewRepresentable` around an `MTKView` subclass that forwards touch strokes. |
| UI | `FingerFlow/ContentView.swift`, `FingerFlow/Views/ControlsView.swift` | Game-style overlay: tool bar, brush size, color picker, playback, presets, visualisation picker and a settings sheet. |

### The numerics, briefly

The solver evolves nine particle-distribution functions per cell on a
grid sized to ~224 cells across the short side of the screen (~110k
cells in portrait). Each frame runs several collide-and-stream steps:

- **Collision**: single-relaxation-time BGK, `τ = 3ν + 0.5`, with the
  standard second-order equilibrium.
- **Streaming**: pull scheme — each cell gathers post-collision values
  from its upstream neighbours.
- **Walls**: half-way bounce-back (drawn cells reflect distributions).
- **Fans/inlets**: forced local equilibrium at the painted direction
  and the global flow speed.
- **Drains/outlets**: equilibrium at reference density with the local
  velocity, so momentum exits without reflecting.
- **Stability**: a Mach guard clamps lattice speed at 0.3, viscosity is
  floored so `τ > 0.51`, and any cell that diverges is quietly
  reinitialised instead of spreading NaNs.

The dye you paint is a passive tracer advected through the LBM velocity
field with bilinear semi-Lagrangian backtracing — it visualises the flow
without affecting it.

### Tips for good physics

- The cylinder preset with default settings sits around Re ≈ 200–400:
  expect a wavy wake that rolls up into a vortex street after a few
  seconds. Lower the viscosity or raise the speed for livelier flow.
- Fans are strongest when painted as short strips facing open space.
- Erasing a wall mid-flow is fine — uncovered cells restart from rest.
- Rotating the device rebuilds the grid for the new aspect ratio; your
  drawn geometry is resampled into it (the flow itself restarts).

## License

MIT — do whatever you like with it.
