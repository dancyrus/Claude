# Punchlist — out-of-scope defects noticed during the UI overhaul

Per the plan's working rules: broken things outside the current phase go
here instead of being fixed inline.

- ~~**Status bar text overlap.**~~ Fixed in phase 3a: the bottom panel
  is now a message line over a separate full-width status strip.
  Remaining nit: at the 900 px minimum width the strip's rightmost
  segments (MLUPS, Re) clip at the edge instead of eliding — revisit
  when 3b finalizes the strip's contents.
- ~~**Legend `ρ` row rounds 1.2 → "1 kg/m³"**~~ Fixed in phase 4:
  `units::fmt_density` keeps one decimal below 10 kg/m³.
- ~~**Legend "Sim rate 0.00× real"**~~ Fixed in phase 4:
  `units::fmt_sim_rate` uses three decimals below 0.1×.
- **Physics ribbon tab exactly fills 900 px** — the Domain group's
  derived "cell = …" line touches the right edge at the minimum window
  width. Cosmetic; revisit if a sixth Physics group ever appears.
- **Defaults-panel smoke picker uses `color_edit_button_srgba`**
  (`ui/inspector.rs` `defaults_panel`). The popup's alpha/blend controls
  treat `def_smoke` as premultiplied: holding the alpha slider darkens
  the stored RGB frame-over-frame (compounding toward black), and only
  r/g/b are ever read when objects are created. Same defect class was
  fixed for the two per-object pickers (now `color_edit_button_rgb`) in
  the smoke-color commit; this one edits an `egui::Color32` field so it
  needs a small u8↔f32 round-trip when converted.
- **Idle value boxes don't show the mockup's `.dv` treatment**
  (`FIELD_BG` fill + `LINE_2` border). egui 0.29 renders at-rest
  DragValues/slider readouts as buttons (`PANEL_2`, borderless);
  `extreme_bg_color` only shows while editing. Phase 3/4 should frame
  value boxes per-widget when converting sliders to DragValues.
