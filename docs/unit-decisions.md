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
- Nozzle chamber-fan speed, clamp layers (U1-era, moved here from
  CLAUDE.md at U4): dialog/auto multiplier and the fan child's
  `fan_mult` clamp to 0.2–2.0; the runtime bounds live in the shaders
  — **LBM `MAX_LATTICE_SPEED = 0.3` binds almost always** (lbm.wgsl),
  Euler's Mach-8 sanity clamp effectively never (euler.wgsl). The cap
  is a *readout*, not a field — the binding constants live in shaders.

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

- Branch provenance (detail moved out of CLAUDE.md): the T2-C branch
  was cut from the T2+U2 integration merge tip.
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

## U4 (fill + eraser + object snaps + domain extent)

Design report: `docs/u4-eraser-design.md` (stamp erase cut from the
first release, approved; bell venting = overdraw with vector walls,
stated in the eraser tooltip next to the stamp refusal).

- **Shared geometry lives in `src/geomops.rs`** — the eraser booleans
  and the bucket's flood/trace share one set of degenerate-case guards
  (the ui/canvas.rs 0.5-cell minimum radii are the precedent): open
  fragments < 1.0 cell drop, closed fragments < 1.0 cell² drop,
  vertices weld at 1e-3.
- **Eraser commits on RELEASE**, not live: a subtract can split an
  object, and live application changes object identity mid-drag. One
  undo entry per stroke via `SketchModel::apply_erase` — a
  `ModelOp::Group` of Modify (first fragment keeps the original id and
  z-slot) + the new `ModelOp::Insert` (remaining fragments contiguous
  above it) + Remove (erased to nothing). The plan's
  `record_modify_coalesced` line survives only for in-place cases;
  splits change the object count, which a coalesced modify cannot
  express.
- Stroke conjugation: world capsules map into each object's STORED
  space exactly (similarity ⇒ disc stays a disc; radius divides by the
  composed scale — the same conjugation as `hit_under`). Centerline
  shapes inflate the radius by half the thickness (WYSIWYG); filled
  polygons do not.
- Filled-polygon subtraction is SEQUENTIAL per capsule (each is
  convex; Greiner–Hormann difference walk in `poly_minus_convex`),
  ordered by BFS from the boundary-crossing capsules so a stroke that
  starts interior but crosses the edge subtracts fine; an interior-only
  footprint refuses (`WouldHole` — `Shape::Poly` has no holes; on the
  plan's deferred list). Two robustness measures found by test, do not
  remove: the stroke is RDP-simplified at r/4 before capsule building,
  and each capsule gets a deterministic sub-0.5% radius jitter keyed by
  stroke index — consecutive same-radius capsules on a straight drag
  otherwise carve slot walls EXACTLY tangent to the next capsule and
  the crossing walk corrupts.
- The two refusals carry DISTINCT messages: stamp (with the
  overdraw/bell-vent workaround) vs interior-hole (with the
  cross-the-edge fix). Rect/Ellipse polygonize to a Poly only on an
  actual cut (a miss keeps them parametric). Locked/hidden skipped
  (`editable` convention); erase is not selection-scoped.
- **Right-drag-erase from 61368c8 is NOT reinstated** — verified
  against the U1/U2 gesture map: the Secondary press now finishes
  polylines and clears selections. The X key and `Tool::Eraser`
  return; Fill = F, Measure = M.
- **Paint bucket** rasterizes the MODEL alone (margin 0, no tunnel
  bands) over the visible grid, floods 4-connected, refuses on
  non-fluid clicks and regions open to the domain edge (distinct
  messages), traces the outer contour (corner walk keeping the region
  on the right; saddle = sharpest turn, matching 4-connectivity),
  RDP eps 0.75, and inserts the filled Poly at the BOTTOM z-slot
  (`insert_at(0)`) so interior island walls keep winning overlaps.
  Known caveat: fluid sealed INSIDE a hollow island gets covered by
  the fill polygon (holes again). Tooltip states the
  boundary-is-a-snapshot limitation.
- **Filled closed Poly**: even-odd scanline fill over cell centres;
  thickness ignored (the filled Rect/Ellipse convention); hit test
  adds point-in-polygon; `can_fill` extended in the inspector.
- **Object snaps**: endpoint > intersection > midpoint > center >
  perpendicular (that fixed order first, distance breaks ties within a
  kind). Radius 10 screen POINTS (`SNAP_RADIUS_PT`), so it holds
  across zoom. Ctrl suspends; an object snap beats the grid snap;
  hidden objects are skipped but LOCKED objects still snap (reference
  geometry); the object being drawn is excluded. One candidate is
  computed per frame from the active pointer and consumed by every
  snapped coordinate that frame. Ellipse quadrant points rank as
  Midpoint; intersections come from a 64-segment pool of nearby
  outlines (different objects only). Perpendicular needs the gesture's
  anchor. MoveSel and the gizmo pivot keep grid-only snapping.
- **Measure** (delegable-with-snaps item, folded in): drag two points,
  distance + angle live on the canvas and into the status line on
  release; creates no object; Shift angle-snaps.
- **Domain extent** is a pure UNIFORM change: `Settings.show_extent`
  (not scene-persisted) makes `write_render_uniform` widen
  `vis_origin/vis_size` to the full grid and shift `lb_origin` by the
  margin — no shader edit, no mapping change, PNG export untouched.
  The canvas overlay shades the margin ring, outlines the interior,
  and labels the margin in cells + physical units via `ui/units.rs`.
  Acceptance held by construction: every readout keeps visible-cell
  coordinates.
- New theme entries: `SNAP_MARK` (amber — a snap candidate must not
  read as a teal selection), `eraser_fill()` (destructive red, α .25),
  `EXTENT_OUTLINE`/`extent_margin_fill()`.
- **Known measurement gap: the object-snap per-frame cost is
  unmeasured by the `--bench` harness** — the harness drives no
  pointer and stays on the Select tool, so `compute_osnap`
  (ui/canvas.rs) never executes in a bench run. Measuring it would
  take an armed Line tool with the pointer over a dense scene. This is
  a gap, not a claim that snaps are free.

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

## T2-D (SI / decimal-inch unit toggle)

- The conversion lives at the UI BOUNDARY ONLY: every canonical value
  in the app — `Settings`, `Cmd`s, the scene format, the value a
  DragValue commits — stays SI; `ui/units.rs` converts on the way out
  (the `fmt_*` formatters) and on the way in (the `InputUnit`
  adapters). Input boxes accept and display the ACTIVE unit system:
  Physics▸Domain width and the legend's Manual range max wire an
  `InputUnit` into `DragValue::custom_formatter`/`custom_parser`, so
  typing 24 in inch mode commits 0.6096 m. The canonical-value
  convention governs storage and call-site formatting, not what the
  user types — an inch mode you cannot type inches into would be
  read-only, defeating the toggle. The type→commit→switch→switch-back
  round trip is pinned by test (no float-conversion drift; the box
  returns to exactly "24.00 in").
- Inch mode is ASME decimal-inch practice: inches only (no feet, no
  fractions), the leading zero dropped below 1 in (".500 in"),
  precision stepping 4/3/2/1 decimals as magnitude grows. Derived
  quantities use the inch–pound–second system: in/s, psi, in²/s —
  EXCEPT density, which is deliberately lbm/ft³ (air ≈ 0.0765), not
  the dimensionally consistent lb/in³ (air 4.34e-5): nobody carries
  gas density in pounds per cubic inch, mixed units are normal in ips
  work, and the goal is the number an engineer recognizes. psi and
  in²/s read fine and stay. Time, angles, Mach, CFL, vorticity (1/s),
  factors, sim rate and zoom are unit-system neutral and identical in
  both modes. Input boxes use plain decimal display (no ASME
  leading-zero drop) — typing convention, not drawing convention.
- The selector is a "units" row (theme::toggle pair SI / inch) in
  Results▸Display: that group had vertical room inside the 64 px
  group height, so the Results tab gains ZERO width at the 900 px
  minimum. Verified interactively at 900×600 in both solver modes
  (LBM and Euler) under Xvfb.
- Track-era store, same precedent as T2-A/T2-B: a `static AtomicBool`
  in `ui/units.rs` behind `unit_system()`/`set_unit_system()`,
  because `app.rs` was frozen while U4 ran concurrently. (Folded at
  the third integration — see §Third integration; the branch-era
  forecast of scene persistence at v10 was decided AGAINST there: the
  unit system is a display preference, not scene state.)
- Bypass audit came up clean: every physical readout routes through
  `units::fmt_*` — legend, status strip, probe-plot axis labels,
  ribbon derived lines, generators, tree, canvas scale bar and drag
  dimensions, inspector derived lines — and both unit-bearing input
  boxes route through the `InputUnit` adapters; `src/ui/` carries no
  hardcoded unit strings outside `units.rs`.

## Third integration (U4 + T2-D)

- Merge order: main → U4 (`claude/eraser-design-report-fwk843`) →
  T2-D (`claude/si-decimal-inch-toggle-x17w11`); zero conflicts, as
  dry-run predicted.
- **T2-D's static folded into `Settings.unit_system` + a
  `Cmd::SetUnitSystem`** — the THIRD and LAST instance of the
  track-era static→Settings pattern (T2-A ranges, T2-B probes, T2-D
  units). The `UnitSystem` enum moved to sim.rs beside
  `ColorMap`/`RangeMode`; the ribbon reads `snap.unit_system` and
  edits via `Cmd`; `update` mirrors the setting into `ui/units.rs`
  once per frame before any panel draws, so the ~60 `fmt_*` call
  sites keep stateless signatures. The static remains ONLY as that
  frame-scoped mirror — single production writer, never a store. Any
  future unit that reaches for a process-wide static instead of
  `Settings` + `Cmd` is re-opening a closed pattern.
- **The unit system is a per-user display preference, NOT
  scene-persisted** (decided here, with the format untouched at v9):
  a scene file is shared work product, and loading a colleague's
  file must not flip your units — the same reasoning that keeps
  `show_extent` (U4) out of the scene. Session-scoped for now;
  eframe-storage persistence can ride any later UI-prefs work.
- CLAUDE.md auto-merged to 143 lines; the fold rewrote the track-debt
  bullet (now 145 of 150) — no detail needed routing beyond what this
  file already carries.
- U4 formatter audit at the merge: U4's new UI (eraser, bucket,
  snaps, measure, extent overlay) already routes its physical
  readouts through `ui/units.rs`, so T2-D's toggle covers them with
  no call-site changes; no hardcoded unit strings entered `src/ui/`.

## Mirror & linear array (deferred out of U4, landed after)

- **Both ops produce INDEPENDENT deep copies, never instances** (the
  plan's one hard requirement; the tooltips state it): fresh ids
  minted first, then parent links remapped WITHIN each copied
  subtree — a copied root keeps the original root's parent, staying
  its sibling. One undo entry each via `add_many`'s Group op.
  `SketchModel::copy_subtree_with` is the shared deep-copy;
  `mirror_subtrees` / `array_subtrees` sit beside it in model.rs so
  the disentanglement tests run without the app shell.
- **A reflection is not a `Sim2`** (det −1 — the U3 similarity family
  cannot express it), so mirroring BAKES into stored geometry per
  shape (`Reflect2` + `SketchObject::reflect`): point shapes reflect
  points; Rect/Ellipse/Stamp conjugate their angle (`x → 2θ − x`,
  exact because the shapes are symmetric in local y); a stamp also
  flips its raster rows and negates the stored fan vectors' y; a
  Group node conjugates its transform to `M ∘ G ∘ M`. Because
  `M ∘ G₁ ∘ G₂ ∘ leaf = (M G₁ M)(M G₂ M)(M leaf)`, the SAME
  reflection applies at every level of a subtree; for a root nested
  under transformed ancestors the world line first conjugates into
  the root's parent space by mapping its two points through
  `parent_abs⁻¹`. Do not re-derive this — it is pinned by
  `mirror_conjugates_through_transformed_ancestors`.
- The array translates ONLY each copy's root (world step converted
  into the kept parent's space); subtrees follow through
  composition. Count is TOTAL including the original (CAD
  convention, stated in the tooltip); step is world cells with the
  physical value on a `theme::derived` line. Zero step refuses with
  a status message (stacked copies read as a silent no-op).
- **Placement: the inspector's selection panels, NOT the ribbon.**
  The Geometry tab has no horizontal room at the 900 px minimum
  (measured: Sketch aids ends ≈890 px), and the app's established
  home for selection ops — Duplicate/Delete/Group/z-order — is the
  inspector, so Mirror H/V, Pick line and Array sit next to those in
  all three selection panels (single/multi/group) via one shared
  `mirror_array_rows`.
- The picked mirror line is a real tool (`Tool::Mirror`, armed from
  the inspector, deliberately NOT in `Tool::ALL` — no bare-key
  shortcut): press–drag–release like Measure, both points
  object-snap (U4 gate extended), Shift angle-snaps, Esc cancels,
  commit-on-release in `finish_gesture`, then the tool disarms back
  to Select with the copies selected. Axis buttons mirror across the
  DOMAIN centerlines (the CFD-useful axes — a selection-centered
  axis would land the copy on top of its source).
- Mirror H/V of a symmetric selection about a line through its own
  geometry can legitimately overlap the original — not a bug; the
  copies remain distinct objects in the tree.

## U5 (share-ready output)

- **One export path, two variants**: `Cmd::ExportPng(path, ExportKind)`
  — `Canvas` saves the GPU readback exactly as before U5
  (bit-identical pixels); `Annotated` appends a sheet BELOW the canvas
  (`ui/export.rs::compose_annotated`), never over it. The sheet:
  burned-in legend with the range's pinned physical value, a 1-2-5
  scale bar (1 px = 1 cell), and the run-conditions block — solver,
  fluid + ρ, Mach·a∞ or Re·ν, grid+margin, cell Δx, domain, elapsed t.
- **The sheet is self-describing**: every value formats through
  `ui/units.rs` in the exporter's ACTIVE system, each value carries
  its unit, and a "Units" line names the system (SI (metric) / ASME
  decimal inch). Two people exporting the same scene print different
  numbers; that is correct and the sheet says which language it
  speaks. Pinned by test alongside the locked-range legend labels.
- Composition is a pure function of an `ExportInfo` struct — GPU-free
  tests cover the strings (both systems, both solvers), locked-range
  labels, canvas-pixel preservation, and the narrow-canvas wrap.
  Range values need no sync step: `Settings.ranges` carries the
  frame-synced physical twins (T2-A), and the export render reuses
  `range_display_gain`, so pixels AND printed range match the screen.
- Text: egui's embedded Hack (the app's monospace face), rasterized
  via `ab_glyph` — already in the tree through epaint; pinned as a
  direct dependency, so U5 adds no new compiled code. Colors from
  `ui/theme.rs`; bars sample the app.rs CPU colormap mirrors.
- **`ab_glyph = "0.2"` in Cargo.toml is an APPROVED exception** to the
  standing ask-before-adding-a-dependency constraint, approved
  retroactively at the final merge gate. The reasoning that earned the
  approval: the crate is already in the dependency tree via epaint,
  the direct pin resolves to the same version, and nothing new
  compiles. Do not read the entry as a violation, and do not treat
  this as precedent for adding crates that bring new compiled code
  without asking first.
- **Quick export**: Ctrl+E (canvas) / Ctrl+Shift+E (annotated) write
  the first free `flowpaint-export-NN[-annotated].png` in the working
  directory — the dialog-free companions to the File-menu items (rfd
  needs a portal/GTK, which headless hosts lack), same `Cmd` path.
- **First-run sample scene: the Pinball preset** (over the RS-25
  insert): it works on the default LBM solver, while a nozzle needs
  chamber spin-up and reads best in compressible mode; Pinball also
  states the mental model (obstacles + tunnel + smoke) in one glance.
  The RS-25 stays discoverable in Study▸Generators. `--bench` is
  unaffected: it replaces the scene at its first frame.
- **First-run tracers ON (100 k) — this deliberately moves U1's
  "tracers are opt-in" default**, stated here per this file's
  contract: verified live that inlet-seeded smoke crosses ~4 % of the
  domain in the first two seconds, so a cold-start Smoke view does
  not read as "flow already moving" — the plan's first-run
  requirement. Particles advect everywhere from frame one and are the
  only instant whole-field motion signal that leaves solver state
  untouched. The Results▸Particles checkbox (U1's off-switch
  machinery, unchanged) turns them off with one click.
- No bench re-run: U5 touches no per-frame path (export-time and
  startup-scene code only; `ui/canvas.rs`, `model.rs`, the rasterizer
  and shaders untouched).

## Queue era (post-v4.1) — protocol amendments

Recorded 2026-08-12 by the session that opened the queue
(`claude/agent-protocol-amendments-x42vvi`), on user instruction.

- **The shader freeze is lifted.** The standing "never edit
  `FlowPaint/src/shaders/`" rule is replaced: shader edits are
  allowed; every shader change is recorded in this file; both solver
  modes (LBM and Euler) are re-run; the paired `--bench` is re-run.
  The CPU colormap stop tables in `app.rs` still mirror `render.wgsl`
  and must stay linked. `scripts/gate.sh` now fails a shader diff
  only when the record or the bench entry is missing, instead of
  failing on any shader diff.
- **Autonomous mode.** Session step 7 (report, stop) is replaced by
  claim-next-and-continue against `docs/queue.md`; only the
  escalation list (`docs/agent-protocol.md` §Escalate and stop)
  stops a session. Claims commit to `main` directly so parallel
  sessions do not collide.
- **`docs/queue.md` created** with the post-v4.1 backlog in
  dependency order (parallel-safe, sequential, exclusive tiers).

## Queue item 1 — ribbon quick-access + Home as scene lifecycle

Branch `claude/queue-1-ribbon-home-x42vvi`. Files: `ui/ribbon.rs`,
`ui/theme.rs` (`ui/menu.rs` deliberately unchanged, below).

- **Quick access lives in the ribbon tab strip**, right-aligned:
  Pause/Resume, Step, Undo, Redo — run control and history reachable
  from every tab, same handlers the old Home groups called. One-line
  `theme::quick_button` (icon beside label; the two-line
  `ribbon_button` does not fit the 27 px strip). Buttons are added
  right-to-left so Pause sits leftmost and the Pause/Resume label
  swap moves only its own left edge; the helper's 58 px minimum
  width absorbs the rest of the jitter.
- **Home is the scene-lifecycle tab**: Scene (New / Open… / Save…),
  Share (View PNG… / Annotated…), Flow (Reset flow). The buttons use
  the same rfd dialogs, `load_scene`/`save_scene`, and
  `Cmd::ExportPng` calls as the File menu — one path, more buttons,
  per the no-second-path invariant.
- **"Clear all" consolidated into "New"**: the two operations were
  identical (`replace_all(vec![])` + `Cmd::ResetFlow`). New keeps the
  destructive coral styling and stays one undoable step. Not a
  feature removal — the operation remains on Home and as File▸New.
- **The menu bar is unchanged.** The File menu keeps the full rare
  set (incl. Quit); ribbon Home is the quick path. Duplication of
  buttons over one code path is the pattern the menu already used
  for Reset flow.
- 900×600 check by width budget: the strip holds 5 tabs (~340 px) +
  4 quick buttons (~260 px); the rebuilt Home body is 3 groups /
  6 buttons, narrower than the old Home. Inch mode adds no strings
  here (no unit-bearing readouts on Home or the strip).
- No bench: only `ui/ribbon.rs` and `ui/theme.rs` changed; no
  perf-sensitive file (`ui/canvas.rs`, `model.rs`, `geomops.rs`,
  `sim.rs`) touched.

## Queue item 2 (gas properties)

- **Gamma is solver state**: `Settings.gamma` (default 1.4) feeds the
  existing `P.gamma` uniform in `euler.wgsl` — the shader was already
  parameterized, so NO shader edit happened. `Cmd::SetGamma` is the
  only writer; presets carry a `gamma` field; the LBM path never
  reads it.
- **Existing presets stay 1.4** — including Water flume and Glycerin,
  where an ideal-gas gamma is physically meaningless anyway. Changing
  them would alter saved Euler scenes' results; don't, without an
  escalated decision.
- **Combustion products preset**: gamma 1.2, a 1620 m/s (~H2/O2-rich
  exhaust at ~3300 K, mean molar mass ~12.5 g/mol), rho 0.05 kg/m^3
  (1 atm), nu 2.2e-3 m^2/s (mu ~1e-4 Pa s over that rho), lattice
  side 0.06/0.02 with 16 sub-steps (the Supersonic precedent).
- **Scene v10 = v9 + gamma appended** (decode-only v9 now; pre-v10
  loads as 1.4 via `SceneV10::from_v9`; load sanity clamp
  1.05..=1.67). Without it a save/reload silently turned combustion
  scenes back into air. Bumps start at v11.
- **Fan drive range is solver-aware** (`sim::fan_mult_range`): LBM
  keeps 0.2..=2.0 — the 0.3-lattice inlet cap binds far below 2x, so
  more range would only lie; Euler gets 0.2..=8.0, which reaches the
  in-kernel Mach-8 inlet clamp from M 1. The Engine readout still
  names the binding layer; the nozzle auto formula keeps its output
  (chamber ~M 0.3) and only its Euler clamp widened.
- **`euler_dt` is now fan-aware**: `GpuSim.max_fan_env` (strongest
  |fan_dir.xy| in the geometry, rescanned in `flush_geometry` on edit
  frames only) sets the u envelope, floored at the legacy 2x and
  capped by the Mach-8 clamp; past the legacy design point (u = 6)
  the acoustic margin grows 1:1 with the jet. Scenes with no fan
  above 2x keep the legacy dt BYTE-IDENTICAL — that is the guard
  against the escalation trigger on solver defaults.
- **Lane extension, recorded**: the queue named sim.rs, generators,
  inspector; the preset table, `Cmd`, snapshot and scene IO live in
  app.rs (unavoidable), and ONE line went into ribbon.rs's preset
  click (`Cmd::SetGamma`) while item 1 holds that file — kept to a
  single line to minimize the merge surface.

## Queue item 6 (object-snap frame-cost bench mode)

- **`--bench-osnap` is a variant, not a change**: same harness
  (Pinball, Euler, tracers pinned 0, 10 warmup + 300 frames), plus
  the Line tool armed, `osnap_enabled` pinned true, and a scripted
  Lissajous pointer sweep in visible-cell coords fed through
  `FlowPaintApp.bench_osnap_cursor`. That field is `None` in every
  normal run and in plain `--bench` — the canvas only reads it after
  `pointer.or(hover_pos())` comes up empty, so the default workload
  and all historical numbers stay like-for-like.
- **No gesture is started**: starting a Line draw would create an
  object and change the raster/solver workload. Consequence: the
  perpendicular snap (anchor-dependent) is not exercised; endpoint,
  intersection, midpoint and center are. Recorded as the mode's known
  measurement boundary.
- The sweep frequencies (0.037/0.023 rad per frame) are deliberately
  incommensurate with the 300-frame window so the cursor never
  settles into a short loop.

## Queue item 7 (unit-system persistence)

- **Std-only prefs file**, `$XDG_CONFIG_HOME|$HOME/.config|%APPDATA%
  /flowpaint/prefs.txt`, `key=value` lines — deliberately NOT eframe's
  `persistence` feature and NOT a `dirs` crate: both compile new
  crates, and a new dependency is an escalation. `mod prefs` in
  app.rs; saves rewrite only their own key so future preferences and
  hand edits survive.
- **Display preference only.** The scene file still never carries the
  unit system (T2-D decision, unchanged). The saved value is applied
  as one `Cmd::SetUnitSystem` on the first frame; every ribbon toggle
  writes the preference back at the `apply_cmd` site.
- **Bench runs ignore the preference** — a host-local inch setting
  would lengthen every formatted readout and quietly change the
  measured text workload; `--bench`/`--bench-osnap` always run the SI
  default.
- Pref spellings `si` / `inch` are stable file format
  (`ui/units.rs::unit_system_pref_str`); unknown values load as None
  and the default stands (round-trip pinned by
  `unit_pref_strings_round_trip_and_reject_unknown`).
