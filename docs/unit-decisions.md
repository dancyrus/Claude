# Per-unit decisions (plan v4.1) — do not re-derive

Decisions each landed unit fixed. Later units must not re-derive,
"fix", or quietly reverse any of these; if one has to move, say so
explicitly in the commit and update this file. CLAUDE.md keeps only
unit status and pointers here.

## U1 (cleanup + navigation)

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

## U2 (multi-select + the tier that rides on it)

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
  from the selection; both persist in the scene format (v6+).
- Multi-selection inspector shows mixed-value indicators; an edit
  applies to the whole editable selection (never silently seeded
  from the first object). Rotate/scale of a set is deferred to U3.

## T2-A (locked color range + colormap)

Full write-up: `docs/t2a-color-range.md`.

- Locked pins the **physical** value: the legend number holds and the
  colors re-derive through the current unit scaling. The sim maps a
  pinned range onto `display_gain` per frame (every `render.wgsl`
  mapping is linear in it); `render.wgsl` flags bit 1 = swap the
  view's colormap away from its default binding.
- Asymmetric manual min/max is **deliberately unimplemented** — it
  needs a per-mode offset in the `render.wgsl` normalization, and only
  the flags-bit colormap edit was approved.
- Range/map controls live in the legend under the color bar, NOT in
  the Results ribbon (no width left at the 900 px minimum). The
  ribbon's display-gain slider disables while the mode's range is
  pinned. Smoke/Dye has no scale, so no range control.
- Track-era plumbing (superseded at the track merge): state sat behind
  `sim::color_ranges()` (a `Mutex` in `sim.rs`) only because `app.rs`
  was frozen for Track 1 — the merge folds it into `Settings` + `Cmd`
  and persists it in the scene format.
