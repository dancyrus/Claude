# FlowPaint V2 — working notes

**Read `docs/agent-protocol.md` first, before anything else here.**

Rust desktop CFD paint app in `FlowPaint/` (also `FingerFlow/`, an iOS
sibling — untouched by the UI overhaul). Two solvers: D2Q9 LBM
(incompressible, `shaders/lbm.wgsl`) and finite-volume Euler MUSCL+HLLC
(compressible, `shaders/euler.wgsl`). Geometry is a persistent vector
object model (`model.rs`): objects stay live and editable, the grid is a
damage-region re-projection, **nothing is ever flattened** — the
eraser is a per-object, undoable subtract (U4).

The working plan was `docs/flowpaint-plan-v4.1.md`, now COMPLETE
(tag `v2.0-plan-v4.1`); the follow-up backlog and its claim state
live in `docs/queue.md`. Everything FlowPaint deliberately
does NOT have is indexed in one place, `docs/deferred.md` (cut vs
deferred, with reasoning pointers) — read it before assuming a feature
is missing, re-adding one, or starting follow-up work. Standing
constraints carry from `docs/flowpaint-ui-overhaul-plan-v3.md`; the
layout mockup `docs/ui-target.html` is authoritative for spacing,
color, type and panel arrangement — not for widget construction or
numeric values.

## Unit status (plan v4.1)

Per-unit decisions that must not be re-derived live in
`docs/unit-decisions.md` — read it before touching a unit's area.

- **U1 (cleanup + navigation): done**, branch
  `claude/flowpaint-v4.1-u1-nav-o8c1im`.
- **U2 (multi-select + tier): done**, branch
  `claude/flowpaint-u2-multi-select-kvfbsi` (off the U1 branch).
- **U3 (transforms + nested groups): done**, branch
  `claude/u3-transforms-nested-groups-qo5qxi`. `Shape::Group`
  similarity nodes + `parent` links; gizmo in `ui/canvas.rs`. §U3.
- **T2-A (color range + colormap): done**, branch
  `claude/t2a-color-range-opuijy` (off `34d57aa`, no U1); write-up in
  `docs/t2a-color-range.md`.
- **T2-B (probes + Re input): done**, branch
  `claude/flowpaint-v4.1-t2b-probes` (off the U1 tip).
- **U4 (fill + eraser + snaps + domain extent): done**, branch
  `claude/eraser-design-report-fwk843` (off the second-track-merge
  main). Vector eraser only. Decisions in §U4; design report
  `docs/u4-eraser-design.md`.
- **Mirror + linear array: done**, branch
  `claude/mirror-linear-array-vlmsub` (off main). Independent deep
  copies, never instances; controls in the inspector's selection
  panels; reflection bakes per shape (`Reflect2` — not a `Sim2`).
  Read §Mirror & linear array before touching it.
- **U5 (share-ready output): done**, branch
  `claude/u5-share-ready-output` (off main). One `Cmd::ExportPng`
  path, Canvas | Annotated (the sheet self-describes in the active
  unit system); first-run scene = Pinball; Ctrl+E / Ctrl+Shift+E. §U5.
- **T2-C (per-edge boundary conditions): done**, branch
  `claude/t2-c-boundary-conditions-s0iupf`. Far field / inlet /
  outlet / wall; periodic is reserved. §T2-C.
- **T2-D (SI / decimal-inch toggle): done**, branch
  `claude/si-decimal-inch-toggle-x17w11` (off main). All conversion at
  the UI boundary (`ui/units.rs` `fmt_*` out, `InputUnit` adapters
  in); canonical values stay SI. §T2-D.
- Scene format: **v9** current (v9 ABSORBED v8 at the second track
  merge — U3's `parent`/`Group` nodes/probes above T2-C's appended
  `edges`), loads v3+ (v3 lacks solver fields; v4/v5 share a layout;
  v6 appends `locked`/`hidden`, pre-v6 decodes via the
  `SketchObjectV5` mirror; v7 appends color ranges; v8 decode-only,
  pre-v8 via `SketchObjectV7`; older files derive edges from
  `wind_tunnel`). Decode funnels … → v7 → v8 → v9; bumps start at v10.
- Track-era static debt is fully paid, three folds, no more: T2-A's
  ranges (`Settings.ranges`, v7+), T2-B's probes (`Settings.probes`,
  v8+), T2-D's unit system (`Settings.unit_system` + `Cmd`, NOT
  scene-persisted — display preference; the `units.rs` static is now a
  per-frame mirror only). §Third integration.

## Transforms & nested groups (U3)

**Transform composition order — fixed, do not re-derive: the child's
transform applies first, then each ancestor outward** (world =
`T_root ∘ … ∘ T_parent`(stored); `SketchModel::abs_of`). Group
transforms are similarities (uniform scale only). Everything else —
cycle prevention, world-space ops, why scaling is uniform — lives in
`docs/unit-decisions.md` §U3; read it before touching transforms.

## Hard rules

- **Shader freeze LIFTED** (queue era): record every shader change in
  `docs/unit-decisions.md`, re-run both solver modes, and re-run the
  paired `--bench`. The inferno/coolwarm stop tables in `app.rs`
  mirror `render.wgsl`; keep them linked.
- **egui stays at 0.29.1 — the upgrade is the exclusive queue item**.
  Moving to 0.35 drags wgpu 22→29 through `sim.rs`; the known API
  breaks are recorded in `docs/deferred.md` (egui entry).
- All visual constants resolve through `src/ui/theme.rs`
  (`docs/theme.md`); no ad-hoc colors, rounding, spacing or font
  sizes. Exceptions: colormap mirrors and `def_smoke` in `app.rs`,
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
    gizmo (corner scale, rotate handle, draggable pivot), the mirror
    line tool, and the free view transform (zoom at cursor,
    middle/space drag pan, `ViewRequest` consumption, pan clamp).
  - `ui/ribbon.rs` — quick access (Pause/Step/Undo/Redo, every tab) +
    tabs: Home = scene lifecycle (New/Open/Save/exports/Reset flow),
    then Geometry, Physics, Study, Results.
  - `ui/menu.rs` — rare ops: file IO, PNG export, resolution, margin
    checkbox. `ui/export.rs` — U5 annotated-sheet composition (pure
    `ExportInfo` → pixels; GPU-free tests).
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
in `docs/unit-decisions.md` §T2-C; read it before touching edges, the
sponge, or the tunnel preset.

## Eraser, fill, snaps (U4)

The eraser is a per-object boolean subtract committed on release (one
undo entry per stroke); the paint bucket emits a traced filled Poly;
both share the degenerate-case guards in `src/geomops.rs`. Stamp
erase and polygon holes are refused (indexed in `docs/deferred.md`;
design report `docs/u4-eraser-design.md`). Every other U4 decision:
`docs/unit-decisions.md` §U4.

## Frame-time bench

Required for any change touching `ui/canvas.rs`, `model.rs`,
`geomops.rs`, or `sim.rs`; mean and p99 must not regress. Procedure:
`docs/agent-protocol.md` §Frame-time bench; history: `docs/theme.md`.
