//! The FlowPaint application: an MS-Paint-style shell (menu bar, tool
//! palette, status bar) around the GPU fluid canvas.

use crate::geometry::{
    BrushContext, GeoRegion, Geometry, GridRect, Material, Preset, UndoEntry,
};
use crate::sim::{
    GpuSim, RenderMode, ViewportMapping, PARTICLE_CHOICES, RESOLUTIONS,
};
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Brush,
    Line,
    Rect,
    Ellipse,
    Eraser,
}

impl Tool {
    const ALL: [(Tool, &'static str, &'static str); 5] = [
        (Tool::Brush, "Brush", "B"),
        (Tool::Line, "Line", "L"),
        (Tool::Rect, "Rectangle", "R"),
        (Tool::Ellipse, "Ellipse", "E"),
        (Tool::Eraser, "Eraser", "X"),
    ];
}

/// An in-progress drag on the canvas. The tool, material and radius are
/// captured at drag start so switching them mid-drag (hotkeys, panel)
/// cannot change or desynchronize an in-flight stroke.
struct DragState {
    start_cell: [f32; 2],
    last_cell: [f32; 2],
    erase: bool,
    tool: Tool,
    material: Material,
    radius: f32,
    /// Fan brush strokes defer their first stamp until the drag direction
    /// is known, so the whole stroke blows the way the pointer moved.
    fan_deferred: bool,
}

impl DragState {
    fn effective_material(&self) -> Material {
        if self.erase || self.tool == Tool::Eraser {
            Material::Erase
        } else {
            self.material
        }
    }
}

/// Settings snapshot read from the sim before building the UI, so panels
/// can show live values without holding the renderer lock.
#[derive(Clone, Copy)]
struct UiSnapshot {
    flow: f32,
    visc: f32,
    steps: u32,
    fade: f32,
    paused: bool,
    tunnel: bool,
    tints: bool,
    can_undo: bool,
    can_redo: bool,
    mode: RenderMode,
}

pub struct FlowPaintApp {
    tool: Tool,
    material: Material,
    brush_radius: f32,
    dye_color: egui::Color32,
    fan_dir: [f32; 2],
    drag: Option<DragState>,
    // Freehand-stroke undo bookkeeping. These live on the app (not the
    // drag state) because canvas events are queued as commands and applied
    // after the UI pass; they are only valid once the commands run.
    // Pre-stroke contents are captured lazily as fixed-size tiles the
    // first time a stamp touches them, so stroke start never has to copy
    // the whole grid.
    pending_stroke_rect: GridRect,
    pending_stroke_tiles: std::collections::HashMap<(i32, i32), GeoRegion>,
    show_about: bool,
    show_shortcuts: bool,
    res_index: usize,
    particle_index: usize,
    status: String,
    // Stats copied out of the sim each frame.
    stats_grid: (usize, usize),
    stats_mlups: f32,
    stats_re: u32,
}

impl FlowPaintApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let rs = cc
            .wgpu_render_state
            .as_ref()
            .expect("FlowPaint needs the wgpu backend");
        let res_index = 2; // High
        let sim = GpuSim::new(
            rs.device.clone(),
            rs.queue.clone(),
            rs.target_format,
            res_index,
        );
        rs.renderer.write().callback_resources.insert(sim);
        Self {
            tool: Tool::Brush,
            material: Material::Wall,
            brush_radius: 8.0,
            dye_color: egui::Color32::from_rgb(90, 217, 255),
            fan_dir: [1.0, 0.0],
            drag: None,
            pending_stroke_rect: GridRect::empty(),
            pending_stroke_tiles: std::collections::HashMap::new(),
            show_about: false,
            show_shortcuts: false,
            res_index,
            particle_index: 2,
            status: String::from("Draw walls with the brush; hold right-click to erase."),
            stats_grid: (0, 0),
            stats_mlups: 0.0,
            stats_re: 0,
        }
    }

    fn brush_ctx(&self) -> BrushContext {
        let c = self.dye_color;
        BrushContext {
            fan_dir: self.fan_dir,
            dye_rgb: [
                c.r() as f32 / 255.0,
                c.g() as f32 / 255.0,
                c.b() as f32 / 255.0,
            ],
        }
    }

}

impl eframe::App for FlowPaintApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.request_repaint(); // continuous simulation

        // Commands gathered from UI this frame, applied to the sim below.
        let mut cmds: Vec<Cmd> = Vec::new();

        // Read the live settings out of the sim for the panels.
        let snapshot = {
            let Some(rs) = frame.wgpu_render_state() else { return };
            let renderer = rs.renderer.read();
            let Some(sim) = renderer.callback_resources.get::<GpuSim>() else { return };
            UiSnapshot {
                flow: sim.settings.flow_speed,
                visc: sim.settings.viscosity,
                steps: sim.settings.steps_per_frame,
                fade: sim.settings.dye_fade,
                paused: sim.settings.paused,
                tunnel: sim.settings.wind_tunnel,
                tints: sim.settings.boundary_tints,
                can_undo: sim.undo.can_undo(),
                can_redo: sim.undo.can_redo(),
                mode: sim.settings.render_mode,
            }
        };

        self.keyboard(ctx, &mut cmds);
        self.menu_bar(ctx, &mut cmds);
        self.side_panel(ctx, snapshot, &mut cmds);
        self.status_bar(ctx);
        self.canvas(ctx, &mut cmds);
        self.windows(ctx);

        // Apply everything to the sim.
        let Some(rs) = frame.wgpu_render_state() else { return };
        let mut renderer = rs.renderer.write();
        let Some(sim) = renderer.callback_resources.get_mut::<GpuSim>() else { return };

        for cmd in cmds {
            apply_cmd(sim, cmd, self);
        }
        sim.flush_geometry();

        // Copy stats out for the status bar next frame.
        self.stats_grid = sim.grid_size();
        let dt = ctx.input(|i| i.stable_dt).max(1e-4);
        let n = (self.stats_grid.0 * self.stats_grid.1) as f32;
        self.stats_mlups = n * sim.steps_last_frame as f32 / dt / 1.0e6;
        self.stats_re = sim.reynolds_estimate();
    }
}

/// Everything the UI can ask the sim to do this frame.
enum Cmd {
    TogglePause,
    ResetFlow,
    ClearAll,
    SetWindTunnel(bool),
    Preset(Preset),
    SetResolution(usize),
    SetParticles(u32),
    SetRenderMode(RenderMode),
    SetFlowSpeed(f32),
    SetViscosity(f32),
    SetSteps(u32),
    SetDyeFade(f32),
    SetBoundaryTints(bool),
    Undo,
    Redo,
    SaveScene(std::path::PathBuf),
    LoadScene(std::path::PathBuf),
    ExportPng(std::path::PathBuf),
    // Painting.
    StampSegment { a: [f32; 2], b: [f32; 2], r: f32, material: Material },
    ShapeCommit { tool: Tool, a: [f32; 2], b: [f32; 2], r: f32, material: Material },
    StrokeBegin,
    StrokeEnd,
    SetMapping(ViewportMapping),
}

/// Tile edge length for lazy pre-stroke capture.
const UNDO_TILE: i32 = 64;

/// Drop any half-tracked stroke state (used when the grid is replaced or
/// wholesale-cleared so stale snapshots can never pair with a new grid).
fn clear_stroke_state(app: &mut FlowPaintApp) {
    app.pending_stroke_rect = GridRect::empty();
    app.pending_stroke_tiles.clear();
    app.drag = None;
}

fn apply_cmd(sim: &mut GpuSim, cmd: Cmd, app: &mut FlowPaintApp) {
    match cmd {
        Cmd::TogglePause => sim.settings.paused = !sim.settings.paused,
        Cmd::ResetFlow => sim.reset_flow(),
        Cmd::ClearAll => {
            clear_stroke_state(app);
            sim.clear_all();
        }
        Cmd::SetWindTunnel(on) => sim.set_wind_tunnel(on),
        Cmd::Preset(p) => {
            clear_stroke_state(app);
            sim.apply_preset(p);
        }
        Cmd::SetResolution(i) => {
            clear_stroke_state(app);
            sim.set_resolution(i);
        }
        Cmd::SetParticles(nn) => sim.settings.particle_count = nn,
        Cmd::SetRenderMode(m) => sim.settings.render_mode = m,
        Cmd::SetFlowSpeed(v) => sim.settings.flow_speed = v,
        Cmd::SetViscosity(v) => sim.settings.viscosity = v,
        Cmd::SetSteps(v) => sim.settings.steps_per_frame = v,
        Cmd::SetDyeFade(v) => sim.settings.dye_fade = v,
        Cmd::SetBoundaryTints(v) => sim.settings.boundary_tints = v,
        Cmd::Undo => sim.undo_action(),
        Cmd::Redo => sim.redo_action(),
        Cmd::SaveScene(p) => {
            app.status = match sim.save_scene(&p) {
                Ok(()) => format!("Saved {}", p.display()),
                Err(e) => format!("Save failed: {e}"),
            };
        }
        Cmd::LoadScene(p) => {
            clear_stroke_state(app);
            app.status = match sim.load_scene(&p) {
                Ok(()) => format!("Loaded {}", p.display()),
                Err(e) => format!("Load failed: {e}"),
            };
        }
        Cmd::ExportPng(p) => {
            app.status = match sim.export_png(&p) {
                Ok(()) => format!("Exported {}", p.display()),
                Err(e) => format!("Export failed: {e}"),
            };
        }
        Cmd::StampSegment { a, b, r, material } => {
            // Capture the pre-stroke contents of every tile this stamp can
            // touch before stamping (first touch wins, so tiles keep their
            // true pre-stroke data for the whole stroke).
            let bound =
                Geometry::capsule_bounds(a, b, r).clampped(sim.geo.w, sim.geo.h);
            if !bound.is_empty() {
                for ty in (bound.y0 / UNDO_TILE)..=((bound.y1 - 1) / UNDO_TILE) {
                    for tx in (bound.x0 / UNDO_TILE)..=((bound.x1 - 1) / UNDO_TILE) {
                        app.pending_stroke_tiles.entry((tx, ty)).or_insert_with(|| {
                            sim.geo.extract(GridRect {
                                x0: tx * UNDO_TILE,
                                y0: ty * UNDO_TILE,
                                x1: (tx + 1) * UNDO_TILE,
                                y1: (ty + 1) * UNDO_TILE,
                            })
                        });
                    }
                }
            }
            let ctx = app.brush_ctx();
            let rect = sim.geo.stamp_capsule(a, b, r, material, &ctx);
            app.pending_stroke_rect = app.pending_stroke_rect.union(rect);
        }
        Cmd::ShapeCommit { tool, a, b, r, material } => {
            let ctx = app.brush_ctx();
            // Dense undo for one-shot shapes.
            let bound = match tool {
                Tool::Line => GridRect {
                    x0: (a[0].min(b[0]) - r).floor() as i32,
                    y0: (a[1].min(b[1]) - r).floor() as i32,
                    x1: (a[0].max(b[0]) + r).ceil() as i32 + 1,
                    y1: (a[1].max(b[1]) + r).ceil() as i32 + 1,
                },
                _ => GridRect {
                    x0: a[0].min(b[0]).floor() as i32 - 1,
                    y0: a[1].min(b[1]).floor() as i32 - 1,
                    x1: a[0].max(b[0]).ceil() as i32 + 1,
                    y1: a[1].max(b[1]).ceil() as i32 + 1,
                },
            }
            .clampped(sim.geo.w, sim.geo.h);
            if bound.is_empty() {
                return;
            }
            let before = sim.geo.extract(bound);
            match tool {
                Tool::Line => sim.geo.stamp_capsule(a, b, r, material, &ctx),
                Tool::Rect => sim.geo.stamp_rect(a, b, material, &ctx),
                Tool::Ellipse => sim.geo.stamp_ellipse(a, b, material, &ctx),
                _ => bound,
            };
            let after = sim.geo.extract(bound);
            sim.undo.push(UndoEntry { before, after });
        }
        Cmd::StrokeBegin => {
            app.pending_stroke_rect = GridRect::empty();
            app.pending_stroke_tiles.clear();
        }
        Cmd::StrokeEnd => {
            let rect = app.pending_stroke_rect.clampped(sim.geo.w, sim.geo.h);
            app.pending_stroke_rect = GridRect::empty();
            let tiles = std::mem::take(&mut app.pending_stroke_tiles);
            if rect.is_empty() {
                return;
            }
            let before = assemble_before(&sim.geo, &tiles, rect);
            let after = sim.geo.extract(rect);
            sim.undo.push(UndoEntry { before, after });
        }
        Cmd::SetMapping(m) => {
            // Refit against the sim's authoritative grid size: on a
            // resolution-switch frame the UI computed this mapping from
            // last frame's dimensions.
            sim.mapping =
                ViewportMapping::fit(m.vp_origin, m.vp_size, sim.geo.w, sim.geo.h);
            sim.write_render_uniform();
        }
    }
}

/// Build the pre-stroke contents of `rect` from the lazily captured tiles.
/// Cells of `rect` never touched by a stamp keep their current contents,
/// which equal their pre-stroke contents by definition.
fn assemble_before(
    geo: &Geometry,
    tiles: &std::collections::HashMap<(i32, i32), GeoRegion>,
    rect: GridRect,
) -> GeoRegion {
    let mut before = geo.extract(rect);
    let rw = (rect.x1 - rect.x0) as usize;
    for (&(tx, ty), tile) in tiles {
        let (t_x0, t_y0, t_x1, t_y1) = tile.rect;
        debug_assert_eq!((t_x0, t_y0), (tx * UNDO_TILE, ty * UNDO_TILE));
        let tw = (t_x1 - t_x0) as usize;
        // Intersection of the tile with the stroke rect.
        let ix0 = rect.x0.max(t_x0);
        let iy0 = rect.y0.max(t_y0);
        let ix1 = rect.x1.min(t_x1);
        let iy1 = rect.y1.min(t_y1);
        for y in iy0..iy1 {
            let src_row = ((y - t_y0) as usize) * tw;
            let dst_row = ((y - rect.y0) as usize) * rw;
            for x in ix0..ix1 {
                let s = src_row + (x - t_x0) as usize;
                let d = dst_row + (x - rect.x0) as usize;
                before.cell[d] = tile.cell[s];
                before.fan[d] = tile.fan[s];
                before.dye_src[d] = tile.dye_src[s];
            }
        }
    }
    before
}

// --- UI sections -----------------------------------------------------

impl FlowPaintApp {
    fn keyboard(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        if ctx.wants_keyboard_input() {
            return;
        }
        // Escape cancels an in-progress drag: shapes are dropped before
        // they commit; freehand strokes just end (their paint is already
        // down, so finalize the undo entry).
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if let Some(drag) = self.drag.take() {
                if matches!(drag.tool, Tool::Brush | Tool::Eraser) {
                    cmds.push(Cmd::StrokeEnd);
                }
            }
        }
        let dragging = self.drag.is_some();
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                cmds.push(Cmd::TogglePause);
            }
            if i.modifiers.command && i.key_pressed(egui::Key::Z) {
                if i.modifiers.shift {
                    cmds.push(Cmd::Redo);
                } else {
                    cmds.push(Cmd::Undo);
                }
            }
            if i.modifiers.command && i.key_pressed(egui::Key::Y) {
                cmds.push(Cmd::Redo);
            }
            // Tool switching is disabled mid-drag; the drag carries its own
            // tool, and switching now would only confuse the preview.
            if !dragging {
                if i.key_pressed(egui::Key::B) {
                    self.tool = Tool::Brush;
                }
                if i.key_pressed(egui::Key::L) {
                    self.tool = Tool::Line;
                }
                if i.key_pressed(egui::Key::R) {
                    self.tool = Tool::Rect;
                }
                if i.key_pressed(egui::Key::E) {
                    self.tool = Tool::Ellipse;
                }
                if i.key_pressed(egui::Key::X) {
                    self.tool = Tool::Eraser;
                }
            }
            if i.key_pressed(egui::Key::OpenBracket) {
                self.brush_radius = (self.brush_radius - 2.0).max(1.0);
            }
            if i.key_pressed(egui::Key::CloseBracket) {
                self.brush_radius = (self.brush_radius + 2.0).min(64.0);
            }
        });
    }

    fn menu_bar(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New (clear everything)").clicked() {
                        cmds.push(Cmd::ClearAll);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Open scene…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("FlowPaint scene", &["flow"])
                            .pick_file()
                        {
                            cmds.push(Cmd::LoadScene(p));
                        }
                        ui.close_menu();
                    }
                    if ui.button("Save scene…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("FlowPaint scene", &["flow"])
                            .set_file_name("scene.flow")
                            .save_file()
                        {
                            cmds.push(Cmd::SaveScene(p));
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Export view as PNG…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("PNG image", &["png"])
                            .set_file_name("flowpaint.png")
                            .save_file()
                        {
                            cmds.push(Cmd::ExportPng(p));
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Undo        Ctrl+Z").clicked() {
                        cmds.push(Cmd::Undo);
                        ui.close_menu();
                    }
                    if ui.button("Redo        Ctrl+Y").clicked() {
                        cmds.push(Cmd::Redo);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Reset flow (keep drawing)").clicked() {
                        cmds.push(Cmd::ResetFlow);
                        ui.close_menu();
                    }
                    if ui.button("Clear everything").clicked() {
                        cmds.push(Cmd::ClearAll);
                        ui.close_menu();
                    }
                });
                ui.menu_button("Simulation", |ui| {
                    ui.menu_button("Presets", |ui| {
                        for p in Preset::ALL {
                            if ui.button(p.label()).clicked() {
                                cmds.push(Cmd::Preset(p));
                                ui.close_menu();
                            }
                        }
                    });
                    ui.menu_button("Grid resolution", |ui| {
                        for (i, (label, _, _)) in RESOLUTIONS.iter().enumerate() {
                            if ui
                                .radio(i == self.res_index, *label)
                                .clicked()
                            {
                                self.res_index = i;
                                cmds.push(Cmd::SetResolution(i));
                                ui.close_menu();
                            }
                        }
                    });
                });
                ui.menu_button("View", |ui| {
                    ui.menu_button("Particles", |ui| {
                        for (i, (label, count)) in PARTICLE_CHOICES.iter().enumerate() {
                            if ui.radio(i == self.particle_index, *label).clicked() {
                                self.particle_index = i;
                                cmds.push(Cmd::SetParticles(*count));
                                ui.close_menu();
                            }
                        }
                    });
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("Keyboard shortcuts").clicked() {
                        self.show_shortcuts = true;
                        ui.close_menu();
                    }
                    if ui.button("About FlowPaint").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    fn side_panel(&mut self, ctx: &egui::Context, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        egui::SidePanel::left("tools").min_width(210.0).show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading("Tools");
            ui.horizontal_wrapped(|ui| {
                for (tool, label, key) in Tool::ALL {
                    let selected = self.tool == tool;
                    if ui
                        .selectable_label(selected, format!("{label} ({key})"))
                        .clicked()
                    {
                        self.tool = tool;
                    }
                }
            });

            ui.add_space(6.0);
            ui.separator();
            ui.heading("Material");
            let mats: [(Material, &str, &str); 4] = [
                (Material::Wall, "Wall", "Solid, no-slip"),
                (Material::Fan, "Fan", "Blows along your stroke"),
                (Material::Smoke, "Smoke", "Passive dye emitter"),
                (Material::Drain, "Drain", "Lets flow leave"),
            ];
            for (m, label, tip) in mats {
                let resp =
                    ui.radio_value(&mut self.material, m, label).on_hover_text(tip);
                // Smoke is only visible in the Smoke view; switch so the
                // first stroke gives immediate feedback.
                if resp.clicked() && m == Material::Smoke {
                    cmds.push(Cmd::SetRenderMode(RenderMode::Dye));
                }
            }
            if self.material == Material::Smoke || self.material == Material::Fan {
                ui.horizontal(|ui| {
                    ui.label("Smoke color:");
                    ui.color_edit_button_srgba(&mut self.dye_color);
                });
            }

            ui.add_space(6.0);
            ui.separator();
            ui.heading("Brush");
            ui.add(
                egui::Slider::new(&mut self.brush_radius, 1.0..=64.0)
                    .text("radius (cells)"),
            );

            ui.add_space(6.0);
            ui.separator();
            ui.heading("View");
            // Local mirrors so widgets show the live values.
            let mut flow = snap.flow;
            let mut visc = snap.visc;
            let mut steps = snap.steps;
            let mut fade = snap.fade;
            let mut tunnel = snap.tunnel;
            let mut tints = snap.tints;
            let paused = snap.paused;
            let (can_undo, can_redo) = (snap.can_undo, snap.can_redo);

            ui.horizontal_wrapped(|ui| {
                for m in RenderMode::ALL {
                    if ui.selectable_label(snap.mode == m, m.label()).clicked() {
                        cmds.push(Cmd::SetRenderMode(m));
                    }
                }
            });
            if ui.checkbox(&mut tints, "Highlight fans && drains").changed() {
                cmds.push(Cmd::SetBoundaryTints(tints));
            }

            ui.add_space(6.0);
            ui.separator();
            ui.heading("Physics");
            if ui
                .add(egui::Slider::new(&mut flow, 0.02..=0.14).text("flow speed"))
                .changed()
            {
                cmds.push(Cmd::SetFlowSpeed(flow));
            }
            if ui
                .add(
                    egui::Slider::new(&mut visc, 0.005..=0.08)
                        .logarithmic(true)
                        .text("viscosity"),
                )
                .changed()
            {
                cmds.push(Cmd::SetViscosity(visc));
            }
            if ui
                .add(egui::Slider::new(&mut steps, 1..=32).text("steps / frame"))
                .changed()
            {
                cmds.push(Cmd::SetSteps(steps));
            }
            if ui
                .add(egui::Slider::new(&mut fade, 0.985..=1.0).text("smoke persistence"))
                .changed()
            {
                cmds.push(Cmd::SetDyeFade(fade));
            }
            if ui.checkbox(&mut tunnel, "Wind tunnel (left to right)").changed() {
                cmds.push(Cmd::SetWindTunnel(tunnel));
            }

            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button(if paused { "▶ Resume" } else { "⏸ Pause" })
                    .clicked()
                {
                    let _ = paused;
                    cmds.push(Cmd::TogglePause);
                }
                if ui.button("Reset flow").clicked() {
                    cmds.push(Cmd::ResetFlow);
                }
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(can_undo, egui::Button::new("↶ Undo")).clicked() {
                    cmds.push(Cmd::Undo);
                }
                if ui.add_enabled(can_redo, egui::Button::new("↷ Redo")).clicked() {
                    cmds.push(Cmd::Redo);
                }
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "grid {} x {}   |   {:.0} MLUPS   |   Re ≈ {}",
                        self.stats_grid.0, self.stats_grid.1, self.stats_mlups, self.stats_re
                    ));
                });
            });
        });
    }

    fn canvas(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                // Sense::drag (not click_and_drag): drags then start on the
                // press frame at the press position, so a plain click
                // stamps a dot and strokes anchor exactly where the user
                // pressed instead of ~6 pt into the gesture.
                let response = ui.allocate_rect(rect, egui::Sense::drag());

                let ppp = ctx.pixels_per_point();
                let (gw, gh) = self.stats_grid;
                if gw == 0 || gh == 0 {
                    return;
                }
                // Round to whole physical pixels the same way egui rounds
                // the render-pass viewport, so the particle overlay's NDC
                // math matches the viewport the GPU actually uses.
                let x0 = (rect.min.x * ppp).round();
                let y0 = (rect.min.y * ppp).round();
                let x1 = (rect.max.x * ppp).round();
                let y1 = (rect.max.y * ppp).round();
                let mapping =
                    ViewportMapping::fit([x0, y0], [x1 - x0, y1 - y0], gw, gh);
                cmds.push(Cmd::SetMapping(mapping));

                self.canvas_interaction(&response, mapping, ppp, cmds);

                // The simulation paints itself via the wgpu callback.
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    FlowPaintCallback,
                ));

                self.canvas_overlays(ui, &response, mapping, ppp);
            });
    }

    fn canvas_interaction(
        &mut self,
        response: &egui::Response,
        mapping: ViewportMapping,
        ppp: f32,
        cmds: &mut Vec<Cmd>,
    ) {
        let to_cell = |pos: egui::Pos2| -> [f32; 2] {
            mapping.px_to_cell([pos.x * ppp, pos.y * ppp])
        };

        let pointer = response.interact_pointer_pos();
        let erase = response.dragged_by(egui::PointerButton::Secondary)
            || response.drag_started_by(egui::PointerButton::Secondary);

        if response.drag_started() {
            if let Some(pos) = pointer {
                let cell = to_cell(pos);
                let freehand = matches!(self.tool, Tool::Brush | Tool::Eraser);
                // Fan brush strokes wait for a direction before stamping.
                let fan_deferred = freehand
                    && !erase
                    && self.tool == Tool::Brush
                    && self.material == Material::Fan;
                let drag = DragState {
                    start_cell: cell,
                    last_cell: cell,
                    erase,
                    tool: self.tool,
                    material: self.material,
                    radius: self.brush_radius,
                    fan_deferred,
                };
                if freehand {
                    cmds.push(Cmd::StrokeBegin);
                    if !fan_deferred {
                        cmds.push(Cmd::StampSegment {
                            a: cell,
                            b: cell,
                            r: drag.radius,
                            material: drag.effective_material(),
                        });
                    }
                }
                self.drag = Some(drag);
            }
        }

        if response.dragged() {
            if let Some(pos) = pointer {
                let cell = to_cell(pos);
                if self.drag.is_some() {
                    let (a, radius, material, tool, deferred, start) = {
                        let drag = self.drag.as_ref().unwrap();
                        (
                            drag.last_cell,
                            drag.radius,
                            drag.effective_material(),
                            drag.tool,
                            drag.fan_deferred,
                            drag.start_cell,
                        )
                    };
                    if matches!(tool, Tool::Brush | Tool::Eraser) {
                        if deferred {
                            // Wait until the pointer has moved a few cells,
                            // then stamp the whole run with that direction.
                            let dx = cell[0] - start[0];
                            let dy = cell[1] - start[1];
                            let len = (dx * dx + dy * dy).sqrt();
                            if len >= 3.0 {
                                self.fan_dir = [dx / len, dy / len];
                                let drag = self.drag.as_mut().unwrap();
                                drag.fan_deferred = false;
                                drag.last_cell = cell;
                                cmds.push(Cmd::StampSegment {
                                    a: start,
                                    b: cell,
                                    r: radius,
                                    material,
                                });
                            }
                        } else {
                            // Fans blow along the stroke direction.
                            let dx = cell[0] - a[0];
                            let dy = cell[1] - a[1];
                            let len = (dx * dx + dy * dy).sqrt();
                            if len > 1.0 && material == Material::Fan {
                                self.fan_dir = [dx / len, dy / len];
                            }
                            self.drag.as_mut().unwrap().last_cell = cell;
                            cmds.push(Cmd::StampSegment { a, b: cell, r: radius, material });
                        }
                    } else {
                        self.drag.as_mut().unwrap().last_cell = cell;
                    }
                }
            }
        }

        if response.drag_stopped() {
            if let Some(drag) = self.drag.take() {
                match drag.tool {
                    Tool::Brush | Tool::Eraser => {
                        if drag.fan_deferred {
                            // A tap: blow along the tunnel axis.
                            self.fan_dir = [1.0, 0.0];
                            cmds.push(Cmd::StampSegment {
                                a: drag.start_cell,
                                b: drag.start_cell,
                                r: drag.radius,
                                material: drag.effective_material(),
                            });
                        }
                        cmds.push(Cmd::StrokeEnd);
                    }
                    Tool::Line | Tool::Rect | Tool::Ellipse => {
                        // Line direction sets the fan direction.
                        if drag.material == Material::Fan && drag.tool == Tool::Line {
                            let dx = drag.last_cell[0] - drag.start_cell[0];
                            let dy = drag.last_cell[1] - drag.start_cell[1];
                            let len = (dx * dx + dy * dy).sqrt();
                            if len > 1.0 {
                                self.fan_dir = [dx / len, dy / len];
                            }
                        }
                        cmds.push(Cmd::ShapeCommit {
                            tool: drag.tool,
                            a: drag.start_cell,
                            b: drag.last_cell,
                            r: drag.radius,
                            material: drag.effective_material(),
                        });
                    }
                }
            }
        }
    }

    fn canvas_overlays(
        &self,
        ui: &egui::Ui,
        response: &egui::Response,
        mapping: ViewportMapping,
        ppp: f32,
    ) {
        let painter = ui.painter();
        let stroke = egui::Stroke::new(1.5, egui::Color32::from_white_alpha(180));
        let cell_to_pos = |c: [f32; 2]| -> egui::Pos2 {
            egui::pos2(
                (mapping.lb_origin[0] + c[0] * mapping.px_per_cell) / ppp,
                (mapping.lb_origin[1] + c[1] * mapping.px_per_cell) / ppp,
            )
        };

        // Brush cursor.
        if let Some(hover) = response.hover_pos() {
            if matches!(self.tool, Tool::Brush | Tool::Eraser | Tool::Line) {
                let r = self.brush_radius * mapping.px_per_cell / ppp;
                painter.circle_stroke(hover, r, stroke);
            }
        }

        // Shape preview while dragging; sized and filled to match what
        // will actually be stamped.
        if let Some(drag) = &self.drag {
            let a = cell_to_pos(drag.start_cell);
            let b = cell_to_pos(drag.last_cell);
            let fill = egui::Color32::from_white_alpha(30);
            match drag.tool {
                Tool::Line => {
                    // Preview at the true committed thickness (a capsule
                    // of radius brush_radius cells).
                    let w = (2.0 * drag.radius * mapping.px_per_cell / ppp).max(1.5);
                    painter.line_segment(
                        [a, b],
                        egui::Stroke::new(w, egui::Color32::from_white_alpha(70)),
                    );
                    painter.line_segment([a, b], stroke);
                }
                Tool::Rect => {
                    let rect = egui::Rect::from_two_pos(a, b);
                    painter.rect_filled(rect, 0.0, fill);
                    painter.rect_stroke(rect, 0.0, stroke);
                }
                Tool::Ellipse => {
                    let rect = egui::Rect::from_two_pos(a, b);
                    // egui has no ellipse primitive; approximate with a
                    // polygon.
                    let c = rect.center();
                    let rx = rect.width() * 0.5;
                    let ry = rect.height() * 0.5;
                    let pts: Vec<egui::Pos2> = (0..48)
                        .map(|i| {
                            let t = i as f32 / 48.0 * std::f32::consts::TAU;
                            egui::pos2(c.x + rx * t.cos(), c.y + ry * t.sin())
                        })
                        .collect();
                    painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
                }
                _ => {}
            }
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        egui::Window::new("About FlowPaint")
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    "FlowPaint solves the 2D Navier-Stokes equations in real \
                     time with a D2Q9 lattice-Boltzmann method running in \
                     GPU compute shaders (wgpu: Vulkan / DX12 / Metal).",
                );
                ui.add_space(6.0);
                ui.label(
                    "Draw walls, place fans and drains, blow smoke through \
                     your design — and hunt for vortex streets. Higher \
                     Reynolds numbers mean livelier flow.",
                );
            });

        egui::Window::new("Keyboard shortcuts")
            .open(&mut self.show_shortcuts)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("shortcuts").striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("Space", "pause / resume"),
                        ("Ctrl+Z / Ctrl+Y", "undo / redo"),
                        ("B / L / R / E / X", "brush / line / rect / ellipse / eraser"),
                        ("[ / ]", "brush size"),
                        ("Right-drag", "erase with any tool"),
                        ("Esc", "cancel an in-progress shape"),
                    ] {
                        ui.label(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
            });
    }
}

// --- The wgpu paint callback -----------------------------------------

struct FlowPaintCallback;

impl egui_wgpu::CallbackTrait for FlowPaintCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(sim) = resources.get_mut::<GpuSim>() {
            sim.encode_compute(encoder);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(sim) = resources.get::<GpuSim>() {
            sim.draw(render_pass);
        }
    }
}
