//! The FlowPaint application: an MS-Paint-style shell (menu bar, tool
//! palette, status bar) around the GPU fluid canvas.

use crate::geometry::{BrushContext, GeoRegion, GridRect, Material, Preset, UndoEntry};
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

/// An in-progress drag on the canvas.
struct DragState {
    start_cell: [f32; 2],
    last_cell: [f32; 2],
    erase: bool,
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
    // after the UI pass; the union rect and the stroke-start snapshot are
    // only valid once the commands actually run.
    pending_stroke_rect: GridRect,
    pending_stroke_before: Option<GeoRegion>,
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
            pending_stroke_before: None,
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

    fn active_material(&self, erase: bool) -> Material {
        if erase || self.tool == Tool::Eraser {
            Material::Erase
        } else {
            self.material
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

fn apply_cmd(sim: &mut GpuSim, cmd: Cmd, app: &mut FlowPaintApp) {
    match cmd {
        Cmd::TogglePause => sim.settings.paused = !sim.settings.paused,
        Cmd::ResetFlow => sim.reset_flow(),
        Cmd::ClearAll => sim.clear_all(),
        Cmd::SetWindTunnel(on) => sim.set_wind_tunnel(on),
        Cmd::Preset(p) => sim.apply_preset(p),
        Cmd::SetResolution(i) => sim.set_resolution(i),
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
            app.pending_stroke_before =
                Some(sim.geo.extract(GridRect::full(sim.geo.w, sim.geo.h)));
        }
        Cmd::StrokeEnd => {
            let rect = app.pending_stroke_rect.clampped(sim.geo.w, sim.geo.h);
            app.pending_stroke_rect = GridRect::empty();
            let before_full = app.pending_stroke_before.take();
            if rect.is_empty() {
                return;
            }
            // Crop the stroke-start snapshot to the painted union rect.
            if let Some(full) = before_full {
                let before = crop_region(&full, rect);
                let after = sim.geo.extract(rect);
                sim.undo.push(UndoEntry { before, after });
            }
        }
        Cmd::SetMapping(m) => {
            sim.mapping = m;
            sim.write_render_uniform();
        }
    }
}

/// Crop a full-grid region snapshot down to `rect`.
fn crop_region(full: &GeoRegion, rect: GridRect) -> GeoRegion {
    let (fx0, fy0, fx1, _fy1) = full.rect;
    let fw = (fx1 - fx0) as usize;
    let rw = (rect.x1 - rect.x0) as usize;
    let rh = (rect.y1 - rect.y0) as usize;
    let mut cell = Vec::with_capacity(rw * rh);
    let mut fan = Vec::with_capacity(rw * rh);
    let mut dye_src = Vec::with_capacity(rw * rh);
    for y in rect.y0..rect.y1 {
        let src_row = ((y - fy0) as usize) * fw;
        for x in rect.x0..rect.x1 {
            let s = src_row + (x - fx0) as usize;
            cell.push(full.cell[s]);
            fan.push(full.fan[s]);
            dye_src.push(full.dye_src[s]);
        }
    }
    GeoRegion { rect: (rect.x0, rect.y0, rect.x1, rect.y1), cell, fan, dye_src }
}

// --- UI sections -----------------------------------------------------

impl FlowPaintApp {
    fn keyboard(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        if ctx.wants_keyboard_input() {
            return;
        }
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
                ui.radio_value(&mut self.material, m, label).on_hover_text(tip);
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
                let response =
                    ui.allocate_rect(rect, egui::Sense::click_and_drag());

                let ppp = ctx.pixels_per_point();
                let (gw, gh) = self.stats_grid;
                if gw == 0 || gh == 0 {
                    return;
                }
                let mapping = ViewportMapping::fit(
                    [rect.min.x * ppp, rect.min.y * ppp],
                    [rect.width() * ppp, rect.height() * ppp],
                    gw,
                    gh,
                );
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
                self.drag = Some(DragState { start_cell: cell, last_cell: cell, erase });
                if matches!(self.tool, Tool::Brush | Tool::Eraser) {
                    cmds.push(Cmd::StrokeBegin);
                    let material = self.active_material(erase);
                    cmds.push(Cmd::StampSegment {
                        a: cell,
                        b: cell,
                        r: self.brush_radius,
                        material,
                    });
                }
            }
        }

        if response.dragged() {
            if let Some(pos) = pointer {
                let cell = to_cell(pos);
                if self.drag.is_some() {
                    let freehand = matches!(self.tool, Tool::Brush | Tool::Eraser);
                    let (a, erase_flag) = {
                        let drag = self.drag.as_mut().unwrap();
                        let a = drag.last_cell;
                        drag.last_cell = cell;
                        (a, drag.erase)
                    };
                    if freehand {
                        // Fans blow along the stroke direction.
                        let dx = cell[0] - a[0];
                        let dy = cell[1] - a[1];
                        let len = (dx * dx + dy * dy).sqrt();
                        if len > 1.0 && self.material == Material::Fan {
                            self.fan_dir = [dx / len, dy / len];
                        }
                        let material = self.active_material(erase_flag);
                        cmds.push(Cmd::StampSegment {
                            a,
                            b: cell,
                            r: self.brush_radius,
                            material,
                        });
                    }
                }
            }
        }

        if response.drag_stopped() {
            if let Some(drag) = self.drag.take() {
                match self.tool {
                    Tool::Brush | Tool::Eraser => {
                        cmds.push(Cmd::StrokeEnd);
                    }
                    Tool::Line | Tool::Rect | Tool::Ellipse => {
                        // Line direction sets the fan direction.
                        if self.material == Material::Fan && self.tool == Tool::Line {
                            let dx = drag.last_cell[0] - drag.start_cell[0];
                            let dy = drag.last_cell[1] - drag.start_cell[1];
                            let len = (dx * dx + dy * dy).sqrt();
                            if len > 1.0 {
                                self.fan_dir = [dx / len, dy / len];
                            }
                        }
                        let material = self.active_material(drag.erase);
                        cmds.push(Cmd::ShapeCommit {
                            tool: self.tool,
                            a: drag.start_cell,
                            b: drag.last_cell,
                            r: self.brush_radius,
                            material,
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

        // Shape preview while dragging.
        if let Some(drag) = &self.drag {
            let a = cell_to_pos(drag.start_cell);
            let b = cell_to_pos(drag.last_cell);
            match self.tool {
                Tool::Line => {
                    painter.line_segment([a, b], stroke);
                }
                Tool::Rect => {
                    painter.rect_stroke(egui::Rect::from_two_pos(a, b), 0.0, stroke);
                }
                Tool::Ellipse => {
                    let rect = egui::Rect::from_two_pos(a, b);
                    // egui has no ellipse primitive; approximate with a
                    // polyline.
                    let c = rect.center();
                    let rx = rect.width() * 0.5;
                    let ry = rect.height() * 0.5;
                    let pts: Vec<egui::Pos2> = (0..=48)
                        .map(|i| {
                            let t = i as f32 / 48.0 * std::f32::consts::TAU;
                            egui::pos2(c.x + rx * t.cos(), c.y + ry * t.sin())
                        })
                        .collect();
                    painter.add(egui::Shape::line(pts, stroke));
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
