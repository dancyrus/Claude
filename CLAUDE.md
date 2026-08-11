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
  Next: U3 and U4 are unblocked (both need U2's selection set).
- **T2-A (color range + colormap): done**, branch
  `claude/t2a-color-range-opuijy` (off `34d57aa`, no U1); write-up in
  `docs/t2a-color-range.md`. Asymmetric manual min/max deliberately
  unimplemented (needs a per-mode offset in `render.wgsl`).
- **T2-B (probes + Re input): done**, branch
  `claude/flowpaint-v4.1-t2b-probes` (off the U1 tip).
- **T2-C (per-edge boundary conditions): done**, branch
  `claude/t2-c-boundary-conditions-s0iupf` (off the T2+U2 integration
  merge tip). Kinds: far field / inlet / outlet / wall; **periodic is
  reserved** (scene discriminant 4, greyed in the UI) — it needs both
  kernels' streaming/stencil indexing changed, blocked by the shader
  freeze. Read `docs/unit-decisions.md` §T2-C before touching edges,
  the sponge, or the tunnel preset.
- Scene format: **v9** current, loads v3+ (v3 lacks solver fields;
  v4/v5 share a layout; v6 appends per-object `locked`/`hidden`, so
  pre-v6 objects decode via the `SketchObjectV5` mirror; v7 appends
  the color ranges + colormap picks; **v8 is U3's**, concurrent — its
  fields and decode arm fold into `SceneV9` at the track merge, a
  planned app.rs seam; v9 appends the four edge kinds, derived from
  `wind_tunnel` for older files — see `SceneV3`…`SceneV9` in
  `app.rs`).
- The track merge folded T2-A's color-range state out of the sim.rs
  Mutex into `Settings.ranges` + `Cmd`, and persists it in scene v7
  (U5's PNG export needs a locked range to survive a save). T2-B's
  probe store still uses the track-era `sim::probes()` Mutex — folding
  it and unifying the plot/legend conversions in `ui/units.rs` are
  open follow-ups (see `docs/unit-decisions.md`).

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

- `src/app.rs` — state, `Cmd` dispatch, keyboard, scene IO (v3–v7
  bincode, version peek on first 4 LE bytes), `--bench` harness,
  `ViewRequest` + view state fields, selection set + clipboard.
- `src/ui/` — one file per panel; child of `app` via `#[path]` so panel
  code reads app state without visibility widening. `ui/mod.rs` owns
  draw order; `ui/theme.rs` owns all style; `ui/units.rs` owns every
  physical-quantity formatter (no inline `{:.N}` on physical values).
  Convention: canonical value in the box, unit in the label, derived
  value on a `theme::derived` secondary line.
  - `ui/canvas.rs` — gestures, overlays, scale bar, and the free view
    transform (wheel/pinch zoom at cursor, middle/space drag pan,
    `ViewRequest` consumption, pan clamp).
  - `ui/ribbon.rs` — tabs Home/Geometry/Physics/Study/Results (Run/Step,
    tools, solver+fluid, generators/presets, field/particles/View).
  - `ui/menu.rs` — rare ops: file IO, resolution, margin checkbox.
  - `ui/inspector.rs` — object panel (staged Rotate/Scale), defaults.
  - `ui/status.rs` — status strip incl. zoom %; `ui/tree.rs` — model
    tree; `ui/legend.rs`, `ui/windows.rs`, `ui/generators.rs`.
- `src/sim.rs` — wgpu engine (`Settings`, `step_once`, choice tables,
  `ViewportMapping`). `src/model.rs` — object model + undo (index-based
  ops; panel edits coalesce via `record_modify_coalesced`).

## Nozzle chamber-fan speed: six clamps, three layers

Dialog/auto multiplier clamps (0.2–2.0, `ui/generators.rs`) → per-cell
clamp baked into the stamp (`generators.rs:287`) → runtime bounds in
shaders: **LBM `MAX_LATTICE_SPEED = 0.3` binds almost always**
(lbm.wgsl); Euler's Mach-8 sanity clamp effectively never (euler.wgsl).
The "(speed-capped)" label is an incompressible-mode artifact. The cap
is a *readout*, not a field — the binding constants live in shaders.

## Edge BCs and the sponge (T2-C coupling)

`Settings.edges` holds the four edge kinds; non-preset sets are baked
as 2-cell bands by `sim::paint_edge_bcs` after the object pass. The
wind-tunnel preset {left inlet, right outlet, top/bottom far field}
instead keeps the model rasterizer's legacy band painting byte-for-byte
(`rasterize_region`'s `wind_tunnel` flag = "edges == preset"). **Any
WALL edge forces the absorbing sponge to width 0** (a sponged wall is
not a wall); inlet/outlet edges keep the sponge — that pairing is the
legacy tunnel and must not change. `wind_tunnel` remains the freestream
switch; toggling it re-arms the preset.

## Frame-time baseline (plan working rules)

`FlowPaint-V2 --bench`: Pinball preset, compressible mode, default grid
(High, 1920×960 + margin), 10-frame warmup, 300 measured frames.
Baseline on pre-theme code (commit 1f00ef2), **this container** (Xvfb +
Mesa lavapipe software Vulkan — relative comparisons only, absolute
numbers are meaningless for real GPUs): see `docs/theme.md` §baseline.
Re-run identically after phase 3; mean and p99 must not regress.
