# DXF/SVG import — report before code (queue item 11)

The queue marks this item REPORT FIRST. The question asked: DXF is
mostly arcs and circles — are we consuming the new arc primitive or
flattening to polylines, and what does that cost?

## Answer: consume the primitives. Flattening is the fallback, not the plan.

| DXF entity | Import target | Fidelity |
|---|---|---|
| LINE | `Shape::Line` | exact |
| CIRCLE | `Shape::Ellipse { r: [r, r] }` | exact, live-editable |
| ARC | `Shape::Arc { c, r, start, sweep }` (item 8) | exact |
| ELLIPSE | `Shape::Ellipse` (full) / sampled `Spline` (partial) | exact / approx |
| LWPOLYLINE, no bulges | `Shape::Poly` | exact |
| LWPOLYLINE with bulges | straight runs as `Poly` + one `Arc` per bulged segment, under one U3 Group | exact |
| SPLINE (NURBS) | `Shape::Spline` through fit-tolerance samples | approximate |
| HATCH boundaries | closed `Poly` / `Rings` (item 9) | exact for line/arc loops |

- **Arcs and circles — the bulk of real DXF — import exactly**, stay
  parametric, keep their centre/endpoint/mid object snaps, and edit
  with the item 8 three-handle re-fit. This is what item 8 was
  sequenced for.
- **Bulged polylines** are the one structural compromise: `Poly`
  cannot hold arc segments, so a bulged polyline splits into a Group
  of Poly runs + Arcs. Geometry is exact; the cost is object count in
  the tree (a door swing with two bulges becomes three objects in one
  group).
- **NURBS splines** are not Catmull-Rom; `Shape::Spline` interpolates
  its stored points, so SPLINE entities import as a fit-tolerance
  sampling (default: max deviation 0.25 cell at import scale) with
  the samples as the stored, editable points.

## What flattening everything would have cost

- Resolution-dependence: a polyline chord error is baked at import
  scale and survives grid rescales; a parametric arc re-rasterizes
  exactly at every resolution.
- File size and tree noise: a circle becomes 24–96 vertices instead
  of one object.
- Editing: no radius/bulge handles, no true centre snap — the exact
  losses the eraser's rect→polygon conversion accepts only AFTER a
  destructive cut, here imposed at import time on pristine geometry.

## The escalation: SVG needs a dependency, DXF does not

- **DXF**: ASCII DXF (the ENTITIES subset above, R12→2018) is a flat
  group-code/value line format. A minimal std-only reader (~300
  lines, the prefs-file precedent) covers it — no new crate. I intend
  to build this as soon as the report is answered.
- **SVG**: XML + the path mini-grammar + nested transforms + CSS-ish
  units. Hand-rolling an XML parser is not honest engineering, and
  every real option (`usvg`, `quick-xml`, `roxmltree`) is a NEW
  DEPENDENCY — which is on the escalation list, full stop.

## Decision requested

1. **DXF-first, std-only** (no dependency): proceed on answer, SVG
   deferred with its own entry — RECOMMENDED.
2. Approve `roxmltree` (small, no transitive deps) and ship both.
3. Flatten-everything import (no item 8/9 integration) — not
   recommended; costs above.

Until answered, item 11 stays IN PROGRESS (report delivered), and no
import code lands.
