//! The FlowPaint V2 application shell, rebuilt around a persistent
//! sketch-object model: everything you draw stays a live, selectable,
//! editable vector object; the solver grid is a continuous projection of
//! the model (see model.rs). Panels: tools, object properties, sketch
//! aids, generators, presets, view, physics — plus the legend.

use crate::model::{ObjMaterial, Shape, SketchModel, SketchObject};
use crate::sim::{
    GpuSim, RenderMode, ViewportMapping, DEFAULT_MARGIN_INDEX, MARGIN_CHOICES,
    PARTICLE_CHOICES, RESOLUTIONS,
};
use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Select,
    Line,
    Rect,
    Ellipse,
    Polyline,
    Pencil,
}

impl Tool {
    const ALL: [(Tool, &'static str, &'static str); 6] = [
        (Tool::Select, "Select", "S"),
        (Tool::Line, "Line", "L"),
        (Tool::Rect, "Rectangle", "R"),
        (Tool::Ellipse, "Ellipse", "E"),
        (Tool::Polyline, "Polyline", "P"),
        (Tool::Pencil, "Pencil", "B"),
    ];
}

/// The in-progress pointer gesture.
enum Gesture {
    None,
    /// Rubber-banding a new line/rect/ellipse from its anchor.
    DrawShape { id: u64, anchor: [f32; 2] },
    /// Building a polyline; persists across clicks until Enter/right-click.
    /// The last vertex is a "rubber" point that follows the cursor.
    DrawPoly { id: u64 },
    /// Freehand pencil stroke collecting points.
    DrawPencil { id: u64 },
    /// Moving a whole object. `before` is the object at gesture start (for
    /// the undo record); `last` is the previous effective pointer position.
    MoveObj { id: u64, before: SketchObject, last: [f32; 2] },
    /// Dragging one vertex/corner handle.
    HandleDrag { id: u64, idx: usize, before: SketchObject },
}

/// Scene file (version 3): the vector model plus core settings —
/// resolution-independent by construction.
#[derive(Serialize, Deserialize)]
struct SceneV3 {
    version: u32,
    objects: Vec<SketchObject>,
    wind_tunnel: bool,
    flow_speed: f32,
    viscosity: f32,
    steps_per_frame: u32,
    domain_width_m: f32,
    fluid_nu: f32,
    fluid_rho: f32,
    /// Visible grid width the coordinates are expressed in.
    ref_width: u32,
}

const SCENE_V3: u32 = 3;

/// A fluid/regime preset: maps a named physical situation onto lattice
/// parameters. (The solver is incompressible, so "supersonic" is a
/// stylized maximum-speed, high-Reynolds regime — no shocks.)
struct FluidPreset {
    name: &'static str,
    desc: &'static str,
    tunnel: bool,
    flow: f32,
    visc: f32,
    steps: Option<u32>,
    /// Physical kinematic viscosity [m^2/s].
    nu: f32,
    /// Physical density [kg/m^3].
    rho: f32,
}

const FLUID_PRESETS: [FluidPreset; 7] = [
    FluidPreset {
        name: "Still air",
        desc: "No wind — place fans to stir the room.",
        tunnel: false,
        flow: 0.06,
        visc: 0.02,
        steps: None,
        nu: 1.5e-5,
        rho: 1.2,
    },
    FluidPreset {
        name: "Gentle breeze (air)",
        desc: "Low-speed laminar-ish air, Re in the hundreds.",
        tunnel: true,
        flow: 0.05,
        visc: 0.03,
        steps: None,
        nu: 1.5e-5,
        rho: 1.2,
    },
    FluidPreset {
        name: "Wind tunnel (air)",
        desc: "The default: lively vortex shedding.",
        tunnel: true,
        flow: 0.09,
        visc: 0.015,
        steps: None,
        nu: 1.5e-5,
        rho: 1.2,
    },
    FluidPreset {
        name: "Storm (air, high Re)",
        desc: "Fast, turbulent air.",
        tunnel: true,
        flow: 0.13,
        visc: 0.006,
        steps: None,
        nu: 1.5e-5,
        rho: 1.2,
    },
    FluidPreset {
        name: "Water flume",
        desc: "Water's kinematic viscosity is ~15x lower than air's: \
               high Reynolds numbers at modest speed.",
        tunnel: true,
        flow: 0.07,
        visc: 0.0055,
        steps: None,
        nu: 1.0e-6,
        rho: 998.0,
    },
    FluidPreset {
        name: "Glycerin / syrup",
        desc: "Creeping flow (Re of order 10): smooth, reversible, \
               no vortex shedding.",
        tunnel: true,
        flow: 0.04,
        visc: 0.08,
        steps: None,
        nu: 1.19e-3,
        rho: 1260.0,
    },
    FluidPreset {
        name: "Supersonic tunnel (stylized)",
        desc: "Maximum speed and Reynolds number, extra sub-steps. \
               Stylized: this solver is incompressible, so you get \
               extreme flow but no shocks.",
        tunnel: true,
        flow: 0.14,
        visc: 0.005,
        steps: Some(16),
        nu: 1.5e-5,
        rho: 1.2,
    },
];


/// Physical scaling derived from the domain size and the fluid: the
/// lattice viscosity fixes the physical time step via
/// `nu_lattice = nu_phys * dt / dx^2`.
#[derive(Clone, Copy)]
struct PhysScale {
    dx: f32, // metres per cell
    dt: f32, // seconds per lattice step
}

impl Default for PhysScale {
    fn default() -> Self {
        Self { dx: 1.0, dt: 1.0 }
    }
}

impl PhysScale {
    fn u_phys(&self, u_lattice: f32) -> f32 {
        u_lattice * self.dx / self.dt
    }
    fn len_m(&self, cells: f32) -> f32 {
        cells * self.dx
    }
    /// Gauge pressure [Pa] for a lattice density deviation
    /// (p = cs^2 * drho, cs^2 = 1/3 lattice units).
    fn pressure_pa(&self, drho: f32, rho_phys: f32) -> f32 {
        drho / 3.0 * (self.dx / self.dt).powi(2) * rho_phys
    }
}

fn fmt_len(m: f32) -> String {
    let a = m.abs();
    if a < 0.01 {
        format!("{:.1} mm", m * 1e3)
    } else if a < 1.0 {
        format!("{:.1} cm", m * 1e2)
    } else if a < 1000.0 {
        format!("{:.2} m", m)
    } else {
        format!("{:.2} km", m * 1e-3)
    }
}

fn fmt_time(s: f32) -> String {
    let a = s.abs();
    if a < 1e-3 {
        format!("{:.1} µs", s * 1e6)
    } else if a < 1.0 {
        format!("{:.2} ms", s * 1e3)
    } else if a < 120.0 {
        format!("{:.2} s", s)
    } else {
        format!("{:.1} min", s / 60.0)
    }
}

fn fmt_speed(v: f32) -> String {
    if v.abs() < 0.1 {
        format!("{:.1} cm/s", v * 100.0)
    } else {
        format!("{:.2} m/s", v)
    }
}

fn fmt_pressure(p: f32) -> String {
    let a = p.abs();
    if a < 0.1 {
        format!("{:.1} mPa", p * 1e3)
    } else if a < 1000.0 {
        format!("{:.2} Pa", p)
    } else {
        format!("{:.2} kPa", p * 1e-3)
    }
}

// CPU mirrors of the shader colormaps, for the legend bars.
fn inferno_color(t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0) * 4.0;
    let stops = [
        [0.001, 0.000, 0.014],
        [0.341, 0.062, 0.429],
        [0.730, 0.216, 0.330],
        [0.973, 0.555, 0.035],
        [0.988, 0.998, 0.645],
    ];
    let i = (t as usize).min(3);
    let f = t - i as f32;
    let c = |k: usize| stops[i][k] + (stops[i + 1][k] - stops[i][k]) * f;
    egui::Color32::from_rgb((c(0) * 255.0) as u8, (c(1) * 255.0) as u8, (c(2) * 255.0) as u8)
}

fn coolwarm_color(t: f32) -> egui::Color32 {
    let t = t.clamp(-1.0, 1.0);
    let cold = [0.230, 0.299, 0.754];
    let white = [0.940, 0.930, 0.920];
    let warm = [0.706, 0.016, 0.150];
    let (a, b, f) = if t < 0.0 { (white, cold, -t) } else { (white, warm, t) };
    let c = |k: usize| a[k] + (b[k] - a[k]) * f;
    egui::Color32::from_rgb((c(0) * 255.0) as u8, (c(1) * 255.0) as u8, (c(2) * 255.0) as u8)
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
    mode: RenderMode,
    display_gain: f32,
    smoke_gain: f32,
    particle_size: f32,
    particle_brightness: f32,
    sponge_strength: f32,
}

/// Commands for the sim (settings and file ops); the sketch model is
/// owned by the app and edited directly.
enum Cmd {
    TogglePause,
    ResetFlow,
    SetWindTunnel(bool),
    SetResolution(usize),
    SetMargin(usize),
    SetParticles(u32),
    SetRenderMode(RenderMode),
    SetFlowSpeed(f32),
    SetViscosity(f32),
    SetSteps(u32),
    SetDyeFade(f32),
    SetBoundaryTints(bool),
    SetDisplayGain(f32),
    SetSmokeGain(f32),
    SetParticleSize(f32),
    SetParticleBrightness(f32),
    SetSpongeStrength(f32),
    ExportPng(std::path::PathBuf),
    SetMapping(ViewportMapping),
}

pub struct FlowPaintApp {
    // Model + editing state.
    model: SketchModel,
    tool: Tool,
    selected: Option<u64>,
    gesture: Gesture,
    // Defaults for newly drawn objects.
    def_material: ObjMaterial,
    def_thickness: f32,
    def_filled: bool,
    def_fan_mult: f32,
    def_fan_gust: f32,
    def_smoke: egui::Color32,
    // Sketch aids.
    snap_enabled: bool,
    snap_spacing: f32,
    snap_angle_deg: f32,
    // Physical scaling.
    domain_width_m: f32,
    fluid_name: &'static str,
    fluid_nu: f32,
    fluid_rho: f32,
    fluid_preset_idx: Option<usize>,
    phys_cache: PhysScale,
    // Generator dialogs.
    show_airfoil_gen: bool,
    show_nozzle_gen: bool,
    airfoil_params: crate::generators::AirfoilParams,
    nozzle_params: crate::generators::NozzleParams,
    nozzle_real_ve: Option<f32>,
    nozzle_fan_auto: bool,
    // UI chrome.
    show_about: bool,
    show_shortcuts: bool,
    show_legend: bool,
    res_index: usize,
    particle_index: usize,
    margin_index: usize,
    status: String,
    hover_cell: Option<[f32; 2]>,
    // Stats.
    stats_grid: (usize, usize),
    stats_full: (usize, usize),
    stats_margin: usize,
    stats_mlups: f32,
    stats_re: u32,
    stats_steps_per_s: f32,
    stats_sim_steps: f64,
    sim_time_s: f64,
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

        let (vw, vh) = (RESOLUTIONS[res_index].1, RESOLUTIONS[res_index].2);
        let mut model = SketchModel::default();
        // Start with the classic cylinder demo as a live, editable object.
        let objs = build_preset(ScenePreset::Cylinder, &mut model, vw, vh);
        model.replace_all(objs);

        Self {
            model,
            tool: Tool::Select,
            selected: None,
            gesture: Gesture::None,
            def_material: ObjMaterial::Wall,
            def_thickness: 6.0,
            def_filled: false,
            def_fan_mult: 1.0,
            def_fan_gust: 0.0,
            def_smoke: egui::Color32::from_rgb(90, 217, 255),
            snap_enabled: false,
            snap_spacing: 10.0,
            snap_angle_deg: 45.0,
            domain_width_m: 1.0,
            fluid_name: "air",
            fluid_nu: 1.5e-5,
            fluid_rho: 1.2,
            fluid_preset_idx: Some(2),
            phys_cache: PhysScale::default(),
            show_airfoil_gen: false,
            show_nozzle_gen: false,
            airfoil_params: crate::generators::AirfoilParams::default(),
            nozzle_params: crate::generators::NozzleParams::default(),
            nozzle_real_ve: None,
            nozzle_fan_auto: true,
            show_about: false,
            show_shortcuts: false,
            show_legend: true,
            res_index,
            particle_index: 0,
            margin_index: DEFAULT_MARGIN_INDEX,
            status: String::from(
                "Draw with the sketch tools; every object stays selectable and editable.",
            ),
            hover_cell: None,
            stats_grid: (vw, vh),
            stats_full: (0, 0),
            stats_margin: 0,
            stats_mlups: 0.0,
            stats_re: 0,
            stats_steps_per_s: 0.0,
            stats_sim_steps: 0.0,
            sim_time_s: 0.0,
        }
    }

    fn phys_scale(&self, visc_lattice: f32) -> PhysScale {
        let vis_w = self.stats_grid.0.max(1);
        let dx = self.domain_width_m / vis_w as f32;
        let dt = visc_lattice.max(1e-5) * dx * dx / self.fluid_nu.max(1e-12);
        PhysScale { dx, dt }
    }

    /// Build a new object from the current defaults.
    fn new_object(&mut self, shape: Shape) -> SketchObject {
        let id = self.model.fresh_id();
        let c = self.def_smoke;
        SketchObject {
            id,
            shape,
            material: self.def_material,
            thickness: self.def_thickness,
            filled: self.def_filled,
            fan_mult: self.def_fan_mult,
            fan_gust: self.def_fan_gust,
            fan_phase: (id as f32 * 0.618_034) % 1.0,
            fan_angle: 0.0,
            smoke_rgb: [
                c.r() as f32 / 255.0,
                c.g() as f32 / 255.0,
                c.b() as f32 / 255.0,
            ],
        }
    }

    /// Insert a generator raster as a Stamp object at the canvas centre.
    fn insert_stamp_object(&mut self, raster: crate::geometry::GeoRegion) {
        self.finish_gesture();
        let (vw, vh) = self.stats_grid;
        let mut obj = self.new_object(Shape::Stamp {
            raster,
            c: [vw as f32 * 0.5, vh as f32 * 0.5],
            scale: 1.0,
            angle: 0.0,
        });
        // Stamp cells carry their own types and fan strengths; the
        // object-level fan knobs start neutral so the stamp inserts
        // exactly as the dialog computed it.
        obj.material = ObjMaterial::Wall;
        obj.fan_mult = 1.0;
        obj.fan_gust = 0.0;
        self.model.add(obj.clone());
        self.selected = Some(obj.id);
        self.tool = Tool::Select;
        self.status =
            "Inserted — drag to place, rotate/scale in the Object panel.".into();
    }

    // --- Sketch aids ---------------------------------------------------

    fn snap_point(&self, p: [f32; 2]) -> [f32; 2] {
        if !self.snap_enabled {
            return p;
        }
        let s = self.snap_spacing.max(1.0);
        [(p[0] / s).round() * s, (p[1] / s).round() * s]
    }

    fn angle_snap(&self, a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-4 {
            return b;
        }
        let step = self.snap_angle_deg.clamp(1.0, 90.0).to_radians();
        let ang = (dy.atan2(dx) / step).round() * step;
        [a[0] + len * ang.cos(), a[1] + len * ang.sin()]
    }
}

impl eframe::App for FlowPaintApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.request_repaint();

        let mut cmds: Vec<Cmd> = Vec::new();

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
                mode: sim.settings.render_mode,
                display_gain: sim.settings.display_gain,
                smoke_gain: sim.settings.smoke_gain,
                particle_size: sim.settings.particle_size,
                particle_brightness: sim.settings.particle_brightness,
                sponge_strength: sim.settings.sponge_strength,
            }
        };
        self.phys_cache = self.phys_scale(snapshot.visc);

        self.keyboard(ctx, &mut cmds);
        self.menu_bar(ctx, snapshot, &mut cmds);
        self.side_panel(ctx, snapshot, &mut cmds);
        self.legend_panel(ctx, snapshot);
        self.status_bar(ctx);
        self.canvas(ctx, &mut cmds);
        self.windows(ctx, snapshot);

        // Apply sim commands, project the model, upload.
        let Some(rs) = frame.wgpu_render_state() else { return };
        let mut renderer = rs.renderer.write();
        let Some(sim) = renderer.callback_resources.get_mut::<GpuSim>() else { return };

        for cmd in cmds {
            apply_cmd(sim, cmd, self);
        }
        if let Some(region) = self.model.take_dirty() {
            let (margin, tunnel) = (sim.margin(), sim.settings.wind_tunnel);
            self.model.rasterize_region(&mut sim.geo, region, margin, tunnel);
        }
        sim.flush_geometry();

        // Stats.
        self.stats_grid = sim.grid_size();
        self.stats_full = sim.full_size();
        self.stats_margin = sim.margin();
        let dt = ctx.input(|i| i.stable_dt).max(1e-4);
        let n = (self.stats_full.0 * self.stats_full.1) as f32;
        self.stats_mlups = n * sim.steps_last_frame as f32 / dt / 1.0e6;
        self.stats_re = sim.reynolds_estimate();
        self.stats_steps_per_s = sim.steps_last_frame as f32 / dt;
        let steps_now = sim.total_steps;
        if steps_now < self.stats_sim_steps {
            self.sim_time_s = 0.0;
        }
        let delta = (steps_now - self.stats_sim_steps).max(0.0);
        self.sim_time_s += delta * self.phys_cache.dt as f64;
        self.stats_sim_steps = steps_now;
    }
}

fn apply_cmd(sim: &mut GpuSim, cmd: Cmd, app: &mut FlowPaintApp) {
    match cmd {
        Cmd::TogglePause => sim.settings.paused = !sim.settings.paused,
        Cmd::ResetFlow => sim.reset_flow(),
        Cmd::SetWindTunnel(on) => {
            sim.set_wind_tunnel(on);
            app.model.mark_all_dirty();
        }
        Cmd::SetResolution(i) => {
            let old_w = sim.grid_size().0;
            sim.set_resolution(i);
            let new_w = sim.grid_size().0;
            if new_w != old_w && old_w > 0 {
                app.model.rescale_all(new_w as f32 / old_w as f32);
            }
            app.model.mark_all_dirty();
            app.stats_grid = sim.grid_size();
        }
        Cmd::SetMargin(i) => {
            sim.set_margin_frac(MARGIN_CHOICES[i].1);
            app.model.mark_all_dirty();
        }
        Cmd::SetParticles(n) => sim.settings.particle_count = n,
        Cmd::SetRenderMode(m) => sim.settings.render_mode = m,
        Cmd::SetFlowSpeed(v) => sim.settings.flow_speed = v,
        Cmd::SetViscosity(v) => sim.settings.viscosity = v,
        Cmd::SetSteps(v) => sim.settings.steps_per_frame = v,
        Cmd::SetDyeFade(v) => sim.settings.dye_fade = v,
        Cmd::SetBoundaryTints(v) => sim.settings.boundary_tints = v,
        Cmd::SetDisplayGain(v) => sim.settings.display_gain = v,
        Cmd::SetSmokeGain(v) => sim.settings.smoke_gain = v,
        Cmd::SetParticleSize(v) => sim.settings.particle_size = v,
        Cmd::SetParticleBrightness(v) => sim.settings.particle_brightness = v,
        Cmd::SetSpongeStrength(v) => sim.settings.sponge_strength = v,
        Cmd::ExportPng(p) => {
            app.status = match sim.export_png(&p) {
                Ok(()) => format!("Exported {}", p.display()),
                Err(e) => format!("Export failed: {e}"),
            };
        }
        Cmd::SetMapping(m) => {
            let (vw, vh) = sim.grid_size();
            sim.mapping = ViewportMapping::fit(m.vp_origin, m.vp_size, vw, vh);
            sim.write_render_uniform();
        }
    }
}

// --- Scene presets as object builders --------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScenePreset {
    Cylinder,
    Airfoil,
    Venturi,
    Step,
    Pinball,
}

impl ScenePreset {
    const ALL: [(ScenePreset, &'static str, &'static str); 5] = [
        (ScenePreset::Cylinder, "Cylinder", "von Kármán vortex street"),
        (ScenePreset::Airfoil, "Airfoil", "NACA 0012 at 5°"),
        (ScenePreset::Venturi, "Venturi", "channel constriction"),
        (ScenePreset::Step, "Step", "backward-facing step"),
        (ScenePreset::Pinball, "Pinball", "staggered cylinders"),
    ];
}

fn base_object(model: &mut SketchModel, shape: Shape) -> SketchObject {
    let id = model.fresh_id();
    SketchObject {
        id,
        shape,
        material: ObjMaterial::Wall,
        thickness: 6.0,
        filled: true,
        fan_mult: 1.0,
        fan_gust: 0.0,
        fan_phase: (id as f32 * 0.618_034) % 1.0,
        fan_angle: 0.0,
        smoke_rgb: [0.35, 0.85, 1.0],
    }
}

fn build_preset(
    p: ScenePreset,
    model: &mut SketchModel,
    vw: usize,
    vh: usize,
) -> Vec<SketchObject> {
    let w = vw as f32;
    let h = vh as f32;
    let mut objs = Vec::new();
    match p {
        ScenePreset::Cylinder => {
            let r = 0.08 * h;
            objs.push(base_object(
                model,
                Shape::Ellipse { c: [0.3 * w, 0.5 * h], r: [r, r], angle: 0.0 },
            ));
        }
        ScenePreset::Airfoil => {
            let mut params = crate::generators::AirfoilParams::default();
            params.camber = 0.0;
            params.thickness = 12.0;
            params.aoa_deg = 5.0;
            params.chord_cells = 0.35 * w;
            let raster = crate::generators::generate_airfoil(&params);
            objs.push(base_object(
                model,
                Shape::Stamp {
                    raster,
                    c: [0.4 * w, 0.5 * h],
                    scale: 1.0,
                    angle: 0.0,
                },
            ));
        }
        ScenePreset::Venturi => {
            // Two smooth wall contours pinching the channel.
            for sign in [-1.0f32, 1.0f32] {
                let n = 24;
                let pts: Vec<[f32; 2]> = (0..=n)
                    .map(|i| {
                        let s = i as f32 / n as f32;
                        let x = s * w;
                        let arg = (s - 0.45) / 0.16;
                        let gap = 1.0 - 0.62 * (-arg * arg).exp();
                        let half = 0.5 * gap * h;
                        [x, 0.5 * h + sign * half]
                    })
                    .collect();
                let mut o = base_object(model, Shape::Poly { pts, closed: false });
                o.filled = false;
                o.thickness = 10.0;
                objs.push(o);
            }
        }
        ScenePreset::Step => {
            objs.push(base_object(
                model,
                Shape::Rect {
                    c: [0.16 * w, 0.25 * h],
                    half: [0.16 * w, 0.25 * h],
                    angle: 0.0,
                },
            ));
        }
        ScenePreset::Pinball => {
            let r = 0.055 * h;
            for (cx, cy) in
                [(0.28, 0.30), (0.28, 0.70), (0.48, 0.50), (0.68, 0.30), (0.68, 0.70)]
            {
                objs.push(base_object(
                    model,
                    Shape::Ellipse {
                        c: [cx * w, cy * h],
                        r: [r, r],
                        angle: 0.0,
                    },
                ));
            }
        }
    }
    objs
}

// --- UI ---------------------------------------------------------------

impl FlowPaintApp {
    fn keyboard(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        if ctx.wants_keyboard_input() {
            return;
        }
        // Enter finishes a polyline; Esc cancels the gesture or deselects.
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.finish_gesture();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            match std::mem::replace(&mut self.gesture, Gesture::None) {
                Gesture::DrawShape { id, .. }
                | Gesture::DrawPoly { id }
                | Gesture::DrawPencil { id } => {
                    self.model.cancel_last_add(id);
                    if self.selected == Some(id) {
                        self.selected = None;
                    }
                }
                Gesture::MoveObj { id, before, .. }
                | Gesture::HandleDrag { id, before, .. } => {
                    // Revert the in-flight edit.
                    if let Some(i) = self.model.find(id) {
                        let after_bounds = self.model.objects[i].bounds();
                        self.model.mark_dirty(after_bounds.union(before.bounds()));
                        self.model.objects[i] = before;
                    }
                }
                Gesture::None => self.selected = None,
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            if !matches!(self.gesture, Gesture::None) {
                // Don't delete out from under an active gesture.
            } else if let Some(id) = self.selected.take() {
                self.model.remove(id);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            cmds.push(Cmd::TogglePause);
        }
        // Duplicate.
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::D)) {
            self.duplicate_selected();
        }
        // Undo / redo on the model.
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
            self.finish_gesture();
            if ctx.input(|i| i.modifiers.shift) {
                self.model.redo();
            } else {
                self.model.undo();
            }
            self.selected = None;
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Y)) {
            self.finish_gesture();
            self.model.redo();
            self.selected = None;
        }
        if matches!(self.gesture, Gesture::None) {
            let mut switch = None;
            ctx.input(|i| {
                for (tool, _, key) in Tool::ALL {
                    let k = match key {
                        "S" => egui::Key::S,
                        "L" => egui::Key::L,
                        "R" => egui::Key::R,
                        "E" => egui::Key::E,
                        "P" => egui::Key::P,
                        _ => egui::Key::B,
                    };
                    if i.key_pressed(k) {
                        switch = Some(tool);
                    }
                }
            });
            if let Some(tool) = switch {
                self.tool = tool;
            }
        }
        // Arrow-key nudge for the selected object.
        if let Some(id) = self.selected {
            if matches!(self.gesture, Gesture::None) {
                let step = if ctx.input(|i| i.modifiers.shift) { 1.0 } else { 4.0 };
                let mut d = [0.0f32; 2];
                ctx.input(|i| {
                    if i.key_pressed(egui::Key::ArrowLeft) {
                        d[0] -= step;
                    }
                    if i.key_pressed(egui::Key::ArrowRight) {
                        d[0] += step;
                    }
                    if i.key_pressed(egui::Key::ArrowUp) {
                        d[1] -= step;
                    }
                    if i.key_pressed(egui::Key::ArrowDown) {
                        d[1] += step;
                    }
                });
                if d != [0.0; 2] {
                    self.edit_object(id, |o| o.translate(d));
                }
            }
        }
    }

    fn duplicate_selected(&mut self) {
        self.finish_gesture();
        if let Some(id) = self.selected {
            if let Some(i) = self.model.find(id) {
                let mut copy = self.model.objects[i].clone();
                copy.id = self.model.fresh_id();
                copy.translate([16.0, 16.0]);
                self.selected = Some(copy.id);
                self.model.add(copy);
                self.status = "Duplicated.".into();
            }
        }
    }

    /// Apply a coalesced, undoable edit to an object (panel widgets).
    fn edit_object(&mut self, id: u64, f: impl FnOnce(&mut SketchObject)) {
        if let Some(i) = self.model.find(id) {
            let before = self.model.objects[i].clone();
            f(&mut self.model.objects[i]);
            self.model.record_modify_coalesced(id, before);
        }
    }

    /// Mutate an object mid-gesture: no undo record (that lands when the
    /// gesture finishes), just damage marking.
    fn mutate_live(&mut self, id: u64, f: impl FnOnce(&mut SketchObject)) {
        if let Some(i) = self.model.find(id) {
            let b0 = self.model.objects[i].bounds();
            f(&mut self.model.objects[i]);
            let b1 = self.model.objects[i].bounds();
            self.model.mark_dirty(b0.union(b1));
        }
    }

    /// Finalize whatever gesture is in flight (polyline Enter, tool
    /// switches, etc.).
    fn finish_gesture(&mut self) {
        match std::mem::replace(&mut self.gesture, Gesture::None) {
            Gesture::DrawPoly { id } => {
                // Drop the rubber vertex that trails the cursor.
                self.mutate_live(id, |o| {
                    if let Shape::Poly { pts, closed } = &mut o.shape {
                        if !*closed && pts.len() > 1 {
                            pts.pop();
                        }
                    }
                });
                let degenerate = self
                    .model
                    .find(id)
                    .map(|i| match &self.model.objects[i].shape {
                        Shape::Poly { pts, .. } => pts.len() < 2,
                        _ => false,
                    })
                    .unwrap_or(true);
                if degenerate {
                    self.model.cancel_last_add(id);
                    if self.selected == Some(id) {
                        self.selected = None;
                    }
                } else {
                    self.model.finalize_last_add(id);
                    self.selected = Some(id);
                }
            }
            Gesture::DrawShape { id, .. } | Gesture::DrawPencil { id } => {
                self.model.finalize_last_add(id);
                self.selected = Some(id);
            }
            Gesture::MoveObj { id, before, .. }
            | Gesture::HandleDrag { id, before, .. } => {
                self.model.record_modify(id, before);
            }
            Gesture::None => {}
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New (clear everything)").clicked() {
                        self.finish_gesture();
                        self.selected = None;
                        self.model.replace_all(Vec::new());
                        cmds.push(Cmd::ResetFlow);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Open scene…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("FlowPaint scene", &["flow"])
                            .pick_file()
                        {
                            self.load_scene(&p, cmds);
                        }
                        ui.close_menu();
                    }
                    if ui.button("Save scene…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("FlowPaint scene", &["flow"])
                            .set_file_name("scene.flow")
                            .save_file()
                        {
                            self.save_scene(&p, snap);
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
                        self.finish_gesture();
                        self.model.undo();
                        self.selected = None;
                        ui.close_menu();
                    }
                    if ui.button("Redo        Ctrl+Y").clicked() {
                        self.finish_gesture();
                        self.model.redo();
                        self.selected = None;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Reset flow (keep sketch)").clicked() {
                        cmds.push(Cmd::ResetFlow);
                        ui.close_menu();
                    }
                });
                ui.menu_button("Simulation", |ui| {
                    ui.menu_button("Grid resolution", |ui| {
                        for (i, (label, _, _)) in RESOLUTIONS.iter().enumerate() {
                            if ui.radio(i == self.res_index, *label).clicked() {
                                self.res_index = i;
                                self.finish_gesture();
                                cmds.push(Cmd::SetResolution(i));
                                ui.close_menu();
                            }
                        }
                    });
                    ui.menu_button("Domain margin", |ui| {
                        ui.label("Extra simulated area around the canvas;");
                        ui.label("edges also get an absorbing sponge layer.");
                        ui.separator();
                        for (i, (label, _)) in MARGIN_CHOICES.iter().enumerate() {
                            if ui.radio(i == self.margin_index, *label).clicked() {
                                self.margin_index = i;
                                cmds.push(Cmd::SetMargin(i));
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
                    if ui.button("About FlowPaint V2").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    fn save_scene(&mut self, path: &std::path::Path, snap: UiSnapshot) {
        let scene = SceneV3 {
            version: SCENE_V3,
            objects: self.model.objects.clone(),
            wind_tunnel: snap.tunnel,
            flow_speed: snap.flow,
            viscosity: snap.visc,
            steps_per_frame: snap.steps,
            domain_width_m: self.domain_width_m,
            fluid_nu: self.fluid_nu,
            fluid_rho: self.fluid_rho,
            ref_width: self.stats_grid.0 as u32,
        };
        match bincode::serialize(&scene) {
            Ok(bytes) => {
                self.status = match std::fs::write(path, bytes) {
                    Ok(()) => format!("Saved {}", path.display()),
                    Err(e) => format!("Save failed: {e}"),
                };
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn load_scene(&mut self, path: &std::path::Path, cmds: &mut Vec<Cmd>) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Load failed: {e}");
                return;
            }
        };
        if bytes.len() < 4
            || u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != SCENE_V3
        {
            self.status =
                "Load failed: not a FlowPaint V2 scene (older .flow files aren't supported)"
                    .into();
            return;
        }
        match bincode::deserialize::<SceneV3>(&bytes) {
            Ok(scene) => {
                self.finish_gesture();
                self.selected = None;
                let mut objects = scene.objects;
                // Rescale into the current grid width.
                let cur_w = self.stats_grid.0 as f32;
                let f = cur_w / (scene.ref_width.max(1) as f32);
                if (f - 1.0).abs() > 1e-3 {
                    for o in &mut objects {
                        o.rescale_all(f);
                    }
                }
                self.model.replace_all(objects);
                self.domain_width_m = scene.domain_width_m.max(0.01);
                self.fluid_nu = scene.fluid_nu.max(1e-9);
                self.fluid_rho = scene.fluid_rho.max(1e-3);
                self.fluid_preset_idx = None;
                self.fluid_name = "custom (loaded)";
                if scene.flow_speed > 0.0 {
                    cmds.push(Cmd::SetFlowSpeed(scene.flow_speed));
                    cmds.push(Cmd::SetViscosity(scene.viscosity));
                    cmds.push(Cmd::SetSteps(scene.steps_per_frame.max(1)));
                }
                cmds.push(Cmd::SetWindTunnel(scene.wind_tunnel));
                cmds.push(Cmd::ResetFlow);
                self.status = format!("Loaded {}", path.display());
            }
            Err(e) => self.status = format!("Load failed: {e}"),
        }
    }

    fn side_panel(&mut self, ctx: &egui::Context, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        egui::SidePanel::left("controls")
            .default_width(248.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.side_panel_contents(ui, snap, cmds);
                    });
            });
    }

    fn side_panel_contents(&mut self, ui: &mut egui::Ui, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        ui.add_space(4.0);
        // Everyday actions live at the top of the panel.
        ui.horizontal(|ui| {
            if ui
                .button(if snap.paused { "▶ Resume" } else { "⏸ Pause" })
                .clicked()
            {
                cmds.push(Cmd::TogglePause);
            }
            if ui.button("Reset flow").clicked() {
                cmds.push(Cmd::ResetFlow);
            }
            if ui
                .button(
                    egui::RichText::new("Clear all")
                        .color(egui::Color32::from_rgb(255, 140, 120)),
                )
                .on_hover_text("Remove every object (undoable) and reset the flow")
                .clicked()
            {
                self.finish_gesture();
                self.selected = None;
                self.model.replace_all(Vec::new());
                cmds.push(Cmd::ResetFlow);
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.model.can_undo(), egui::Button::new("↶ Undo"))
                .clicked()
            {
                self.finish_gesture();
                self.model.undo();
                self.selected = None;
            }
            if ui
                .add_enabled(self.model.can_redo(), egui::Button::new("↷ Redo"))
                .clicked()
            {
                self.finish_gesture();
                self.model.redo();
                self.selected = None;
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.heading("Tools");
        ui.horizontal_wrapped(|ui| {
            for (tool, label, key) in Tool::ALL {
                let selected = self.tool == tool;
                if ui
                    .selectable_label(selected, format!("{label} ({key})"))
                    .clicked()
                    && !selected
                {
                    self.finish_gesture();
                    self.tool = tool;
                }
            }
        });
        ui.label(
            egui::RichText::new(
                "Everything you draw stays a live object — pick Select (S) any \
                 time to move it, drag its vertices, or retune its physics.",
            )
            .small()
            .weak(),
        );

        ui.add_space(6.0);
        ui.separator();
        if self.selected.is_some() && !matches!(self.gesture, Gesture::None) {
            // Mid-gesture: the object panel would fight the drag.
            ui.heading("Object");
            ui.label(egui::RichText::new("(finish the gesture…)").weak());
        } else if let Some(id) = self.selected {
            self.object_panel(ui, id, cmds);
        } else {
            self.defaults_panel(ui, cmds);
        }

        ui.add_space(6.0);
        ui.separator();
        ui.heading("Sketch aids");
        let ps = self.phys_cache;
        ui.horizontal(|ui| {
            ui.label("angle snap (Shift)");
            ui.add(
                egui::DragValue::new(&mut self.snap_angle_deg)
                    .range(1.0..=90.0)
                    .speed(0.5)
                    .suffix("°"),
            );
        });
        ui.horizontal_wrapped(|ui| {
            for a in [5.0f32, 15.0, 22.5, 30.0, 45.0, 90.0] {
                if ui.small_button(format!("{a}°")).clicked() {
                    self.snap_angle_deg = a;
                }
            }
        });
        ui.checkbox(&mut self.snap_enabled, "Snap to grid");
        if self.snap_enabled {
            let spacing_label =
                format!("spacing ({})", fmt_len(ps.len_m(self.snap_spacing)));
            ui.add(
                egui::Slider::new(&mut self.snap_spacing, 2.0..=50.0).text(spacing_label),
            );
        }

        ui.add_space(6.0);
        ui.separator();
        ui.heading("Generators");
        ui.horizontal(|ui| {
            if ui.button("✈ Airfoil…").clicked() {
                self.show_airfoil_gen = true;
            }
            if ui.button("🚀 Nozzle…").clicked() {
                self.show_nozzle_gen = true;
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.heading("Scene presets");
        ui.horizontal_wrapped(|ui| {
            for (p, short, desc) in ScenePreset::ALL {
                if ui
                    .button(short)
                    .on_hover_text(format!("{desc} — replaces the scene"))
                    .clicked()
                {
                    self.finish_gesture();
                    self.selected = None;
                    let (vw, vh) = self.stats_grid;
                    let objs = build_preset(p, &mut self.model, vw, vh);
                    self.model.replace_all(objs);
                    cmds.push(Cmd::ResetFlow);
                    self.status = format!("Scene preset: {short} (editable objects)");
                }
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.heading("View");
        ui.horizontal_wrapped(|ui| {
            for m in RenderMode::ALL {
                if ui.selectable_label(snap.mode == m, m.label()).clicked() {
                    cmds.push(Cmd::SetRenderMode(m));
                }
            }
        });
        let mut tints = snap.tints;
        if ui.checkbox(&mut tints, "Highlight fans && drains").changed() {
            cmds.push(Cmd::SetBoundaryTints(tints));
        }
        ui.checkbox(&mut self.show_legend, "Show legend");
        egui::ComboBox::from_label("particles")
            .selected_text(PARTICLE_CHOICES[self.particle_index].0)
            .show_ui(ui, |ui| {
                for (i, (label, count)) in PARTICLE_CHOICES.iter().enumerate() {
                    if ui
                        .selectable_label(i == self.particle_index, *label)
                        .clicked()
                    {
                        self.particle_index = i;
                        cmds.push(Cmd::SetParticles(*count));
                    }
                }
            });

        ui.add_space(6.0);
        ui.separator();
        ui.heading("Physics");
        let combo_label = match self.fluid_preset_idx {
            Some(i) => FLUID_PRESETS[i].name,
            None => "Custom",
        };
        egui::ComboBox::from_label("fluid")
            .selected_text(combo_label)
            .show_ui(ui, |ui| {
                for (i, p) in FLUID_PRESETS.iter().enumerate() {
                    let sel = self.fluid_preset_idx == Some(i);
                    if ui
                        .selectable_label(sel, p.name)
                        .on_hover_text(p.desc)
                        .clicked()
                    {
                        self.fluid_preset_idx = Some(i);
                        self.fluid_name = p.name;
                        self.fluid_nu = p.nu;
                        self.fluid_rho = p.rho;
                        if p.tunnel != snap.tunnel {
                            cmds.push(Cmd::SetWindTunnel(p.tunnel));
                        }
                        cmds.push(Cmd::SetFlowSpeed(p.flow));
                        cmds.push(Cmd::SetViscosity(p.visc));
                        // Presets own the sub-step count too, so e.g.
                        // Supersonic's 16 steps don't leak into the
                        // next regime.
                        cmds.push(Cmd::SetSteps(p.steps.unwrap_or(8)));
                        self.status = format!("Fluid preset: {}", p.name);
                    }
                }
            });
        let mut flow = snap.flow;
        let mut visc = snap.visc;
        let mut steps = snap.steps;
        let mut fade = snap.fade;
        let mut tunnel = snap.tunnel;
        let flow_label = format!("flow speed ({})", fmt_speed(ps.u_phys(flow)));
        if ui
            .add(egui::Slider::new(&mut flow, 0.02..=0.14).text(flow_label))
            .changed()
        {
            self.fluid_preset_idx = None;
            cmds.push(Cmd::SetFlowSpeed(flow));
        }
        if ui
            .add(
                egui::Slider::new(&mut visc, 0.005..=0.08)
                    .logarithmic(true)
                    .text(format!("viscosity (Δt {})", fmt_time(ps.dt))),
            )
            .changed()
        {
            self.fluid_preset_idx = None;
            cmds.push(Cmd::SetViscosity(visc));
        }
        if ui
            .add(egui::Slider::new(&mut steps, 1..=32).text("steps / frame"))
            .changed()
        {
            self.fluid_preset_idx = None;
            cmds.push(Cmd::SetSteps(steps));
        }
        if ui
            .add(egui::Slider::new(&mut fade, 0.985..=1.0).text("smoke persistence"))
            .changed()
        {
            cmds.push(Cmd::SetDyeFade(fade));
        }
        if ui.checkbox(&mut tunnel, "Wind tunnel (left to right)").changed() {
            self.fluid_preset_idx = None;
            cmds.push(Cmd::SetWindTunnel(tunnel));
        }

        ui.add_space(6.0);
        egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
            ui.add(
                egui::Slider::new(&mut self.domain_width_m, 0.05..=100.0)
                    .logarithmic(true)
                    .text("domain width (m)"),
            )
            .on_hover_text(
                "Physical size the canvas represents; anchors every unit \
                 readout (cell size, time step, speeds, pressures)",
            );
            let mut gain = snap.display_gain;
            if ui
                .add(
                    egui::Slider::new(&mut gain, 0.25..=4.0)
                        .logarithmic(true)
                        .text("display gain"),
                )
                .on_hover_text("Scales the speed/vorticity/pressure color mapping")
                .changed()
            {
                cmds.push(Cmd::SetDisplayGain(gain));
            }
            let mut sgain = snap.smoke_gain;
            if ui
                .add(egui::Slider::new(&mut sgain, 0.25..=3.0).text("smoke brightness"))
                .changed()
            {
                cmds.push(Cmd::SetSmokeGain(sgain));
            }
            let mut sponge = snap.sponge_strength;
            if ui
                .add(egui::Slider::new(&mut sponge, 0.0..=0.3).text("edge damping"))
                .on_hover_text(
                    "Absorbing sponge at the domain edge (needs a margin); \
                     kills reflections of pressure waves",
                )
                .changed()
            {
                cmds.push(Cmd::SetSpongeStrength(sponge));
            }
            let mut psize = snap.particle_size;
            if ui
                .add(egui::Slider::new(&mut psize, 0.8..=5.0).text("particle size"))
                .changed()
            {
                cmds.push(Cmd::SetParticleSize(psize));
            }
            let mut pbright = snap.particle_brightness;
            if ui
                .add(
                    egui::Slider::new(&mut pbright, 0.05..=1.0)
                        .text("particle brightness"),
                )
                .changed()
            {
                cmds.push(Cmd::SetParticleBrightness(pbright));
            }
        });
    }

    /// Properties of the selected object: every knob edits the live model
    /// (undoably, with per-widget coalescing).
    fn object_panel(&mut self, ui: &mut egui::Ui, id: u64, cmds: &mut Vec<Cmd>) {
        let Some(i) = self.model.find(id) else {
            self.selected = None;
            return;
        };
        let before = self.model.objects[i].clone();
        let mut obj = before.clone();
        let mut changed = false;

        let kind = match &obj.shape {
            Shape::Line { .. } => "Line",
            Shape::Poly { closed: true, .. } => "Polygon",
            Shape::Poly { .. } => "Polyline",
            Shape::Rect { .. } => "Rectangle",
            Shape::Ellipse { .. } => "Ellipse",
            Shape::Stamp { .. } => "Generated part",
        };
        ui.heading(format!("Object — {kind}"));

        let is_stamp = matches!(obj.shape, Shape::Stamp { .. });
        let can_fill = matches!(obj.shape, Shape::Rect { .. } | Shape::Ellipse { .. });
        let ps = self.phys_cache;

        if !is_stamp {
            let mats: [(ObjMaterial, &str); 4] = [
                (ObjMaterial::Wall, "Solid, no-slip"),
                (ObjMaterial::Fan, "Blows along the shape"),
                (ObjMaterial::Smoke, "Passive dye emitter"),
                (ObjMaterial::Drain, "Lets flow leave"),
            ];
            ui.horizontal_wrapped(|ui| {
                for (m, tip) in mats {
                    let resp = ui
                        .selectable_label(obj.material == m, m.label())
                        .on_hover_text(tip);
                    if resp.clicked() && obj.material != m {
                        obj.material = m;
                        changed = true;
                        if m == ObjMaterial::Smoke {
                            cmds.push(Cmd::SetRenderMode(RenderMode::Dye));
                        }
                    }
                }
            });

            if can_fill && ui.checkbox(&mut obj.filled, "Filled").changed() {
                changed = true;
            }
            if !(can_fill && obj.filled) {
                let thick_label =
                    format!("thickness ({})", fmt_len(ps.len_m(obj.thickness)));
                changed |= ui
                    .add(egui::Slider::new(&mut obj.thickness, 1.0..=24.0).text(thick_label))
                    .changed();
            }
        }

        // Fan physics: for drawn fans, and for generated parts that carry
        // fan cells (a rocket nozzle's chamber inlet).
        let stamp_has_fans = match &obj.shape {
            Shape::Stamp { raster, .. } => {
                raster.cell.iter().any(|&c| c == crate::geometry::CELL_INLET)
            }
            _ => false,
        };
        if obj.material == ObjMaterial::Fan || stamp_has_fans {
            ui.add_space(2.0);
            changed |= ui
                .add(egui::Slider::new(&mut obj.fan_mult, 0.2..=2.0).text("fan speed ×"))
                .on_hover_text("Multiplier on the global flow speed")
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut obj.fan_gust, 0.0..=1.0).text("gustiness"))
                .on_hover_text(
                    "Time-varying wander in the fan's direction and strength — \
                     0 is steady, 1 is a blustery day",
                )
                .changed();
            // Chained shapes blow along their segments; solid shapes and
            // stamps have a free direction (stamps rotate with the part).
            if obj.material == ObjMaterial::Fan
                && (matches!(obj.shape, Shape::Rect { .. } | Shape::Ellipse { .. })
                    && obj.filled)
            {
                let mut deg = obj.fan_angle.to_degrees();
                if ui
                    .add(
                        egui::Slider::new(&mut deg, -180.0..=180.0)
                            .text("blow direction °"),
                    )
                    .changed()
                {
                    obj.fan_angle = deg.to_radians();
                    changed = true;
                }
            }
        }
        if obj.material == ObjMaterial::Fan || obj.material == ObjMaterial::Smoke {
            let mut c = egui::Color32::from_rgb(
                (obj.smoke_rgb[0] * 255.0) as u8,
                (obj.smoke_rgb[1] * 255.0) as u8,
                (obj.smoke_rgb[2] * 255.0) as u8,
            );
            ui.horizontal(|ui| {
                ui.label("Smoke color:");
                if ui.color_edit_button_srgba(&mut c).changed() {
                    obj.smoke_rgb = [
                        c.r() as f32 / 255.0,
                        c.g() as f32 / 255.0,
                        c.b() as f32 / 255.0,
                    ];
                    changed = true;
                }
            });
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label("Rotate");
            for (label, da) in [("-15°", -15.0f32), ("+15°", 15.0), ("+90°", 90.0)] {
                if ui.small_button(label).clicked() {
                    obj.rotate_by(da.to_radians());
                    changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Scale");
            for (label, f) in [("×0.8", 0.8f32), ("×1.25", 1.25)] {
                if ui.small_button(label).clicked() {
                    obj.scale_by(f);
                    changed = true;
                }
            }
        });

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if ui.button("Duplicate (Ctrl+D)").clicked() {
                self.duplicate_selected();
            }
            if ui.button("Delete (Del)").clicked() {
                self.selected = None;
                self.model.remove(id);
            }
        });
        // Deleting or duplicating invalidates `i`/`before`; bail out.
        if self.selected != Some(id) {
            return;
        }

        if changed {
            if let Some(i) = self.model.find(id) {
                self.model.objects[i] = obj;
                self.model.record_modify_coalesced(id, before);
            }
        } else {
            ui.label(
                egui::RichText::new(
                    "Drag the object to move it; drag its handles to reshape. \
                     Arrows nudge, Esc deselects.",
                )
                .small()
                .weak(),
            );
        }
    }

    /// Defaults applied to newly drawn objects.
    fn defaults_panel(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        ui.heading("New objects");
        let mats: [(ObjMaterial, &str); 4] = [
            (ObjMaterial::Wall, "Solid, no-slip"),
            (ObjMaterial::Fan, "Blows along the shape"),
            (ObjMaterial::Smoke, "Passive dye emitter"),
            (ObjMaterial::Drain, "Lets flow leave"),
        ];
        ui.horizontal_wrapped(|ui| {
            for (m, tip) in mats {
                let resp = ui
                    .selectable_label(self.def_material == m, m.label())
                    .on_hover_text(tip);
                if resp.clicked() {
                    self.def_material = m;
                    // Smoke is only visible in the Smoke view; switch so
                    // the first stroke gives immediate feedback.
                    if m == ObjMaterial::Smoke {
                        cmds.push(Cmd::SetRenderMode(RenderMode::Dye));
                    }
                }
            }
        });
        let ps = self.phys_cache;
        let thick_label = format!(
            "thickness ({})",
            fmt_len(ps.len_m(self.def_thickness))
        );
        ui.add(egui::Slider::new(&mut self.def_thickness, 1.0..=24.0).text(thick_label))
            .on_hover_text("Lines, polylines and shape outlines draw at this thickness");
        ui.checkbox(&mut self.def_filled, "Filled rect / ellipse")
            .on_hover_text("Off = SolidWorks-style outlines at the set thickness");
        if self.def_material == ObjMaterial::Fan {
            ui.add(
                egui::Slider::new(&mut self.def_fan_mult, 0.2..=2.0).text("fan speed ×"),
            );
            ui.add(
                egui::Slider::new(&mut self.def_fan_gust, 0.0..=1.0).text("gustiness"),
            );
        }
        if self.def_material == ObjMaterial::Fan || self.def_material == ObjMaterial::Smoke
        {
            ui.horizontal(|ui| {
                ui.label("Smoke color:");
                ui.color_edit_button_srgba(&mut self.def_smoke);
            });
        }
    }


    /// The right-hand legend: the important flow numbers in physical
    /// units, plus a color-scale bar for the current view.
    fn legend_panel(&mut self, ctx: &egui::Context, snap: UiSnapshot) {
        if !self.show_legend {
            return;
        }
        let ps = self.phys_cache;
        let (_vw, vh) = self.stats_grid;
        egui::SidePanel::right("legend").default_width(200.0).show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading("Flow numbers");
            egui::Grid::new("legend_grid")
                .num_columns(2)
                .striped(true)
                .min_col_width(80.0)
                .show(ui, |ui| {
                    let mut row = |k: &str, v: String| {
                        ui.label(k);
                        ui.monospace(v);
                        ui.end_row();
                    };
                    row("Fluid", self.fluid_name.to_string());
                    row("ν", format!("{:.2e} m²/s", self.fluid_nu));
                    row("ρ", format!("{:.0} kg/m³", self.fluid_rho));
                    row(
                        "Domain",
                        format!(
                            "{} × {}",
                            fmt_len(self.domain_width_m),
                            fmt_len(ps.len_m(vh as f32))
                        ),
                    );
                    row("Cell Δx", fmt_len(ps.dx));
                    row("Step Δt", fmt_time(ps.dt));
                    row("Inlet U∞", fmt_speed(ps.u_phys(snap.flow)));
                    row(
                        "Ref. length",
                        fmt_len(ps.len_m(0.16 * vh as f32)),
                    );
                    row("Reynolds", format!("{}", self.stats_re));
                    row(
                        "Dyn. press.",
                        fmt_pressure(
                            0.5 * self.fluid_rho * ps.u_phys(snap.flow).powi(2),
                        ),
                    );
                    row(
                        "Sim rate",
                        format!("{:.2}× real", self.stats_steps_per_s * ps.dt),
                    );
                    row("Sim time", fmt_time(self.sim_time_s as f32));
                });
            ui.add_space(6.0);
            ui.separator();

            // Color-scale legend for the current view. The saturation
            // points invert the shader's normalizations.
            let gain = snap.display_gain.max(1e-3);
            match snap.mode {
                RenderMode::Dye => {
                    ui.label("Smoke view: dye brightness");
                    ui.label(
                        egui::RichText::new(
                            "(passive tracer — arbitrary units)",
                        )
                        .small()
                        .weak(),
                    );
                }
                RenderMode::Speed => {
                    ui.label("Speed |u|");
                    let u_sat = ps.u_phys(snap.flow * 1.6 / gain);
                    Self::colormap_bar(ui, |t| inferno_color(t));
                    ui.horizontal(|ui| {
                        ui.small("0");
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| ui.small(format!("≥ {}", fmt_speed(u_sat))),
                        );
                    });
                }
                RenderMode::Vorticity => {
                    ui.label("Vorticity ω (curl)");
                    let w_sat = snap.flow.max(0.02) / (4.0 * gain) / ps.dt;
                    Self::colormap_bar(ui, |t| coolwarm_color(t * 2.0 - 1.0));
                    ui.horizontal(|ui| {
                        ui.small(format!("-{:.1} 1/s", w_sat));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| ui.small(format!("+{:.1} 1/s", w_sat)),
                        );
                    });
                    ui.small("red: clockwise · blue: counter-clockwise");
                }
                RenderMode::Pressure => {
                    ui.label("Pressure Δp (gauge)");
                    let p_sat = ps.pressure_pa(1.0 / (25.0 * gain), self.fluid_rho);
                    Self::colormap_bar(ui, |t| coolwarm_color(t * 2.0 - 1.0));
                    ui.horizontal(|ui| {
                        ui.small(format!("-{}", fmt_pressure(p_sat)));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| ui.small(format!("+{}", fmt_pressure(p_sat))),
                        );
                    });
                    ui.small("relative to ambient (0 = undisturbed)");
                }
            }
        });
    }

    fn colormap_bar(ui: &mut egui::Ui, color: impl Fn(f32) -> egui::Color32) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(184.0), 14.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        let n = 48;
        for i in 0..n {
            let t0 = i as f32 / n as f32;
            let t1 = (i + 1) as f32 / n as f32;
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.min.x + rect.width() * t0, rect.min.y),
                    egui::pos2(rect.min.x + rect.width() * t1, rect.max.y),
                ),
                0.0,
                color((t0 + t1) * 0.5),
            );
        }
    }


    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(c) = self.hover_cell {
                    ui.monospace(format!("({:.0}, {:.0})", c[0], c[1]));
                    ui.separator();
                }
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "{} objects   |   canvas {} x {} (sim {} x {}, +{} margin)   |   {:.0} MLUPS   |   Re ≈ {}",
                        self.model.objects.len(),
                        self.stats_grid.0,
                        self.stats_grid.1,
                        self.stats_full.0,
                        self.stats_full.1,
                        self.stats_margin,
                        self.stats_mlups,
                        self.stats_re
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
                // press frame at the press position, so clicks land exactly
                // where the user pressed instead of ~6 pt into the gesture.
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

                self.canvas_interaction(&response, mapping, ppp);

                // The simulation paints itself via the wgpu callback.
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    FlowPaintCallback,
                ));

                self.canvas_overlays(ui, mapping, ppp);
            });
    }

    fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    }

    /// Ramer–Douglas–Peucker simplification for pencil strokes, so
    /// freehand curves become clean, light polylines with draggable
    /// vertices.
    fn simplify_stroke(pts: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
        if pts.len() <= 2 {
            return pts.to_vec();
        }
        fn rdp(pts: &[[f32; 2]], eps: f32, out: &mut Vec<[f32; 2]>) {
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            let mut worst = 0.0f32;
            let mut wi = 0usize;
            let ab = [b[0] - a[0], b[1] - a[1]];
            let l2 = ab[0] * ab[0] + ab[1] * ab[1];
            for (k, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
                let d = if l2 < 1e-9 {
                    ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt()
                } else {
                    ((p[0] - a[0]) * ab[1] - (p[1] - a[1]) * ab[0]).abs() / l2.sqrt()
                };
                if d > worst {
                    worst = d;
                    wi = k;
                }
            }
            if worst > eps && wi > 0 {
                rdp(&pts[..=wi], eps, out);
                out.pop(); // the split point would land twice
                rdp(&pts[wi..], eps, out);
            } else {
                out.push(a);
                out.push(b);
            }
        }
        let mut out = Vec::new();
        rdp(pts, eps, &mut out);
        out
    }

    fn canvas_interaction(
        &mut self,
        response: &egui::Response,
        mapping: ViewportMapping,
        ppp: f32,
    ) {
        let to_cell = |pos: egui::Pos2| -> [f32; 2] {
            mapping.px_to_cell([pos.x * ppp, pos.y * ppp])
        };
        self.hover_cell = response.hover_pos().map(to_cell);

        let px_per_cell = mapping.px_per_cell.max(1e-3);
        // Pick thresholds in screen space so they feel the same at every
        // zoom, with a floor in cells for very zoomed-in grids.
        let handle_r = (8.0 * ppp / px_per_cell).max(2.0);
        let click_slop = (4.0 * ppp / px_per_cell).max(2.0);
        let (shift, alt) =
            response.ctx.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let pointer = response.interact_pointer_pos();

        // Live polyline rubber vertex follows the cursor between clicks.
        if let Gesture::DrawPoly { id } = &self.gesture {
            let id = *id;
            if let Some(pos) = response.hover_pos() {
                let raw = to_cell(pos);
                let prev = self.model.find(id).and_then(|i| {
                    match &self.model.objects[i].shape {
                        Shape::Poly { pts, .. } if pts.len() >= 2 => {
                            Some(pts[pts.len() - 2])
                        }
                        _ => None,
                    }
                });
                let p = if shift {
                    match prev {
                        Some(a) => self.angle_snap(a, raw),
                        None => self.snap_point(raw),
                    }
                } else {
                    self.snap_point(raw)
                };
                self.mutate_live(id, |o| {
                    if let Shape::Poly { pts, .. } = &mut o.shape {
                        if let Some(last) = pts.last_mut() {
                            *last = p;
                        }
                    }
                });
            }
        }

        // --- Presses --------------------------------------------------

        if response.drag_started_by(egui::PointerButton::Secondary) {
            match self.tool {
                // Right-click finishes the polyline, CAD-style.
                Tool::Polyline => self.finish_gesture(),
                Tool::Select => {
                    if matches!(self.gesture, Gesture::None) {
                        self.selected = None;
                    }
                }
                _ => {}
            }
        }

        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer {
                let raw = to_cell(pos);
                match self.tool {
                    Tool::Select => self.select_press(raw, handle_r, click_slop),
                    Tool::Line => {
                        self.finish_gesture();
                        let a = self.snap_point(raw);
                        let obj = self.new_object(Shape::Line { a, b: a });
                        let id = obj.id;
                        self.model.add(obj);
                        self.gesture = Gesture::DrawShape { id, anchor: a };
                        self.selected = Some(id);
                    }
                    Tool::Rect | Tool::Ellipse => {
                        self.finish_gesture();
                        let a = self.snap_point(raw);
                        let shape = if self.tool == Tool::Rect {
                            Shape::Rect { c: a, half: [0.5, 0.5], angle: 0.0 }
                        } else {
                            Shape::Ellipse { c: a, r: [0.5, 0.5], angle: 0.0 }
                        };
                        let obj = self.new_object(shape);
                        let id = obj.id;
                        self.model.add(obj);
                        self.gesture = Gesture::DrawShape { id, anchor: a };
                        self.selected = Some(id);
                    }
                    Tool::Polyline => {
                        if let Gesture::DrawPoly { id } = &self.gesture {
                            let id = *id;
                            self.poly_click(id, raw, shift, handle_r);
                        } else {
                            self.finish_gesture();
                            let p = self.snap_point(raw);
                            let obj = self.new_object(Shape::Poly {
                                pts: vec![p, p],
                                closed: false,
                            });
                            let id = obj.id;
                            self.model.add(obj);
                            self.gesture = Gesture::DrawPoly { id };
                            self.selected = Some(id);
                            self.status =
                                "Polyline: click to add vertices, Enter/right-click to \
                                 finish, click the first vertex to close."
                                    .into();
                        }
                    }
                    Tool::Pencil => {
                        self.finish_gesture();
                        let obj = self
                            .new_object(Shape::Poly { pts: vec![raw], closed: false });
                        let id = obj.id;
                        self.model.add(obj);
                        self.gesture = Gesture::DrawPencil { id };
                        self.selected = Some(id);
                    }
                }
            }
        }

        // --- Drag updates ---------------------------------------------

        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer {
                let raw = to_cell(pos);
                if let Gesture::DrawShape { id, anchor } = &self.gesture {
                    let (id, anchor) = (*id, *anchor);
                    self.update_draw_shape(id, anchor, raw, shift, alt);
                } else if let Gesture::DrawPencil { id } = &self.gesture {
                    let id = *id;
                    self.mutate_live(id, |o| {
                        if let Shape::Poly { pts, .. } = &mut o.shape {
                            let far = pts
                                .last()
                                .map(|l| Self::dist(*l, raw) >= 2.0)
                                .unwrap_or(true);
                            if far {
                                pts.push(raw);
                            }
                        }
                    });
                } else if let Gesture::MoveObj { id, last, .. } = &self.gesture {
                    let (id, last) = (*id, *last);
                    let eff = if self.snap_enabled { self.snap_point(raw) } else { raw };
                    let d = [eff[0] - last[0], eff[1] - last[1]];
                    if d != [0.0; 2] {
                        self.mutate_live(id, |o| o.translate(d));
                        if let Gesture::MoveObj { last, .. } = &mut self.gesture {
                            *last = eff;
                        }
                    }
                } else if let Gesture::HandleDrag { id, idx, .. } = &self.gesture {
                    let (id, idx) = (*id, *idx);
                    let p = if shift {
                        // Shift angle-snaps a line endpoint about the other.
                        let other = self.model.find(id).and_then(|i| {
                            match &self.model.objects[i].shape {
                                Shape::Line { a, b } => {
                                    Some(if idx == 0 { *b } else { *a })
                                }
                                _ => None,
                            }
                        });
                        match other {
                            Some(o) => self.angle_snap(o, raw),
                            None => self.snap_point(raw),
                        }
                    } else {
                        self.snap_point(raw)
                    };
                    self.mutate_live(id, |o| o.set_handle(idx, p));
                }
            }
        }

        // --- Releases -------------------------------------------------

        if response.drag_stopped_by(egui::PointerButton::Primary) {
            enum Fin {
                Edit,
                Shape(u64),
                Pencil(u64),
                Nothing,
            }
            let fin = match &self.gesture {
                Gesture::MoveObj { .. } | Gesture::HandleDrag { .. } => Fin::Edit,
                Gesture::DrawShape { id, .. } => Fin::Shape(*id),
                Gesture::DrawPencil { id } => Fin::Pencil(*id),
                // A polyline persists across clicks.
                _ => Fin::Nothing,
            };
            match fin {
                Fin::Edit => self.finish_gesture(),
                Fin::Shape(id) => {
                    let degenerate = self
                        .model
                        .find(id)
                        .map(|i| match &self.model.objects[i].shape {
                            Shape::Line { a, b } => Self::dist(*a, *b) < 1.5,
                            Shape::Rect { half, .. }
                            | Shape::Ellipse { r: half, .. } => {
                                half[0] < 1.0 && half[1] < 1.0
                            }
                            _ => false,
                        })
                        .unwrap_or(true);
                    if degenerate {
                        // A click without a drag: don't leave a speck.
                        self.gesture = Gesture::None;
                        self.model.cancel_last_add(id);
                        if self.selected == Some(id) {
                            self.selected = None;
                        }
                    } else {
                        self.finish_gesture();
                    }
                }
                Fin::Pencil(id) => {
                    self.gesture = Gesture::None;
                    self.mutate_live(id, |o| {
                        if let Shape::Poly { pts, .. } = &mut o.shape {
                            *pts = Self::simplify_stroke(pts, 1.2);
                        }
                    });
                    let degenerate = self
                        .model
                        .find(id)
                        .map(|i| match &self.model.objects[i].shape {
                            Shape::Poly { pts, .. } => pts.len() < 2,
                            _ => false,
                        })
                        .unwrap_or(true);
                    if degenerate {
                        self.model.cancel_last_add(id);
                        if self.selected == Some(id) {
                            self.selected = None;
                        }
                    } else {
                        self.model.finalize_last_add(id);
                        self.selected = Some(id);
                    }
                }
                Fin::Nothing => {}
            }
        }
    }

    /// A press with the Select tool: grab a handle of the selected object,
    /// else pick (and start moving) the topmost object under the cursor,
    /// else clear the selection.
    fn select_press(&mut self, p: [f32; 2], handle_r: f32, click_slop: f32) {
        self.finish_gesture();
        if let Some(id) = self.selected {
            if let Some(i) = self.model.find(id) {
                let handles = self.model.objects[i].handles();
                let mut best: Option<(usize, f32)> = None;
                for (idx, h) in handles.iter().enumerate() {
                    let d = Self::dist(p, *h);
                    if d <= handle_r && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                        best = Some((idx, d));
                    }
                }
                if let Some((idx, _)) = best {
                    let before = self.model.objects[i].clone();
                    self.gesture = Gesture::HandleDrag { id, idx, before };
                    return;
                }
            }
        }
        if let Some(id) = self.model.hit_test(p, click_slop) {
            self.selected = Some(id);
            if let Some(i) = self.model.find(id) {
                let before = self.model.objects[i].clone();
                let start = if self.snap_enabled { self.snap_point(p) } else { p };
                self.gesture = Gesture::MoveObj { id, before, last: start };
            }
        } else {
            self.selected = None;
        }
    }

    /// A click while building a polyline: fix the rubber vertex (or close
    /// the polygon when clicking the first vertex).
    fn poly_click(&mut self, id: u64, raw: [f32; 2], shift: bool, handle_r: f32) {
        let Some(i) = self.model.find(id) else {
            self.gesture = Gesture::None;
            return;
        };
        let (first, prev, len) = match &self.model.objects[i].shape {
            Shape::Poly { pts, .. } => (
                pts.first().copied().unwrap_or(raw),
                if pts.len() >= 2 { pts[pts.len() - 2] } else { raw },
                pts.len(),
            ),
            _ => {
                self.gesture = Gesture::None;
                return;
            }
        };
        let p = if shift { self.angle_snap(prev, raw) } else { self.snap_point(raw) };
        if len >= 4 && Self::dist(p, first) <= handle_r {
            // Close the polygon: drop the rubber vertex and mark closed.
            self.gesture = Gesture::None;
            self.mutate_live(id, |o| {
                if let Shape::Poly { pts, closed } = &mut o.shape {
                    pts.pop();
                    *closed = true;
                }
            });
            self.model.finalize_last_add(id);
            self.selected = Some(id);
            self.status = "Closed the polygon.".into();
            return;
        }
        // Fix the rubber vertex at p and start the next one.
        self.mutate_live(id, |o| {
            if let Shape::Poly { pts, .. } = &mut o.shape {
                if let Some(last) = pts.last_mut() {
                    *last = p;
                }
                pts.push(p);
            }
        });
    }

    /// Rubber-band update for line/rect/ellipse drawing, with the CAD
    /// constraints: Shift angle-snaps lines and squares circles; Alt grows
    /// rects/ellipses from their centre.
    fn update_draw_shape(
        &mut self,
        id: u64,
        anchor: [f32; 2],
        raw: [f32; 2],
        shift: bool,
        alt: bool,
    ) {
        let is_line = self
            .model
            .find(id)
            .map(|i| matches!(self.model.objects[i].shape, Shape::Line { .. }))
            .unwrap_or(false);
        if is_line {
            let b = if shift { self.angle_snap(anchor, raw) } else { self.snap_point(raw) };
            self.mutate_live(id, |o| {
                if let Shape::Line { b: bb, .. } = &mut o.shape {
                    *bb = b;
                }
            });
            return;
        }
        let mut q = self.snap_point(raw);
        if shift {
            let dx = q[0] - anchor[0];
            let dy = q[1] - anchor[1];
            let m = dx.abs().max(dy.abs());
            q = [anchor[0] + m * dx.signum(), anchor[1] + m * dy.signum()];
        }
        let (c, half) = if alt {
            (
                anchor,
                [
                    (q[0] - anchor[0]).abs().max(0.5),
                    (q[1] - anchor[1]).abs().max(0.5),
                ],
            )
        } else {
            (
                [(anchor[0] + q[0]) * 0.5, (anchor[1] + q[1]) * 0.5],
                [
                    ((q[0] - anchor[0]).abs() * 0.5).max(0.5),
                    ((q[1] - anchor[1]).abs() * 0.5).max(0.5),
                ],
            )
        };
        self.mutate_live(id, |o| match &mut o.shape {
            Shape::Rect { c: cc, half: hh, .. } | Shape::Ellipse { c: cc, r: hh, .. } => {
                *cc = c;
                *hh = half;
            }
            _ => {}
        });
    }

    /// Selection outline, vertex handles, dimensions and the snap grid.
    fn canvas_overlays(&mut self, ui: &egui::Ui, mapping: ViewportMapping, ppp: f32) {
        let to_screen = |c: [f32; 2]| -> egui::Pos2 {
            egui::pos2(
                (mapping.lb_origin[0] + c[0] * mapping.px_per_cell) / ppp,
                (mapping.lb_origin[1] + c[1] * mapping.px_per_cell) / ppp,
            )
        };
        let painter = ui.painter();

        // Faint snap grid while a draw tool is armed.
        if self.snap_enabled && self.tool != Tool::Select {
            let s = self.snap_spacing.max(1.0);
            let step_pt = s * mapping.px_per_cell / ppp;
            if step_pt >= 8.0 {
                let (vw, vh) = self.stats_grid;
                let stroke = egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 12),
                );
                let mut x = 0.0f32;
                while x <= vw as f32 + 0.1 {
                    painter.line_segment(
                        [to_screen([x, 0.0]), to_screen([x, vh as f32])],
                        stroke,
                    );
                    x += s;
                }
                let mut y = 0.0f32;
                while y <= vh as f32 + 0.1 {
                    painter.line_segment(
                        [to_screen([0.0, y]), to_screen([vw as f32, y])],
                        stroke,
                    );
                    y += s;
                }
            }
        }

        // The active object: the one being drawn/edited, else the selection.
        let active = match &self.gesture {
            Gesture::DrawShape { id, .. }
            | Gesture::DrawPoly { id }
            | Gesture::DrawPencil { id }
            | Gesture::MoveObj { id, .. }
            | Gesture::HandleDrag { id, .. } => Some(*id),
            Gesture::None => self.selected,
        };
        let Some(id) = active else { return };
        let Some(i) = self.model.find(id) else { return };
        let obj = &self.model.objects[i];

        let accent = egui::Color32::from_rgb(255, 200, 90);
        let stroke = egui::Stroke::new(1.5, accent);

        match &obj.shape {
            Shape::Line { a, b } => {
                painter.line_segment([to_screen(*a), to_screen(*b)], stroke);
            }
            Shape::Poly { pts, closed } => {
                let path: Vec<egui::Pos2> = pts.iter().map(|p| to_screen(*p)).collect();
                if *closed {
                    painter.add(egui::Shape::closed_line(path, stroke));
                } else {
                    painter.add(egui::Shape::line(path, stroke));
                }
            }
            Shape::Rect { .. } => {
                let path: Vec<egui::Pos2> =
                    obj.handles().iter().map(|p| to_screen(*p)).collect();
                painter.add(egui::Shape::closed_line(path, stroke));
            }
            Shape::Ellipse { c, r, angle } => {
                let (s, co) = angle.sin_cos();
                let path: Vec<egui::Pos2> = (0..48)
                    .map(|k| {
                        let t = k as f32 / 48.0 * std::f32::consts::TAU;
                        let lx = r[0] * t.cos();
                        let ly = r[1] * t.sin();
                        to_screen([
                            c[0] + lx * co - ly * s,
                            c[1] + lx * s + ly * co,
                        ])
                    })
                    .collect();
                painter.add(egui::Shape::closed_line(path, stroke));
            }
            Shape::Stamp { raster, c, scale, angle } => {
                let w = (raster.rect.2 - raster.rect.0).max(0) as f32 * 0.5 * scale;
                let h = (raster.rect.3 - raster.rect.1).max(0) as f32 * 0.5 * scale;
                let (s, co) = angle.sin_cos();
                let path: Vec<egui::Pos2> = [
                    (-w, -h),
                    (w, -h),
                    (w, h),
                    (-w, h),
                ]
                .into_iter()
                .map(|(lx, ly)| {
                    to_screen([c[0] + lx * co - ly * s, c[1] + lx * s + ly * co])
                })
                .collect();
                painter.add(egui::Shape::closed_line(path, stroke));
            }
        }

        // Vertex handles.
        for h in obj.handles() {
            let pos = to_screen(h);
            let r = egui::Rect::from_center_size(pos, egui::vec2(7.0, 7.0));
            painter.rect_filled(r, 1.0, egui::Color32::WHITE);
            painter.rect_stroke(r, 1.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
        }

        // Dimensions in physical units.
        let ps = self.phys_cache;
        let dims = match &obj.shape {
            Shape::Line { a, b } => {
                let l = Self::dist(*a, *b);
                let ang = -(b[1] - a[1]).atan2(b[0] - a[0]).to_degrees();
                format!("L {}   ∠ {:.1}°", fmt_len(ps.len_m(l)), ang)
            }
            Shape::Poly { pts, closed } => {
                let n = pts.len();
                let segs = if *closed { n } else { n.saturating_sub(1) };
                let mut l = 0.0;
                for k in 0..segs {
                    l += Self::dist(pts[k], pts[(k + 1) % n]);
                }
                format!("{n} pts   L {}", fmt_len(ps.len_m(l)))
            }
            Shape::Rect { half, .. } => format!(
                "{} × {}",
                fmt_len(ps.len_m(half[0] * 2.0)),
                fmt_len(ps.len_m(half[1] * 2.0))
            ),
            Shape::Ellipse { r, .. } => format!(
                "⌀ {} × {}",
                fmt_len(ps.len_m(r[0] * 2.0)),
                fmt_len(ps.len_m(r[1] * 2.0))
            ),
            Shape::Stamp { raster, scale, .. } => format!(
                "{} × {}",
                fmt_len(ps.len_m((raster.rect.2 - raster.rect.0) as f32 * scale)),
                fmt_len(ps.len_m((raster.rect.3 - raster.rect.1) as f32 * scale))
            ),
        };
        let b = obj.bounds();
        let pos = to_screen([b.x0 as f32, b.y0 as f32]) - egui::vec2(0.0, 4.0);
        painter.text(
            pos,
            egui::Align2::LEFT_BOTTOM,
            dims,
            egui::FontId::monospace(12.0),
            accent,
        );
    }


    fn generator_windows(&mut self, ctx: &egui::Context, snap: UiSnapshot) {
        use crate::generators as gen;

        let mut show = self.show_airfoil_gen;
        egui::Window::new("Airfoil generator")
            .open(&mut show)
            .resizable(false)
            .show(ctx, |ui| {
                let p = &mut self.airfoil_params;
                egui::ComboBox::from_label("Famous airfoils")
                    .selected_text("Choose a preset…")
                    .show_ui(ui, |ui| {
                        for (name, m, cp, t, aoa) in gen::AIRFOIL_PRESETS {
                            if ui.selectable_label(false, name).clicked() {
                                p.camber = m;
                                p.camber_pos = if cp > 0.0 { cp } else { 40.0 };
                                p.thickness = t;
                                p.aoa_deg = aoa;
                            }
                        }
                    });
                ui.add_space(4.0);
                ui.add(egui::Slider::new(&mut p.camber, 0.0..=9.0).text("camber %"));
                ui.add(
                    egui::Slider::new(&mut p.camber_pos, 15.0..=70.0)
                        .text("camber position %"),
                );
                ui.add(egui::Slider::new(&mut p.thickness, 4.0..=24.0).text("thickness %"));
                ui.add(egui::Slider::new(&mut p.aoa_deg, -15.0..=20.0).text("angle of attack °"));
                ui.add(
                    egui::Slider::new(&mut p.chord_cells, 60.0..=600.0)
                        .text("chord (cells)"),
                );
                let pos_digit = if p.camber > 0.0 {
                    (p.camber_pos / 10.0).round()
                } else {
                    0.0 // symmetric airfoils are NACA 00xx
                };
                ui.label(format!(
                    "≈ NACA {:.0}{:.0}{:02.0} at {:.1}°",
                    p.camber, pos_digit, p.thickness, p.aoa_deg
                ));
                ui.add_space(6.0);
                if ui.button("Insert into scene").clicked() {
                    let stamp = gen::generate_airfoil(p);
                    self.insert_stamp_object(stamp);
                }
            });
        self.show_airfoil_gen = show;

        let mut show = self.show_nozzle_gen;
        egui::Window::new("Rocket nozzle generator")
            .open(&mut show)
            .resizable(false)
            .show(ctx, |ui| {
                let p = &mut self.nozzle_params;
                egui::ComboBox::from_label("Famous engines")
                    .selected_text("Choose a preset…")
                    .show_ui(ui, |ui| {
                        for (name, eps, contour, ve) in gen::NOZZLE_PRESETS {
                            if ui.selectable_label(false, name).clicked() {
                                // Planar 2D analogue of an axisymmetric area
                                // ratio: width ratio = sqrt(eps).
                                p.exit_ratio = eps.sqrt().clamp(1.2, 20.0);
                                p.contour = contour;
                                p.div_ratio =
                                    (1.5 * (p.exit_ratio - 1.0)).clamp(2.0, 16.0);
                                // Scale the chamber fan so the throat jet
                                // approximates the engine's exhaust as
                                // closely as the solver's speed cap allows.
                                p.fan_mult = (0.27
                                    / (snap.flow * p.chamber_ratio).max(1e-4))
                                .clamp(0.2, 2.0);
                                self.nozzle_fan_auto = true;
                                self.nozzle_real_ve = Some(ve);
                            }
                        }
                    });
                ui.add_space(4.0);
                ui.add(
                    egui::Slider::new(&mut p.throat_cells, 12.0..=100.0)
                        .text("throat width (cells)"),
                );
                ui.add(
                    egui::Slider::new(&mut p.exit_ratio, 1.2..=20.0)
                        .text("exit / throat width"),
                );
                ui.add(
                    egui::Slider::new(&mut p.chamber_ratio, 1.5..=4.0)
                        .text("chamber / throat width"),
                );
                ui.add(
                    egui::Slider::new(&mut p.conv_ratio, 1.0..=4.0)
                        .text("converging length / throat"),
                );
                ui.add(
                    egui::Slider::new(&mut p.div_ratio, 2.0..=16.0)
                        .text("bell length / throat"),
                );
                ui.add(egui::Slider::new(&mut p.wall_cells, 3.0..=12.0).text("wall (cells)"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut p.contour, gen::NozzleContour::Bell, "Bell");
                    ui.radio_value(&mut p.contour, gen::NozzleContour::Conical, "Conical (15°-style)");
                });
                ui.checkbox(&mut p.chamber_fan, "Fan in the chamber (self-powered)");
                if ui
                    .add(
                        egui::Slider::new(&mut p.fan_mult, 0.2..=2.0)
                            .text("chamber fan ×"),
                    )
                    .changed()
                {
                    self.nozzle_fan_auto = false;
                }
                // Expected jet speeds in real units, next to the engine's
                // actual exhaust velocity. The solver clamps lattice speed
                // at 0.3, so the readout is capped the same way.
                let throat_lattice = snap.flow * p.fan_mult * p.chamber_ratio;
                let capped = throat_lattice > 0.3;
                let throat_sim = self.phys_cache.u_phys(throat_lattice.min(0.3));
                ui.label(format!(
                    "sim throat jet ≈ {}{}",
                    fmt_speed(throat_sim),
                    if capped { " (speed-capped)" } else { "" }
                ));
                if let Some(ve) = self.nozzle_real_ve {
                    let factor = ve / throat_sim.max(1e-6);
                    ui.label(
                        egui::RichText::new(format!(
                            "real engine exhaust ≈ {:.0} m/s (~{:.0}× faster — the \
                             incompressible solver caps jet speed, so this is a \
                             scaled approximation)",
                            ve, factor
                        ))
                        .small()
                        .weak(),
                    );
                }
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(
                        "Note: this solver is incompressible (low Mach), so you get \
                         the shape and a jet, not real choked-nozzle gas dynamics.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(6.0);
                if ui.button("Insert into scene").clicked() {
                    // Track the preset formula unless the user overrode
                    // the slider — what the dialog shows is what inserts.
                    if self.nozzle_fan_auto {
                        p.fan_mult = (0.27 / (snap.flow * p.chamber_ratio).max(1e-4))
                            .clamp(0.2, 2.0);
                    }
                    let stamp = gen::generate_nozzle(p);
                    self.insert_stamp_object(stamp);
                }
            });
        self.show_nozzle_gen = show;
    }


    fn windows(&mut self, ctx: &egui::Context, snap: UiSnapshot) {
        self.generator_windows(ctx, snap);

        egui::Window::new("About FlowPaint V2")
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    "FlowPaint V2 solves the 2D Navier-Stokes equations in real \
                     time with a D2Q9 lattice-Boltzmann method in GPU compute \
                     shaders (wgpu: Vulkan / DX12 / Metal).",
                );
                ui.add_space(6.0);
                ui.label(
                    "Everything you sketch is a live object: select it any time \
                     to move, rotate, resize or retune its physics while the \
                     fluid reacts.",
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
                        (
                            "S / L / R / E / P / B",
                            "select / line / rect / ellipse / polyline / pencil",
                        ),
                        ("Del", "delete the selected object"),
                        ("Ctrl+D", "duplicate the selected object"),
                        ("Arrows (+Shift for fine)", "nudge the selected object"),
                        ("Shift while drawing", "angle-snapped lines · squares · circles"),
                        ("Alt while drawing", "rect/ellipse from centre"),
                        ("Enter / right-click", "finish the polyline"),
                        ("Esc", "cancel gesture / deselect"),
                    ] {
                        ui.label(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
            });
    }
}

// --- The wgpu paint callback ------------------------------------------

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
