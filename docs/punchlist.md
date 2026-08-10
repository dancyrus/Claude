# Punchlist — out-of-scope defects noticed during the UI overhaul

Per the plan's working rules: broken things outside the current phase go
here instead of being fixed inline.

- ~~**Status bar text overlap.**~~ Fixed in phase 3a: the bottom panel
  is now a message line over a separate full-width status strip.
  Remaining nit: at the 900 px minimum width the strip's rightmost
  segments (MLUPS, Re) clip at the edge instead of eliding — revisit
  when 3b finalizes the strip's contents.
- **Legend `ρ` row rounds 1.2 → "1 kg/m³"** (`{:.0}` in
  `ui/legend.rs`). Phase 4 (units consolidation) owns formatter decimal
  counts.
- **Legend "Sim rate 0.00× real"** on slow (software-GL) machines —
  formatter floors small rates to 0.00; phase 4 should switch to an
  adaptive precision.
- **Idle value boxes don't show the mockup's `.dv` treatment**
  (`FIELD_BG` fill + `LINE_2` border). egui 0.29 renders at-rest
  DragValues/slider readouts as buttons (`PANEL_2`, borderless);
  `extreme_bg_color` only shows while editing. Phase 3/4 should frame
  value boxes per-widget when converting sliders to DragValues.
