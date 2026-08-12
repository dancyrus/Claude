# Work queue (post-plan-v4.1)

The autonomous-mode backlog, in dependency order. Claiming procedure:
`docs/agent-protocol.md` §Queue and claiming — claims commit to
`main` directly so parallel sessions do not collide. Never claim an
item whose dependencies are not DONE.

Statuses: UNCLAIMED · IN PROGRESS (branch) · DONE.

## Parallel-safe — claim any

1. **Ribbon quick-access + Home rebuilt as scene-lifecycle tab**
   (`ui/ribbon.rs`, `ui/menu.rs`, `ui/theme.rs`) — DONE
   (`claude/queue-1-ribbon-home-x42vvi`)
2. **Gas properties: combustion-products fluid** (gamma ~1.2,
   a0 ~1620 m/s); widen Euler's `0.2..=2.0` fan range; correct the
   stale CUT entry for the fan cap in `docs/deferred.md`
   (`sim.rs`, `ui/generators.rs`, `ui/inspector.rs`) — DONE
   (`claude/agent-protocol-amendments-ohhfmn`)
3. **Periodic boundary conditions** — now unblocked; the variant is
   already reserved as v9 discriminant 4 (`lbm.wgsl`, `euler.wgsl`,
   `sim.rs`) — DONE (`claude/agent-protocol-amendments-xskax5`)
4. **Asymmetric manual min/max on color ranges** — now unblocked;
   needs a per-mode offset in `render.wgsl` (`render.wgsl`,
   `ui/legend.rs`) — IN PROGRESS (`claude/queue-4-range-minmax-x42vvi`)
5. **Plot/legend inversion-factor unification** (`ui/legend.rs`,
   `ui/status.rs`, `ui/units.rs` — the original entry said
   `ui/windows.rs`; the probe-plot copy lived in status.rs) — DONE
   (`claude/agent-protocol-amendments-xskax5`)
6. **Object-snap frame-cost measurement**: a bench mode that arms the
   Line tool and drives a pointer; do NOT change the default workload
   (bench harness, `docs/theme.md`) — DONE
   (`claude/agent-protocol-amendments-ohhfmn`)
7. **Unit-system persistence across sessions**, as a user preference
   not scene state (`ui/units.rs`, `app.rs` prefs) — DONE
   (`claude/agent-protocol-amendments-ohhfmn`)

## Sequential — each depends on the item above it

8. **Arcs and splines as `Shape` variants** — DONE
   (`claude/agent-protocol-amendments-ohhfmn`)
9. **Holes in filled polygons** (removes the U4 interior-erase
   refusal) — depends on 8 — IN PROGRESS
   (`claude/agent-protocol-amendments-ohhfmn`)
10. **Union and intersect booleans** — depends on 9 — UNCLAIMED
11. **DXF/SVG import — REPORT FIRST**: DXF is mostly arcs and
    circles. Say whether you are consuming the new arc primitive or
    flattening to polylines, and what that costs — depends on 10 —
    UNCLAIMED

## Exclusive — only when everything above is DONE and no other session is active

12. **egui 0.29 → 0.35 upgrade** (drags wgpu 22→29 through `sim.rs`;
    the API break list is in `docs/deferred.md`; the go/no-go is on
    the escalation list) — UNCLAIMED
