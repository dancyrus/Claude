# FlowPaint V2 — UI overhaul plan v3

Supersedes v2. Every code-level claim below is now verified against the phase 0
inventory (`docs/ui-inventory.md`, HEAD `1bf417f`). Where v2 guessed from a release
binary and guessed wrong, this version says what is actually true.

Three things v2 got wrong, stated plainly so nobody re-derives them:

- **The eraser is not a regression.** Commit `146e797` (#10) removed it deliberately,
  along with the Brush tool and the raster canvas it depended on. Erasing meant painting
  an `Erase` material into the grid, which is incompatible with the rebuild's stated
  invariant that nothing is ever flattened. The README diff edited the eraser out of the
  feature list. Phase 5 below is a **new feature**, not a fix.
- **Only `Shape::Stamp` carries a raster.** v2 recommended a mask layer on the strength of
  a `raster` field it believed all objects had. Line, rect, ellipse, polyline, and pencil
  objects are pure vector. This changes the eraser design and roughly doubles its scope.
- **There is no single speed cap on the chamber fan.** There are six clamps in three
  layers, and the two that actually bind are shader constants. v2's instruction to make
  the cap an editable field is impossible without touching shaders.

Companion file: `docs/ui-target.html`. Open it in a browser.

- **Authoritative** for spacing, color, typography, panel widths, and panel arrangement.
  The `:root` block holds the real values; port them into the theme module directly.
- **Not authoritative** for widget construction, numeric values, or canvas rendering. The
  HTML structure is not the egui widget tree; egui is immediate mode with fixed panel
  nesting. Do not add a flexbox shim or CSS-emulation layer. Every number in the mockup
  is invented for legibility, as are the plume gradient and shock-train marks.

## What this repo is

Rust desktop app. `eframe` / `egui` 0.29.1 on `egui-wgpu` 0.29.1, `wgpu` 22.1.0,
`winit` 0.30.13. Two solvers: D2Q9 lattice-Boltzmann in GPU compute shaders
(incompressible) and finite-volume Euler with MUSCL reconstruction and HLLC fluxes
(compressible). Geometry is a persistent vector object model — every drawn thing stays a
live, selectable, editable `SketchObject`; the grid is a damage-region re-projection and
nothing is destructively committed. Undo is index-based per-object with slider coalescing.

`src/app.rs` is 2977 lines and contains the entire egui UI. No other file draws widgets.

**Do not change solver physics.** No new numerical schemes, no changes to shader math, no
changes to default physical values. No shader edits at all in this effort.

---

## Deferred: the egui upgrade

v2 opened with an `egui` 0.29.1 → 0.35 upgrade. **Do not do it.** Reasons, from the
inventory:

`egui-wgpu` forces `wgpu` in lockstep: 0.30→23, 0.31→24, 0.32→25, 0.33→26 then 27,
0.34→28 then 29. That is seven major `wgpu` versions across the eleven compute pipelines,
bind groups, and pass lifetimes in `sim.rs`. v2's "roughly 40 changed lines" budget was
fantasy. The payoff is font hinting.

Separately, the inventory flagged at low confidence that 0.34 may deprecate `SidePanel`
and `TopBottomPanel` in favor of a unified `Panel`, and may move `eframe` toward
`fn logic` / `fn ui` replacing `App::update`. If true, upgrading after the layout rebuild
means building the layout twice. Revisit only after this effort ships, and verify those
deprecations against release notes before committing to anything.

Known API breaks waiting whenever the upgrade does happen, recorded so the work isn't
re-done: `egui::menu::bar` plus 11 `ui.close_menu()` sites, `painter.rect_stroke` gaining
a `StrokeKind` argument, `Rounding` becoming `CornerRadius` as `u8`, and `Frame::none()`.
Put this list in `CLAUDE.md`.

---

## Phase 1 — split `app.rs`. No behavior changes.

2977 lines in one file means the layout rebuild lands as one unreviewable diff. Split
first, mechanically, with no logic edits:

```
src/ui/mod.rs          panel orchestration, called from update()
src/ui/menu.rs         menu_bar
src/ui/panels.rs       side_panel, side_panel_contents
src/ui/inspector.rs    object_panel, defaults_panel
src/ui/legend.rs       legend_panel, colormap_bar
src/ui/status.rs       status_bar
src/ui/canvas.rs       canvas, canvas_interaction, gesture helpers, canvas_overlays
src/ui/generators.rs   generator_windows
src/ui/windows.rs      about, keyboard shortcuts
```

`apply_cmd`, `keyboard`, `save_scene`, `load_scene`, and state structs stay in `app.rs`.
The commit diff should be almost entirely moved lines. Verify by running the app and
confirming nothing changed visually.

---

## Phase 2 — theme module

Smaller than v2 assumed. The inventory found only nine `Color32` constructions, three
corner-radius literals, three `Stroke::new`, and one `FontId`, all in `app.rs`.

Create `src/ui/theme.rs`. Take the palette, radius, type sizes, and panel dimensions from
the `:root` block of `docs/ui-target.html`.

- **All numeric readouts render in the monospace `TextStyle` with tabular figures.** The
  single largest change in how the app reads. Currently only the on-canvas dimension
  label uses `FontId::monospace(12.0)` (app.rs:2643); the legend and status bar do not.
- **Fix the spacing system.** `add_space` appears 21 times with three magic values —
  6.0 (×13), 4.0 (×4), 2.0 (×4) — chosen per site with no system. Set
  `Spacing::item_spacing`, `button_padding`, `indent`, and `slider_width` once in the
  theme and delete the ad-hoc spacing.
- Two accents maximum. The app already has a de-facto accent at app.rs:2542
  (`255, 200, 90`, used for selection outlines, handles, and dimension text) and a
  destructive tint at app.rs:1303 (`255, 140, 120`, on the "Clear all" label). Keep both
  roles, restyle the values to match the mockup.
- Near-square rounding (3 px). The current pill-shaped `selectable_label` toggles are a
  large part of the toy impression.
- Add `egui-phosphor` for ribbon icons, one variant bundled. Icons never carry meaning
  alone; every ribbon button keeps a text label under its glyph.

**Do not move the colormap stop tables** (app.rs:272–278 inferno, 287–289 coolwarm) into
the theme independently. They are CPU mirrors of the shader colormaps; re-theming them
alone desynchronizes the legend from the rendered field. Either move them as a linked pair
with the shader constants, or leave them where they are. Leaving them is fine.

`def_smoke` (app.rs:430) is scene content, not chrome. Leave it.

After this phase, `grep -rn "Color32::from_rgb" src/` outside `theme.rs` should return
only the colormap mirrors and the `smoke_rgb` conversion site (app.rs:1747).

---

## Phase 3 — layout rebuild

### The decision this phase makes, stated openly

This reverses commit `1e76f5a` (#7), which deliberately moved everyday controls out of
menus and into the side panel, and added the `ScrollArea` so the full control set survives
short windows. That reversal is intentional: the ribbon groups controls by task rather than
by frequency, and gives the model tree and settings column somewhere to live. **Two
constraints carry forward from #7 and must not be lost:**

- Menus keep only rare operations. Do not move File, Open, Save, Export, grid resolution,
  or domain margin back into panels.
- **Short windows must still work.** #7's `ScrollArea` solved a real problem. The settings
  panel keeps a vertical scroll area, and the ribbon must not assume a tall viewport.

v2 also proposed re-adding a View menu. **Cut that.** #7 deleted it deliberately once
emptied, and nothing in this plan needs it.

### The shell, top to bottom

1. `TopBottomPanel::top` — menu bar: File, Edit, Simulation, Help. Unchanged from today.
2. `TopBottomPanel::top` — **ribbon.** Tab strip of `selectable_label` items over a body
   around 86 px. Grouped icon-and-label buttons separated by vertical rules, small caption
   under each group.
   - **Home** — pause/resume, reset flow, clear all, undo, redo
   - **Geometry** — select, line, rectangle, ellipse, polyline, pencil, **eraser**;
     material picker; thickness; filled toggle; angle snap and snap-to-grid
   - **Physics** — solver mode, fluid preset, inlet Mach (Euler) or flow speed and
     viscosity (LBM), steps per frame, smoke persistence, wind tunnel toggle
   - **Study** — airfoil and nozzle generators, five scene presets
   - **Results** — field selector, legend toggle, particle count, display gain, smoke
     brightness, edge damping, particle size and brightness, **domain extent toggle**
     (phase 6)
3. `SidePanel::left`, resizable, default 212 px — **model tree.** A real tree over
   `SketchModel::objects`. Root for the domain, child for solver settings, then one node
   per object labeled by material and id. Clicking selects (writes
   `FlowPaintApp::selected`). Right-click gives duplicate and delete. Note that
   `selected: Option<u64>` already exists at app.rs:349 with 20-odd writer sites — the
   tree is a new reader and writer of existing state, not new state.
4. `SidePanel::left`, resizable, default 258 px, with a `ScrollArea` — **settings.** Draws
   the property block for the tree selection and nothing else. Preserve the existing
   three-way branch at app.rs:1360–1367: mid-gesture placeholder, object inspector,
   defaults panel. The mid-gesture guard exists because the inspector fights an active
   drag; keep it.
5. `CentralPanel` — graphics window. Keep the existing right-side legend panel
   (app.rs:1870) as-is in this phase; it is already close to what the mockup shows.
6. `TopBottomPanel::bottom` — message line for solver events, then the **status strip.**
   `status_bar` already exists at app.rs:2040. Extend it to carry, in monospace: grid
   dimensions, cell size, time step, elapsed sim time, inlet speed in m/s, **CFL number**,
   frame time, and a stability indicator.

CFL is not currently exposed anywhere. For a compressible Euler solver it is the number
that says whether the run is about to come apart, and it is cheap to compute. The status
strip is what makes this read as a solver; ribbon chrome alone will not.

### Label and text discipline

- Fix `Highlight fans && drains` (app.rs:1443). The ampersand is not escaped.
- Sentence case everywhere, consistently.
- Slider value boxes go on one side, the same side, in every panel. (Observed in the
  running build, not confirmed in the inventory — check before assuming a defect.)
- `Clear all` (app.rs:1300) and `Reset flow` (app.rs:1297) are both destructive. Neither
  confirms; only one is tinted. Give both a confirmation and treat them alike.
- Apply ASD-STE100 Issue 7 to every tooltip and help string written or rewritten. One
  instruction per sentence, approved vocabulary, no gerund strings. The 13 existing
  `on_hover_text` sites are in scope where they violate it.

---

## Phase 4 — units consolidation

Smaller than v2 assumed: `fmt_len`, `fmt_speed`, `fmt_time`, and `fmt_pressure` already
exist and are already used for derived readouts. This phase consolidates rather than
invents.

- Move the `fmt_*` helpers into `src/ui/units.rs`. No readout formats itself inline
  afterward.
- Fix the mixed convention. Thickness sliders (app.rs:1701, 1846) show a cell count with a
  derived length folded into the slider *label text*; domain width (app.rs:1588) shows
  metres in the label. Pick one: canonical value in the box, unit in the label, derived
  convenience value on its own secondary line. The mockup shows the intended pattern —
  copy the pattern, not the numbers.
- Mach (app.rs:1532) already derives m/s via `fmt_speed(mach * fluid_a)`. Keep the
  derivation, move it out of the slider label onto a secondary line, and state `a` so
  `u = M · a` is visible.
- Convert to `DragValue` with `suffix` anything an engineer would type exactly: Mach, flow
  speed, domain width, thickness, steps per frame, edge damping. Keep sliders for
  display-only gains where sweeping is the point. Note `snap_angle_deg` (app.rs:1377) is
  already a `DragValue` with a suffix — match that pattern.
- Every displayed value rounds at the formatter.

---

## Phase 5 — nozzle engine property group

### What is actually missing

Nozzle stamps **already get** fan speed and gustiness. `stamp_has_fans` (app.rs:1708–1713)
detects `CELL_INLET` cells in the raster and unlocks those sliders even though
`insert_stamp_object` forces `material = Wall` (app.rs:519).

What they do not get:

- **Blow direction** — gated on `material == Fan` *and* a filled rect or ellipse
  (app.rs:1729–1732).
- **Smoke color** — gated on `material == Fan || Smoke` (app.rs:1746).

So the gap is two properties, not a whole block. Fix the gating so stamps with fan cells
reach both, and present all four under an **Engine** group heading rather than a generic
fan block. The chamber fan is not a fan in the user's mental model; it is an engine.

### The speed cap is a readout, not a field

Six clamps, three layers:

| # | where | value | binds? |
|---|---|---|---|
| 1 | generators.rs:287 | `fan_mult.clamp(0.2, 2.0)` per-cell | multiplier only |
| 2 | app.rs:2939–2941 | auto LBM multiplier, clamped 0.2–2.0 | multiplier only |
| 3 | app.rs:2944 | auto Euler multiplier, clamped 0.2–2.0 | multiplier only |
| 4 | app.rs:2778–2782 | display estimate capped at 0.3 | no — readout only |
| 5 | lbm.wgsl:26, :124 | `MAX_LATTICE_SPEED = 0.3` | **yes, LBM** |
| 6 | euler.wgsl:93–95 | Mach 8 sanity limit | yes, but almost never |

The binding constraint is a shader constant, and shaders are out of scope. So:

- Replace the cap field with a **readout naming which layer is currently binding**, and by
  how much the requested chamber speed exceeds it.
- State the asymmetry in the panel: LBM clamps hard at 0.3 lattice, Euler effectively does
  not clamp at all. The "(speed-capped)" label users see today is almost entirely an
  incompressible-mode artifact, and the panel should say so rather than implying a cap
  applies in both modes.
- Keep the existing `nozzle_fan_auto` behavior (app.rs:2755–2766): the auto formula
  refreshes each frame until the user touches the slider. Surface that state so it isn't
  invisible.
- Write the tooltips in STE.
- Regression check: hand-placed fans keep their existing property block unchanged.

---

## Phase 6 — eraser and domain extent

### 6a. Eraser — new feature, design before code

The eraser was removed in `146e797` with the raster canvas. Its old mechanism cannot
return: painting an `Erase` material into the grid violates the rebuild's invariant that
objects stay live and nothing is flattened. Any new eraser is a per-object operation.

Two shape families need different handling, which is the scope reality v2 missed:

- **Vector shapes** (`Line`, `Rect`, `Ellipse`, `Polyline`, `Pencil`) — boolean subtract on
  `pts`, which can split one object into several and produces degenerate-geometry cases
  that need guarding. The existing `[0.5]`-cell minimum radii (app.rs:2468–2477) are the
  precedent for those guards.
- **Stamps** (generator output) — a mask layer over the existing `raster`, subtracting at
  rasterization time. Cheap, because the raster is already there.

Before writing code, report: the design for each family, whether stamps need erase support
in the first release at all (dropping them halves the work and generator output is arguably
the thing users least want to hand-erase), and how each interacts with
`record_modify_coalesced`. Then stop.

Requirements once approved: undoable through the existing per-object undo stack with clean
coalescing, and no violation of the never-flatten invariant. A subtract that rewrites `pts`
is still non-destructive at the model level as long as it is one recorded, reversible
modification.

Also note: `Tool::Eraser` and the `X` key existed pre-rebuild (`61368c8`). Reusing the same
key and glyph costs nothing and matches muscle memory for anyone who used v1.

### 6b. Domain extent toggle

A domain margin already exists — `MARGIN_CHOICES` (sim.rs:21–26) offers none, +25 %, +50 %,
+100 % of canvas height via `Cmd::SetMargin` → `GpuSim::set_margin_frac`, in
Menu ▸ Simulation ▸ Domain margin. Edge damping (`sponge_strength`, app.rs:1616) is the
absorbing layer inside it. So this toggle **visualizes existing state**; it adds no physics.

- **View only.** `domain_width_m` (app.rs:1588) anchors every unit readout. The toggle must
  not change the physical scale, the grid, the margin fraction, the camera calibration, or
  any readout. If turning it on changes one number in the status strip, that is a bug.
- Draw the margin region distinctly and outline the usable interior. Label the margin in
  cells and in physical units via `fmt_len`.
- Default off. Lives in the Results ribbon group. See the mockup's Results tab with the
  toggle on.
- Worth surfacing while you are here: the margin selector is buried in a submenu, and
  users have no way to see what it did. That is likely the root of the original complaint.

---

## Working rules

- One commit per phase, each building and running clean. Do not stack phases.
- Stop and report at the end of phases 1, 2, and 6a-design before continuing.
- Delegate to subagents where work partitions: theme and units are independent of the
  eraser design. Do not delegate the layout rebuild or the `app.rs` split; both need one
  coherent author.
- No unrelated refactors. Anything broken outside this scope goes in `docs/punchlist.md`.
- **Frame time must not regress, measured this way:** before phase 2, load the Pinball
  preset in compressible mode at the default grid resolution, run 300 frames, record mean
  and p99 frame time. Repeat identically after phase 3. Report all four numbers. Build the
  harness first so both runs are comparable.
- Update `CLAUDE.md` as you go. Under 150 lines, routing detail to `docs/ui-inventory.md`,
  `docs/theme.md`, and `docs/punchlist.md`. Record: the deferred-upgrade decision and the
  API break list, where theme and units live plus the rule that nothing bypasses them, the
  six-clamp chain and which layer binds, the never-flatten invariant as a constraint on the
  eraser, and the frame-time baseline.
- Stop and ask before: adding any dependency not named here, changing any default physical
  value, touching a shader, or reversing a decision recorded in `1e76f5a` or `146e797`
  beyond the reversal this plan already authorizes.

## Acceptance

1. `app.rs` is under 900 lines; the UI lives in `src/ui/`.
2. `grep -rn "Color32::from_rgb" src/` outside `theme.rs` returns only the colormap mirrors
   and the `smoke_rgb` conversion.
3. Every number on screen is monospace and column-aligned.
4. No panel mixes base units; Mach shows `u = M · a` with `a` stated.
5. The model tree reflects real scene contents and drives the settings panel; the
   mid-gesture guard still suppresses the inspector during a drag.
6. The settings panel still scrolls, and the full control set is reachable at the 900×600
   minimum window size.
7. The status strip shows live grid size, cell size, time step, CFL, and stability.
8. Selecting a nozzle shows an Engine group with blow direction, smoke color, and a readout
   naming the binding speed limit and the LBM/Euler asymmetry.
9. The eraser removes part of a vector shape, and undo restores it in one step.
10. The domain toggle shows the margin and changes no readout.
11. `Highlight fans and drains` renders correctly.
12. No file under `shaders/` is modified.
13. Frame time after phase 3 is no worse than baseline on mean and p99.
