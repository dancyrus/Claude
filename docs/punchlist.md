# Punchlist — out-of-scope defects noticed during the UI overhaul

Per the plan's working rules: broken things outside the current phase go
here instead of being fixed inline.

- **Status bar text overlap.** At 1440×900 the left-aligned status
  message and the right-aligned stats readout overlap mid-bar (seen in
  the phase 1 verification screenshot). Phase 3's message-line/status
  strip split resolves it; if phase 3 slips, truncate the message.
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
