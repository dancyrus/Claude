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
