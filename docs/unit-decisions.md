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

## U3 (transforms + nested groups)

- **Composition order (the one U3 decision that must never drift):
  child transform first, then each ancestor outward** — world =
  `T_root ∘ … ∘ T_parent`(stored). Stored coordinates are PARENT-space;
  `SketchModel::abs_of` walks inner→outer, left-composing. Also stated
  in CLAUDE.md, per the plan.
- A group is an object: `Shape::Group { t, rot, scale }` (appended
  enum variant + appended `parent: Option<u64>` field keep bincode
  positional compatibility; pre-v8 objects decode via the
  `SketchObjectV7` mirror). Group transforms are **similarities**
  (uniform scale only): the only family closed under composition that
  the shape set can represent — non-uniform world scaling of a rotated
  nest is shear, which Rect/Ellipse/Stamp cannot store. Hence gizmo
  and panel scaling are uniform; per-axis reshaping stays on the
  single-object vertex handles, and the stamp tooltip declares
  non-uniform stamp scaling out of scope (plan requirement).
- Group nodes carry no z-slot semantics (they never rasterize) and
  subtrees are NOT kept contiguous in `model.objects` — flat list
  order stays the z-order; the tree nests by `parent` walk. Z-order
  ops expand to whole subtrees.
- Transforming a group is O(1) (its node's transform edits); members
  follow through composition. World-space edits go through
  `translate_world`/`rotate_world`/`scale_world` — a similarity
  conjugates a rotation/uniform-scale to the same rotation/scale about
  the mapped pivot, so gestures apply world deltas in stored space
  exactly. Gizmo drags restore `before` and re-apply the accumulated
  total each frame (no per-frame error compounding).
- Interactive `scale_about` does not scale leaf `thickness` (U1/U2
  precedent); flattening through a scaled GROUP does (`apply_sim`) —
  a scaled-down group renders proportionally thinner strokes. Known,
  deliberate asymmetry.
- **Cycle prevention is mandatory and double-layered**: live
  `reparent` refuses self/descendant targets (tested in model.rs);
  `sanitize_parents` (app.rs) repairs crafted files — dangling or
  non-group parents detach, cycles break. Every ancestor walk is
  hop-capped besides.
- Selection: click picks the OUTERMOST group; double-click enters a
  group (`entered_group`) so clicks pick one level below; Esc leaves,
  then deselects. When an ancestor and its descendant are both
  selected, only the ancestor transforms (`transform_targets`).
  Lock/hide are effective through the chain (`eff_locked/eff_hidden`).
- Copy flattens copied roots to world space (paste is scene-load
  robust); duplicate keeps copies as siblings inside their group.
  Delete removes whole subtrees child-first as one undo entry.
- The gizmo rotate handle snaps to `snap_angle_deg` while Shift is
  held (the draw-tool convention); corner scale is uniform, so plan
  v4.1's "Shift constrains scale to uniform" is trivially satisfied.
- Scene **v8**: `parent` + `Group` nodes + probes (positions,
  quantity, show_plot) persisted; loads v3–v7 (T2-C takes v9).
- T2-B debt closed: `sim::probes()` static Mutex removed. Store =
  `Settings.probes`, edits = `Cmd::{AddProbe,RemoveProbe,
  SetProbeQuantity,SetProbePlot,ClearProbeSamples,LoadProbes}`, read =
  per-frame `ProbeUi` snapshot (sample series cloned only while the
  plot shows). Probe markers draw from the canvas's own mapping
  (`canvas_mapping`) — the sim no longer publishes a view transform;
  probe placement is a real canvas interaction now (`probe_arming`).
  The plot/legend conversion unification into `ui/units.rs` remains
  open (tracked under T2-B's notes below).
- Nozzle free win: `generate_nozzle` emits walls only;
  `nozzle_fan_layout` mirrors its layout math for the chamber-fan
  rect, inserted as a real Fan child in an Engine group (one
  `add_many` undo entry). The Engine inspector keys off the parent
  link (Fan child + stamp child); the raster-scan path survives ONLY
  for pre-v8 scenes with baked fan cells.

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

## Second track merge (U3 + T2-C)

- The planned app.rs seam resolved as designed: **v9 absorbed v8** —
  U3's fields (`parent`/`Group` objects, probes, plot prefs) sit above
  T2-C's appended `edges`, keeping the whole v3→v9 lineage
  append-only. `SceneV8` remains as a DECODE-ONLY layout for files
  written on the U3 branch pre-merge; the decode chain funnels
  … → v7 → v8 → v9 (`SceneV8::from_v7`, `SceneV9::from_v8`). A v8
  file's edges default from its `wind_tunnel` flag — all far field,
  or the tunnel preset — exactly like every other pre-v9 file.
- The two cross-unit behavioral invariants are pinned by tests:
  `sim::edge_bc_tests::legacy_wind_tunnel_projection_is_unchanged`
  (full geometry projection of a legacy tunnel scene is byte-identical
  to the pre-T2-C rasterizer alone) and the v7/v8/v9 round-trip tests
  in `app::scene_tests` (a locked color range survives every
  conversion path; groups, probes and edges survive or default).
- No decision from either unit moved. U3's probe fold and T2-C's edge
  machinery compose without overlap: probes ride `Settings.probes`,
  edges ride `Settings.edges`, and both persist in v9.
