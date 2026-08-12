# Invariants — expected final state

What FlowPaint must be true of, regardless of which unit runs next.
A checker agent verifies the build against this file; a unit session
runs that check before merging.

## How to use this file

**It is bidirectional.** If the code contradicts an assertion here,
that does not automatically mean the code is wrong. Report which one
is wrong, with evidence, and fix that one. A specification written
before the work is a guess, and guesses in this project have been
wrong four times — a scene version that already existed, stale line
references, a preset documented to paint edges it never painted, and
a shader gate recorded as closed after it had been opened.

An assertion that cannot be checked is worse than no assertion. Every
line below is either pinned by a named test, or checkable by running
the app and looking.

**Verdicts:** HOLDS, VIOLATED (code is wrong), or STALE (this file is
wrong). Never silently pick one.

## Pinned by test

Run `cargo test --release`. These encode decisions no later unit may
undo without an explicit decision recorded in
`docs/unit-decisions.md`.

| Invariant | Test |
|---|---|
| Legacy `wind_tunnel` scenes simulate byte-identically | `legacy_wind_tunnel_projection_is_unchanged` |
| A wall edge disables the sponge; inlet/outlet keep it | `sponge_disabled_only_by_walls` |
| Scenes v3 through v9 load; v9 round-trips whole | `v5_bytes_decode_and_convert`, `v6_*`, `v7_*`, `v8_*`, `v9_roundtrip_persists_groups_probes_ranges_edges` |
| Unknown/reserved edge kinds decode safely | `reserved_and_unknown_edge_kinds_decode_safely` |
| Periodic wrap engages only as an opposite pair, per axis | `wrap_bits_require_opposite_pairs` |
| A periodic pulse crosses the seam in both solvers (GPU-executing) | `periodic_wrap_crosses_the_seam_in_both_solvers` |
| A group can never become its own descendant | `reparent_refuses_cycles`, `sanitize_parents_breaks_cycles_and_dangles` |
| Transform composition is child first, then each ancestor outward | `composition_is_child_then_ancestors_outward` |
| Rubber-band selection means intersect, not fully-contain | `rubber_band_intersect_semantics` |
| Locked and hidden objects are not click-selectable or rasterized | `hit_test_skips_locked_and_hidden`, `group_flags_apply_to_subtree` |
| One user action is one undo entry, across a whole selection | `add_many_*`, `remove_many_*`, `modify_many_*`, `group_is_one_undo_entry`, `array_steps_in_world_space_one_undo_entry`, `apply_erase_split_is_one_undo_entry_*` |
| A mirrored group is a deep copy, never an instance | `mirrored_group_is_deep_and_disentangled` |
| An erase wholly interior to a filled shape refuses | `interior_stroke_refuses_with_hole` |
| A flood fill open to the domain edge refuses | `flood_open_region_refuses` |
| Inch mode round-trips to SI without drift | `inch_mode_formats_and_si_round_trip` |
| An exported PNG carries units and names its system | `run_conditions_carry_units_and_name_the_system`, `legend_prints_locked_range_value` |

If a change makes one of these fail, the change is wrong until a
recorded decision says otherwise.

## Checkable only by running the app

No test covers these. A checker agent must launch the build and look.

1. **900x600.** Every ribbon tab renders with nothing clipped, in
   **both** unit systems. Inch strings are longer; this has nearly
   broken twice.
2. **Zoom changes no readout.** Zoom and pan are a view transform on
   `px_per_cell` and `lb_origin` only. Grid, margin, `domain_width_m`,
   cell size, dt, u-infinity, CFL: all unchanged at any zoom. Any
   coupling is a bug.
3. **The scale bar is physically correct** at three zoom levels, in
   both unit systems.
4. **Hit-testing and handles survive extreme zoom**, in and out. Snap
   thresholds are screen-space, not cell-space.
5. **First run shows moving flow** within a second or two of launch,
   not an empty canvas.
6. **A locked color range survives** a save, a reload, and a solver
   switch, and the pinned physical value appears in an exported PNG
   matching the screen.
7. **Both solvers run**: LBM incompressible and Euler compressible,
   with the scene switching between them intact.
8. **Refusals are specific.** The eraser refuses in two unrelated
   cases (over a stamp, wholly interior to a filled shape). Each says
   something different and true.

## Structural

- No file under `FlowPaint/src/shaders/` differs from `main` without a
  recorded entry in `docs/unit-decisions.md` (the freeze is lifted;
  the record, both-solver re-run, and paired bench are still owed).
- No process-wide static holds application state. `Settings` is the
  store of record; edits go through `Cmd`. One documented frame-scoped
  mirror exists (`INCH_MODE`) and is explicitly not a store.
- No inline formatting of a physical quantity outside `ui/units.rs`.
- No second path for an operation that already has one. Export extends
  `Cmd::ExportPng`; it does not add a sibling.
- `CLAUDE.md` under 150 lines.
- Every claim in `docs/deferred.md` still true of `main` — gates open
  over time, and a stale "blocked" entry is how work gets skipped that
  is no longer blocked.

## Known gaps, deliberately

Not violations. Do not "fix" them without a decision.

- Object-snap per-frame cost is measured only at Pinball scale:
  `--bench-osnap` (queue item 6) arms the Line tool and drives a
  scripted pointer; the measured cost there is below the host noise
  floor (docs/theme.md). The DEFAULT bench still drives no pointer
  and stays on Select — historical numbers stay like-for-like — and
  the perpendicular snap (anchor-dependent) is outside the mode's
  reach.
- Absolute bench numbers are not comparable across hosts or across
  container rebuilds. Only paired same-session A/B means anything.
- Holes in filled polygons are unrepresentable, which is why the
  interior-erase refusal exists.
