# U4 eraser — design report (pre-implementation)

Plan v4.1 §U4 requires this report before any eraser code is written:
both designs (vector boolean subtract, stamp mask layer) and a
recommendation on whether stamps need erase support in the first
release. No implementation has been started.

The eraser is a new feature, not a restoration: `146e797` removed the
raster eraser with the raster canvas, and raster subtraction cannot
return — the grid is a projection of the object model and nothing is
ever flattened. Every erase below is a per-object, undoable edit.

## Shared mechanics (both designs)

**Stroke model.** An erase stroke is a swept disc: the drag's input
points define segments, each segment sweeps a capsule, the stroke's
footprint is the union of those capsules. Radius is in cells (the old
brush convention), guarded to a 0.5-cell minimum — the same floor the
rubber-band Rect/Ellipse half-extents use (`ui/canvas.rs:987-996`).

**Commit-on-release, not live subtraction.** During the drag the
canvas draws a translucent preview of the swept footprint; the
geometric edit applies once, on pointer release. Live per-frame
subtraction was considered and rejected: a subtract can split an
object, so live application changes object identity mid-drag (ids
appearing and disappearing under the cursor), and the filled-polygon
hole refusal (below) would flicker between refusing and succeeding as
the stroke evolves. Commit-on-release gives one clean undo entry per
stroke by construction.

**Undo.** The plan names `record_modify_coalesced`; that fits the two
in-place cases exactly — the stamp mask edit and a trim that leaves
one object one object. A split (one object → several) or an erase to
nothing changes the object count, which a coalesced modify cannot
express; those strokes commit as one `ModelOp::Group` entry (the U2
machinery: modify of the surviving fragment + `add_many` for the rest,
or a remove). Either way: exactly one reversible undo entry per
stroke.

**Transform chain.** Erasing happens in world space; stored
coordinates are parent-space. Because U3 groups are similarities
(uniform scale only), a world-space disc maps to an exact disc in any
object's stored space — centre through `m.inverse().apply`, radius
divided by `m.s` — the same conjugation `hit_under` already performs
(`model.rs:512`). All subtraction therefore runs in stored space with
exact circles; no per-object polygonal approximation of the eraser is
needed. This is a direct payoff of the U3 uniform-scale decision.

**Scope.** The eraser is not selection-scoped: it affects every
editable leaf under the stroke. Effectively locked or hidden objects
(`eff_locked`/`eff_hidden`, ancestors included) are skipped, matching
`editable_selection`'s rule for every other mutating operation. Groups
are never erased directly; strokes resolve to leaves. Damage-marking
follows the existing convention: before-bounds and after-bounds of
every touched object.

**Tool plumbing.** Reinstate `Tool::Eraser` and the `X` key as they
were at `61368c8` (tool table entry + keyboard case). That build also
had "right-drag erases with any tool"; whether right-drag is still
free after the U1 gesture rework needs a check against the current
canvas gesture map before reinstating — deferred to implementation,
default is to reinstate only if the button is unclaimed.

## Design A — vector shapes: boolean subtract on the point list

Per-shape behavior, in increasing order of difficulty:

- **Line** — clip the segment against the footprint. Yields 0, 1 or 2
  sub-segments; each survives as a `Line`. Pure 1-D interval math
  (segment–capsule intersection parameters).
- **Open Poly** — interval clipping along the polyline: walk the
  segments, cut where they enter/leave the footprint, emit each
  surviving run as an open `Poly`. Same 1-D machinery as Line.
- **Closed, unfilled Poly** (outline ring) — the ring is a closed
  centerline. One cut opens it into a single open polyline; k cuts
  yield k open polylines. Interval clipping on a circular index.
- **Closed, filled Poly** — a true polygon boolean: polygon minus the
  swept footprint. The footprint is a union of **convex** capsules, so
  no general boolean library is needed: subtract capsule by capsule,
  and each step is polygon-minus-convex-region — a classic, tractable
  clipping problem that stays in simple polygons. Hand-rolled, no new
  dependency (the RDP simplifier precedent). Result: 0..n filled
  closed `Poly`s.
- **Rect / Ellipse** — polygonized to a closed `Poly` on first actual
  intersection (rect: 4 corners; ellipse: sampled at a curvature
  tolerance, like the rasterizer's segment loop at `model.rs:1790`),
  then handled as above. The object permanently loses its parametric
  handles and gains vertex handles — standard vector-editor behavior,
  stated in the tooltip. A stroke that misses leaves the shape
  parametric.
- **Stamp** — Design B, or refusal (see recommendation).

**Thickness.** Subtraction runs on the centerline with the eraser
radius inflated by half the object's thickness, so what disappears
matches the ink on screen rather than an invisible centerline. Cut
ends render with the rasterizer's existing capsule caps.

**The hole case (the one hard refusal).** A stroke wholly interior to
a filled polygon subtracts to a polygon-with-hole, and `Shape::Poly`
has no hole representation. Options considered:

1. **Refuse with a status message** when the committed stroke would
   create an island hole (i.e. the footprint does not reach the
   polygon boundary). The paint bucket's "open to the domain edge"
   refusal is the precedent for a tool declining with a reason.
2. Keyhole slit (connect hole to boundary with a zero-width channel) —
   rejected: coincident edges rasterize unreliably and the handle
   layout becomes nonsense.
3. Add hole support to `Poly` — rejected for U4: scene-format bump
   plus fill-rasterizer churn for a case a user can resolve by
   dragging the stroke out to the edge.

Option 1 is the design. The refusal is per-object: other objects under
the same stroke still erase.

**Degenerate-case guards** (written once, shared with the paint
bucket's contour tracing — the reason fill and eraser are one unit):

- Open fragments: at least 2 points and total length ≥ 1.0 cell, else
  dropped (twice the 0.5-cell radius floor).
- Closed fragments: at least 3 points and area ≥ ~1 cell², else
  dropped (a 0.5-cell-radius disc is ~0.79 cell² — same scale as the
  existing minimum-radii precedent).
- Intersection tolerance sized so near-tangent grazes produce no
  micro-fragments (the sliver dies below the guards instead of
  emitting them).
- Vertex dedup epsilon, winding normalization after each boolean step,
  and an RDP pass (`simplify_stroke`, `ui/canvas.rs:290`) over
  capsule-arc boundaries so sampled arcs come out as light polylines.

**Fragment identity and properties.** All fragments inherit material,
thickness, fan knobs, smoke color and `parent` from the original. The
first fragment keeps the original id (a modify — tree state and
coalescing stay stable); the rest get fresh ids and are inserted at
the original's z-slot, contiguous, so overlap resolution doesn't
shift. Chained-fan polylines keep blowing along their own segments per
fragment.

## Design B — stamps: a mask layer subtracted at rasterization time

A stamp's `GeoRegion` raster is generator output and must stay intact
(never-flatten). Erase is therefore a **mask**, not a raster edit:

- `Shape::Stamp` gains `mask: Option<Vec<u8>>`, one entry per raster
  cell, dims from `raster.rect`. `None` = never erased — zero cost and
  zero behavior change for every existing scene.
- **Scene format bumps to v10** with a decode-only mirror for v9
  objects (the `SketchObjectV5`/`V7` precedent — adding a field inside
  an enum variant moves bincode's positional layout).
- **Rasterizer hook**: one early-`continue` in the stamp arm's sample
  loop (`model.rs:1802`) — a masked source cell emits nothing: no
  wall, no fan, no dye. The legacy-tunnel byte-identical test is
  untouched because legacy scenes carry `mask: None`.
- **Hit test**: the stamp arm at `model.rs:491` also consults the
  mask, so erased areas stop being clickable.
- **Stroke → mask**: conjugate the world capsule through the ancestor
  chain *and* the stamp's own `c`/`angle`/`scale` — all similarities,
  so it is still an exact disc in raster coordinates with radius
  `r / (m.s · scale)` — and paint it into the mask.
- **Undo**: the mask is ordinary object state, so
  `record_modify_coalesced` applies verbatim — this is the case the
  plan's undo line describes with no extension needed. Cost note: each
  snapshot clones the whole `SketchObject` including the raster;
  per-stroke coalescing keeps that to one clone per stroke.
- **Guards**: mask painting clips to the raster rect; a stroke that
  masks every remaining solid cell converts to a delete (no invisible
  ghost objects).
- **Cross-feature touch**: the pre-v8 Engine inspector path scans
  stamp rasters for fan cells; it must skip masked cells or the panel
  misreports an erased chamber. Small, but it is a second subsystem.
- **Resolution caveat for the tooltip**: erase resolution is the stamp
  raster's, not the grid's — a scaled-up stamp erases chunkier.

## Recommendation: ship vector erase; cut stamp erase from the first release

Stamps are the half to cut. Five reasons, in order of weight:

1. **The pairing rationale doesn't cover stamps.** Fill and eraser
   share a unit because contour tracing and boolean subtraction are
   neighboring problems with common degenerate-case guards. Every one
   of those shared guards lives in Design A. Design B shares nothing
   with tracing — it is raster masking. Cutting it costs the pairing
   logic nothing.
2. **It carries all of the format and rasterizer risk.** Stamp erase
   is the only part needing a scene bump (v10 + decode mirror) and the
   only part touching the rasterizer's stamp arm — the code a
   byte-identical regression test stands guard over. Highest risk in
   the unit, attached to the narrowest benefit.
3. **Stamp content is the wrong target for freehand erasing.** Stamps
   are generator output — nozzle bells, preset walls. Users reshape
   that by regenerating with different parameters, transforming, or
   deleting whole; hand-drawn content, which is what people actually
   scrub at, is all vector and gets full support.
4. **The degrade is clean.** A stroke over a stamp refuses with a
   status message (the paint-bucket refusal precedent), and
   whole-object delete still works. No silent no-op.
5. **Deferral is purely additive.** The mask field appends at the next
   natural format bump; the conjugation math, stroke capture, preview,
   and undo grouping all ship with Design A and are reused as-is.
   Nothing gets reworked when stamps arrive.

The honest counterargument: the one real stamp-erase user story is
venting a nozzle bell (cutting a side hole in generated walls). If
that is expected in the first release, Design B earns its cost — but
regenerating the nozzle or overdrawing with vector walls covers it in
the meantime.

Rough scope check: Design A is capsule geometry + interval clipping +
polygon-minus-convex + guards + tool/undo/preview plumbing; Design B
is a format bump, two rasterizer/hit-test touches, mask painting, and
the Engine-scan fix. "Dropping stamps roughly halves the work" holds.

## Bookkeeping when U4 lands

CLAUDE.md is at its line budget; the eraser decisions above go into
`docs/unit-decisions.md` §U4 at landing time, with CLAUDE.md carrying
only the unit-status line and a pointer — and existing detail moves
out of CLAUDE.md before anything is added.
