# FlowPaint V2 — UI inventory (phase 0)

Phase 0 deliverable of `docs/flowpaint-ui-overhaul-plan-v2.md`. Read-only
inventory of the UI as of commit `1bf417f`. Line numbers cite that
commit and will drift as later phases land.

# UI Inventory — first half (control table + items 1–3)

All paths relative to `/home/user/Claude/FlowPaint/src/` unless noted. Verified at HEAD `1bf417f`.

## A. Control table

### Menu bar (`FlowPaintApp::menu_bar`, app.rs:1038–1140)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| menu item (Button) | "New (clear everything)" | none | `SketchModel::objects` (via `replace_all`) + `Cmd::ResetFlow` | app.rs:1042 | Menu ▸ File |
| menu item (Button) | "Open scene…" | none | whole app state via `load_scene` (`SceneV3`/`SceneV4` decode) | app.rs:1050 | Menu ▸ File |
| menu item (Button) | "Save scene…" | none | writes `SceneV4` from model + snapshot | app.rs:1059 | Menu ▸ File |
| menu item (Button) | "Export view as PNG…" | none | `Cmd::ExportPng(path)` → `GpuSim::export_png` | app.rs:1070 | Menu ▸ File |
| menu item (Button) | "Quit" | none | `ViewportCommand::Close` | app.rs:1081 | Menu ▸ File |
| menu item (Button) | "Undo        Ctrl+Z" | none | `SketchModel` undo stack (`model.undo()`) | app.rs:1086 | Menu ▸ Edit |
| menu item (Button) | "Redo        Ctrl+Y" | none | `SketchModel` redo stack (`model.redo()`) | app.rs:1092 | Menu ▸ Edit |
| menu item (Button) | "Reset flow (keep sketch)" | none | `Cmd::ResetFlow` → `GpuSim::reset_flow` | app.rs:1099 | Menu ▸ Edit |
| radio ×4 (in submenu "Grid resolution") | "Low (960 x 480)" … "Ultra (2560 x 1280)" (`RESOLUTIONS`, sim.rs:10–15) | cells baked into label | `FlowPaintApp::res_index` + `Cmd::SetResolution(i)` → `GpuSim::set_resolution` | app.rs:1107 | Menu ▸ Simulation ▸ Grid resolution |
| radio ×4 (in submenu "Domain margin") | "None", "Small (+25 %)", "Medium (+50 %)", "Large (+100 %)" (`MARGIN_CHOICES`, sim.rs:21–26) | % of canvas height baked into label | `FlowPaintApp::margin_index` + `Cmd::SetMargin(i)` → `GpuSim::set_margin_frac` | app.rs:1120 | Menu ▸ Simulation ▸ Domain margin |
| menu item (Button) | "Keyboard shortcuts" | none | `FlowPaintApp::show_shortcuts` | app.rs:1129 | Menu ▸ Help |
| menu item (Button) | "About FlowPaint V2" | none | `FlowPaintApp::show_about` | app.rs:1133 | Menu ▸ Help |

### Side panel — action row (`side_panel_contents`, app.rs:1287–1331)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| Button | "▶ Resume" / "⏸ Pause" | none | `Cmd::TogglePause` → `Settings::paused` | app.rs:1291–1295 | Side ▸ action row |
| Button | "Reset flow" | none | `Cmd::ResetFlow` → `GpuSim::reset_flow` | app.rs:1297 | Side ▸ action row |
| Button (red RichText) | "Clear all" | none | `SketchModel::objects` via `replace_all(vec![])` + `Cmd::ResetFlow` | app.rs:1300–1312 | Side ▸ action row |
| Button (add_enabled) | "↶ Undo" | none | `SketchModel` undo stack | app.rs:1316 | Side ▸ action row |
| Button (add_enabled) | "↷ Redo" | none | `SketchModel` redo stack | app.rs:1324 | Side ▸ action row |

### Side panel — Tools (app.rs:1335–1356)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| selectable_label | "Select (S)" | none | `FlowPaintApp::tool = Tool::Select` | app.rs:1340 | Side ▸ Tools |
| selectable_label | "Line (L)" | none | `FlowPaintApp::tool = Tool::Line` | app.rs:1340 | Side ▸ Tools |
| selectable_label | "Rectangle (R)" | none | `FlowPaintApp::tool = Tool::Rect` | app.rs:1340 | Side ▸ Tools |
| selectable_label | "Ellipse (E)" | none | `FlowPaintApp::tool = Tool::Ellipse` | app.rs:1340 | Side ▸ Tools |
| selectable_label | "Polyline (P)" | none | `FlowPaintApp::tool = Tool::Polyline` | app.rs:1340 | Side ▸ Tools |
| selectable_label | "Pencil (B)" | none | `FlowPaintApp::tool = Tool::Pencil` | app.rs:1340 | Side ▸ Tools |

### Side panel — Object panel (`object_panel`, app.rs:1649–1815; shown only when `selected.is_some()` and gesture is `None`, app.rs:1360–1365)

All edits go through a clone-and-commit path: widgets mutate a local `obj` copy, then `model.objects[i] = obj; model.record_modify_coalesced(id, before)` at app.rs:1800–1804.

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| selectable_label | "Wall" (`ObjMaterial::label`, model.rs:24–31) | none | `SketchObject::material = Wall` | app.rs:1682 | Side ▸ Object |
| selectable_label | "Fan" | none | `SketchObject::material = Fan` (also pushes `Cmd::SetRenderMode(Dye)` when Smoke picked, app.rs:1688) | app.rs:1682 | Side ▸ Object |
| selectable_label | "Smoke" | none | `SketchObject::material = Smoke` | app.rs:1682 | Side ▸ Object |
| selectable_label | "Drain" | none | `SketchObject::material = Drain` | app.rs:1682 | Side ▸ Object |
| Checkbox (only if `can_fill`) | "Filled" | none | `SketchObject::filled` | app.rs:1694 | Side ▸ Object |
| Slider 1.0..=24.0 | "thickness ({})" | cells; derived length in label via `fmt_len(ps.len_m(..))` (mm/cm/m) | `SketchObject::thickness` | app.rs:1701 | Side ▸ Object |
| Slider 0.2..=2.0 (Fan or fan-carrying stamp) | "fan speed ×" | multiplier on global flow speed | `SketchObject::fan_mult` | app.rs:1717 | Side ▸ Object |
| Slider 0.0..=1.0 | "gustiness" | dimensionless 0–1 | `SketchObject::fan_gust` | app.rs:1721 | Side ▸ Object |
| Slider -180..=180 (filled Rect/Ellipse fans only) | "blow direction °" | degrees | `SketchObject::fan_angle` (stored radians, converted at app.rs:1741) | app.rs:1735–1738 | Side ▸ Object |
| color_edit_button_srgba (Fan or Smoke) | "Smoke color:" | none | `SketchObject::smoke_rgb` | app.rs:1754 | Side ▸ Object |
| small_button ×3 | "Rotate" row: "-15°", "+15°", "+90°" | degrees baked into labels | `SketchObject` geometry via `rotate_by` | app.rs:1769 | Side ▸ Object |
| small_button ×2 | "Scale" row: "×0.8", "×1.25" | factor baked into labels | `SketchObject` geometry via `scale_by` | app.rs:1778 | Side ▸ Object |
| Button | "Duplicate (Ctrl+D)" | none | new `SketchObject` via `duplicate_selected` (app.rs:917) | app.rs:1787 | Side ▸ Object |
| Button | "Delete (Del)" | none | `SketchModel::remove(id)`; clears `FlowPaintApp::selected` | app.rs:1790 | Side ▸ Object |

### Side panel — Defaults panel (`defaults_panel`, app.rs:1818–1865; shown when nothing selected)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| selectable_label ×4 | "Wall" / "Fan" / "Smoke" / "Drain" | none | `FlowPaintApp::def_material` (Smoke also pushes `Cmd::SetRenderMode(Dye)`, app.rs:1836) | app.rs:1829 | Side ▸ New objects |
| Slider 1.0..=24.0 | "thickness ({})" | cells; derived length in label via `fmt_len` | `FlowPaintApp::def_thickness` | app.rs:1846 | Side ▸ New objects |
| Checkbox | "Filled rect / ellipse" | none | `FlowPaintApp::def_filled` | app.rs:1848 | Side ▸ New objects |
| Slider 0.2..=2.0 (Fan only) | "fan speed ×" | multiplier | `FlowPaintApp::def_fan_mult` | app.rs:1852 | Side ▸ New objects |
| Slider 0.0..=1.0 (Fan only) | "gustiness" | dimensionless 0–1 | `FlowPaintApp::def_fan_gust` | app.rs:1855 | Side ▸ New objects |
| color_edit_button_srgba (Fan/Smoke) | "Smoke color:" | none | `FlowPaintApp::def_smoke` (Color32) | app.rs:1862 | Side ▸ New objects |

### Side panel — Sketch aids (app.rs:1372–1397)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| DragValue 1..=90 | "angle snap (Shift)" | "°" suffix | `FlowPaintApp::snap_angle_deg` | app.rs:1376–1381 | Side ▸ Sketch aids |
| small_button ×6 | "5°" "15°" "22.5°" "30°" "45°" "90°" | degrees baked into labels | `FlowPaintApp::snap_angle_deg` | app.rs:1385 | Side ▸ Sketch aids |
| Checkbox | "Snap to grid" | none | `FlowPaintApp::snap_enabled` | app.rs:1390 | Side ▸ Sketch aids |
| Slider 2.0..=50.0 (shown only when snap on) | "spacing ({})" | cells; derived length in label via `fmt_len` | `FlowPaintApp::snap_spacing` | app.rs:1394–1396 | Side ▸ Sketch aids |

### Side panel — Generators (app.rs:1401–1409)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| Button | "✈ Airfoil…" | none | `FlowPaintApp::show_airfoil_gen = true` | app.rs:1403 | Side ▸ Generators |
| Button | "🚀 Nozzle…" | none | `FlowPaintApp::show_nozzle_gen = true` | app.rs:1406 | Side ▸ Generators |

### Side panel — Scene presets (app.rs:1413–1430)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| Button | "Cylinder" | none | `SketchModel::objects` via `build_preset` + `replace_all` + `Cmd::ResetFlow` | app.rs:1416–1426 | Side ▸ Scene presets |
| Button | "Airfoil" | none | same | app.rs:1416–1426 | Side ▸ Scene presets |
| Button | "Venturi" | none | same | app.rs:1416–1426 | Side ▸ Scene presets |
| Button | "Step" | none | same | app.rs:1416–1426 | Side ▸ Scene presets |
| Button | "Pinball" | none | same | app.rs:1416–1426 | Side ▸ Scene presets |

### Side panel — View (app.rs:1434–1459)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| selectable_label ×4 | "Smoke" / "Speed" / "Vorticity" / "Pressure" (`RenderMode::label`, sim.rs:59–66) | none | `Cmd::SetRenderMode(m)` → `Settings::render_mode` | app.rs:1437 | Side ▸ View |
| Checkbox | "Highlight fans && drains" | none | `Cmd::SetBoundaryTints` → `Settings::boundary_tints` | app.rs:1443 | Side ▸ View |
| Checkbox | "Show legend" | none | `FlowPaintApp::show_legend` | app.rs:1446 | Side ▸ View |
| ComboBox | label "particles"; entries "Off", "100 k", "500 k", "1 M", "2 M" (`PARTICLE_CHOICES`, sim.rs:29–35) | count baked into labels | `FlowPaintApp::particle_index` + `Cmd::SetParticles(count)` → `Settings::particle_count` | app.rs:1447–1459 | Side ▸ View |

### Side panel — Physics (app.rs:1463–1584)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| selectable_label | "Incompressible" | none | `Cmd::SetSolver(Lbm)` → `Settings::solver` (resets flow on change, app.rs:629–636) | app.rs:1479–1486 | Side ▸ Physics |
| selectable_label | "Compressible" | none | `Cmd::SetSolver(Euler)` → `Settings::solver` | app.rs:1479–1486 | Side ▸ Physics |
| ComboBox | label "fluid"; 7 entries from `FLUID_PRESETS` (app.rs:110–192), fallback "Custom" | none | `FlowPaintApp::fluid_preset_idx/fluid_name/fluid_nu/fluid_rho/fluid_a` + `Cmd::SetWindTunnel/SetFlowSpeed/SetViscosity/SetSteps` | app.rs:1493–1520 | Side ▸ Physics |
| Slider 0.3..=3.0 (Euler only) | "inlet Mach ({})" | Mach; derived m/s in label via `fmt_speed(mach * fluid_a)` | `Cmd::SetMach` → `Settings::mach` | app.rs:1532–1537 | Side ▸ Physics |
| Slider 0.02..=0.14 (LBM only) | "flow speed ({})" | lattice units; derived m/s in label via `fmt_speed(ps.u_phys(flow))` | `Cmd::SetFlowSpeed` → `Settings::flow_speed` (clears `fluid_preset_idx`) | app.rs:1548–1555 | Side ▸ Physics |
| Slider 0.005..=0.08 log (LBM only) | "viscosity (Δt {})" | lattice units; derived time step in label via `fmt_time(ps.dt)` | `Cmd::SetViscosity` → `Settings::viscosity` | app.rs:1556–1566 | Side ▸ Physics |
| Slider 1..=32 | "steps / frame" | steps/frame | `Cmd::SetSteps` → `Settings::steps_per_frame` | app.rs:1568–1574 | Side ▸ Physics |
| Slider 0.985..=1.0 | "smoke persistence" | none (raw retention factor) | `Cmd::SetDyeFade` → `Settings::dye_fade` | app.rs:1575–1580 | Side ▸ Physics |
| Checkbox | "Wind tunnel (left to right)" | none | `Cmd::SetWindTunnel` → `GpuSim::set_wind_tunnel` → `Settings::wind_tunnel` | app.rs:1581–1584 | Side ▸ Physics |

### Side panel — Advanced (CollapsingHeader, app.rs:1587–1644)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| Slider 0.05..=100.0 log | "domain width (m)" | metres in label text | `FlowPaintApp::domain_width_m` (drives `PhysScale`) | app.rs:1588–1596 | Side ▸ Advanced |
| Slider 0.25..=4.0 log | "display gain" | none | `Cmd::SetDisplayGain` → `Settings::display_gain` | app.rs:1598–1608 | Side ▸ Advanced |
| Slider 0.25..=3.0 | "smoke brightness" | none | `Cmd::SetSmokeGain` → `Settings::smoke_gain` | app.rs:1609–1615 | Side ▸ Advanced |
| Slider 0.0..=0.3 | "edge damping" | none | `Cmd::SetSpongeStrength` → `Settings::sponge_strength` | app.rs:1616–1626 | Side ▸ Advanced |
| Slider 0.8..=5.0 | "particle size" | none (px-ish) | `Cmd::SetParticleSize` → `Settings::particle_size` | app.rs:1627–1633 | Side ▸ Advanced |
| Slider 0.05..=1.0 | "particle brightness" | none | `Cmd::SetParticleBrightness` → `Settings::particle_brightness` | app.rs:1634–1643 | Side ▸ Advanced |

### Legend panel (`legend_panel`, app.rs:1870–2016)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| — (no interactive widgets) | "Flow numbers" grid + colormap bars are read-only readouts (`fmt_len`/`fmt_time`/`fmt_speed`/`fmt_pressure`) | mixed physical units in readouts | none — its only toggle is the "Show legend" checkbox (`FlowPaintApp::show_legend`, app.rs:1446) | app.rs:1876–2015 | Legend (right SidePanel) |

### Airfoil generator dialog (`generator_windows`, app.rs:2652–2697)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| Window close box | "Airfoil generator" | none | `FlowPaintApp::show_airfoil_gen` (via `.open(&mut show)`) | app.rs:2653–2654, 2697 | Airfoil dialog |
| ComboBox (selectable_label ×7 entries) | "Famous airfoils" / `AIRFOIL_PRESETS` names (generators.rs:38–46) | none | `AirfoilParams::camber/camber_pos/thickness/aoa_deg` | app.rs:2658–2669 | Airfoil dialog |
| Slider 0.0..=9.0 | "camber %" | % of chord | `AirfoilParams::camber` | app.rs:2671 | Airfoil dialog |
| Slider 15.0..=70.0 | "camber position %" | % of chord | `AirfoilParams::camber_pos` | app.rs:2672–2675 | Airfoil dialog |
| Slider 4.0..=24.0 | "thickness %" | % of chord | `AirfoilParams::thickness` | app.rs:2676 | Airfoil dialog |
| Slider -15.0..=20.0 | "angle of attack °" | degrees | `AirfoilParams::aoa_deg` | app.rs:2677 | Airfoil dialog |
| Slider 60.0..=600.0 | "chord (cells)" | cells | `AirfoilParams::chord_cells` | app.rs:2678–2681 | Airfoil dialog |
| Button | "Insert into scene" | none | new Stamp `SketchObject` via `generate_airfoil` + `insert_stamp_object` | app.rs:2692–2695 | Airfoil dialog |

(The "≈ NACA XXXX at Y°" line at app.rs:2687 is a read-only readout.)

### Nozzle generator dialog (app.rs:2699–2825)

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| Window close box | "Rocket nozzle generator" | none | `FlowPaintApp::show_nozzle_gen` | app.rs:2700–2701, 2824 | Nozzle dialog |
| ComboBox (selectable_label ×6 entries) | "Famous engines" / `NOZZLE_PRESETS` names (generators.rs:178–185) | ε ratio baked into names | `NozzleParams::exit_ratio/contour/div_ratio/fan_mult` + `FlowPaintApp::nozzle_fan_auto = true`, `nozzle_real_ve` | app.rs:2705–2724 | Nozzle dialog |
| Slider 12.0..=100.0 | "throat width (cells)" | cells | `NozzleParams::throat_cells` | app.rs:2726–2729 | Nozzle dialog |
| Slider 1.2..=20.0 | "exit / throat width" | ratio (none) | `NozzleParams::exit_ratio` | app.rs:2730–2733 | Nozzle dialog |
| Slider 1.5..=4.0 | "chamber / throat width" | ratio (none) | `NozzleParams::chamber_ratio` | app.rs:2734–2737 | Nozzle dialog |
| Slider 1.0..=4.0 | "converging length / throat" | ratio (none) | `NozzleParams::conv_ratio` | app.rs:2738–2741 | Nozzle dialog |
| Slider 2.0..=16.0 | "bell length / throat" | ratio (none) | `NozzleParams::div_ratio` | app.rs:2742–2745 | Nozzle dialog |
| Slider 3.0..=12.0 | "wall (cells)" | cells | `NozzleParams::wall_cells` | app.rs:2746 | Nozzle dialog |
| radio_value | "Bell" | none | `NozzleParams::contour = Bell` | app.rs:2748 | Nozzle dialog |
| radio_value | "Conical (15°-style)" | none | `NozzleParams::contour = Conical` | app.rs:2749 | Nozzle dialog |
| Checkbox | "Fan in the chamber (self-powered)" | none | `NozzleParams::chamber_fan` | app.rs:2751 | Nozzle dialog |
| Slider 0.2..=2.0 | "chamber fan ×" | multiplier on global flow speed | `NozzleParams::fan_mult`; on user change sets `FlowPaintApp::nozzle_fan_auto = false` (auto-refreshed each frame at app.rs:2755–2757 while auto) | app.rs:2758–2766 | Nozzle dialog |
| Button | "Insert into scene" | none | new Stamp `SketchObject` via `generate_nozzle` + `insert_stamp_object` | app.rs:2819–2822 | Nozzle dialog |

(Read-only readouts: "sim throat jet ≈ …" app.rs:2784–2788, real-exhaust note app.rs:2789–2807, solver note app.rs:2809–2817.)

### Other windows and canvas

| widget | current label | current units | backing state field | file:line | owning panel |
|---|---|---|---|---|---|
| Window close box | "About FlowPaint V2" | none | `FlowPaintApp::show_about` | app.rs:2831–2833 | About window |
| Window close box | "Keyboard shortcuts" | none | `FlowPaintApp::show_shortcuts` | app.rs:2855–2857 | Shortcuts window |
| — | Canvas (`CentralPanel`) contains **no widgets** — a single `allocate_rect(rect, Sense::drag())` drives the gesture state machine; the status bar (app.rs:2040–2068) is read-only | — | — | app.rs:2070–2106 | Canvas |

Row count: 98 control rows (counting the ×N radio/preset groups as single rows).

## B. Item 1 — how the UI code is split today

**Exactly one file draws widgets: `app.rs` (2977 lines).** The others:

| file | lines | draws widgets? |
|---|---|---|
| app.rs | 2977 | yes — the entire egui UI |
| sim.rs | 1306 | no — supplies UI label constants (`RESOLUTIONS` sim.rs:10, `MARGIN_CHOICES` sim.rs:21, `PARTICLE_CHOICES` sim.rs:29, `RenderMode::label` sim.rs:59) and `Settings` (sim.rs:70) |
| model.rs | 975 | no — supplies `ObjMaterial::label()` (model.rs:24–31) |
| generators.rs | 294 | no — param structs + preset name strings consumed by the dialogs |
| geometry.rs | 115 | no |
| main.rs | 42 | no — window title/size only (main.rs:17–20) |
| shaders/*.wgsl | 1009 total | no |

**app.rs section map** (function → line range → what it draws):

| function | lines | draws |
|---|---|---|
| `update` (eframe::App) | 553–622 | orchestrator: snapshot, then calls every panel fn, applies `Cmd`s |
| `apply_cmd` | 625–680 | no widgets; `Cmd` → `Settings`/`GpuSim` dispatch |
| `keyboard` | 805–915 | no widgets; keyboard shortcuts |
| `menu_bar` | 1038–1140 | top menu bar (File/Edit/Simulation/Help) |
| `save_scene` / `load_scene` | 1142–1173 / 1175–1273 | no widgets; file IO invoked from the menu |
| `side_panel` | 1275–1285 | left `SidePanel` + ScrollArea shell |
| `side_panel_contents` | 1287–1645 | action row, Tools, dispatch to object/defaults panel, Sketch aids, Generators, Scene presets, View, Physics, Advanced |
| `object_panel` | 1649–1815 | selected-object inspector |
| `defaults_panel` | 1818–1865 | "New objects" defaults |
| `legend_panel` | 1870–2016 | right legend `SidePanel` (readouts + colormap bars) |
| `colormap_bar` | 2018–2037 | painter-drawn gradient bar for the legend |
| `status_bar` | 2040–2068 | bottom `TopBottomPanel` readout strip |
| `canvas` | 2070–2106 | `CentralPanel` + wgpu paint callback + interaction hookup |
| `canvas_interaction` | 2150–2352 | pointer gesture state machine (no widgets) |
| `select_press` / `poly_click` / `update_draw_shape` | 2357–2386 / 2390–2430 / 2435–2488 | gesture helpers (no widgets) |
| `canvas_overlays` | 2491–2646 | painter-drawn selection outline, handles, snap grid, dimension text |
| `generator_windows` | 2649–2825 | Airfoil + Nozzle dialog `Window`s |
| `windows` | 2828–2882 | About + Keyboard-shortcuts `Window`s |

## C. Item 2 — object selection state and the inspector dispatch

**Where selection lives:** `FlowPaintApp::selected: Option<u64>` (app.rs:349) — an object id into `SketchModel::objects`, resolved by `model.find(id)`. There is no selection state in `SketchModel` itself. Writers (all in app.rs): `new` (423), Esc/Delete/undo-redo keyboard handlers (819–832, 839, 858, 863), `duplicate_selected` (924), `insert_stamp_object` (523), `finish_gesture` (975/979, 997–1002, 1022–1027), menu "New" (1044), `load_scene` (1217), "Clear all" (1309), side-panel Undo/Redo (1321, 1329), scene-preset buttons (1422), `object_panel` self-heal + Delete button (1651–1652, 1791), `select_press` (2377, 2384), `poly_click` (2417), and right-click deselect (2209).

**Gesture guard:** the inspector is suppressed mid-gesture. app.rs:1360–1367:

```rust
if self.selected.is_some() && !matches!(self.gesture, Gesture::None) {
    // Mid-gesture: the object panel would fight the drag.
    ui.heading("Object");
    ui.label(egui::RichText::new("(finish the gesture…)").weak());
} else if let Some(id) = self.selected {
    self.object_panel(ui, id, cmds);
} else {
    self.defaults_panel(ui, cmds);
}
```

So the panel three-way branches: mid-gesture placeholder → object inspector → defaults panel.

**How the inspector picks property blocks** (`object_panel`, app.rs:1649–1815). The dispatch is *not* keyed on `ObjMaterial` alone — it mixes shape predicates with material checks:

- app.rs:1668–1669: `let is_stamp = matches!(obj.shape, Shape::Stamp { .. });` and `let can_fill = matches!(obj.shape, Shape::Rect { .. } | Shape::Ellipse { .. });`
- app.rs:1672 `if !is_stamp { … }` — the 4-way material selector, Filled checkbox, and thickness slider are hidden entirely for stamps (a stamp's cells carry their own types).
- app.rs:1694 `if can_fill && ui.checkbox(&mut obj.filled, "Filled")…` and app.rs:1697 `if !(can_fill && obj.filled)` — thickness only when the shape isn't a filled rect/ellipse.
- app.rs:1708–1713 fan detection for stamps: `let stamp_has_fans = match &obj.shape { Shape::Stamp { raster, .. } => raster.cell.iter().any(|&c| c == crate::geometry::CELL_INLET), _ => false };`
- app.rs:1714 `if obj.material == ObjMaterial::Fan || stamp_has_fans` — fan speed × and gustiness sliders (this is how a nozzle stamp gets throttle control despite `material == Wall`).
- app.rs:1729–1732: blow-direction slider only when `obj.material == ObjMaterial::Fan && (matches!(obj.shape, Shape::Rect{..} | Shape::Ellipse{..}) && obj.filled)` — chained shapes blow along their segments instead.
- app.rs:1746 `if obj.material == ObjMaterial::Fan || obj.material == ObjMaterial::Smoke` — smoke color button.

Commit path: widgets set `changed`; app.rs:1800–1804 writes the mutated clone back and records a coalesced undo (`record_modify_coalesced`).

## D. Item 3 — nozzle generator construction and every speed cap on the chamber fan

**Construction:** `generate_nozzle(p: &NozzleParams) -> GeoRegion`, generators.rs:225–294. Rasterizes into `cell`/`fan`/`dye_src` vecs (generators.rs:240–242):
- Chamber back wall: generators.rs:266–271 (`if ax < 0.0 { if ay <= rc + wall { cell[i] = CELL_WALL } }`).
- Side/bell walls: generators.rs:275–281 — watertight band using min/max of three neighbouring columns' contour, `cell[i] = CELL_WALL` at 281.
- **Chamber fan cells:** generators.rs:282–289 — for in-bore cells with `ax < 3.0` when `p.chamber_fan` is on: `cell[i] = CELL_INLET` (286), `fan[i] = [p.fan_mult.clamp(0.2, 2.0), 0.0, 0.0, 0.0]` (287) i.e. a +x fan vector whose magnitude is the per-cell speed multiplier, and a dye source (288) so the plume shows immediately.

**Insertion:** `FlowPaintApp::insert_stamp_object`, app.rs:507–527 — wraps the raster in `Shape::Stamp` centred on the canvas, then **forces** `obj.material = ObjMaterial::Wall` (519), `obj.fan_mult = 1.0` (520), `obj.fan_gust = 0.0` (521), so the stamp lands exactly as the dialog computed it; the object-level `fan_mult` is a later multiplier the inspector can retune (it applies on top of the per-cell 287 value via the model rasterizer).

**Every clamp/cap touching the chamber fan speed:**

1. **Generator fan_mult clamp** — generators.rs:287 `p.fan_mult.clamp(0.2, 2.0)`: bounds the per-cell multiplier baked into the stamp's fan vector, regardless of what the dialog slider/auto formula produced.
2. **`nozzle_auto_fan_mult`, LBM arm** — app.rs:2939–2941 `(0.27 / (snap.flow * chamber_ratio).max(1e-4)).clamp(0.2, 2.0)`: picks the multiplier so the continuity-estimated throat speed hits 0.27 lattice (just under the LBM cap), then bounds it to the slider's 0.2–2.0 range.
3. **`nozzle_auto_fan_mult`, Euler arm** — app.rs:2944 `(0.3 / snap.mach.max(0.1)).clamp(0.2, 2.0)`: feeds the chamber at ~Mach 0.3 whatever the inlet Mach slider says, again bounded to 0.2–2.0.
4. **Dialog readout 0.3 cap** — app.rs:2778–2782: the "sim throat jet ≈" readout computes `throat_lattice = snap.flow * p.fan_mult * p.chamber_ratio` and displays `u_phys(throat_lattice.min(0.3))`, flagging "(speed-capped)" when `throat_lattice > 0.3` — it caps only the *displayed estimate*, mirroring the shader clamp; it changes no state.
5. **LBM shader clamp** — lbm.wgsl:26 `const MAX_LATTICE_SPEED: f32 = 0.3;`, applied to inlet cells at lbm.wgsl:124 (`if (usp > MAX_LATTICE_SPEED) { u *= MAX_LATTICE_SPEED / usp; }` on `u = dir * P.inlet_speed`): the actual runtime bound on the fan's imposed lattice velocity — no matter the fan multiplier, an inlet cell never injects more than 0.3 cells/step. (The same constant also clamps outlet momentum at 144 and post-collision fluid velocity at 163.)
6. **Euler shader inlet clamp** — euler.wgsl:93–95 in `inlet_prim`: `var u = dir * P.mach; … if (sp > 8.0) { u *= 8.0 / sp; }` — bounds the chamber inlet velocity at Mach 8 (nondimensional, a∞ = 1), a far looser sanity cap than the LBM one; the fan vector's magnitude multiplies the global inlet Mach.

Chain summary: dialog auto/slider (clamped 0.2–2.0 at app.rs:2941/2944 and by the slider range app.rs:2760) → baked per-cell at generators.rs:287 (clamped 0.2–2.0 again) → object-level `fan_mult` forced to 1.0 at app.rs:520 (retunable 0.2–2.0 via the inspector slider app.rs:1717, since `stamp_has_fans` is true) → runtime speed bounded by lbm.wgsl:124 (0.3 lattice) or euler.wgsl:95 (Mach 8).

---

# UI inventory — items 4–6 (second half of docs/ui-inventory.md)

Sources examined: `/home/user/Claude/FlowPaint/src/{app.rs, sim.rs, model.rs, geometry.rs, generators.rs, main.rs}`, `/home/user/Claude/FlowPaint/Cargo.lock`, `/home/user/Claude/docs/flowpaint-ui-overhaul-plan-v2.md`, commits `1e76f5a`, `146e797`, `61368c8`. Every line number below was verified by reading the cited lines.

---

## Item 4 — hardcoded visual constants (src/*.rs, shaders excluded)

### Counts by file

| file | Color32 constructions | Rounding/corner-radius literals | Stroke::new | FontId | add_space | panel/window px constants | interaction thresholds & misc px |
|---|---|---|---|---|---|---|---|
| app.rs | 9 | 3 | 3 | 1 | 21 | 4 | 8 |
| main.rs | 0 | 0 | 0 | 0 | 0 | 2 | 0 |
| sim.rs | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| model.rs | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| geometry.rs | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| generators.rs | 0 | 0 | 0 | 0 | 0 | 0 | 0 |

All visual constants live in `app.rs` plus two window-size constants in `main.rs`. There is no `Rounding` type usage anywhere; the three "rounding" entries are literal corner-radius arguments to painter calls. Grep note: `Color32` matches 13 lines in app.rs, but 4 are type annotations (`fn inferno_color(...) -> egui::Color32` at 270, 285; field decl at 357; closure param at 2018), leaving 9 actual constructions.

### Color32 (9 constructions, app.rs)

| file:line | value | role |
|---|---|---|
| app.rs:282 | `from_rgb` from interpolated stops | inferno colormap legend bar (CPU mirror of shader colormap — see borderline note) |
| app.rs:292 | `from_rgb` from interpolated stops | coolwarm colormap legend bar (same note) |
| app.rs:430 | `from_rgb(90, 217, 255)` | default smoke dye color (`def_smoke`) — content default, borderline |
| app.rs:1303 | `from_rgb(255, 140, 120)` | destructive tint on the "Clear all" button label |
| app.rs:1747 | `from_rgb(obj.smoke_rgb * 255)` | conversion from object state for the color picker (not a fixed constant, but a hardcoded 255-scale conversion site) |
| app.rs:2508 | `from_rgba_unmultiplied(255, 255, 255, 12)` | faint snap-grid overlay lines |
| app.rs:2542 | `from_rgb(255, 200, 90)` | selection/gesture accent (outline + handles + dimension text) — the app's de-facto accent color |
| app.rs:2600 | `Color32::WHITE` | vertex handle fill |
| app.rs:2601 | `Color32::BLACK` | vertex handle outline |

### Corner-radius literals (no `Rounding` type used)

| file:line | value | role |
|---|---|---|
| app.rs:2033 | `0.0` | colormap bar segment `rect_filled` |
| app.rs:2600 | `1.0` | vertex handle `rect_filled` |
| app.rs:2601 | `1.0` | vertex handle `rect_stroke` |

### Stroke::new (3)

| file:line | value | role |
|---|---|---|
| app.rs:2506–2509 | `Stroke::new(1.0, white@12)` | snap grid lines |
| app.rs:2543 | `Stroke::new(1.5, accent)` | active-object outline |
| app.rs:2601 | `Stroke::new(1.0, BLACK)` | vertex handle border |

### FontId (1)

| file:line | value | role |
|---|---|---|
| app.rs:2643 | `FontId::monospace(12.0)` | on-canvas dimension readout. All other text styling goes through `RichText::small()/.weak()` (8 `RichText::new` sites: 1302, 1350, 1363, 1539, 1807, 1948, 2806, 2817) and `ui.heading/label/small/monospace` — no other explicit font sizes. |

### add_space (21 in app.rs — three magic values)

- `add_space(6.0)` × 13 — section separator rhythm in the side panel and generator windows: 1333, 1358, 1370, 1399, 1411, 1432, 1461, 1586, 1938, 2691, 2818, 2841, 2847
- `add_space(4.0)` × 4 — panel-top padding: 1288, 1877, 2670, 2725
- `add_space(2.0)` × 4 — intra-block micro spacing: 1715, 1765, 1785, 2808

This is the "spacing varies between panels for no reason" the plan's phase 2 targets; the variation is 2/4/6 with no system.

### Panel / window pixel constants

| file:line | value | role |
|---|---|---|
| app.rs:1277 | `.default_width(248.0)` | left controls `SidePanel` |
| app.rs:1876 | `.default_width(200.0)` | right legend `SidePanel` |
| app.rs:1882 | `.min_col_width(80.0)` | legend "Flow numbers" `Grid` |
| app.rs:2020 | `vec2(available.min(184.0), 14.0)` | colormap bar size |
| main.rs:18 | `[1440.0, 900.0]` | initial window size |
| main.rs:19 | `[900.0, 600.0]` | minimum window size |

### Interaction thresholds and misc pixel constants (app.rs)

| file:line | value | role |
|---|---|---|
| 2164 | `handle_r = (8.0 * ppp / px_per_cell).max(2.0)` | vertex-handle hit radius: 8 px screen space, 2-cell floor |
| 2165 | `click_slop = (4.0 * ppp / px_per_cell).max(2.0)` | click-selection slop: 4 px, 2-cell floor |
| 2504 | `if step_pt >= 8.0` | snap grid hidden below 8 pt spacing |
| 2599 | `vec2(7.0, 7.0)` | vertex handle square size |
| 2638 | `- egui::vec2(0.0, 4.0)` | dimension label offset above object bounds |
| 2564–2566 | `48` segments | ellipse outline tessellation for the selection overlay |
| 2024 | `n = 48` | colormap bar segment count |
| 2020 | `14.0` | colormap bar height |

### Borderline cases (noted, not counted as UI chrome)

- **Colormap stop tables** app.rs:272–278 (inferno) and 287–289 (coolwarm): visual, but they are CPU mirrors of the shader colormaps (comment at 269). Re-theming them independently of `shaders/*.wgsl` would desynchronize legend and field rendering — they should move to the theme layer only as a linked pair, or stay put.
- **`def_smoke` (90, 217, 255)** app.rs:430: default dye color for new smoke objects — scene content, not chrome.
- **`snap_spacing: 10.0`** (432) and slider range `2.0..=50.0` (1395): grid-cell units, not pixels — sketch-aid behavior, not visual chrome. Same for `snap_angle_deg: 45.0` (433) and presets `[5, 15, 22.5, 30, 45, 90]` (1384).
- **`def_thickness: 6.0`** (426) and thickness ranges `1.0..=24.0` (1701, 1846): cells → physical wall thickness; physics-adjacent, do not theme.
- Physics constants (fluid_nu 1.5e-5, fluid_a 343.0, slider ranges for Mach/viscosity/etc.) excluded per instructions.
- `[0.5]`-cell minimum shape radii (2468–2477) are geometry guards, not visual.

---

## Item 5 — dependency versions and egui 0.29 → 0.35 API exposure

### Resolved versions (FlowPaint/Cargo.lock)

| crate | version | lock lines |
|---|---|---|
| egui | **0.29.1** | 1029–1030 |
| eframe | **0.29.1** | 992–993 |
| egui-wgpu | **0.29.1** | 1043–1044 |
| epaint | **0.29.1** | 1156–1157 |
| wgpu | **22.1.0** | 3736–3737 |
| winit | **0.30.13** | 4133–4134 |

### egui API surface actually used (app.rs + main.rs; sim.rs uses no egui, only wgpu)

Containers/panels: `SidePanel::left` (1276) / `::right` (1876), `TopBottomPanel::top` (1039) / `::bottom` (2041), `CentralPanel::default().frame(...)` (2071–2072), `Window::new` ×4 (2653, 2700, 2831, 2855) with `.open/.resizable/.collapsible`, `ScrollArea::vertical().auto_shrink` (1279–1280), `CollapsingHeader::new` (1587), `Grid::new` ×2 (1879 with `.num_columns/.min_col_width`, 2860 with `.striped`), `egui::menu::bar` (1040), `Frame::none()` (2072).

Widgets: `Slider::new(...).text(...)` ×~29, `DragValue::new(...).range/.speed/.suffix` (1377–1380), `ComboBox::from_label(...).selected_text(...).show_ui` ×4 (1447, 1493, 2659, plus nozzle window), `Button::new` ×2 via `add_enabled` (1316, 1324), `ui.button` ×17, `ui.selectable_label` (1340, 1437, 1452, 1480, 1499, 1682, 1829, 2662), `ui.radio_value` ×2, `ui.checkbox` ×7, `ui.small_button` ×3, `ui.color_edit_button_srgba` ×2 (1754), `RichText` ×8, `ui.heading` ×10, `ui.monospace` ×2, `ui.small` ×10.

Layout/response/input: `ui.horizontal(_wrapped)`, `with_layout(Layout::right_to_left(Align::Center))` ×4, `add_space` ×21, `separator` ×14, `allocate_rect(rect, Sense::drag())` (2078), `allocate_exact_size` (2019), `response.drag_started_by/dragged_by(PointerButton::…)` (2203, 2216, 2280), `hover_pos`, `interact_pointer_pos`, `ctx.input` ×14 (`Key::*` ×18), `ctx.pixels_per_point` (2080), `ctx.send_viewport_cmd(ViewportCommand::Close)`, `ctx.request_repaint`, `ui.close_menu()` ×11 (1047–1135), `ui.menu_button` ×6, `on_hover_text` ×13.

Painting: `ui.painter()`, `painter.line_segment/rect_filled/rect_stroke/text/add(Shape::line|closed_line)`, `Align2::LEFT_BOTTOM`, `pos2/vec2`.

egui-wgpu: `egui_wgpu::Callback::new_paint_callback` (2099), `impl egui_wgpu::CallbackTrait` with `prepare(&Device, &Queue, &ScreenDescriptor, &mut CommandEncoder, &mut CallbackResources) -> Vec<CommandBuffer>` and `paint(PaintCallbackInfo, &mut wgpu::RenderPass<'static>, &CallbackResources)` (2952–2977), `egui_wgpu::WgpuConfiguration { power_preference, device_descriptor }` (main.rs:21–33), `eframe::Renderer::Wgpu`, `ViewportBuilder`.

### Changes between 0.29 and 0.35 that hit this code

I fetched the upstream changelogs (egui CHANGELOG.md and crates/egui-wgpu/CHANGELOG.md) through the proxy successfully; claims are tagged accordingly. The fetch is summarized by a small model, so version attribution of individual PRs should be re-confirmed against release notes in phase 1 where flagged.

**Breaks (will not compile / must change):**

1. **`egui::menu::bar` + `ui.close_menu()` — 12 call sites** (app.rs:1040; 1047, 1057, 1067, 1078, 1090, 1096, 1101, 1111, 1123, 1131, 1135). Menu API rewritten on `egui::Popup` in 0.32 (#5716), old API deprecated; **all deprecated items removed in 0.35 (#8105)** [verified from changelog]. Replacement is `egui::MenuBar::new().ui(...)` and `ui.close()` (0.32 added `Ui::close`, #5729) [replacement names from memory, verify in phase 1]. Behavior change too: menus now close on click by default, so most `close_menu` calls simply disappear.
2. **`painter.rect_stroke` requires a `StrokeKind` parameter** — 0.31 (#5648) [verified from changelog]. One site: app.rs:2601.
3. **`Rounding` → `CornerRadius`, stored as `u8`** — 0.31 (#5673, #5563) [verified from changelog]. No `Rounding` type usage here, but the literal `1.0`/`0.0` radius arguments at 2033/2600/2601 must satisfy `impl Into<CornerRadius>`; whether `From<f32>` exists is [from memory, verify in phase 1].
4. **`Frame::none()`** (app.rs:2072) — replaced by `Frame::NONE` / `Frame::new()` around the 0.31 Frame rework (#5575) [from memory, verify in phase 1]; if it carried a `#[deprecated]`, the 0.35 purge (#8105) removed it.
5. **wgpu must move 22.1.0 → 29.x.** [verified from egui-wgpu changelog]: egui-wgpu 0.30→wgpu 23, 0.31→24, 0.32→25, 0.33→26 then 27, 0.34→28 then 29, 0.34.2→29.0.1, 0.35 stays. That is a seven-major-version jump touching `main.rs:21–33` (`DeviceDescriptor` fields have churned across those releases) and the entire GPU surface of `sim.rs` (buffers, bind groups, ~11 pipelines, `RenderPass`/`ComputePass` lifetimes). This is the largest risk to phase 1's "roughly 40 changed lines outside Cargo.toml" budget — the CallbackTrait shape itself survives (no signature changes reported 0.30–0.35 [verified from changelog], 0.34 "attach stencil buffer" noted), but wgpu-side types inside sim.rs may not.

**Deprecations reported for 0.34 with large potential blast radius — verify in phase 1 before trusting:**

6. Changelog fetch reports 0.34 deprecates `SidePanel`/`TopBottomPanel` in favor of a unified `Panel` (#5659), deprecates showing panels directly on `Context` (#7781) and `CentralPanel::show` (#7783), renames `Context::style` → `global_style` (#7772), and moves eframe toward `fn logic`/`fn ui` replacing `App::update` (#7775). If accurate, every panel call site (1039, 1276, 1876, 2041, 2071) and the `eframe::App` impl are affected, and the 0.35 deprecation purge may have removed the old forms. The PR-number/version pairing looks suspect (e.g. #5659 is an old PR), and 0.34/0.35 post-date solid training coverage — **[from changelog fetch, low confidence on version attribution; verify in phase 1 before planning the upgrade diff]**.
7. `SelectableLabel` widget struct removed (#7277) [verified from changelog, version attribution unclear]. The code uses the `ui.selectable_label()` *method*, which remains (rebuilt on Button/atoms in 0.32).

**Non-issues (checked explicitly):**

- `id_source` → `id_salt`: renamed in 0.29 itself (#5025), aliases removed 0.35 (#8105) [verified from changelog]. **The code never uses `id_source`** — `Grid::new`/`Window::new` take ids directly and `ComboBox::from_label` is used, so nothing to do.
- `ComboBox::from_label` ×4 — unchanged through 0.35 (only `from_id_source`→`from_id_salt` churn) [verified from changelog].
- `Slider::text` — no reported changes through 0.35 [verified from changelog].
- `Shape::line` / `Shape::closed_line` — no reported changes [verified from changelog].
- `Button::new` ×2, `ui.button` ×17: 0.32's Atom/AtomLayout rewrite (plan's stated hot spot) changed Button's *implementation* and extended the API; the constructor forms used here survive [verified from changelog + memory]. Expect visual diffs (padding, truncation), not compile errors, at these sites.
- `Response::clicked_by` change in 0.30 (#4192) — not used (only `clicked`, `drag_started_by`, `dragged_by`).
- `screen_rect` deprecation in 0.33 (#7578) — not used.
- `Area::new(Id)` change in 0.30 — `Area` not used directly.
- winit 0.30.13 is already in eframe 0.30+'s range; eframe manages it.

---

## Item 6 — the two prior UI decisions, and where the plan reverses them

Commit numbering used by the plan maps to FlowPaint-era commits counted from `a11a022` ("Add FlowPaint", #1): **#7 = `1e76f5a`**, **#10 = `146e797`**. Verified against `git log`.

### Commit 1e76f5a — "Move everyday controls to the side panel; menu bar keeps rare ops" (2026-08-08, 52+/24− in app.rs + README)

What it deliberately decided (from message + full diff):

- **Frequency-based split**: everyday controls (scene presets, particle count) moved *out of menus into the side panel*; menu bar retained *only* rare operations — File (new/open/save/export/quit), Edit (undo/redo/reset/clear), Simulation (grid resolution, domain margin), Help.
- **Deleted the View menu entirely** once emptied (particle selector became a side-panel combo box under a "View" heading).
- Presets became short side-panel buttons with hover descriptions; particle count became `ComboBox::from_label("particles")`.
- **Introduced the single scrolling control column**: wrapped the panel in `ScrollArea::vertical` and factored out `side_panel_contents` so "the fuller layout works at small window heights" — the exact structure still present at app.rs:1275–1284.

### Commit 146e797 — "FlowPaint V2 - object-model sketching rebuild" (2026-08-10, 6 files, 2661+/2820−)

What it deliberately decided (from message + README diff + code):

- Replaced the MS-Paint raster model with a **persistent vector object model**: "everything drawn stays a live, selectable, editable object"; the grid is a damage-region re-projection; README rewrite states the core invariant — *"nothing is ever 'flattened' or destructively committed."*
- **Tool set changed from Brush/Line/Rect/Ellipse/Polyline/Eraser/Select to Select/Line/Rect/Ellipse/Polyline/Pencil** (Pencil = RDP-simplified polyline replacing Brush).
- Undo changed from region snapshots to index-based per-object undo/redo with slider coalescing.
- Right-drag repurposed: from "erase with any tool" to "finish polyline / clear selection" (current app.rs:2203–2213).
- Per-object property editing (material, thickness, fan physics, smoke color) moved into an Object panel; scene files became resolution-independent v3.

### Eraser history — factual findings

- **Pre-rebuild (`61368c8`)**: `git show 61368c8:FlowPaint/src/app.rs` has `Tool::Eraser` (line 20; palette entry line 31, key "X"), a `Material::Erase` path (`if self.erase || self.tool == Tool::Eraser { Material::Erase }`, lines 234–235), right-drag-erases-with-any-tool (lines 2419–2429, shortcut help line 3112). Erasing was **raster subtraction**: painting the Erase material into the grid.
- **Rebuild (`146e797`)**: `git show 146e797:FlowPaint/src/app.rs | grep -i erase` returns **nothing** — the rebuild removed Eraser and Brush entirely, and its README diff deletes the advertised line "**Eraser** (X). Right-drag erases with any tool."
- **Current HEAD**: `grep -i erase FlowPaint/src/*.rs` returns nothing; the `Tool` enum (app.rs:16–23) is Select/Line/Rect/Ellipse/Polyline/Pencil. The only removal path is whole-object delete.
- Conclusion for phase 6a step 1: **the eraser was dropped in commit #10, exactly where the plan says to look first; no bisect needed.** It was not "broken" later — the raster-erase mechanism was incompatible with the new never-flattened object model and was removed with the raster canvas, without a vector-era replacement. (Whether its omission was an explicit decision or an unfinished gap, the commit message and README rewrite show it was a knowing removal: the feature list was edited to drop it.)

### Plan items that reverse these decisions (explicit list)

1. **Phase 3 ribbon reverses 1e76f5a's central decision.** "Everyday controls live in the side panel" becomes "everyday controls live in ribbon tabs": pause/reset/undo/save (Home), tools/materials/thickness (Geometry), solver/fluid/Mach (Physics), presets/generators (Study), field/particles/gain (Results). The single scrolling control column that 1e76f5a created (`side_panel_contents` + ScrollArea) is eliminated. The *principle* of 1e76f5a — menus keep only rare ops — is preserved, but its *arrangement* is fully reversed.
2. **Phase 3 re-adds a View menu** (plan line 132: "File, Edit, Simulation, View, Help") that 1e76f5a deliberately deleted as empty after moving its contents to the side panel.
3. **Scene presets and particle count move again** — presets from the side panel (where 1e76f5a moved them *from a menu*) into the Study ribbon tab; particle count from the side-panel View section into the Results tab. Both are second relocations of controls 1e76f5a placed on purpose.
4. **Phase 3's model tree + settings panel "replaces the current flat object flow entirely"** — this replaces 146e797's Object-panel arrangement but is *consistent with* its object model (the tree is built on the very `SketchObject` list the rebuild created). Not a reversal of the data model; a reversal of its presentation.
5. **Phase 6a (eraser) and the Geometry-tab eraser button reinstate a capability commit #10 removed.** The old eraser's mechanism (raster subtraction into the grid) cannot return verbatim: it violates the rebuild's stated invariant that nothing is destructively committed and objects stay editable. Any new eraser must be a per-object operation (the plan's boolean-subtract or raster-mask options both are), and per 146e797's undo redesign it must produce clean per-object undo entries. Reporting only, per instructions: the plan itself acknowledges this tension by requiring diagnosis before design and undo integration.
6. **Not reversed, worth stating**: the plan keeps 1e76f5a's "menu bar keeps rare ops" rule, keeps 146e797's object model, selection-driven property editing, and per-object undo. Undo/redo appear in both the Edit menu (1e76f5a) and the current side panel (app.rs:1316–1324); the plan moves the buttons to the Home ribbon while the Edit menu remains — same duplication pattern as today, relocated.
