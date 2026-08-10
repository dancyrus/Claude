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

- **U1 (cleanup + navigation): done**, branch `claude/flowpaint-v4.1-u1-nav-o8c1im`.
  Next: U2 (one author, no delegation) and T2-A may run concurrently;
  T2-B waits for U1 to merge (`ui/status.rs`).
- **U2 (multi-select + tier): done**, branch
  `claude/flowpaint-u2-multi-select-kvfbsi` (off the U1 branch).
  Next: U3 and U4 are unblocked (both need U2's selection set).
- Scene format: **v6** current, loads v3+ (v3 lacks solver fields;
  v4/v5 share a layout; v6 appends per-object `locked`/`hidden`, so
  pre-v6 objects decode via the `SketchObjectV5` mirror — see
  `SceneV3`/`SceneV4`/`SceneV6` in `app.rs`). **Version-number
  correction**: plan v4.1's U2 text says "scene format to v5", but v5
  was already current before U2 — U2 went to **v6**. U3 and T2-C: the
  next bump is **v7**; do not repeat the plan's off-by-one.
- T2-A's locked color ranges and colormap picks live behind a Mutex in
  `sim.rs` on the T2-A branch (app.rs is frozen while both tracks run)
  and are NOT in scene v6. Persisting them is a track-merge task; U5
  depends on it.
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
- U2 decisions a later unit must not re-derive:
  - Selection is `selected: Vec<u64>` — an ordered set: insertion
    order, no duplicates, last = primary. ALL writes go through the
    helpers in `app.rs` (`select_only`/`select_add`/`select_toggle`/
    `deselect`/`deselect_all`); `prune_selection` runs once per frame.
    Locked objects can enter the selection only via the tree and are
    filtered by `editable_selection()` at every mutating operation.
  - **Rubber-band = INTERSECT**, not fully-contain
    (`SketchObject::intersects_rect`): FlowPaint scenes are dominated
    by thin open geometry (lines, polylines, outline shapes), where
    fully-contain would force lassoing an object's whole extent to
    grab it. Touching the band selects.
  - Modifiers: canvas Shift-click toggles membership; tree Ctrl-click
    toggles, Shift-click ranges from `tree_anchor`. Tree lists objects
    TOP-FIRST (reverse model order); `model.objects` order IS z-order
    (later = rasterized later = wins overlaps), reordered undoably via
    `SketchModel::reorder`.
  - One undo entry per user action across a selection:
    `ModelOp::Group` (+ `add_many`/`remove_many`/`record_modify_many`
    (`_coalesced`)); group undo applies members in reverse.
  - Ctrl+C/V NEVER arrive as key events — egui swallows them into
    `Event::Copy`/`Event::Paste`, and Paste only fires when the system
    clipboard holds text, so `copy_selected` writes the
    `CLIPBOARD_MARKER` breadcrumb text and paste matches it. The
    object clipboard itself is app-internal (`clipboard`/`paste_gen`).
  - Nudge is 1 cell, Shift = 8 (plan v4.1 flipped U1's Shift-for-fine).
  - Hidden objects are skipped by `rasterize_region` and `hit_test`;
    locked ones by `hit_test` only. Engaging either drops the object
    from the selection; both persist in scene v6.
  - Multi-selection inspector shows mixed-value indicators; an edit
    applies to the whole editable selection (never silently seeded
    from the first object). Rotate/scale of a set is deferred to U3.

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

- `src/app.rs` — state, `Cmd` dispatch, keyboard, scene IO (v3–v6
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

## Frame-time baseline (plan working rules)

`FlowPaint-V2 --bench`: Pinball preset, compressible mode, default grid
(High, 1920×960 + margin), 10-frame warmup, 300 measured frames.
Baseline on pre-theme code (commit 1f00ef2), **this container** (Xvfb +
Mesa lavapipe software Vulkan — relative comparisons only, absolute
numbers are meaningless for real GPUs): see `docs/theme.md` §baseline.
Re-run identically after phase 3; mean and p99 must not regress.
