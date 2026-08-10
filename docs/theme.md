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

Baseline (pre-theme, commit 1f00ef2):

```
bench: 300 frames  mean 1885.08 ms  p99 2394.53 ms  min 1654.87 ms  max 6344.08 ms
```

The identical run repeats after phase 3; mean and p99 must not regress.
