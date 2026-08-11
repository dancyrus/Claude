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

## T2-B (persistent probes + Reynolds input)

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

## T2-C (per-edge boundary conditions)

- Shipped edge kinds: **far field, inlet, outlet, wall** (`EdgeKind`,
  `Settings.edges` in sim.rs). **Periodic is reserved, not shipped**:
  it needs wraparound in the LBM streaming (`lbm.wgsl:84`) and the
  Euler stencil clamping (`euler.wgsl:101`) — a change in BOTH kernels
  that the shader freeze blocks. Scene v9 reserves discriminant 4 for
  it and the UI greys it out; implementing it is a **post-freeze
  task** needing no further format bump.
- **`wind_tunnel: true` never was "no painted edge cells"** (this was
  not written down anywhere and cost T2-C a re-read): the model
  rasterizer has always painted 2-cell bands at the true grid edges —
  left `CELL_INLET` (fan (1,0), 2-of-12 dye seed stripes) and right
  `CELL_OUTLET` (`rasterize_region`, model.rs). Top and bottom were
  far field (zero-gradient edge + sponge). The preset therefore maps
  to **{left inlet, right outlet, top/bottom far field}**, not
  all-far-field — that mapping is what keeps legacy scenes' output
  identical.
- The preset keeps the rasterizer's own band painting (byte-identical
  legacy output); only non-preset edge sets go through
  `sim::paint_edge_bcs`, which clips to the repainted region (so
  damage uploads never balloon to the full perimeter) and runs after
  the object pass — inside the 2-cell bands the domain boundary wins.
- **Sponge coupling: any WALL edge forces sponge width 0** (a sponged
  wall is not a wall — the sponge relaxes near-edge cells toward the
  freestream and silently overrides no-slip). Inlet/outlet edges KEEP
  the sponge: that pairing IS the legacy wind tunnel, and dropping it
  would change existing scenes' output. This deliberately narrows the
  original decision "any non-far-field edge kills the sponge", which
  was made against the wrong premise above.
- `Settings.wind_tunnel` stays as the freestream switch (`free_u`,
  reset state); `Cmd::SetWindTunnel` re-arms the legacy edge preset.
  Every pre-v9 load path derives `edges` from it (`EdgeBcs::legacy`).
