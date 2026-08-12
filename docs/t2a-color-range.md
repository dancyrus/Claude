# T2-A — locked color range and colormap choice (plan v4.1)

State of the unit after the second landing. The colormap picker is now
implemented (the shader edit was approved: `render.wgsl` flags bit 1,
no uniform added or reordered). Asymmetric manual min/max remains
deliberately unimplemented (see the report section at the bottom).

**Track-merge update:** the two deferred items below are done. The
`sim::color_ranges()` Mutex is gone — the state is `Settings.ranges`,
edited through `Cmd::SetRangeMode`/`SetRangeMax`/`SetColorMap`, with
the per-frame twin sync running in `update` before the panels draw.
Locked ranges and colormap picks persist in scene format v7. The
"How it works" sections below describe the track-era plumbing.

## What exists

Per render mode (Speed, Vorticity, Pressure), the color-scale range is
one of:

- **Auto** — the saturation point follows the inlet condition and the
  display gain, exactly the pre-T2-A behavior.
- **Locked** — one click pins the physical value that was on screen;
  the scale stops following the flow settings.
- **Manual** — the user types the saturation value in physical units
  (m/s, 1/s, Pa).

Each view also picks its **colormap**: Inferno (sequential) or
Coolwarm (diverging), instead of the fixed per-mode binding. The
default binding is unchanged (Speed → Inferno, Vorticity/Pressure →
Coolwarm); the shader draws the other map when `RenderParams.flags`
bit 1 is set (`GpuSim::render_flags` sets it when the pick differs
from `ColorMap::default_for(mode)`). A diverging quantity on Inferno
maps −1..1 across the ramp; Speed on Coolwarm maps 0..1 across the
full blue-white-red ramp. The legend bar mirrors the pick through the
existing `inferno_color`/`coolwarm_color` CPU tables in app.rs, which
are unchanged.

Controls live in the legend, directly under the color bar (`range`
combo + `max` entry + `map` combo). They are not in the Results ribbon because the
Results tab has no width left at the 900 px minimum (measured ~820 px
used; a new group pushed it past 900). The ribbon's `display gain`
slider disables while the current mode's range is pinned, with a
disabled-hover explaining why. Smoke has no scale, so Dye mode has no
range control.

## How it works without touching app.rs

- Every mapping in `render.wgsl` is linear in `display_gain`, so a
  fixed saturation point is a per-frame effective gain.
  `GpuSim::range_display_gain` (sim.rs) inverts the shader's
  normalization for the active mode and writes that gain into the
  render uniform — the live view and `export_png` both use it, so an
  exported PNG of a locked range matches the screen.
- The state (`[FieldRange; 4]`, mode + saturation in render units and
  in physical units) lives in `sim.rs` behind a `Mutex` handle
  (`sim::color_ranges()`), because the UI panels never hold
  `&mut GpuSim` (it lives in egui-wgpu `CallbackResources`) and the
  `Cmd`/`UiSnapshot` plumbing in app.rs belongs to Track 1 while the
  tracks run concurrently. **Fold into `Settings` + `Cmd` after the
  tracks merge.**
- All unit conversion stays app-side: `sync_color_ranges`
  (ui/legend.rs) runs every frame (before the legend's early-out) and
  rewrites the render-unit twin from the pinned physical value —
  Locked/Manual ranges hold m/s / 1/s / Pa, so the pinned label never
  moves, while colors re-derive through the current physical scaling.
  Under Auto it keeps both twins tracking the settings, which is what
  makes "Lock" capture-free: the on-screen value is already in
  `sat_phys` when the user clicks.
- Locking is per render mode and survives solver switches (the value
  is physical, so it translates; the render twin is re-derived).

## Semantics decisions (recorded)

- **Locked pins the physical value**, not the render-unit value. If
  the user edits viscosity / domain width / fluid while locked, the
  legend number holds and the colors re-map to it. Chosen because the
  audience reads the physical number, and "locked" must mean the label
  does not move.
- The min end is not adjustable: Speed saturates from a pinned 0,
  Vorticity and Pressure stay symmetric about 0. That is the shape of
  the shader's normalization; an asymmetric min/max needs a per-mode
  offset uniform in `render.wgsl`, which was not approved (only the
  flags-bit colormap edit was).
- Range state is not persisted in scene files (scene IO is app.rs,
  frozen for Track 1). Revisit at the merge.

## Report: the remainder

**Asymmetric manual min/max** stays unimplemented by decision: it
needs a per-mode offset in the `render.wgsl` normalization (either two
uniforms or a `min`/`max` pair replacing `display_gain` for these
modes), and only the flags-bit colormap edit was approved. The shipped
Manual control is the shader-expressible form — one max, with Speed's
min pinned at 0 and Vorticity/Pressure symmetric about 0.

**Queue-item-4 update (post-v4.1):** asymmetric manual min/max is now
implemented — the shader freeze lifted and `render.wgsl` gained the
per-mode `range_offset` uniform this report asked for. The min end is
adjustable in Manual mode; details in `docs/unit-decisions.md`
§Queue item 4. The report section below is historical.
