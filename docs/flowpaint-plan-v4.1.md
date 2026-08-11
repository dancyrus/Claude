# FlowPaint V2 — plan v4.1: merged phases, two parallel tracks

Same scope as v4. Restructured for throughput: eleven sequential phases become six work
units across two branches that can run concurrently.

Read v4 for the reasoning behind individual features. This document supersedes v4's phase
numbering and adds the branch structure, the merge order, and the delegation rules.

## Standing constraints, carried from v3

- No shader edits. No changes to solver math or default physical values. Two places may
  legitimately need an exception (T2-A colormaps, T2-C boundary conditions) and both are
  marked **report before touching**.
- The never-flatten invariant from `146e797` holds: every drawn thing stays a live,
  selectable, editable `SketchObject`.
- Menus keep only rare operations. Full control set reachable at 900×600 (`main.rs:19`).
- ASD-STE100 Issue 7 for every tooltip and help string.
- Frame time against the v3 phase 2 baseline (Pinball, compressible, default grid, 300
  frames, mean and p99) at the end of any unit that touches the canvas or rasterizer.
- `egui` stays at 0.29.1. `egui::Scene` is unusable here — the canvas is a custom wgpu paint
  callback, so pan and zoom are a transform on the existing `mapping` in `ui/canvas.rs`.
- Ask before adding a dependency, touching a shader, or reversing a recorded decision.

## What already exists, so it isn't rebuilt

`filled: bool` (`model.rs:58`) gated to Rect and Ellipse by `can_fill`. Rasterization order
is `model.objects` index order, so z-order is a Vec reorder. `simplify_stroke` (RDP) exists
for Pencil and is the fill contour simplifier. `handle_r` and `click_slop`
(`ui/canvas.rs:104-105`) already divide by `px_per_cell`, so hit-testing adapts to zoom.
The v3 model tree is the layers panel. Both `inferno` and `coolwarm` exist in `app.rs`,
bound per render mode with no user choice. `stats_re` already computes Reynolds number.

---

# Branch structure

Two tracks, separate branches off the current head, merged at the points marked below.

```
Track 1 — geometry        Track 2 — solver usability
files: app.rs, model.rs,  files: sim.rs, ui/legend.rs,
ui/tree.rs, ui/canvas.rs, ui/status.rs, ui/units.rs,
ui/inspector.rs           Physics ribbon group

U1  cleanup + navigation  T2-A  color range + colormap
U2  selection + tier      T2-B  probes + Re input
U3  transforms + groups   T2-C  per-edge boundaries
U4  fill + eraser + snaps T2-D  unit toggle
        |                          |
        +----------- merge --------+
                     |
                U5  share-ready output
```

File overlap between the tracks is near zero. Track 2 touches `ui/units.rs` (T2-D) and
Track 1 does not; Track 1 touches `ui/status.rs` only in U1 (zoom readout, scale bar) while
Track 2 touches it in T2-B. **Coordinate `ui/status.rs`**: land U1 before starting T2-B, or
accept one small conflict there.

## What must not run in parallel

`ui/canvas.rs` is rewritten by U1 (view transform), U3 (gizmo), and U4 (fill, eraser,
snaps). Those three are sequential within Track 1, always. Running them concurrently costs
more in conflict resolution than it saves.

## Delegation rules

Use subagents where work partitions by file. Do not delegate work that needs one coherent
author.

- **Delegate**: T2-A, T2-B, T2-C, T2-D are four near-independent sub-items and partition
  cleanly. U1's two halves (inspector fields, canvas view transform) partition.
- **Do not delegate**: U2's selection refactor (~20 sites, one consistent decision about set
  semantics), U3's transform composition (nested rotation goes subtly wrong if two authors
  disagree on order), U4's degenerate-case guards (shared between fill tracing and boolean
  subtract).

## Token discipline

Keep `CLAUDE.md` current with the current unit, the file map, and the decisions already
made, so a fresh session does not re-derive the layout or re-read the whole plan. Under 150
lines, routing detail to `docs/`. The largest waste in this project so far has been
re-reading `app.rs` and the plan every turn. Record in `CLAUDE.md` at minimum: which unit is
in flight, the scene-format version and its load paths, the transform composition order once
U3 defines it, and the frame-time baseline.

---

# Track 1 — geometry

## U1 — control cleanup and navigation

Merges v4 phases 7 and 8. Two halves, delegable to two subagents.

### Half A — control widgets (`ui/inspector.rs`, ribbon, `sim.rs` constants)

`sim.rs` folds an off state into two value lists. Both become a checkbox plus a value, and
both remember the last value so unchecking and rechecking does not reset to a default.
`snap_enabled` plus `snap_spacing` is the pattern to match.

1. `PARTICLE_CHOICES`: remove `("Off", 0)`. Particles get an on/off checkbox in the Results
   group; the dropdown holds only real counts.
2. `MARGIN_CHOICES`: remove `("None", 0.0)`. Checkbox plus the three real fractions.
3. Single-step button in the Home group beside Pause. The v3 mockup shows one; it does not
   exist. Advance exactly one frame, honoring `steps_per_frame`.
4. Replace the object panel's `-15°`/`+15°`/`+90°` and `×0.8`/`×1.25` buttons with
   `DragValue` fields: rotation in degrees, scale in percent. Keep `+90°` as a convenience.
   U3 extends these to a selection, so get the single-object form right here.

### Half B — zoom and pan (`ui/canvas.rs`, `ui/status.rs`)

- Scroll wheel zooms **at the cursor**. Zoom is a view transform on `px_per_cell` and
  `lb_origin`; it must not change the grid, the margin, `domain_width_m`, or any readout.
- Middle-drag pans; space-drag pans as the trackpad-friendly alternative.
- Fit-to-window, zoom-to-selection, 1:1 reset — Results group plus shortcuts.
- Clamp the zoom range; prevent panning the domain fully off-screen.
- Zoom level in the status strip.
- Persistent **scale bar** on the canvas, labeled through `ui/units.rs`.

Check at extreme zoom: hit-testing, vertex handles, and the snap grid's 8 pt hide threshold
(`ui/canvas.rs`) all still read correctly now that zoom is free rather than derived.

**Report and stop.** Land U1 before T2-B starts, to keep `ui/status.rs` clean.

## U2 — multi-select and the cheap tier

Merges v4 phases 9 and 10. One commit, one author. The selection refactor is the whole cost;
everything else falls out of it, and reviewing them together is easier than apart.

### Selection

`selected: Option<u64>` (`app.rs:377`) becomes an ordered set. ~20 writer sites.

- Shift-click adds and removes. Rubber-band drag selects; **document whether that means
  intersect or fully-contain** (intersect is the usual choice for thin geometry, which yours
  is).
- The model tree supports range and additive selection with the same modifiers.
- Ctrl+A selects all; Esc clears.
- Every operation that took one object now takes the set: delete, duplicate, material,
  thickness. Where a property differs across the selection, show a mixed-value indicator
  rather than silently overwriting with the first object's value.
- Preserve the mid-gesture guard — the inspector stays suppressed during an active drag.
- Undo: one entry per user action across the whole selection, not one per object.

### The tier that rides on it

- **Z-order**: raise, lower, bring to front, send to back, as a reorder of `model.objects`.
  This decides which material wins on overlap, so it is functional, not cosmetic. Surface
  ordering in the tree.
- **Copy, paste, paste-in-place**, arrow-key nudge (Shift for a coarse step).
- **Lock and hide** per object from the tree. Locked objects are not click-selectable and
  not editable; hidden objects are not rasterized. Both persist.
- Scene format to **v5**, keeping the v3 and v4 load paths.

**Report and stop.**

## U3 — transforms and nested groups

Merges v4 phases 11 and 12. Needs U2. One author — transform composition is not delegable.

### Numeric and on-canvas transforms

- U1's numeric fields extend to a selection: rotation, scale, center X/Y, all about a
  **common pivot** (selection bounding-box center).
- Gizmo: extend the existing vertex handles and selection overlay in `ui/canvas.rs` rather
  than starting fresh. Corner handles scale, an edge ring rotates, pivot is draggable.
- Live numeric readout while dragging, through `ui/units.rs`.
- `snap_angle_deg` constrains rotation; Shift constrains scale to uniform.
- Stamps keep their single `scale: f32`. Non-uniform scaling of a raster stamp is out of
  scope — say so in the tooltip rather than silently ignoring an axis.

### Nested groups

Implement as `parent: Option<u64>` on `SketchObject`. Arbitrary depth costs almost nothing
beyond one level once transforms compose by walking the parent chain.

- Group and ungroup from the tree and by shortcut. Groups are tree nodes with children;
  drag-to-reparent in the tree.
- **Cycle prevention on reparent is mandatory.** A group cannot become its own descendant.
- **Define the transform composition order once and write it into `CLAUDE.md`**: child
  transform, then each ancestor outward. Get this wrong and nested rotation goes visibly
  wrong in a way that is hard to trace later.
- Selecting a group selects its subtree for transforms; entering a group allows selecting a
  child individually.
- Deleting a group deletes its subtree as one undo entry.
- **Free win**: the nozzle bell already owns its chamber fan implicitly, detected by scanning
  for `CELL_INLET` cells (`ui/inspector.rs:117`). Make that an actual parent link. Generator
  output becomes a group and the Engine group stops relying on raster inspection.
- Scene format to **v6**, keeping v3 through v5 load paths.

**Report and stop.**

## U4 — fill, eraser, snaps, domain extent

Merges v4 phases 13 and 14. Needs U1 (view transform) and U2 (selection). One author for the
geometry guards; the snap work can be delegated once the guards exist.

Fill and eraser are paired because contour tracing and boolean subtraction are neighboring
problems and the degenerate-case guards get written once.

### Filled closed polylines

Extend `can_fill` to `Shape::Poly { closed: true }`. Small, and probably most of what a fill
tool needs to be.

### Paint bucket

Flood fill the rasterized grid from the click, trace the contour, simplify with the existing
`simplify_stroke`, emit a normal filled `Shape::Poly`.

- Output is a traced object, not painted cells — that is what keeps the never-flatten
  invariant.
- **State the limitation in the tooltip**: the traced boundary is a snapshot. Move a bounding
  wall afterward and the fill does not follow. Every raster-era paint bucket works this way;
  the tooltip exists so nobody expects parametric behavior.
- A click in a region open to the domain edge **refuses** with a status message. It does not
  fill the domain.
- The filled object takes the current default material, so Smoke and Drain fills work too.

### Eraser

New feature, not a restoration. `146e797` removed the old one with the raster canvas, and
raster subtraction cannot return.

- **Vector shapes**: boolean subtract on the point list. A subtract can split one object into
  several; the 0.5-cell minimum radii (`ui/canvas.rs`) are the precedent for guards.
- **Stamps**: a mask layer over the existing `raster`, subtracting at rasterization time.
- **Report both designs and a recommendation on whether stamps need erase support in the
  first release, before writing code.** Dropping stamp support roughly halves the work.
- One reversible undo entry per erase stroke via `record_modify_coalesced`.
- Reinstate `Tool::Eraser` and the `X` key as they were pre-rebuild (`61368c8`).

### Object snaps

What CAD users mean by snapping, distinct from the existing grid snap. Delegable.

- Endpoint, midpoint, center, intersection, perpendicular, with a visual indicator at the
  candidate point and a modifier to suspend snapping.
- Define the priority order when several candidates are in range. One snap radius in **screen
  pixels**, not cells, so it holds across zoom.
- Grid snap and angle snap keep working alongside.
- Fold in a **measure tool** (pick two points, get distance and angle) if it fits — it is
  cheap once snaps exist.

### Domain extent toggle

Trivial now that U1 made the view transform explicit and Half A made the margin a checkbox.

- View-only toggle in the Results group: draw the full grid including the sponge margin,
  margin region distinct, usable interior outlined.
- Label the margin in cells and physical units via `ui/units.rs`.
- **Changing no readout is the acceptance test.**
- Worth noting in the report: the margin selector was buried in a Simulation submenu with no
  way to see its effect, which is likely the root of the original complaint.

**Report and stop.**

### Deferred out of U4, on purpose

**Mirror and array** (v4 phase 15). Mirror about an axis or a picked line; linear array with
count and spacing; copies are independent objects, not instances. Both operate on the
selection, one undo entry each. Small, and it can land any time after U3 — slot it wherever
there is room rather than blocking U4.

---

# Track 2 — solver usability

Independent of Track 1. If the geometry work stalls, this is the track that most changes how
the app is received. Four sub-items, all delegable.

## T2-A — locked color range and colormap choice

The legend range floats today, so two screenshots of the same scene are not comparable and a
growing plume dims everything else.

- Auto, lock-to-current, and manual min/max, per render mode.
- Let the user pick between the existing `inferno` and `coolwarm` maps instead of binding
  each to a render mode.
- The CPU colormap tables mirror the shader colormaps and changing one alone desynchronizes
  the legend from the field. This is the one place a shader constant may need to move —
  **report before touching a shader.**

## T2-B — persistent probes and Reynolds input

Start after U1 lands, to avoid a `ui/status.rs` conflict.

The v3 mockup promises a "Probe plot" tab with nothing behind it.

- Click to drop a probe; it persists and appears in the model tree.
- Plot the selected field against time. For a compressible solver this is what says whether
  the flow has settled, which smoke cannot show.
- Cap the history length and state the cap.
- **Reynolds number as an input**: engineers pick Re and let viscosity follow. `stats_re`
  already does the forward direction; the inverse is arithmetic. Offer both directions in the
  Physics group with the dependent value shown as derived.

## T2-C — per-edge boundary conditions

The largest item in this track, and the single change that most makes the app read as a
solver rather than a sandbox. Today it is one `wind_tunnel: bool` (`sim.rs:72`).

- Each of the four domain edges becomes inlet, outlet, wall, or periodic.
- `wind_tunnel: true` becomes the preset "left inlet, right outlet, top and bottom wall", so
  existing scenes load unchanged.
- Both solvers need the edge handling. **Check what the existing inlet and outlet paths
  already support before designing, and report if either solver cannot express a case
  without a shader change.** Those paths are the per-cell `CELL_INLET`/`CELL_OUTLET` arms
  in `lbm.wgsl` and `euler.wgsl`, plus the 2-cell tunnel bands the rasterizer paints at
  the grid edges when `wind_tunnel` is on (`model.rs`, `rasterize_region`). (This bullet
  used to cite `sim.rs:126`/`sim.rs:182` — line numbers from a pre-overhaul revision that
  had gone stale and pointed at nothing relevant.)
- Scene format bump, earlier load paths retained. Coordinate the version number with U2 and
  U3 — whichever track lands second takes the higher number.

## T2-D — unit system toggle

SI or decimal inch, on the `ui/units.rs` formatter layer from v3 phase 4. ASME decimal-inch
conventions in inch mode. The audience thinks in inches.

---

# U5 — share-ready output

After both tracks merge. Needs T2-A (locked range) and U1 (scale bar).

The app exists to be shown to other engineers, and a raw screenshot loses every number that
gives it meaning.

- PNG export that burns in the legend with its locked range, the scale bar, and a run-
  conditions block: solver, fluid, Mach or Re, grid, cell size, elapsed time.
- Extend the existing `Cmd::ExportPng` path; do not add a second one.
- Canvas-only and annotated variants.
- **Load a sample scene on first run** instead of an empty canvas. Someone who opens
  FlowPaint and sees flow already moving understands the tool immediately. The Pinball preset
  or an RS-25 nozzle insert are both candidates.

---

# Cut, with reasons

- **Dimensional constraint solver.** What makes SolidWorks sketching powerful and also what
  makes it slow and fussy. Months of work. This tool sketches flow scenes, not parametric
  parts.
- **Align and distribute.** Reads as a slide-layout tool. These objects are physical geometry
  positioned by dimension, not by even spacing.
- **Construction geometry.** Only pays off alongside a measure tool and a constraint solver,
  and the solver is cut.

# Deferred, on purpose

- **Arcs and splines** as new `Shape` variants. Each needs a new rasterizer path in
  `model.rs`, and every transform, snap, and boolean op has to learn about them. After the
  geometry operations above are stable.
- **Offset curve** (parallel copy at a distance). Fastest way to build duct and channel
  walls, and the natural companion to arcs.
- **Union and intersect booleans.** Subtract arrives with the eraser in U4; the other two can
  follow once its degenerate handling has seen real use.
- **Holes in filled polygons** (`Shape::Poly` with interior rings). Deferred for the same
  reason as arcs and splines: a hole representation touches the format, the fill rasterizer,
  hit tests, handles, and every boolean. It is a real limit — annuli, ducts with a bore,
  plates with a port — and the U4 eraser refuses an erase wholly interior to a filled shape
  because of it (see `docs/u4-eraser-design.md`). Until it lands, such parts are built from
  outline rings or two overlapping shapes.
- **DXF or SVG import.** The highest-value item on the whole list for an audience that
  already has geometry in SolidWorks, and the strongest reason for anyone else to try the
  tool. It wants a stable object model under it, so build it after U3. **Deferred, not cut.**
- **The egui 0.29 to 0.35 upgrade**, with the API break list recorded in v3.

# Merge order and checkpoints

```
U1                          T2-A                 (concurrent)
U2                          T2-B  (after U1)     (concurrent)
U3                          T2-C                 (concurrent)
U4                          T2-D                 (concurrent)
        merge both tracks
U5
        mirror and array — anywhere after U3
```

Merge Track 2 into Track 1 rather than the reverse; Track 1 carries the wider `app.rs`
churn. Rebase Track 2 before each merge, and run both solver modes plus the 900×600 check
after every merge, not only at the end.
