# Agent protocol

How a session works on FlowPaint. Read this before touching anything.
A prompt should not need to repeat what is written here; if a prompt
contradicts this file, the prompt wins and you say so in your report.

## Session shape

1. Branch off `main`. Never off another unit's branch, never off a
   stale tip. `git fetch` first.
2. Read `CLAUDE.md`, `docs/invariants.md`, and only the section of the
   plan covering your unit. Do not read the whole plan; that waste is
   the single largest recorded cost on this project.
3. Implement. Stay inside the files your unit owns (below).
4. Run `scripts/gate.sh origin/main`. Paste its output in your report.
5. Run the invariant check (below) before merging.
6. Merge to `main` yourself with `--ff-only` and push. Do not open a
   PR and wait. If the fast-forward fails, stop and report.
7. Report, then claim the next claimable item in `docs/queue.md`
   (§Queue and claiming) and continue. Stop only for the escalation
   list (§Escalate and stop).

## Standing constraints

- **The shader freeze is LIFTED** (2026-08-12 amendment). Shader
  edits are allowed. Record every shader change in
  `docs/unit-decisions.md`, re-run both solver modes (LBM and
  Euler), and re-run the paired `--bench`. The CPU colormap stop
  tables in `app.rs` still mirror `render.wgsl` and must stay
  linked.
- **Ask before adding a dependency.** One retroactive exception exists
  (`ab_glyph`, already in-tree via epaint); also not precedent.
- **egui stays at 0.29.1** until the exclusive queue item runs; the
  upgrade needs an explicit go/no-go (escalation list). The API
  break list is in `docs/deferred.md`.
- **Nothing is ever flattened.** Every drawn thing stays a live,
  selectable, editable `SketchObject`. Any destructive-looking feature
  is a per-object, undoable operation.
- **The full control set stays reachable at 900x600.** There is no
  spare ribbon width. Two units already had to move controls out of
  the ribbon (T2-A to the legend, mirror/array to the inspector).
  Check inch mode too: longer strings have nearly clipped twice.
- **All visual constants resolve through `ui/theme.rs`.** No ad-hoc
  colors, rounding, spacing, or font sizes.
- **Physical values format through `ui/units.rs`.** No inline `{:.N}`
  on a physical quantity, ever. Canonical value in the box, unit in
  the label, derived value on a secondary line.
- **Menus keep only rare operations.**
- **ASD-STE100 Issue 7** for every tooltip, help string, and status
  message.
- **`CLAUDE.md` stays under 150 lines.** Route detail to
  `docs/unit-decisions.md`. It is a routing file, not a record.

## File lanes

Your prompt names the files your unit owns. Everything else is
forbidden for the duration. This is why twelve branches merged with
almost no code conflicts; the exceptions were always a lane violation.

If you need something outside your lane, do not reach for it. Report
what you need and stop, or design around it and record the workaround
in `docs/unit-decisions.md` as debt with an explicit owner.

Process-wide statics are not a legal workaround for a frozen file.
Three units used one, and all three had to be undone at a merge. The
pattern is closed.

## Frame-time bench

Required at the end of any unit touching `ui/canvas.rs`, `model.rs`,
`geomops.rs`, or `sim.rs`. Not required otherwise — say so explicitly
in your report rather than skipping silently.

Procedure, and every clause is here because it was learned the hard
way:

- `FlowPaint-V2 --bench`: Pinball preset, compressible, default grid,
  10-frame warmup, 300 measured frames.
- Paired A/B, **both orders**, in one session. A single run against a
  recorded baseline proves nothing; this host's allocation drifts up
  to 57% on an unchanged commit.
- **Build both release binaries before any measurement starts.** A run
  taken while a build is running came out ~90% high once.
- The harness pins scene, solver, **and particles**. Do not unpin any
  of them. U5's first-run 100k tracers leaked in once and showed a
  fake +7.2%, because every historical number predates that default.
- Absolute numbers are lavapipe software rendering. Relative only, and
  only within one session on one host.
- Record all four runs in `docs/theme.md` in the existing format.
- If the deltas flip sign with run order, that is this host's known
  session-position pattern, not a regression. Say which it is and why.

## Gates: stop and ask, or decide

**Stop and ask** only for the escalation list in §Escalate and stop.

**Decide and record** everything else in `docs/unit-decisions.md`. A
report-and-stop at a unit boundary with no open question is pure
cost.

When the plan and the code disagree, **the code wins and the plan is
wrong**. Say so, fix the plan text, and record it. This has happened
four times: a scene version already in use, stale line references, a
preset that painted nothing it claimed to paint, and a gate that had
already been opened.

## Queue and claiming

The backlog lives in `docs/queue.md`, in dependency order, each item
UNCLAIMED, IN PROGRESS (branch), or DONE. Parallel sessions must not
collide, so claims go through `main`:

1. `git fetch`; then on `main`, set your chosen item to IN PROGRESS
   with your branch name; commit; push to `main` directly.
2. If the push is rejected, someone claimed it first — pull, pick
   the next claimable item, retry.
3. Do the work on your branch. Gate, self-merge, mark the item DONE,
   push.
4. Repeat until nothing is claimable.

Never claim an item whose dependencies are not DONE. The exclusive
item runs only when everything above it is DONE and no other session
is active.

## Escalate and stop

Stop and ask the user only for these. Everything else: decide,
record it in `docs/unit-decisions.md`, and continue.

- a new dependency
- deleting or disabling a user-facing feature
- a change to default physical values or solver defaults that alters
  existing scenes' results
- the egui upgrade go/no-go
- the gate script failing for a reason you cannot fix without
  violating a constraint

## Documents you are responsible for

- `docs/queue.md` — the backlog and claim state; update on claim and
  on completion, on `main` directly.
- `docs/unit-decisions.md` — every decision a later unit must not
  re-derive, and every piece of debt with an owner.
- `docs/deferred.md` — anything you cut or defer, with why and what
  unblocks it. Check it before proposing a feature; something may
  already be cut with reasons.
- `docs/theme.md` — bench records.
- `CLAUDE.md` — unit status and routing only.
