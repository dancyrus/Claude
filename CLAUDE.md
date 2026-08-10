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

- **Branch names** (real git branches; `ui/u1-navigation` in older notes
  is wrong — that branch does not exist):
  - U1: `claude/flowpaint-v4.1-u1-nav-o8c1im`
  - T2-A: `claude/t2a-color-range-opuijy` (off `34d57aa`, like U1)
  - T2-B: `claude/flowpaint-v4.1-t2b-probes` (off the U1 tip)
- **U1 (cleanup + navigation): done.** Next: U2 (one author, no
  delegation) may run concurrently with Track 2.
- **T2-A (color range + colormap): done** on its branch. Nothing is
  merged yet; the default branch has neither track. Its decisions, which
  T2-B and later units must not re-derive or "fix" into app.rs:
  - Color-range + colormap state lives behind `sim::color_ranges()` (a
    `Mutex` in `sim.rs`) ONLY because `app.rs` (`Cmd`/`UiSnapshot`) is
    frozen for Track 1 and panels never hold `&mut GpuSim`. Fold into
    `Settings` + `Cmd` at the track merge.
  - Locked ranges and colormap picks are **not scene-persisted** (scene
    IO is app.rs); U5 needs them persisted — add with the format bump at
    the merge.
  - `render.wgsl` has ONE approved edit: flags bit 1 = swap the view's
    colormap (no uniform added or reordered). Not a violation of the
    no-shader rule.
- **T2-B (probes + Re input): done** on its branch (needs U1's
  `ui/status.rs`; independent of T2-A). Decisions:
  - Probe storage follows the T2-A precedent: `sim::probes()` (a `Mutex`
    in `sim.rs`) holds positions, ring-capped histories (2048 frames,
    stated in the plot panel) and plot preferences; not scene-persisted.
    Fold into app state at the merge.
  - Sampling is a per-frame GPU copy of single cells into a 3-deep
    mapped staging ring (copy → map next frame → read when done; never
    blocks a frame). Field buffers gained `COPY_SRC` for it.
  - The sim publishes the canvas view transform through the probe store
    so `ui/status.rs` can draw probe markers without touching Track 1's
    `ui/canvas.rs`; probe placement is an armed raw click (tree button →
    next canvas click, Esc cancels).
  - Probe plot conversions duplicate the legend's shader-inversion
    factors (T2-A keeps its own copy in `ui/legend.rs`) — unify both
    into `ui/units.rs` at the merge.
- Scene format: **v5** current, loads v3+ (v3 lacks solver fields; v4/v5
  share a layout — see `SceneV3`/`SceneV4` in `app.rs`).
- U1 decisions a later unit must not re-derive:
  - Inspector Rotate/Scale DragValues are **staged deltas per selection**
    (`inspector_stage`), not absolute properties — a selection has no
    single angle, so this is the form U3 extends to a common pivot.
  - Zoom/pan is a view transform on `px_per_cell`/`lb_origin` only:
    canvas owns `view_zoom`/`view_center`/`view_fit`; `Cmd::SetMapping`
    is a passthrough (no re-fit); ribbon/Ctrl+0/1/2 talk to the canvas
    via `ViewRequest`. Grid, margin, `domain_width_m`, readouts:
    untouched by design — treat any coupling as a bug.
  - Space pauses on **release** (held Space + drag pans); pick
    thresholds are screen-space (no cell floors).
  - `MARGIN_CHOICES`/`PARTICLE_CHOICES` have no off entry; off is a
    checkbox (`margin_on`/`particles_on`) and the index remembers the
    last value. Single-step = `Cmd::StepOnce` → `GpuSim::step_once`.

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

- `src/app.rs` — state, `Cmd` dispatch, keyboard, scene IO (v3–v5
  bincode, version peek on first 4 LE bytes), `--bench` harness,
  `ViewRequest` + view state fields.
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

## Frame-time baseline (plan working rules)

`FlowPaint-V2 --bench`: Pinball preset, compressible mode, default grid
(High, 1920×960 + margin), 10-frame warmup, 300 measured frames.
Baseline on pre-theme code (commit 1f00ef2), **this container** (Xvfb +
Mesa lavapipe software Vulkan — relative comparisons only, absolute
numbers are meaningless for real GPUs): see `docs/theme.md` §baseline.
Re-run identically after phase 3; mean and p99 must not regress.
