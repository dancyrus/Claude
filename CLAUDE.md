# FlowPaint V2 — working notes

Rust desktop CFD paint app in `FlowPaint/` (also `FingerFlow/`, an iOS
sibling — untouched by the UI overhaul). Two solvers: D2Q9 LBM
(incompressible, `shaders/lbm.wgsl`) and finite-volume Euler MUSCL+HLLC
(compressible, `shaders/euler.wgsl`). Geometry is a persistent vector
object model (`model.rs`): objects stay live and editable, the grid is a
damage-region re-projection, **nothing is ever flattened** — any eraser
must be a per-object, undoable operation (see plan phase 6a).

The UI overhaul spec is `docs/flowpaint-ui-overhaul-plan-v3.md` (v3
supersedes v2; corrected against `docs/ui-inventory.md`). The layout
mockup `docs/ui-target.html` is authoritative for spacing, color, type
and panel arrangement — not for widget construction or numeric values.

## Hard rules

- **Never edit anything under `FlowPaint/src/shaders/`** during the UI
  effort. The inferno/coolwarm stop tables in `app.rs` mirror
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
