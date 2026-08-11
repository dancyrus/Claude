# Deferred and cut items — the single index

One entry per feature FlowPaint deliberately does not have. **CUT**
means not coming back; **DEFERRED** means later — plan v4.1 draws that
distinction on purpose and this file preserves it. Each entry carries a
pointer to the document holding the full reasoning; those documents
stay authoritative. This is an index, not a migration — nothing was
moved out of the source docs.

Every entry below was verified against main on 2026-08-11 (the code
anchors named are symbol names, not line numbers, so they drift less).
Items whose gates later opened are NOT listed — see "Not on this list"
at the bottom before adding anything back.

---

## Cut

### Dimensional constraint solver
- Parametric sketch constraints (SolidWorks-style driving dimensions).
- **CUT.**
- Why: months of work, and it is what makes CAD sketchers slow and
  fussy. FlowPaint sketches flow scenes, not parametric parts.
- Unblocks: nothing — a deliberate scope boundary.
- Reasoning: `docs/flowpaint-plan-v4.1.md` §"Cut, with reasons".

### Align and distribute
- Slide-layout-style even-spacing and alignment commands.
- **CUT.**
- Why: reads as a presentation tool. FlowPaint objects are physical
  geometry positioned by dimension, not by even spacing.
- Unblocks: nothing.
- Reasoning: `docs/flowpaint-plan-v4.1.md` §"Cut, with reasons".

### Construction geometry
- Non-rasterizing reference lines and points.
- **CUT.**
- Why: only pays off alongside a measure tool and a constraint solver;
  the solver is cut. (The measure tool alone did ship, in U4 — that
  does not reopen this.)
- Unblocks: nothing while the constraint solver stays cut.
- Reasoning: `docs/flowpaint-plan-v4.1.md` §"Cut, with reasons".

### View menu
- Re-adding a View menu to the menu bar.
- **CUT.**
- Why: commit `1e76f5a` deleted it deliberately once emptied; plan v2
  proposed re-adding it and plan v3 explicitly cut that. Menus keep
  only rare operations; nothing in the overhaul needs a View menu.
  Verified: `ui/menu.rs` has File / Edit / Simulation / Help only.
- Unblocks: nothing.
- Reasoning: `docs/flowpaint-ui-overhaul-plan-v3.md` §Phase 3;
  history in `docs/ui-inventory.md` item 6.

### Stamp erase support
- Erasing generator output (nozzle/airfoil stamps) with the eraser.
- **CUT** — the source's exact language is "cut stamp erase from the
  first release", approved. A stroke over a stamp refuses with a
  specific status message ("stamps can't be erased", `app.rs`); the
  sanctioned workaround for bell venting is overdrawing with vector
  walls or regenerating the nozzle, stated in the tooltip.
- Why: it carried all of the format risk (scene v10 + decode mirror)
  and rasterizer risk for the narrowest benefit; stamps are reshaped
  by regenerating, not scrubbing. Verified: `Shape::Stamp` has no
  `mask` field on main.
- Unblocks: a decision that bell venting matters enough in a release.
  The full mask-layer design (Design B) is already written and the
  design report records that reviving it is purely additive — the mask
  field appends at the next natural format bump.
- Reasoning: `docs/u4-eraser-design.md` (whole document);
  `docs/unit-decisions.md` §U4.

### Right-drag-erase with any tool
- The pre-rebuild (`61368c8`) gesture: right-drag erases regardless of
  the active tool.
- **CUT.**
- Why: verified against the U1/U2 gesture map at U4 time — the
  secondary button is claimed: it finishes polylines and clears
  selections (`ui/canvas.rs`, Secondary press handling). The X key and
  `Tool::Eraser` did return; only the gesture is gone.
- Unblocks: nothing planned — it would need the secondary button
  reassigned.
- Reasoning: `docs/unit-decisions.md` §U4;
  `docs/u4-eraser-design.md` §"Tool plumbing".

### Editable chamber-fan speed cap
- A user-editable field for the nozzle chamber jet's speed limit.
- **CUT.**
- Why: the binding limits are shader constants — LBM
  `MAX_LATTICE_SPEED = 0.3` (`lbm.wgsl`) binds almost always, Euler's
  Mach-8 sanity clamp effectively never — and shaders are frozen. The
  shipped form is a readout naming which layer binds
  (`ui/inspector.rs`, Engine group), not a field.
- Unblocks: in principle the shader freeze lifting, but no intent to
  revisit is recorded anywhere — the cap-is-a-readout design stands on
  its own.
- Reasoning: `docs/flowpaint-ui-overhaul-plan-v3.md` §Phase 5 (the
  six-clamp table); `docs/ui-inventory.md` item 3;
  `docs/unit-decisions.md` §U3 (clamp layers).

### Non-uniform stamp scaling
- Scaling a raster stamp by different factors per axis.
- **CUT.**
- Why: structural. Group transforms are similarities (uniform scale
  only) — the only family closed under composition that the shape set
  can represent; non-uniform world scaling of a rotated nest is shear,
  which Rect/Ellipse/Stamp cannot store. `Shape::Stamp` keeps a single
  `scale: f32` (the field's own comment and the inspector tooltip say
  so). Verified on main.
- Unblocks: a shear-capable shape representation, which nothing plans.
- Reasoning: `docs/unit-decisions.md` §U3;
  `docs/flowpaint-plan-v4.1.md` §U3.

---

## Deferred

### Arcs and splines
- New `Shape` variants for circular arcs and splines.
- **DEFERRED.**
- Why: each needs a new rasterizer path in `model.rs`, and every
  transform, snap, and boolean op has to learn about them. Sequenced
  after the geometry operations were stable.
- Unblocks: the stability condition is met (U1–U4 landed), so this is
  now purely a scheduling question. Still absent on main (the `Shape`
  enum has no arc/spline variants).
- Reasoning: `docs/flowpaint-plan-v4.1.md` §"Deferred, on purpose".

### Offset curve
- Parallel copy of a curve at a distance — the fast way to build duct
  and channel walls.
- **DEFERRED.**
- Why: the natural companion to arcs; deferred with them.
- Unblocks: arcs and splines landing first.
- Reasoning: `docs/flowpaint-plan-v4.1.md` §"Deferred, on purpose".

### Union and intersect booleans
- The other two boolean ops beside the eraser's subtract.
- **DEFERRED.**
- Why: subtract shipped with the U4 eraser; union and intersect follow
  once its degenerate-case handling has seen real use. Verified:
  `geomops.rs` exposes `subtract_polygon` and the clip/trace helpers,
  no union/intersect.
- Unblocks: field experience with the shipped subtract (the shared
  guards in `geomops.rs` are the foundation they would reuse).
- Reasoning: `docs/flowpaint-plan-v4.1.md` §"Deferred, on purpose";
  guard details in `docs/unit-decisions.md` §U4.

### Holes in filled polygons
- `Shape::Poly` with interior rings — annuli, ducts with a bore,
  plates with a port.
- **DEFERRED.**
- Why: a hole representation touches the scene format, the fill
  rasterizer, hit tests, handles, and every boolean. Two shipped
  behaviors are consequences of its absence, both verified on main:
  the eraser refuses a stroke wholly interior to a filled polygon
  (`PolySubtract::WouldHole`, with its own status message), and the
  paint bucket covers fluid sealed inside a hollow island. Until it
  lands, such parts are built from outline rings or two overlapping
  shapes.
- Unblocks: an interior-ring representation in `Shape::Poly`, plus the
  format bump and the rasterizer/hit-test/boolean support that comes
  with it.
- Reasoning: `docs/flowpaint-plan-v4.1.md` §"Deferred, on purpose";
  `docs/u4-eraser-design.md` §"The hole case";
  `docs/unit-decisions.md` §U4 (bucket caveat).

### DXF or SVG import
- Importing geometry drawn elsewhere (SolidWorks exports, vector art).
- **DEFERRED — the plan marks it "Deferred, not cut" explicitly.**
- Why: called the highest-value item on the whole list, but it wants a
  stable object model under it, so it was sequenced after U3.
- Unblocks: already unblocked (U3 landed) — awaiting scheduling. Still
  absent on main.
- Reasoning: `docs/flowpaint-plan-v4.1.md` §"Deferred, on purpose".

### egui 0.29 → 0.35 upgrade
- Moving off the pinned `egui`/`egui-wgpu` 0.29.1.
- **DEFERRED.**
- Why: `egui-wgpu` drags `wgpu` 22 → 29 in lockstep, through every
  compute pipeline, bind group, and pass lifetime in `sim.rs`, for a
  payoff of font hinting. Verified: `Cargo.toml` still pins 0.29.
- Unblocks: the UI overhaul shipping, then a deliberate upgrade pass.
  The known API breaks are recorded so the analysis isn't redone
  (menu bar/`close_menu`, `rect_stroke` `StrokeKind`,
  `Rounding`→`CornerRadius`, `Frame::none()`); the 0.34
  panel/`App::update` deprecation flags are low-confidence and must be
  verified against release notes first.
- Reasoning: `docs/flowpaint-ui-overhaul-plan-v3.md` §"Deferred: the
  egui upgrade"; API exposure detail in `docs/ui-inventory.md` item 5;
  summary in `CLAUDE.md` §Hard rules.

### Periodic boundary condition
- A fifth per-edge boundary kind: periodic wraparound.
- **DEFERRED** (T2-C's word is "reserved").
- Why: it needs wraparound in the LBM streaming (`lbm.wgsl`) and the
  Euler stencil clamping (`euler.wgsl`) — a change in both kernels
  that the shader freeze blocks. Verified on main: `EdgeKind::Periodic`
  exists, scene v9 reserves discriminant 4 for it, the edges dialog
  greys it out with a hover explaining the freeze, and the sim treats
  it as far field until it lands.
- Unblocks: the shader freeze lifting. No further scene-format bump is
  needed — the discriminant is already reserved.
- Reasoning: `docs/unit-decisions.md` §T2-C.

### Asymmetric manual min/max on color ranges
- Typing both ends of a color scale, instead of one saturation value
  (Speed's min pinned at 0, Vorticity/Pressure symmetric about 0).
- **DEFERRED.**
- Why: the shipped Manual control is the shader-expressible form. An
  asymmetric range needs a per-mode offset in the `render.wgsl`
  normalization (two uniforms, or a min/max pair replacing
  `display_gain`), and only the flags-bit colormap edit was approved
  under the shader freeze. Verified: `FieldRange` on main holds a
  single saturation value per mode.
- Unblocks: an approved `render.wgsl` edit adding the per-mode offset
  (i.e. the shader freeze lifting for this one uniform).
- Reasoning: `docs/t2a-color-range.md` §"Report: the remainder";
  `docs/unit-decisions.md` §T2-A.

### Unit-system persistence across sessions
- Remembering the SI / decimal-inch choice between app runs.
- **DEFERRED.** (Distinct from scene persistence, which was decided
  AGAINST, permanently: a scene file is shared work product, and
  loading a colleague's file must not flip your units.)
- Why: the toggle is a per-user display preference; session-scoped was
  enough to ship T2-D. Verified: nothing writes it to eframe storage
  on main.
- Unblocks: any later UI-preferences work — eframe-storage persistence
  rides along whenever that happens.
- Reasoning: `docs/unit-decisions.md` §"Third integration".

### Plot/legend inversion-factor unification
- One home in `ui/units.rs` for the factors that invert the shader's
  field normalization, instead of two copies.
- **DEFERRED** (internal debt, not a user-visible feature).
- Why: T2-A's legend and T2-B's probe plot each grew their own copy
  while the tracks ran concurrently. Verified still duplicated on
  main: `ui/legend.rs` (`phys_factor`-style match) and `ui/status.rs`
  (probe sample conversion) both carry the per-mode factors.
- Unblocks: nothing — a cleanup awaiting someone touching that code.
  The last track-era static debt was paid at the third integration;
  this is the one remaining open item from that era.
- Reasoning: `docs/unit-decisions.md` §T2-B; `CLAUDE.md` unit status.

### Object-snap frame-cost measurement
- A `--bench` number for what `compute_osnap` costs per frame.
- **DEFERRED** (a measurement gap, recorded so it doesn't read as "the
  snaps were measured free").
- Why: the bench harness drives no pointer and stays on the Select
  tool, so the snap path never executes in a bench run.
- Unblocks: a bench mode that arms a draw tool and moves a pointer
  over a dense scene.
- Reasoning: `docs/unit-decisions.md` §U4 (known measurement gap);
  `docs/theme.md` (frame-time history note).

---

## Not on this list — gates that later opened

Verified shipped on main; do not re-add them here:

- **Colormap picker (T2-A).** Was shader-gated in
  `docs/t2a-color-range.md`, then approved and shipped via the
  `render.wgsl` flags-bit-1 edit (no uniform added or reordered). The
  one approved exception to the shader freeze.
- **Measure tool.** Plan v4.1 listed it as "fold in … if it fits"; it
  fit, and shipped in U4 (key M).
- **Mirror and linear array.** Deferred out of U4 on purpose ("slot it
  wherever there is room rather than blocking U4"); the slot arrived
  after U4 and it shipped via `claude/mirror-linear-array-vlmsub` —
  independent deep copies, inspector placement, `Reflect2` baking
  (`docs/unit-decisions.md` §"Mirror & linear array").
- **Scene persistence of color ranges.** Track-era T2-A deferred it to
  the merge; it landed in scene v7.
- **The three track-era statics** (T2-A ranges, T2-B probes, T2-D unit
  system) — all folded into `Settings` + `Cmd`; that debt is fully
  paid (see `docs/unit-decisions.md` §"Third integration").

Out-of-scope *defects* are tracked in `docs/punchlist.md`, not here —
this file indexes deliberately cut or deferred features and work.
