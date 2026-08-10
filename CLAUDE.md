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

- **Branch names** (the real git branches; earlier notes calling U1's
  branch `ui/u1-navigation` are wrong — that branch does not exist):
  - Track 1 / U1: `claude/flowpaint-v4.1-u1-nav-o8c1im`
  - Track 2 / T2-A: `claude/t2a-color-range-opuijy`
- **T2-A (color range + colormap): done** except asymmetric manual
  min/max (deliberately unimplemented — needs a per-mode offset in the
  `render.wgsl` normalization). T2-B not started; it waits for U1 to
  merge (`ui/status.rs`).
- T2-A decisions a later unit must not re-derive (details in
  `docs/t2a-color-range.md`):
  - Color-range + colormap state lives in `sim.rs` behind
    `sim::color_ranges()` (a `Mutex`) ONLY because `app.rs`
    (`Cmd`/`UiSnapshot`, frozen for Track 1) never hands panels
    `&mut GpuSim`. **Fold into `Settings` + `Cmd` at the track merge.**
  - Locked/manual ranges and colormap picks are **not persisted in
    scene files** (scene IO is app.rs). U5's share-ready export needs
    them persisted — add at the merge with the scene-format bump.
  - Locked pins the **physical** value: the legend number holds and the
    colors re-derive through the current unit scaling. The sim maps a
    pinned range onto `display_gain` per frame (every render.wgsl
    mapping is linear in it); `render.wgsl` flags bit 1 = swap the
    view's colormap away from its default binding.

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

- `src/app.rs` — state, `Cmd` dispatch, keyboard, scene IO (v3/v4
  bincode, version peek on first 4 LE bytes), `--bench` harness.
- `src/ui/` — one file per panel; child of `app` via `#[path]` so panel
  code reads app state without visibility widening. `ui/mod.rs` owns
  draw order; `ui/theme.rs` owns all style; `ui/units.rs` owns every
  physical-quantity formatter (no inline `{:.N}` on physical values).
  Convention: canonical value in the box, unit in the label, derived
  value on a `theme::derived` secondary line.
- `src/sim.rs` — wgpu engine. `src/model.rs` — object model + undo
  (index-based ops; panel edits coalesce via `record_modify_coalesced`).

## Nozzle chamber-fan speed: six clamps, three layers

Dialog/auto multiplier clamps (0.2–2.0, `ui/generators.rs`) → per-cell
clamp baked into the stamp (`generators.rs:287`) → runtime bounds in
shaders: **LBM `MAX_LATTICE_SPEED = 0.3` binds almost always**
(lbm.wgsl); Euler's Mach-8 sanity clamp effectively never (euler.wgsl).
The "(speed-capped)" label is an incompressible-mode artifact. The cap
is a *readout*, not a field — the binding constants live in shaders.

## Frame-time baseline (plan working rules)

`FlowPaint-V2 --bench`: Pinball preset, compressible mode, default grid
(High, 1920×960 + margin), 10-frame warmup, 300 measured frames.
Baseline on pre-theme code (commit 1f00ef2), **this container** (Xvfb +
Mesa lavapipe software Vulkan — relative comparisons only, absolute
numbers are meaningless for real GPUs): see `docs/theme.md` §baseline.
Re-run identically after phase 3; mean and p99 must not regress.
