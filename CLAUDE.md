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
  `claude/u3-transforms-nested-groups-qo5qxi`. `Shape::Group`
  similarity nodes + `parent` links; gizmo in `ui/canvas.rs`. §U3.
- **T2-A (color range + colormap): done**, branch
  `claude/t2a-color-range-opuijy` (off `34d57aa`, no U1); write-up in
  `docs/t2a-color-range.md`. Asymmetric manual min/max deliberately
  unimplemented (needs a per-mode offset in `render.wgsl`).
- **T2-B (probes + Re input): done**, branch
  `claude/flowpaint-v4.1-t2b-probes` (off the U1 tip).
- **U4 (fill + eraser + snaps + domain extent): done**, branch
  `claude/eraser-design-report-fwk843` (off the second-track-merge
  main). Vector eraser only — stamp erase cut (approved), holes
  refused. Decisions in §U4; design report `docs/u4-eraser-design.md`.
  Next: U5 and mirror/array are unblocked.
- **T2-C (per-edge boundary conditions): done**, branch
  `claude/t2-c-boundary-conditions-s0iupf`. Far field / inlet /
  outlet / wall; **periodic is reserved** (blocked by the shader
  freeze). Read §T2-C before touching edges, sponge, tunnel preset.
- **T2-D (SI / decimal-inch toggle): done**, branch
  `claude/si-decimal-inch-toggle-x17w11` (off main). All conversion at
  the UI boundary — `ui/units.rs` `fmt_*` out, `InputUnit` adapters in
  (inputs take the active unit); canonical values stay SI. §T2-D.
- Scene format: **v9** current (second track merge: v9 ABSORBED v8 —
  U3's `parent` links / `Group` nodes / probe set sit above T2-C's
  appended `edges`), loads v3+ (v3 lacks solver fields; v4/v5 share a
  layout; v6 appends `locked`/`hidden` — pre-v6 objects use the
  `SketchObjectV5` mirror; v7 appends color ranges; v8 is decode-only,
  pre-v8 objects use the `SketchObjectV7` mirror; older files derive
  edges from `wind_tunnel`). Decode funnels … → v7 → v8 → v9; later
  bumps start at v10.
- Track-era Mutex debt is fully paid: T2-A's ranges live in
  `Settings.ranges` + `Cmd` (v7+), T2-B's probes in `Settings.probes`
  + probe `Cmd`s read via the per-frame `ProbeUi` snapshot (v8+).
  Still open: unify plot/legend inversion factors in `ui/units.rs`.

## Transforms & nested groups (U3)

**Transform composition order — fixed, do not re-derive: the child's
transform applies first, then each ancestor outward** (world =
`T_root ∘ … ∘ T_parent`(stored); `SketchModel::abs_of`). Group
transforms are similarities (uniform scale only). Everything else —
cycle prevention, world-space ops, why scaling is uniform — lives in
`docs/unit-decisions.md` §U3; read it before touching transforms.

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

- `src/app.rs` — state, `Cmd` dispatch, keyboard, scene IO (v3–v9
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

## Nozzle engines, edge BCs, the sponge

A generated nozzle is an **Engine group** keyed off the parent link
(clamp layers and the shader speed caps: `docs/unit-decisions.md`
§U3). Edge BCs: **any WALL edge forces the sponge to width 0**;
inlet/outlet keep it — that pairing IS the legacy tunnel. Full rules
(preset band painting, `paint_edge_bcs` ordering) in
`docs/unit-decisions.md` §T2-C; read it before touching edges, the
sponge, or the tunnel preset.

## Eraser, fill, snaps (U4)

The eraser is a per-object boolean subtract committed on release (one
undo entry per stroke); the paint bucket emits a traced filled Poly;
both share the degenerate-case guards in `src/geomops.rs`. Stamps ship
WITHOUT erase support (approved cut — `docs/u4-eraser-design.md`);
holes in filled polygons are refused and deferred (plan's deferred
list). Snap priority, refusal messages, the domain-extent uniform
trick and every other U4 decision: `docs/unit-decisions.md` §U4.

## Frame-time baseline (plan working rules)

`FlowPaint-V2 --bench`: Pinball preset, compressible mode, default grid
(High, 1920×960 + margin), 10-frame warmup, 300 measured frames — Xvfb
+ Mesa lavapipe, so relative paired A/B comparisons only (history in
`docs/theme.md`). Re-run as a paired A/B after any unit touching the
canvas or rasterizer; mean and p99 must not regress.
