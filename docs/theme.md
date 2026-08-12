# Theme — src/ui/theme.rs

Phase 2 of `docs/flowpaint-ui-overhaul-plan-v3.md`. Every visual
constant resolves through `src/ui/theme.rs`; the values are ported from
the `:root` block of `docs/ui-target.html`. Change the mockup first,
then mirror here.

## Mapping (mockup variable → theme const → egui use)

| mockup | theme | applied to |
|---|---|---|
| `--chrome` #22262c | `CHROME` | (phase 3: menu/title strips) |
| `--panel` #292e35 | `PANEL` | `panel_fill`, `window_fill`, noninteractive bg |
| `--panel-2` #2f353d | `PANEL_2` | `faint_bg_color`, inactive/hovered bg |
| `--view` #12151a | `VIEW_BG` | (canvas clear stays in render.wgsl), handle outline |
| `.dv` bg #1c2025 | `FIELD_BG` | `extreme_bg_color` (value boxes **while editing**; at rest egui draws DragValues as buttons on `PANEL_2` with no border — the mockup's `.dv` idle look needs per-widget frames, deferred to phase 3), `code_bg_color` |
| `--line` #3a4149 | `LINE` | `window_stroke`, separators, noninteractive stroke |
| `--line-2` #464e58 | `LINE_2` | hovered `bg_stroke` |
| `--ink` #dfe4ea | `INK` | hovered text, handle fill |
| `--ink-2` #9aa4b0 | `INK_2` | default text (`fg_stroke` idle) |
| `--ink-3` #6d7783 | `INK_3` | (phase 3: captions/units) |
| `--sel` #3fb8ae | `SEL` | selection stroke, hyperlink, canvas selection accent |
| `--sel-bg` #1e3f3e | `SEL_BG` | `selection.bg_fill`, active widget fill |
| `.bt.on` #cfeeeb | `SEL_INK` | active widget text |
| `--bad` #cf6f62 | `BAD` | destructive tint ("Clear all"), `error_fg_color` |
| `--warn` #d1a24a | `WARN` | `warn_fg_color` (readouts only, not interactive) |
| `--r` 3px | `RADIUS` | every widget rounding, windows, menus |

Panel dimensions for phase 3 live in `theme::dim` (tree 212, settings
258, ribbon 86/27, menu 26, message 22, status 24 — all from the mockup
markup).

## Type scale

Body/Button 12, Small 10, Heading 14, Monospace 12.
`style.drag_value_text_style = Monospace` puts every DragValue and
slider value box in tabular monospace digits. The status-bar stats
line, legend value rows, colorbar min/max labels, and the on-canvas
dimension readout are monospace at call sites. Fonts are egui's
bundled families (the mockup's Inter/JetBrains Mono are not shipped;
bundling real font files is a dependency decision the plan doesn't
authorize). `egui-phosphor` (Regular variant only) is registered in the
font definitions for phase 3's ribbon icons; no icons are placed yet.

## Spacing

`item_spacing (6,5)`, `button_padding (9,3)`, `indent 12`,
`slider_width 120`, `interact_size.y 18`, window/menu margins 8/6 —
set once in `theme::apply`. All 21 ad-hoc `add_space` calls were
deleted; all 14 pre-existing separators divide real sections or menu
groups and stay.

## Helpers

Persistent selection can't be expressed through `Visuals` alone (egui
takes selected-label text from `selection.stroke` and drops the border),
so `theme::toggle` draws the mockup's `.bt.on` treatment (`SEL_BG` fill,
1 px `SEL` border, `SEL_INK` text) and all selection call sites use it.
`theme::heading` lifts section headers to primary `INK` (default text is
`INK_2`), and `theme::mono_small` is the small tabular readout used by
the legend colorbar labels and generator dialogs.

## Left alone, deliberately

- Inferno/coolwarm stop tables in `app.rs` — CPU mirrors of
  `shaders/render.wgsl`; re-theming them alone desynchronizes the
  legend from the field rendering.
- `def_smoke` in `app.rs` — default scene content.
- `smoke_rgb` ↔ `Color32` conversion in `ui/inspector.rs` — a
  conversion site, not a constant.

## Frame-time baseline (pre-theme, plan working rules)

Harness: `FlowPaint-V2 --bench` (commit 1f00ef2) — Pinball preset,
compressible mode, default grid, 10-frame warmup, 300 measured frames.
Environment: this repo's CI-less dev container, Xvfb + Mesa **lavapipe
(software Vulkan)** — numbers are CPU-rendering times, meaningful only
relative to a rerun in the same environment, not to any real GPU.

Baseline (pre-theme, commit 1f00ef2, recorded before phase 2):

```
bench: 300 frames  mean 1885.08 ms  p99 2394.53 ms  min 1654.87 ms  max 6344.08 ms
```

## Post-phase-3 measurement (recorded after 3b)

The first post-phase-3 run measured mean 3072.27 / p99 10190.93 — but
re-running the UNCHANGED baseline commit the same hour measured mean
2968.61 / p99 5057.22, +57% over its own recorded baseline: the
container's CPU allocation had drifted massively (full release builds
of identical scope swung 27 s – 4 m 30 s across the session). The
original baseline is therefore not reproducible on this host, and the
valid comparison is the paired back-to-back A/B, run in both orders:

```
A = baseline commit 1f00ef2, B = phase 3b        mean       p99
pair 1 (A first): A 2968.61 / 5057.22   B 3043.18 / 7932.90
pair 2 (B first): B 3030.26 / 8376.75   A 2932.78 / 3963.24
```

Order-independent result: B costs ~+3% mean (~+100 ms/frame) and ~2×
p99 **under Mesa lavapipe, where every UI pixel is rasterized on the
same CPU cores as the solver**. The solver work is bit-identical
(`git diff 1f00ef2..HEAD -- FlowPaint/src/sim.rs FlowPaint/src/shaders`
shows only the `cfl_estimate` readout helper; zero shader diffs), so
the delta is the ribbon/tree/settings chrome being software-rendered.
On real GPU hardware egui's few hundred triangles are sub-millisecond;
re-run `FlowPaint-V2 --bench` on target hardware with the CI artifacts
of 1f00ef2 vs phase 3b for a hardware-true verdict.

## Post-U1 measurement (plan v4.1, free zoom/pan + control cleanup)

Paired back-to-back A/B on a fresh container (new lavapipe/Mesa install,
so absolute numbers are not comparable to the tables above):

```
A = pre-U1 856cca4   bench: 300 frames  mean 1045.28 ms  p99 1253.02 ms
B = U1 merged        bench: 300 frames  mean 1055.65 ms  p99 1230.71 ms
```

Mean +1.0 %, p99 −1.8 % — inside this host's run-to-run noise. Expected:
the bench never zooms, and fit mode is algebraically identical to the
old letterbox mapping; the only additions on the hot path are the scale
bar (a few egui shapes) and one status segment.

## Post-track-merge measurement (U2 + T2-A + T2-B integrated)

Paired back-to-back A/B in one session (same container/session as no
other load; lavapipe, so relative comparison only), run in both orders:

```
A = U2 tip e6071d8, B = merged (T2-A + T2-B + fold/v7/900px)   mean       p99
pair 1 (A first): A 1059.41 / 1228.81   B 1058.69 / 1225.03
pair 2 (B first): B 1060.85 / 1207.71   A 1075.35 / 1224.56
```

Order-independent result: mean −0.1 % / +0.1 %, p99 −0.3 % / −1.4 % —
no regression; the merged build sits inside run-to-run noise in both
orders. Expected: the bench draws the Home tab and the Dye view, so
T2-A's range sync (a 3-entry loop of scalar math per frame) and T2-B's
probe sampling (no probes placed → no GPU copies) add nothing
measurable; the solver path is untouched by the merge except T2-A's
render.wgsl flags-bit branch, which is uniform per frame.

## Post-U3 measurement (transforms + nested groups, canvas rewrite)

Paired back-to-back A/B in one session (same container, no other load;
lavapipe, so relative comparison only), run in both orders:

```
A = merged tip 6127fff, B = U3 (groups/gizmo/v8/probe fold)   mean       p99
pair 1 (A first): A 1080.97 / 1247.74   B 1077.31 / 1267.35
pair 2 (B first): B 1071.67 / 1231.64   A 1089.61 / 1484.24
```

Order-independent result: mean −0.3 % / −1.6 %, p99 +1.6 % / −17 % —
inside run-to-run noise in both orders; no regression. Expected: the
bench scene has no groups, so every per-object `parent_abs` walk
short-circuits at `parent: None` and rasterization takes the identity
fast path (no flatten clones); the gizmo only draws with a selection
and the Select tool active (the bench selects nothing); no probes are
placed, so the probe fold changes nothing on the hot path. The solver
path (`sim.rs` compute + shaders) is untouched by U3 except the removal
of the probe store's Mutex locking, which if anything is a hair
cheaper.

## Post-second-track-merge measurement (U3 + T2-C, scene v9)

Paired back-to-back A/B in one session (same container, no other load;
lavapipe, so relative comparison only), run in both orders:

```
A = U3 tip ef1a53c, B = merged (T2-C + v9-absorbs-v8)   mean       p99
pair 1 (A first): A 1052.69 / 1246.25   B 1065.23 / 1415.65
pair 2 (B first): B 1032.27 / 1242.16   A 1048.76 / 1424.93
```

Order-independent result: mean +1.2 % / −1.6 %, p99 +13.6 % / −12.8 %
— the deltas flip sign with run order (in BOTH pairs the fat p99 tail
sits on whichever build runs second in the pair), so the differences
are session-position noise, not a build effect; no regression.
Expected: the bench scene is the wind-tunnel preset, so
`paint_edge_bcs` early-returns without touching a cell
(`is_tunnel_preset`, pinned byte-identical by
`legacy_wind_tunnel_projection_is_unchanged`), and `apply_edge_bcs` is
one branch per damage event — the bench has exactly one damage event
(the preset load). The solver kernels are untouched by both units.

## Post-U4 measurement (fill + eraser + snaps + domain extent)

Paired back-to-back A/B in one session (fresh container — absolute
numbers are not comparable to earlier hosts; lavapipe, so relative
comparison only), run in both orders. An earlier attempt ran
concurrently with release builds and UI smoke tests and was discarded:
its A baseline came out ~90 % over a quiet run of the same binary —
the bench needs an otherwise idle host.

```
A = main 9f77dae, B = U4 tip 8d9012b                mean       p99
pair 1 (A first): A 1905.54 / 2600.33   B 1932.00 / 3041.14
pair 2 (B first): B 1875.39 / 2618.09   A 1926.11 / 2747.90
```

Order-independent result: mean +1.4 % / −2.6 %, p99 +17 % / −4.7 % —
the deltas flip sign with run order and the fat p99 tail again sits on
whichever build runs second in the pair (the session-position pattern
first seen at the second-track merge); no regression. Expected: the
bench drives no pointer, so the eraser/bucket/measure paths never run
and the per-frame object-snap resolution is gated off (it only
computes while a draw tool or handle drag is active — the bench stays
on Select). The domain-extent toggle is off, so write_render_uniform
emits the same values as before, and the rasterizer's only new branch
(filled closed Poly) sits in the Poly arm, which the Pinball scene's
Rect/Ellipse objects never enter.

## Third-integration measurement (U4 + T2-D merged)

Paired back-to-back A/B in one session (fresh container — absolute
numbers are again not host-comparable; lavapipe, relative only), run
in both orders on an otherwise idle host: both release binaries were
built BEFORE any measurement started, per the poisoned-baseline
caveat above.

```
A = main 9f77dae, B = integration tip 1b1cb24       mean       p99
pair 1 (A first): A 2797.79 / 3147.86   B 2800.76 / 3357.78
pair 2 (B first): B 2812.88 / 3233.13   A 2817.81 / 3363.21
```

Order-independent result: mean +0.1 % / −0.2 %, p99 +6.7 % / −3.9 % —
mean deltas are inside noise, and the fat p99 tail sits on whichever
build runs second in the pair for the third record running; no
regression. Expected: T2-D is format-time-only (the bench draws no
panels that change cost with the unit system, and the per-frame
Settings→units mirror is one relaxed atomic store), and U4's paths
stay pointer-gated as recorded above. Note the standing gap logged in
unit-decisions §U4: this harness never executes compute_osnap, so the
snap cost remains unmeasured.

## Post-mirror/array measurement (deferred-out-of-U4 item)

Paired back-to-back A/B in one session (same container, otherwise
idle; lavapipe, so relative comparison only), run in both orders:

```
A = main 9f40ca2, B = mirror/array 3656271          mean       p99
pair 1 (A first): A 1086.71 / 1261.81   B 1081.03 / 1358.54
pair 2 (B first): B 1078.07 / 1275.27   A 1075.08 / 1394.25
```

Order-independent result: mean −0.5 % / +0.3 %, p99 +7.7 % / −8.5 %
— the deltas flip sign with run order and the fat p99 tail again
sits on whichever build runs second in the pair (the known
session-position pattern); no regression. Expected: the bench runs
the Select tool with nothing selected, so the new inspector rows
never draw (they live in the selection panels), the object-snap
gate's added `Tool::Mirror` arm is never entered, the mirror-line
overlay only draws during its gesture, and the model hot paths
(rasterize, `parent_abs`) are untouched — mirror/array code runs
only when explicitly invoked.

## Final-merge measurement (mirror/array + deferred index + U5)

Paired back-to-back A/B on a fresh container (new lavapipe/Mesa
install — absolute numbers not comparable to earlier records), both
orders, otherwise idle host, both binaries built before measuring.

First pass caught a real workload leak, not a code regression: U5's
first-run default (100 k tracers ON) reached the bench, which pins
scene and solver but did not pin particle state — every historical
number was recorded under the pre-U5 no-tracer default.

```
A = main 9f40ca2, B = merge tip (tracers leaked)     mean       p99
pair 1 (A first): A 1822.55 / 2011.34   B 1964.26 / 2152.97
pair 2 (B first): B 1954.76 / 2130.57   A 1833.13 / 2016.30
```

Order-independent +7.2 % mean / +6.4 % p99 — the cost of advecting
and rendering 100 k tracers, confirming U5's "touches no per-frame
path" claim was wrong once the default flipped. Fix: `bench_tick`'s
frame-1 determinism block now pins `Cmd::SetParticles(0)` alongside
scene and solver (commit "Bench harness: pin tracers off"). Re-run
with the fixed harness:

```
A = main 9f40ca2, B = merge tip + harness pin        mean       p99
pair 1 (A first): A 1808.36 / 2073.59   B 1826.11 / 2046.74
pair 2 (B first): B 1806.67 / 2001.50   A 1794.17 / 2023.38
```

Order-independent result: mean +1.0 % / +0.7 %, p99 −1.3 % / −1.1 %
— mean deltas sit inside A's own run-to-run spread (1794–1833,
2.2 %), p99 slightly better; no regression. The user-facing first-run
default keeps its 100 k tracers — the pin only restores the bench's
recorded workload (canvas + rasterizer + solver, no tracers), keeping
every past and future A/B like-for-like.

## Queue item 2 measurement (gas properties: gamma + fan-aware Euler dt)

Paired back-to-back A/B in one session (same container, otherwise
idle; lavapipe under Xvfb — fresh runtime installs of
libxkbcommon-x11-0 / mesa-vulkan-drivers, so absolute numbers are not
comparable to earlier records), both orders, both binaries built
before any measurement:

```
A = main ee27ba7, B = item 2 16857e2                mean       p99
pair 1 (A first): A 1889.86 / 2361.58   B 1840.95 / 2093.29
pair 2 (B first): B 1858.88 / 2064.45   A 1908.65 / 2239.58
```

Order-independent result: mean −2.6 % / −2.6 %, p99 −11.4 % / −7.8 %
— B a touch faster in both orders, inside host spread; no regression.
Expected: the bench scene (Pinball, tunnel bands at 1x drive) keeps
`max_fan_env` ≤ 2, so `euler_dt` is byte-identical, and the new
`flush_geometry` fan rescan runs only on edit frames (the bench edits
geometry once, at its frame-1 scene build). The A runs' fat max-frame
outliers (7.7 s / 9.9 s) sat in the first and last run positions —
the known session-position pattern, not code.

## Queue item 6 measurement (object-snap frame cost, --bench-osnap)

Six-run session (same container and lavapipe/Xvfb install as the
item 2 record; both binaries built before any measurement; otherwise
idle): A = main 51ea1ba `--bench`, B = item 6 2b584cc `--bench`,
C = the same item 6 binary `--bench-osnap`. Forward A-B-C, then
reverse C-B-A, so both comparisons get both orders:

```
                                                    mean       p99
fwd: A 1970.90 / 4109.53   B 1981.14 / 4188.76   C 1959.60 / 2659.85
rev: C 1898.53 / 2175.81   B 1920.41 / 3498.70   A 1893.16 / 3307.08
```

- **Default-workload regression (B vs A)**: +0.5 % / +1.4 % mean in
  the two orders — inside this session's own A-run spread; the
  bench-only cursor override is None-guarded off the default path.
  No regression.
- **Object-snap cost (C vs B)**: −1.1 % / −1.1 % mean, BOTH orders —
  the armed Line tool + scripted sweep lands below the host's noise
  floor. On the Pinball scene, `compute_osnap` per frame is
  indistinguishable from zero; the candidate walk scales with object
  count, so a future many-object scene could re-measure with the same
  mode. This session's p99s ran fat and erratic (2.2–4.2 s) across
  ALL runs including baseline — the known session-position pattern,
  not code.

## Periodic-BC measurement (queue item 3 — first shader change)

Fresh container (new lavapipe install — this host had NO Vulkan ICD;
`mesa-vulkan-drivers` + `libxkbcommon-x11-0` were apt-installed, so
absolute numbers are not comparable to any earlier record). Paired
back-to-back A/B under Xvfb, both orders, both binaries built before
measuring, otherwise idle host.

First pass caught a real regression: the naive wrap implementation — a
runtime `P.wrap` uniform branch in every hot stencil load — cost:

```
A = main 2821906, B = naive wrap 9d2eead             mean       p99
pair 1 (A first): A 2965.77 / 6801.05   B 3315.47 / 4568.41
pair 2 (B first): B 3386.33 / 7835.18   A 3007.60 / 6847.76
```

Mean +11.8 % / +12.6 % — the SAME sign in both orders, so a real
regression, not the session-position pattern. Fix: WGSL
`override WRAP_ENABLED` pipeline specialization — the default kernel
variants constant-fold the wrap logic out (non-periodic scenes run the
pre-periodic machine code); the wrap variants bind only while
`EdgeBcs::wrap_bits != 0`. Re-run with specialization:

```
A = main 2821906, B = specialized wrap 60aae15       mean       p99
pair 1 (A first): A 3024.86 / 5768.01   B 2950.27 / 7508.22
pair 2 (B first): B 2914.69 / 6971.38   A 2781.25 / 3306.20
```

Mean −2.5 % / +4.8 % — the deltas flip sign with run order: the known
session-position pattern, no regression. The p99 tail on this
container is dominated by multi-second host stalls (max frames ran
4.1–17.9 s across the eight runs; A's own p99 spans 3306–6848 ms on
an unchanged binary) and is uncorrelated with the build — the naive
B, 12 % slower on mean, recorded the session's LOWEST p99 (4568).
Tail verdict: unusable on this container beyond order-of-magnitude;
the mean is the signal here. The bench scene has no periodic edges,
so B additionally runs the byte-identical default pipelines — the
flipping mean confirms the specialization did its job.

## Queue item 8 measurement (arcs and splines)

Paired back-to-back A/B (same container/lavapipe install as the
item 2 and item 6 records; both binaries built first; otherwise
idle), both orders:

```
A = main 6dfb392, B = item 8 ff5b155                mean       p99
pair 1 (A first): A 1884.60 / 2634.47   B 1828.96 / 2144.04
pair 2 (B first): B 1868.66 / 2646.57   A 1888.90 / 2774.05
```

Order-independent result: mean −3.0 % / −1.1 %, B faster in both
orders — inside host spread; no regression. Expected: the bench scene
contains no arc or spline, so the new code contributes only match
arms on the default paths; the shared samplers run solely for
arc/spline objects, and rasterize_chain is the old Poly body moved,
not changed. The A runs again carried the fat max-frame outliers
(6.1 s / 6.2 s) in the first/last session positions — the known
session-position pattern.

## Queue item 9 measurement (holes in filled polygons)

Paired back-to-back A/B (same container/lavapipe install as the
items 2/6/8 records; both binaries built first; otherwise idle),
both orders:

```
A = main 9c4db9b, B = item 9 bb047d7                mean       p99
pair 1 (A first): A 1832.91 / 1976.37   B 1874.88 / 2146.94
pair 2 (B first): B 1932.24 / 3013.81   A 1982.34 / 3753.44
```

The mean deltas FLIP SIGN with run order (+2.3 % / −2.5 %) — the
known session-position pattern, not a regression: both binaries ran
~4 % slower in the second pair, and the fat max-frame outliers
(11.3 s / 13.6 s) sat in the session's last two runs regardless of
binary. Expected null result: the bench scene has no Rings object, so
the new code contributes only match arms on default paths; the union
walk and subtract_rings run exclusively inside an erase commit.

## Queue item 10 measurement (union/intersect booleans)

Paired back-to-back A/B (same container/lavapipe install as the
items 2/6/8/9 records; both binaries built first; otherwise idle),
both orders:

```
A = main 71c7fcb, B = item 10 93ed230               mean       p99
pair 1 (A first): A 2161.33 / 10314.23  B 1968.67 / 2708.70
pair 2 (B first): B 1863.42 / 2582.98   A 1847.27 / 2483.76
```

Mean deltas flip sign with run order (−8.9 % / +0.9 %) — the session
-position pattern again, with the first run of the session (an A run)
visibly disturbed (p99 10.3 s, max 15.2 s). Pair 2, the clean pair,
is ±0.9 %. No regression — expected: the boolean walks and jitter run
ONLY inside the inspector button handler; no per-frame path changed
(the geomops edits are new functions plus the two GH walks' interior
point tests, which execute only during an erase or a boolean).
