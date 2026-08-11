# FlowPaint V2 — working notes

Rust desktop CFD paint app in `FlowPaint/` (also `FingerFlow/`, an iOS
sibling — untouched by the UI overhaul). Two solvers: D2Q9 LBM
(incompressible, `shaders/lbm.wgsl`) and finite-volume Euler MUSCL+HLLC
(compressible, `shaders/euler.wgsl`). Geometry is a persistent vector
object model (`model.rs`): objects stay live and editable, the grid is a
damage-region re-projection, **nothing is ever flattened** — any eraser
must be a per-object, undoable operation (see plan phase 6a).

The working plan is `docs/flowpaint-plan-v4.1.md` (six units on two
tracks; supersedes v4's phase numbering — the v4 reasoning doc was never
checked in). Standing constraints carry from
`docs/flowpaint-ui-overhaul-plan-v3.md`; the layout mockup
`docs/ui-target.html` is authoritative for spacing, color, type and
panel arrangement — not for widget construction or numeric values.

## Unit status (plan v4.1)

Per-unit decisions later units must not re-derive live in
`docs/unit-decisions.md` — read it before starting any unit.

- **U1 (cleanup + navigation): done**, branch
  `claude/flowpaint-v4.1-u1-nav-o8c1im`.
- **U2 (multi-select + tier): done**, branch
  `claude/flowpaint-u2-multi-select-kvfbsi` (off the U1 branch).
- **U3 (transforms + nested groups): done**, branch
  `claude/u3-transforms-nested-groups-qo5qxi` (off the track-merge
  tip). Groups are `Shape::Group` similarity nodes + `parent` links;
  gizmo in `ui/canvas.rs`. Decisions in `docs/unit-decisions.md` §U3.
  Next: U4 is unblocked.
- **T2-A (color range + colormap): done**, branch
  `claude/t2a-color-range-opuijy` (off `34d57aa`, no U1); write-up in
  `docs/t2a-color-range.md`. Asymmetric manual min/max deliberately
  unimplemented (needs a per-mode offset in `render.wgsl`).
- **T2-B (probes + Re input): done**, branch
  `claude/flowpaint-v4.1-t2b-probes` (off the U1 tip).
- Scene format: **v8** current (U3), loads v3+ (v3 lacks solver
  fields; v4/v5 share a layout; v6 appends per-object
  `locked`/`hidden` — pre-v6 objects decode via the `SketchObjectV5`
  mirror; v7 appends color ranges; v8 appends `parent` links, `Group`
  nodes and the probe set — pre-v8 objects decode via the
  `SketchObjectV7` mirror). **T2-C takes v9**; later bumps start at
  v10 (plan v4.1's numbers run one low — don't repeat the off-by-one).
- The track merge folded T2-A's color-range state out of the sim.rs
  Mutex into `Settings.ranges` + `Cmd` (persisted since v7); U3 did
  the same for T2-B's probes — `sim::probes()` is gone, the store is
  `Settings.probes` + probe `Cmd`s, read via the app's per-frame
  `ProbeUi` snapshot, positions persisted in v8. Still open: unify the
  plot/legend shader-inversion factors in `ui/units.rs`.

## Transforms & nested groups (U3)

**Transform composition order — fixed, do not re-derive: the child's
transform applies first, then each ancestor outward** (world =
`T_root ∘ … ∘ T_parent`(stored); `SketchModel::abs_of`). An object's
stored coordinates live in its parent group's space; `Shape::Group`
carries a **similarity** (translate/rotate/uniform-scale) — the only
family that composes through rotated nests without shear, which is why
gizmo/panel scaling is uniform (stamps keep a single `scale: f32`; the
tooltip declares non-uniform stamp scaling out of scope). Cycle
prevention lives in `SketchModel::reparent` (refuses; tested) and
`sanitize_parents` (repairs crafted files on load). Edits from world
space go through `translate_world`/`rotate_world`/`scale_world`, which
damage-mark through the chain.

## Hard rules

- **Never edit anything under `FlowPaint/src/shaders/`** during the UI
  effort. One approved exception has landed: T2-A's colormap-swap
  branches in `render.wgsl` (`flags` bit 1; no uniform added or
  reordered). The inferno/coolwarm stop tables in `app.rs` mirror
  `render.wgsl` and must stay linked to it (don't re-theme them alone).
- **egui stays at 0.29.1 — the upgrade is deferred** (plan v3). Moving
  to 0.35 drags wgpu 22→29 through all of `sim.rs`. API breaks waiting
  when it happens: `egui::menu::bar` + 11 `close_menu` sites,
  `rect_stroke` gains `StrokeKind`, `Rounding`→`CornerRadius` (u8),
  `Frame::none()`; low-confidence flags on 0.34 panel/`App::update`
  deprecations — verify against release notes first.
- All visual constants resolve through `src/ui/theme.rs` (see
  `docs/theme.md`); nothing sets colors, rounding, spacing or font sizes
  ad hoc. Exceptions: colormap mirrors and `def_smoke` in `app.rs`, and
  the `smoke_rgb` picker conversion in `ui/inspector.rs`.
- Numeric readouts render monospace (`TextStyle::Monospace`,
  `drag_value_text_style`); no proportional digits in value boxes.

## Structure

- `src/app.rs` — state, `Cmd` dispatch, keyboard, scene IO (v3–v8
  bincode, version peek on first 4 LE bytes), `--bench` harness,
  `ViewRequest` + view state fields, selection set + clipboard.
- `src/ui/` — one file per panel; child of `app` via `#[path]` so panel
  code reads app state without visibility widening. `ui/mod.rs` owns
  draw order; `ui/theme.rs` owns all style; `ui/units.rs` owns every
  physical-quantity formatter (no inline `{:.N}` on physical values).
  Convention: canonical value in the box, unit in the label, derived
  value on a `theme::derived` secondary line.
  - `ui/canvas.rs` — gestures, overlays, scale bar, the U3 transform
    gizmo (corner scale, rotate handle, draggable pivot), and the free
    view transform (wheel/pinch zoom at cursor, middle/space drag pan,
    `ViewRequest` consumption, pan clamp).
  - `ui/ribbon.rs` — tabs Home/Geometry/Physics/Study/Results (Run/Step,
    tools, solver+fluid, generators/presets, field/particles/View).
  - `ui/menu.rs` — rare ops: file IO, resolution, margin checkbox.
  - `ui/inspector.rs` — object/group/multi panels (staged
    Rotate/Scale/Center about the common pivot), defaults.
  - `ui/status.rs` — status strip incl. zoom %; `ui/tree.rs` — model
    tree; `ui/legend.rs`, `ui/windows.rs`, `ui/generators.rs`.
- `src/sim.rs` — wgpu engine (`Settings`, `step_once`, choice tables,
  `ViewportMapping`). `src/model.rs` — object model + undo (index-based
  ops; panel edits coalesce via `record_modify_coalesced`).

## Nozzle chamber-fan speed: clamp layers

Since U3 a generated nozzle is an **Engine group**: the bell stamp is
walls-only and the chamber fan is a real filled Fan-rect CHILD
(`generators::nozzle_fan_layout`) — the Engine panel keys off the
parent link, not raster inspection. Clamps: dialog/auto multiplier and
the fan child's `fan_mult` (0.2–2.0) → runtime bounds in shaders:
**LBM `MAX_LATTICE_SPEED = 0.3` binds almost always** (lbm.wgsl);
Euler's Mach-8 sanity clamp effectively never (euler.wgsl). The cap is
a *readout*, not a field — the binding constants live in shaders.
(Pre-v8 scenes still carry fan-cell stamps; the inspector's raster-scan
Engine path remains for them only.)

## Frame-time baseline (plan working rules)

`FlowPaint-V2 --bench`: Pinball preset, compressible mode, default grid
(High, 1920×960 + margin), 10-frame warmup, 300 measured frames.
Baseline on pre-theme code (commit 1f00ef2), **this container** (Xvfb +
Mesa lavapipe software Vulkan — relative comparisons only, absolute
numbers are meaningless for real GPUs): see `docs/theme.md` §baseline.
Re-run identically after phase 3; mean and p99 must not regress.
