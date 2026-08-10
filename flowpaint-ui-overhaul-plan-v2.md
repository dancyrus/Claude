# FlowPaint V2 — UI overhaul and three functional fixes

## Status of this document

The stack and data-model details below were recovered from a release binary and a
screenshot, not from the source. **Treat every code-level claim here as unverified until
phase 0 confirms it.** Where phase 0 contradicts this document, phase 0 wins and this
document gets corrected.

Companion file: `docs/ui-target.html`. Open it in a browser. It is a static mockup of the
target layout.

- **Authoritative** for spacing, color, typography, panel widths, and panel arrangement.
  The CSS variable block at the top of that file holds the real values; port them into
  `theme.rs` directly rather than eyeballing them.
- **Not authoritative** for widget construction, numeric values, or canvas rendering. The
  HTML structure is not the egui widget tree. egui is immediate mode with fixed panel
  nesting; a `<div class="grp">` with a caption beneath it becomes a `ui.vertical` inside a
  `ui.horizontal` with a `Separator`, not a translated div. Do not add a flexbox shim or a
  CSS-emulation layer to make the translation literal. Every number in the mockup
  (`4.31e-5 s`, `A* = 27.3 mm`, `Re = 3.21e5`) is invented for legibility, as are the plume
  gradient and shock-train marks.

## What this repo is

Rust desktop app. `eframe` / `egui` 0.29.1 on `egui-wgpu` with `wgpu` 22.1.0 and `winit`,
OpenGL/WGL fallback via `glutin`. Two solvers: D2Q9 lattice-Boltzmann in GPU compute
shaders for the incompressible mode, finite-volume Euler with MUSCL reconstruction and
HLLC fluxes for the compressible mode. Sketched geometry is a list of live objects; the
serialized shape appeared to be:

```
ObjMaterial = Wall | Fan | Smoke | Drain
SketchObject { id, shape, material, thickness, filled, fan_mult, fan_gust,
               fan_phase, fan_angle, smoke_rgb, pts, closed, c, halfangle, r,
               raster, scale }
```

This came out of a binary that may predate commit #10 ("object-model sketching rebuild"),
so the field list may be stale. Confirm before relying on it.

## The goal

The UI does not read as an engineering tool. Target the Ansys / COMSOL arrangement:
tabbed ribbon, model tree, a settings column that shows the selected tree node, a graphics
window, and a message and status strip.

Do not change solver physics. No new numerical schemes, no changes to shader math, no
changes to default physical values. If a layout change requires touching solver state,
stop and report instead.

---

## Phase 0 — inventory. Do not modify any code.

Read the UI source and produce `docs/ui-inventory.md` containing one table row per user
facing control:

| widget | current label | current units | backing state field | file:line | owning panel |

Also record in that file:

1. How the UI code is split today. Every file that draws widgets, with line counts.
2. Where object selection state lives, and how the object inspector decides which property
   block to draw for a given `ObjMaterial`.
3. Where the nozzle generator constructs its objects, and specifically where the chamber
   fan gets created and where its speed cap is applied.
4. Every hardcoded `Color32`, `Rounding`, `Stroke`, `FontId`, and magic pixel constant,
   with counts by file.
5. The current `egui` and `eframe` versions in `Cargo.lock`, and whether any egui API the
   code uses was removed between 0.29 and 0.35.
6. **Read the messages and diffs of commits #7 ("Move everyday controls to the side panel;
   menu bar keeps rare ops") and #10 ("object-model sketching rebuild").** The current
   single-column control panel is a recent deliberate decision, not neglect. If anything in
   this plan reverses a decision made in those commits, say so explicitly in the report
   rather than silently overwriting it.

Delegate items 1–3 to one subagent and 4–6 to another; they do not overlap.

**Stop after phase 0 and report.** No implementation code until the inventory is reviewed.

---

## Phase 1 — egui upgrade, isolated

`egui` is at 0.35 (released 2026-06-25); this repo is on 0.29.1, six releases behind. 0.34
replaced the font rasterizer with skrifa and vello_cpu and added hinting, which fixes the
soft text in the current build on its own. 0.32 introduced `Atom` and `AtomLayout` and
rewrote `Button`, so button construction sites will need edits.

Do the upgrade as its own commit, with no styling or layout changes mixed in. Build, run,
and confirm the app renders and the solver still steps before you commit. If the upgrade
needs more than roughly 40 changed lines outside `Cargo.toml`, stop and report the diff
scope rather than pushing through.

If `wgpu` must move in lockstep with `egui-wgpu`, treat that as part of this phase and
verify all three backends still initialize (Vulkan, DX12, and the OpenGL fallback path).

---

## Phase 2 — theme layer

Create `src/ui/theme.rs`. Everything visual resolves through it. After this phase,
`grep -rn "Color32::from_rgb" src/` outside `theme.rs` must return nothing.

Take the palette, radius, type sizes, and panel dimensions from the `:root` block of
`docs/ui-target.html`.

Requirements:

- One `Theme` struct built once at startup and applied through `ctx.set_style`, with a dark
  default. Expose it in state so panels read from it rather than defining locals.
- **All numeric readouts render in the monospace `TextStyle`, with tabular figures.** This
  is the single largest change in how the app reads. Proportional digits in a value box
  make an instrument look like a toy; column-aligned monospace digits do not.
- Set `Spacing::item_spacing`, `button_padding`, `indent`, and `slider_width` once. Current
  spacing varies between panels for no reason.
- Two accent colors maximum: one for selection and active state, one for destructive
  actions. Everything else is a neutral ramp.
- Near-square widget rounding (3 px). The current pill-shaped toggles are a large part of
  the toy impression.
- Add `egui-phosphor` for ribbon icons. Bundle one variant only. Icons never carry meaning
  alone; every ribbon button has a text label under its glyph.

---

## Phase 3 — layout rebuild

Replace the single scrolling control column with this shell, top to bottom. Widths and
heights are in `docs/ui-target.html`.

1. `TopBottomPanel::top` — menu bar. File, Edit, Simulation, View, Help. Thin, text only.
2. `TopBottomPanel::top` — **ribbon.** A tab strip of `selectable_label` items (Home,
   Geometry, Physics, Study, Results) over a body around 86 px tall. The body holds grouped
   icon-and-label buttons separated by vertical rules, with a small caption under each
   group. Group assignments:
   - **Home** — pause, step, reset flow, clear all, undo, redo, save, snapshot
   - **Geometry** — select, line, rectangle, ellipse, polyline, pencil, **eraser**;
     material picker (Wall, Fan, Smoke, Drain); thickness; filled toggle; angle snap
   - **Physics** — solver mode, fluid preset, inlet Mach, steps per frame, sponge strength,
     wind tunnel toggle, domain width
   - **Study** — generators (airfoil, rocket nozzle) and scene presets
   - **Results** — field selector (smoke, speed, vorticity, pressure), legend toggle,
     particle count, display gain, brightness, particle size, **domain extent toggle**
     (phase 6b)
3. `SidePanel::left`, resizable, default 212 px — **model tree.** A real tree over actual
   scene contents. Root node for the domain, a child for solver settings, then one node per
   `SketchObject` labeled by material and id, with generator children nested under their
   parent (the nozzle bell owns the chamber engine). Clicking selects. Right-click gives
   duplicate and delete. This replaces the current flat object flow entirely.
4. `SidePanel::left`, resizable, default 258 px — **settings.** Draws the property block for
   whatever the tree has selected, and nothing else. This is where phase 5's engine group
   lives. With nothing selected, show domain and solver settings, not an empty panel.
5. `CentralPanel` — graphics window. The canvas keeps its full area; nothing overlaps it
   except the color legend and the probe readout.
6. `TopBottomPanel::bottom` — a message line for solver events (choked flow detected, cell
   reinitialized by the blow-up guard), then the **status strip**: live, monospace, updated
   every frame — grid dimensions, cell size, time step, elapsed sim time, inlet speed in
   m/s, **CFL number**, frame time, and a stability indicator.

The status strip is what makes this read as a solver; ribbon chrome alone will not. CFL is
not currently exposed anywhere in the app and should be. For a compressible Euler solver it
is the number that says whether the run is about to come apart, and it is cheap to compute.

### Text and label discipline in this phase

- Fix `Highlight fans && drains`. The ampersand is not escaped.
- Sentence case for every label, consistently.
- Slider value boxes go on one side, the same side, everywhere. They currently sit left of
  the label in Physics and right of it in Sketch aids.
- `Clear all` and `Reset flow` are both destructive and neither confirms. Give both a
  confirmation, and use the destructive accent for both or neither.
- Apply ASD-STE100 Issue 7 to every tooltip and help string you write or rewrite. One
  instruction per sentence, approved vocabulary, no gerund strings. Existing tooltips are in
  scope where they violate it.

---

## Phase 4 — units and number formatting

Create `src/ui/units.rs`. No readout formats itself inline after this phase.

- One formatter per physical quantity, each owning its own decimal count. SI is the default
  for this app; follow ASME decimal-inch conventions in any inch mode.
- Fix mixed base units inside single panels. Thickness currently reads `6.0` with a
  `(3.1 mm)` derived note while domain width reads `1.00` with `(m)` in the label. Pick one
  convention: canonical SI value in the box, unit in the label, derived convenience value on
  a secondary line or in the tooltip.
- Mach is dimensionless. Show `1.600` as the primary value with the derived `548.8 m/s` as
  secondary text, and make the derivation visible (`u = M · a`, with `a` stated). The
  mockup's derived-value lines show the intended pattern; copy the pattern, not the numbers.
- Replace bare `Slider` with `DragValue` plus `suffix` for anything an engineer would type
  an exact value into: Mach, domain width, thickness, steps per frame, sponge strength. Keep
  sliders for display-only gains where sweeping is the point.
- Every value that reaches the screen gets rounded at the formatter. No raw float artifacts.

---

## Phase 5 — nozzle engine property group

The nozzle generator inserts an object described in the current UI as `Fan in the chamber
(self-powered) (speed-capped)`. It does not expose the fan properties a hand-placed fan gets
(`fan_mult`, `fan_gust`, `fan_angle`).

Build a separate engine property group rather than reusing the generic fan block. The
chamber fan is not a fan in the user's mental model, it is an engine.

- Add an engine parameter struct owned by the nozzle object, holding at minimum chamber
  drive strength, gustiness, and the speed cap.
- **The speed cap becomes an explicit, visible, editable field with a stated reason**, not a
  hidden clamp. Find why it was capped and put that reason in the tooltip in STE. If the cap
  protects the solver, keep a hard bound and say so; do not silently clamp a typed value
  without telling the user.
- The group renders in the settings panel when the nozzle or its engine node is selected.
- Add a solver-dependency note in that panel stating plainly which controls do nothing in
  the active mode, rather than hiding them. In compressible mode the bell accelerates the
  flow through the throat, so chamber drive and throat geometry are meaningful. In
  incompressible mode there is no choked-flow behavior at all.
- Regression check: hand-placed fans keep their existing property block unchanged.

---

## Phase 6 — the two remaining defects

### 6a. Eraser regression

The eraser no longer removes part of a shape; the only removal path left is deleting a whole
object. Diagnose before designing.

1. **Check commit #10 ("object-model sketching rebuild") first.** An object-model rebuild is
   the most likely place for a subtractive edit path to have been dropped. If the eraser code
   changed there, you are done searching.
2. Otherwise `git bisect` the eraser path, **time-boxed to about six steps.** If bisect does
   not converge, the feature may never have been finished; say so and move to designing it
   fresh rather than continuing to search.
3. Report the mechanism, then two candidate designs with a recommendation, and stop:
   - **Boolean subtract** on the vector geometry, mutating `pts` and possibly splitting one
     object into several.
   - **Per-object mask** layered over `raster`, leaving vector geometry intact and
     subtracting at rasterization time.
   Objects already carry a `raster` field, so the mask is likely far cheaper and avoids the
   degenerate-polygon cases boolean subtraction creates. Say so if you agree; say why if you
   don't.
4. Whatever you implement must be undoable through the existing undo stack and must work on
   all shape types including generator output.

### 6b. Domain extent toggle

Add a view toggle in the Results ribbon group that draws the full simulation grid, including
the absorbing sponge margin, so the margin is visible instead of implied.

- **View only.** `domain width (m)` anchors every unit readout in the app. The toggle must
  not change the physical scale, the grid, the camera calibration, or any readout. If turning
  it on changes a single number in the status strip, that is a bug.
- Draw the sponge band as a distinct region and outline the usable interior. Label the band's
  thickness in cells and in physical units. See the mockup's Results tab with the toggle on.
- Default off.

---

## Working rules

- One commit per phase, each building and running clean. Do not stack phases.
- Delegate to subagents where the work partitions: theme and units are independent of the
  eraser diagnosis, and phase 0 splits cleanly in two. Do not delegate the layout rebuild; it
  needs one coherent author.
- No unrelated refactors. If you find something broken outside this scope, add it to
  `docs/punchlist.md` and keep moving.
- **Frame time must not regress, measured this way:** before phase 2, load the rocket nozzle
  preset in compressible mode at the default grid, let it run 300 frames, and record mean and
  99th-percentile frame time. Repeat identically after phase 3. Report all four numbers. Build
  the measurement harness first so both runs are comparable.
- Update `CLAUDE.md` as you learn. Keep it under 150 lines and route detail out to
  `docs/ui-inventory.md`, `docs/theme.md`, and `docs/punchlist.md` rather than inlining it.
  Record specifically: the egui version and any API landmines found, where theme and units
  live plus the rule that nothing bypasses them, the reason behind the engine speed cap, and
  the frame-time baseline numbers.
- Stop and ask before: adding any dependency not named in this document, changing any default
  physical value, or changing anything in a shader.

## Acceptance

1. `grep -rn "Color32::from_rgb" src/` returns nothing outside `theme.rs`.
2. Every number on screen is monospace and column-aligned.
3. No panel mixes base units, and Mach shows its derivation.
4. The model tree reflects real scene contents and drives the settings panel.
5. The status strip shows live grid size, cell size, time step, CFL, and a stability state.
6. Selecting a nozzle shows an engine group with a visible, explained speed cap.
7. The eraser removes part of a shape, and undo restores it.
8. The domain toggle shows the sponge margin and changes no readout.
9. `Highlight fans and drains` renders correctly.
10. Frame time after phase 3 is no worse than the recorded baseline, on both mean and p99.
