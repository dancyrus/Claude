//! The FlowPaint application: an MS-Paint-style shell (menu bar, tool
//! palette, status bar) around the GPU fluid canvas.

use crate::geometry::{
    BrushContext, GeoRegion, Geometry, GridRect, Material, Preset, UndoEntry,
};
use crate::sim::{
    GpuSim, RenderMode, ViewportMapping, DEFAULT_MARGIN_INDEX, MARGIN_CHOICES,
    PARTICLE_CHOICES, RESOLUTIONS,
};
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Brush,
    Line,
    Rect,
    Ellipse,
    Polyline,
    Eraser,
    Select,
}

impl Tool {
    const ALL: [(Tool, &'static str, &'static str); 7] = [
        (Tool::Brush, "Brush", "B"),
        (Tool::Line, "Line", "L"),
        (Tool::Rect, "Rectangle", "R"),
        (Tool::Ellipse, "Ellipse", "E"),
        (Tool::Polyline, "Polyline", "P"),
        (Tool::Eraser, "Eraser", "X"),
        (Tool::Select, "Select", "S"),
    ];
}

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

/// A floating selection: raster content lifted off the grid, live-stamped
/// at its current transform so the fluid keeps reacting while you move it.
pub struct Selection {
    /// Source raster, rect based at (0, 0).
    source: GeoRegion,
    /// Centre position in visible-canvas cell coordinates.
    pos: [f32; 2],
    angle_deg: f32,
    scale: f32,
    flip_h: bool,
    flip_v: bool,
}

impl Selection {
    fn source_dims(&self) -> (f32, f32) {
        let (x0, y0, x1, y1) = self.source.rect;
        ((x1 - x0) as f32, (y1 - y0) as f32)
    }

    /// Rotated/scaled half-extents of the stamped footprint.
    fn half_extents(&self) -> (f32, f32) {
        let (sw, sh) = self.source_dims();
        let (s, c) = self.angle_deg.to_radians().sin_cos();
        let hx = sw * 0.5 * self.scale;
        let hy = sh * 0.5 * self.scale;
        ((hx * c).abs() + (hy * s).abs(), (hx * s).abs() + (hy * c).abs())
    }

    /// Map a point in visible-cell coordinates to source raster
    /// coordinates (None if outside the source rect).
    fn point_to_source(&self, p: [f32; 2]) -> Option<(usize, usize)> {
        let (sw, sh) = self.source_dims();
        let (s, c) = self.angle_deg.to_radians().sin_cos();
        let dx = p[0] - self.pos[0];
        let dy = p[1] - self.pos[1];
        // Inverse rotation (y-down grid), then inverse scale and flips.
        let rx = dx * c + dy * s;
        let ry = -dx * s + dy * c;
        let mut sx = rx / self.scale;
        let mut sy = ry / self.scale;
        if self.flip_h {
            sx = -sx;
        }
        if self.flip_v {
            sy = -sy;
        }
        let sx = sx + sw * 0.5;
        let sy = sy + sh * 0.5;
        if sx >= 0.0 && sy >= 0.0 && sx < sw && sy < sh {
            Some((sx as usize, sy as usize))
        } else {
            None
        }
    }

    /// Rotate a fan vector by the selection's transform. The magnitude
    /// (speed multiplier) is preserved; gustiness and phase pass through.
    fn transform_fan(&self, v: [f32; 4]) -> [f32; 4] {
        let (s, c) = self.angle_deg.to_radians().sin_cos();
        let vx = if self.flip_h { -v[0] } else { v[0] };
        let vy = if self.flip_v { -v[1] } else { v[1] };
        [vx * c - vy * s, vx * s + vy * c, v[2], v[3]]
    }

    /// Outline corners in visible-cell coordinates (for the overlay).
    fn corners(&self) -> [[f32; 2]; 4] {
        let (sw, sh) = self.source_dims();
        let (s, c) = self.angle_deg.to_radians().sin_cos();
        let hx = sw * 0.5 * self.scale;
        let hy = sh * 0.5 * self.scale;
        let mut out = [[0.0f32; 2]; 4];
        for (k, (lx, ly)) in
            [(-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy)].into_iter().enumerate()
        {
            out[k] = [
                self.pos[0] + lx * c - ly * s,
                self.pos[1] + lx * s + ly * c,
            ];
        }
        out
    }
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
    /// Select tool: true when dragging the floating selection itself
    /// (translate), false when rubber-banding a new marquee.
    sel_move: bool,
    /// Sketch tools: the pending-sketch vertex being dragged, if any.
    sketch_handle: Option<usize>,
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

/// Sketch entity kinds (CAD-style: drawn as editable outlines first,
/// rasterized only on commit).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SketchKind {
    Line,
    Rect,
    Ellipse,
    Polyline,
}

/// An in-progress sketch: vertices stay editable (drag the handles)
/// until the sketch is committed with Enter / right-click, or cancelled
/// with Esc. Line/Rect/Ellipse store two defining vertices (for
/// rect/ellipse: opposite corners); Polyline stores every vertex.
struct PendingSketch {
    kind: SketchKind,
    verts: Vec<[f32; 2]>,
}

impl PendingSketch {
    /// Editable handles in visible-cell coords (rect/ellipse expose all
    /// four corners).
    fn handles(&self) -> Vec<[f32; 2]> {
        match self.kind {
            SketchKind::Rect | SketchKind::Ellipse => {
                let a = self.verts[0];
                let b = self.verts[1];
                vec![a, [b[0], a[1]], b, [a[0], b[1]]]
            }
            _ => self.verts.clone(),
        }
    }

    /// Prepare to drag handle `h`: returns the vertex index to move.
    /// For rect/ellipse the defining pair is rearranged so the dragged
    /// corner becomes verts[1] and its diagonal opposite verts[0].
    fn begin_handle_drag(&mut self, h: usize) -> usize {
        match self.kind {
            SketchKind::Rect | SketchKind::Ellipse => {
                let hs = self.handles();
                let dragged = hs[h.min(3)];
                let opp = hs[(h + 2) % 4];
                self.verts[0] = opp;
                self.verts[1] = dragged;
                1
            }
            _ => h.min(self.verts.len().saturating_sub(1)),
        }
    }
}

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
    can_undo: bool,
    can_redo: bool,
    mode: RenderMode,
    display_gain: f32,
    smoke_gain: f32,
    particle_size: f32,
    particle_brightness: f32,
    sponge_strength: f32,
}

pub struct FlowPaintApp {
    tool: Tool,
    material: Material,
    brush_radius: f32,
    dye_color: egui::Color32,
    fan_dir: [f32; 2],
    // Fan physics for newly painted fans.
    fan_speed_mult: f32,
    fan_gustiness: f32,
    fan_phase: f32,
    fan_phase_counter: u32,
    // Fan-editing sliders for the current selection.
    sel_fan_mult: f32,
    sel_fan_turb: f32,
    // CAD sketch aids.
    snap_enabled: bool,
    snap_spacing: f32,
    /// Angle-snap increment for Shift-constrained lines, degrees.
    snap_angle_deg: f32,
    /// Wall thickness for committed sketch entities, cells.
    wall_thickness: f32,
    /// Commit rect/ellipse as filled slabs instead of outlines.
    shape_filled: bool,
    /// The editable, not-yet-committed sketch entity.
    pending_sketch: Option<PendingSketch>,
    drag: Option<DragState>,
    // Freehand-stroke undo bookkeeping. These live on the app (not the
    // drag state) because canvas events are queued as commands and applied
    // after the UI pass; they are only valid once the commands run.
    // Pre-stroke contents are captured lazily as fixed-size tiles the
    // first time a stamp touches them, so stroke start never has to copy
    // the whole grid.
    pending_stroke_rect: GridRect,
    pending_stroke_tiles: std::collections::HashMap<(i32, i32), GeoRegion>,
    // Floating selection state. `selection_bg` holds the pre-stamp content
    // of the currently stamped footprint so moving the selection can
    // restore what was underneath.
    selection: Option<Selection>,
    selection_bg: Option<GeoRegion>,
    clipboard: Option<GeoRegion>,
    // Generator dialogs.
    show_airfoil_gen: bool,
    show_nozzle_gen: bool,
    // Fluid preset combo state (None = custom).
    fluid_preset_idx: Option<usize>,
    /// Real exhaust velocity of the last-picked nozzle engine preset.
    nozzle_real_ve: Option<f32>,
    // Physical scaling: the canvas maps to a real domain of this width,
    // and the current fluid's physical properties anchor all unit
    // conversions (lattice viscosity fixes the physical time step).
    domain_width_m: f32,
    fluid_name: &'static str,
    fluid_nu: f32,
    fluid_rho: f32,
    show_legend: bool,
    stats_steps_per_s: f32,
    stats_sim_steps: f64,
    /// Cached physical scale for this frame (from the latest snapshot).
    phys_cache: PhysScale,
    // Whether the current selection contains fan cells (cached).
    sel_has_fans: bool,
    // Cursor cell coordinates for the status bar.
    hover_cell: Option<[f32; 2]>,
    airfoil_params: crate::generators::AirfoilParams,
    nozzle_params: crate::generators::NozzleParams,
    show_about: bool,
    show_shortcuts: bool,
    res_index: usize,
    particle_index: usize,
    margin_index: usize,
    status: String,
    // Stats copied out of the sim each frame.
    stats_grid: (usize, usize),
    stats_full: (usize, usize),
    stats_margin: usize,
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
            fan_speed_mult: 1.0,
            fan_gustiness: 0.0,
            fan_phase: 0.0,
            fan_phase_counter: 0,
            sel_fan_mult: 1.0,
            sel_fan_turb: 0.0,
            snap_enabled: false,
            snap_spacing: 10.0,
            snap_angle_deg: 45.0,
            wall_thickness: 6.0,
            shape_filled: false,
            pending_sketch: None,
            drag: None,
            pending_stroke_rect: GridRect::empty(),
            pending_stroke_tiles: std::collections::HashMap::new(),
            selection: None,
            selection_bg: None,
            clipboard: None,
            show_airfoil_gen: false,
            show_nozzle_gen: false,
            fluid_preset_idx: Some(2),
            nozzle_real_ve: None,
            domain_width_m: 1.0,
            fluid_name: "air",
            fluid_nu: 1.5e-5,
            fluid_rho: 1.2,
            show_legend: true,
            stats_steps_per_s: 0.0,
            stats_sim_steps: 0.0,
            phys_cache: PhysScale::default(),
            sel_has_fans: false,
            hover_cell: None,
            airfoil_params: crate::generators::AirfoilParams::default(),
            nozzle_params: crate::generators::NozzleParams::default(),
            show_about: false,
            show_shortcuts: false,
            res_index,
            particle_index: 0,
            margin_index: DEFAULT_MARGIN_INDEX,
            status: String::from("Draw walls with the brush; hold right-click to erase."),
            stats_grid: (0, 0),
            stats_full: (0, 0),
            stats_margin: 0,
            stats_mlups: 0.0,
            stats_re: 0,
        }
    }

    fn brush_ctx(&self) -> BrushContext {
        let c = self.dye_color;
        BrushContext {
            fan_dir: self.fan_dir,
            fan_mult: self.fan_speed_mult,
            fan_turb: self.fan_gustiness,
            fan_phase: self.fan_phase,
            dye_rgb: [
                c.r() as f32 / 255.0,
                c.g() as f32 / 255.0,
                c.b() as f32 / 255.0,
            ],
        }
    }

    /// Physical scale for the current settings.
    fn phys_scale(&self, visc_lattice: f32) -> PhysScale {
        let vis_w = self.stats_grid.0.max(1);
        let dx = self.domain_width_m / vis_w as f32;
        let dt = visc_lattice.max(1e-5) * dx * dx / self.fluid_nu.max(1e-12);
        PhysScale { dx, dt }
    }

    /// New stroke, new gust phase: each painted fan wanders on its own
    /// schedule, while all cells of one stroke stay coherent.
    fn roll_fan_phase(&mut self) {
        self.fan_phase_counter = self.fan_phase_counter.wrapping_add(1);
        self.fan_phase = (self.fan_phase_counter as f32 * 0.618_034) % 1.0;
    }

    // --- CAD sketch helpers ------------------------------------------

    /// Snap a point to the sketch grid (shape tools only).
    fn snap_point(&self, p: [f32; 2]) -> [f32; 2] {
        if !self.snap_enabled {
            return p;
        }
        let s = self.snap_spacing.max(1.0);
        [(p[0] / s).round() * s, (p[1] / s).round() * s]
    }

    /// Snap the segment a->b to the configured angle increment
    /// (CAD ortho/polar snapping).
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

    /// The endpoints a shape tool will actually commit, given the raw drag
    /// endpoints and current modifiers: grid snap, Shift = 45-degree lines
    /// / squares / circles, Alt = rect/ellipse from centre.
    fn effective_shape(
        &self,
        tool: Tool,
        a: [f32; 2],
        b_raw: [f32; 2],
        shift: bool,
        alt: bool,
    ) -> ([f32; 2], [f32; 2]) {
        let mut b = self.snap_point(b_raw);
        match tool {
            Tool::Line | Tool::Polyline => {
                if shift {
                    b = self.angle_snap(a, b);
                }
                (a, b)
            }
            Tool::Rect | Tool::Ellipse => {
                if shift {
                    // Square / circle: equal extents, keeping direction.
                    let dx = b[0] - a[0];
                    let dy = b[1] - a[1];
                    let m = dx.abs().max(dy.abs());
                    b = [a[0] + m * dx.signum(), a[1] + m * dy.signum()];
                }
                if alt {
                    // Draw from centre: a is the centre, mirror to corner.
                    ([2.0 * a[0] - b[0], 2.0 * a[1] - b[1]], b)
                } else {
                    (a, b)
                }
            }
            _ => (a, b_raw),
        }
    }

    /// Constrained position for a dragged sketch handle: grid snap, plus
    /// Shift = angle snap (lines/polylines) or square/circle
    /// (rect/ellipse, relative to the fixed opposite corner).
    fn constrained_handle_pos(
        &self,
        kind: SketchKind,
        idx: usize,
        cell: [f32; 2],
        shift: bool,
    ) -> [f32; 2] {
        let mut p = self.snap_point(cell);
        if !shift {
            return p;
        }
        if let Some(sk) = &self.pending_sketch {
            match kind {
                SketchKind::Line => {
                    if sk.verts.len() >= 2 {
                        let other = sk.verts[1 - idx.min(1)];
                        p = self.angle_snap(other, p);
                    }
                }
                SketchKind::Polyline => {
                    let anchor = if idx > 0 {
                        sk.verts.get(idx - 1).copied()
                    } else {
                        sk.verts.get(1).copied()
                    };
                    if let Some(a) = anchor {
                        p = self.angle_snap(a, p);
                    }
                }
                SketchKind::Rect | SketchKind::Ellipse => {
                    let a = sk.verts[0];
                    let dx = p[0] - a[0];
                    let dy = p[1] - a[1];
                    let m = dx.abs().max(dy.abs());
                    p = [a[0] + m * dx.signum(), a[1] + m * dy.signum()];
                }
            }
        }
        p
    }

    /// The sketch kind a tool draws, if any.
    fn sketch_kind(tool: Tool) -> Option<SketchKind> {
        match tool {
            Tool::Line => Some(SketchKind::Line),
            Tool::Rect => Some(SketchKind::Rect),
            Tool::Ellipse => Some(SketchKind::Ellipse),
            Tool::Polyline => Some(SketchKind::Polyline),
            _ => None,
        }
    }

    /// Rasterize and commit the pending sketch as one undo entry.
    /// Lines/polylines/outlines are capsule chains at the configured
    /// wall thickness; rect/ellipse optionally commit filled.
    fn commit_sketch(&mut self, cmds: &mut Vec<Cmd>) {
        let Some(sk) = self.pending_sketch.take() else { return };
        let r = (self.wall_thickness * 0.5).max(0.75);
        self.roll_fan_phase();
        // Pin the phase at queue time: another stroke starting this same
        // frame will roll again, and stamps read the phase at apply time.
        cmds.push(Cmd::SetFanPhase(self.fan_phase));

        // Chain of capsule segments as one stroke session.
        let chain = |cmds: &mut Vec<Cmd>, pts: &[[f32; 2]], closed: bool| {
            cmds.push(Cmd::StrokeBegin);
            if pts.len() == 1 {
                cmds.push(Cmd::StampSegment {
                    a: pts[0],
                    b: pts[0],
                    r,
                    material: self.material,
                });
            } else {
                let n = pts.len();
                let segs = if closed { n } else { n - 1 };
                for s in 0..segs {
                    let a = pts[s];
                    let b = pts[(s + 1) % n];
                    let dx = b[0] - a[0];
                    let dy = b[1] - a[1];
                    let len = (dx * dx + dy * dy).sqrt();
                    // Fan ducts blow along each segment.
                    if self.material == Material::Fan && len > 1e-3 {
                        cmds.push(Cmd::SetFanDir([dx / len, dy / len]));
                    }
                    cmds.push(Cmd::StampSegment { a, b, r, material: self.material });
                }
            }
            cmds.push(Cmd::StrokeEnd);
        };

        match sk.kind {
            SketchKind::Line | SketchKind::Polyline => chain(cmds, &sk.verts, false),
            SketchKind::Rect => {
                let a = sk.verts[0];
                let b = sk.verts[1];
                if self.shape_filled {
                    cmds.push(Cmd::ShapeCommit {
                        tool: Tool::Rect,
                        a,
                        b,
                        r,
                        material: self.material,
                    });
                } else {
                    let corners =
                        [a, [b[0], a[1]], b, [a[0], b[1]]];
                    chain(cmds, &corners, true);
                }
            }
            SketchKind::Ellipse => {
                let a = sk.verts[0];
                let b = sk.verts[1];
                if self.shape_filled {
                    cmds.push(Cmd::ShapeCommit {
                        tool: Tool::Ellipse,
                        a,
                        b,
                        r,
                        material: self.material,
                    });
                } else {
                    let c = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                    let rx = ((a[0] - b[0]).abs() * 0.5).max(0.5);
                    let ry = ((a[1] - b[1]).abs() * 0.5).max(0.5);
                    let n = 64usize;
                    let pts: Vec<[f32; 2]> = (0..n)
                        .map(|i| {
                            let t = i as f32 / n as f32 * std::f32::consts::TAU;
                            [c[0] + rx * t.cos(), c[1] + ry * t.sin()]
                        })
                        .collect();
                    chain(cmds, &pts, true);
                }
            }
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
                display_gain: sim.settings.display_gain,
                smoke_gain: sim.settings.smoke_gain,
                particle_size: sim.settings.particle_size,
                particle_brightness: sim.settings.particle_brightness,
                sponge_strength: sim.settings.sponge_strength,
            }
        };

        self.phys_cache = self.phys_scale(snapshot.visc);

        self.keyboard(ctx, &mut cmds);
        // A selection only lives under the Select tool; switching away
        // commits it. This runs AFTER keyboard() so a same-frame tool
        // hotkey still gets the commit queued before any canvas commands.
        if self.selection.is_some() && self.tool != Tool::Select {
            cmds.push(Cmd::SelectCommit);
        }
        // Likewise a pending sketch commits when the tool changes to one
        // that doesn't edit it.
        if let Some(sk) = &self.pending_sketch {
            if Self::sketch_kind(self.tool) != Some(sk.kind) {
                self.commit_sketch(&mut cmds);
            }
        }
        self.menu_bar(ctx, &mut cmds);
        self.side_panel(ctx, snapshot, &mut cmds);
        self.legend_panel(ctx, snapshot);
        self.status_bar(ctx);
        self.canvas(ctx, &mut cmds);
        self.windows(ctx, snapshot, &mut cmds);

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
        self.stats_full = sim.full_size();
        self.stats_margin = sim.margin();
        let dt = ctx.input(|i| i.stable_dt).max(1e-4);
        let n = (self.stats_full.0 * self.stats_full.1) as f32;
        self.stats_mlups = n * sim.steps_last_frame as f32 / dt / 1.0e6;
        self.stats_re = sim.reynolds_estimate();
        self.stats_steps_per_s = sim.steps_last_frame as f32 / dt;
        self.stats_sim_steps = sim.total_steps;
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
    /// Per-segment fan direction override (used by the polyline tool so
    /// each segment of a fan duct blows along itself).
    SetFanDir([f32; 2]),
    /// Pin the gust phase for subsequently applied stamps (queued at
    /// stroke-queue time so same-frame strokes keep distinct phases).
    SetFanPhase(f32),
    /// Retune the fan cells inside the floating selection. Only the
    /// properties that are Some are written, so touching one slider
    /// doesn't homogenize the other across mixed fans.
    SetSelectionFanPhysics { mult: Option<f32>, turb: Option<f32> },
    // Selection (coordinates in visible cells).
    SelectCut { a: [f32; 2], b: [f32; 2] },
    /// Click-select the connected fan under this point (visible cells).
    SelectFanAt([f32; 2]),
    SelectUpdate,
    SelectCommit,
    SelectCancel,
    SelectDelete,
    CopySelection,
    PasteClipboard,
    /// Insert generated content as a new floating selection.
    InsertStamp(GeoRegion),
}

/// Tile edge length for lazy pre-stroke capture.
const UNDO_TILE: i32 = 64;

/// Drop any half-tracked stroke state (used when the grid is replaced or
/// wholesale-cleared so stale snapshots can never pair with a new grid).
fn clear_stroke_state(app: &mut FlowPaintApp) {
    app.pending_stroke_rect = GridRect::empty();
    app.pending_stroke_tiles.clear();
    app.drag = None;
    // Sketch vertices are absolute visible-cell coordinates; they are
    // meaningless on a replaced grid.
    app.pending_sketch = None;
    // A floating selection is abandoned outright: the grid it was stamped
    // into is being replaced wholesale.
    app.selection = None;
    app.selection_bg = None;
}

/// Capture the pre-session contents of every 64x64 tile `rect` overlaps
/// that has not been captured yet this stroke/selection session.
fn capture_tiles(app: &mut FlowPaintApp, sim: &GpuSim, rect: GridRect) {
    if rect.is_empty() {
        return;
    }
    for ty in (rect.y0 / UNDO_TILE)..=((rect.y1 - 1) / UNDO_TILE) {
        for tx in (rect.x0 / UNDO_TILE)..=((rect.x1 - 1) / UNDO_TILE) {
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

/// Finish the current undo session: build one entry from the captured
/// tiles and the current grid over the accumulated union rect.
fn push_stroke_undo(sim: &mut GpuSim, app: &mut FlowPaintApp) {
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

/// Re-stamp the floating selection at its current transform: restore what
/// was under the previous stamp, then stamp at the new position.
fn selection_update(sim: &mut GpuSim, app: &mut FlowPaintApp) {
    let m = sim.margin() as f32;
    let Some(sel) = app.selection.as_ref() else { return };

    if let Some(bg) = app.selection_bg.take() {
        sim.geo.restore(&bg);
    }

    let (ex, ey) = sel.half_extents();
    let center = [sel.pos[0] + m, sel.pos[1] + m];
    let bbox = GridRect {
        x0: (center[0] - ex).floor() as i32 - 1,
        y0: (center[1] - ey).floor() as i32 - 1,
        x1: (center[0] + ex).ceil() as i32 + 1,
        y1: (center[1] + ey).ceil() as i32 + 1,
    }
    .clampped(sim.geo.w, sim.geo.h);
    if bbox.is_empty() {
        return; // dragged fully off the grid; nothing stamped
    }

    capture_tiles(app, sim, bbox);
    let sel = app.selection.as_ref().unwrap();
    app.selection_bg = Some(sim.geo.extract(bbox));

    let src = &sel.source;
    let src_w = (src.rect.2 - src.rect.0) as usize;
    let gw = sim.geo.w;
    for y in bbox.y0..bbox.y1 {
        for x in bbox.x0..bbox.x1 {
            let p_vis = [x as f32 + 0.5 - m, y as f32 + 0.5 - m];
            let Some((sx, sy)) = sel.point_to_source(p_vis) else { continue };
            let si = sy * src_w + sx;
            let non_empty = src.cell[si] != crate::geometry::CELL_FLUID
                || src.dye_src[si][3] > 0.0;
            if !non_empty {
                continue; // fluid is transparent
            }
            let gi = (y as usize) * gw + x as usize;
            sim.geo.cell[gi] = src.cell[si];
            sim.geo.fan[gi] = sel.transform_fan(src.fan[si]);
            sim.geo.dye_src[gi] = src.dye_src[si];
        }
    }
    sim.geo.touch(bbox);
    app.pending_stroke_rect = app.pending_stroke_rect.union(bbox);
}

/// Commit the floating selection: its stamp stays, one undo entry covers
/// the whole session.
fn commit_selection(sim: &mut GpuSim, app: &mut FlowPaintApp) {
    if app.selection.take().is_some() {
        app.selection_bg = None;
        push_stroke_undo(sim, app);
    }
}

/// Lift `rect` (full-grid coords) off the grid into a new floating
/// selection, beginning an undo session. Shared by marquee cuts and
/// click-to-select-fan.
fn cut_rect_into_selection(sim: &mut GpuSim, app: &mut FlowPaintApp, rect: GridRect) {
    if rect.is_empty() {
        return;
    }
    let m = sim.margin() as f32;
    app.pending_stroke_rect = GridRect::empty();
    app.pending_stroke_tiles.clear();
    capture_tiles(app, sim, rect);
    let mut source = sim.geo.extract(rect);
    let w = rect.x1 - rect.x0;
    let h = rect.y1 - rect.y0;
    source.rect = (0, 0, w, h);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let i = (y as usize) * sim.geo.w + x as usize;
            sim.geo.cell[i] = crate::geometry::CELL_FLUID;
            sim.geo.fan[i] = [0.0; 4];
            sim.geo.dye_src[i] = [0.0; 4];
        }
    }
    sim.geo.touch(rect);
    app.pending_stroke_rect = rect;
    let pos = [
        (rect.x0 + rect.x1) as f32 * 0.5 - m,
        (rect.y0 + rect.y1) as f32 * 0.5 - m,
    ];
    app.selection = Some(Selection {
        source,
        pos,
        angle_deg: 0.0,
        scale: 1.0,
        flip_h: false,
        flip_v: false,
    });
    app.selection_bg = None;
    refresh_sel_fan_cache(app);
    selection_update(sim, app);
}

/// Cache whether the selection holds fan cells, and seed the fan-editing
/// sliders from the first one found.
fn refresh_sel_fan_cache(app: &mut FlowPaintApp) {
    app.sel_has_fans = false;
    if let Some(sel) = app.selection.as_ref() {
        for (i, &c) in sel.source.cell.iter().enumerate() {
            if c == crate::geometry::CELL_INLET {
                let f = sel.source.fan[i];
                app.sel_fan_mult = (f[0] * f[0] + f[1] * f[1]).sqrt().clamp(0.2, 2.0);
                app.sel_fan_turb = f[2].clamp(0.0, 1.0);
                app.sel_has_fans = true;
                break;
            }
        }
    }
}

/// Start a new selection session around `source` (already based at 0,0).
/// Used by paste and generator insertion; fan cells in the incoming
/// content get a fresh gust phase so separate insertions decorrelate.
fn start_selection(sim: &mut GpuSim, app: &mut FlowPaintApp, mut source: GeoRegion, pos: [f32; 2]) {
    commit_selection(sim, app);
    app.roll_fan_phase();
    for (i, &c) in source.cell.iter().enumerate() {
        if c == crate::geometry::CELL_INLET {
            source.fan[i][3] = app.fan_phase;
        }
    }
    app.pending_stroke_rect = GridRect::empty();
    app.pending_stroke_tiles.clear();
    app.selection = Some(Selection {
        source,
        pos,
        angle_deg: 0.0,
        scale: 1.0,
        flip_h: false,
        flip_v: false,
    });
    app.selection_bg = None;
    app.tool = Tool::Select;
    refresh_sel_fan_cache(app);
    selection_update(sim, app);
}

/// Largest raster (in cells) `bake_selection` will produce.
const MAX_BAKE_CELLS: usize = 16 << 20;

/// Bake the selection's current transform into a flat raster (for copy).
/// Returns None when the transformed footprint is unreasonably large.
fn bake_selection(sel: &Selection) -> Option<GeoRegion> {
    let (ex, ey) = sel.half_extents();
    let bw = (2.0 * ex).ceil() as usize + 2;
    let bh = (2.0 * ey).ceil() as usize + 2;
    if bw.saturating_mul(bh) > MAX_BAKE_CELLS {
        return None;
    }
    let src_w = (sel.source.rect.2 - sel.source.rect.0) as usize;
    let mut out = GeoRegion {
        rect: (0, 0, bw as i32, bh as i32),
        cell: vec![crate::geometry::CELL_FLUID; bw * bh],
        fan: vec![[0.0; 4]; bw * bh],
        dye_src: vec![[0.0; 4]; bw * bh],
    };
    // Sample through a probe selection centred in the bake raster.
    let probe = Selection {
        source: GeoRegion {
            rect: sel.source.rect,
            cell: Vec::new(), // not used by point_to_source
            fan: Vec::new(),
            dye_src: Vec::new(),
        },
        pos: [bw as f32 * 0.5, bh as f32 * 0.5],
        angle_deg: sel.angle_deg,
        scale: sel.scale,
        flip_h: sel.flip_h,
        flip_v: sel.flip_v,
    };
    for y in 0..bh {
        for x in 0..bw {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let Some((sx, sy)) = probe.point_to_source(p) else { continue };
            let si = sy * src_w + sx;
            if sel.source.cell[si] != crate::geometry::CELL_FLUID
                || sel.source.dye_src[si][3] > 0.0
            {
                let di = y * bw + x;
                out.cell[di] = sel.source.cell[si];
                out.fan[di] = sel.transform_fan(sel.source.fan[si]);
                out.dye_src[di] = sel.source.dye_src[si];
            }
        }
    }
    Some(out)
}

fn apply_cmd(sim: &mut GpuSim, cmd: Cmd, app: &mut FlowPaintApp) {
    match cmd {
        Cmd::TogglePause => sim.settings.paused = !sim.settings.paused,
        Cmd::ResetFlow => sim.reset_flow(),
        Cmd::ClearAll => {
            commit_selection(sim, app);
            clear_stroke_state(app);
            sim.clear_all();
        }
        Cmd::SetWindTunnel(on) => {
            // Resolve the selection first so its session snapshots can't
            // resurrect stale tunnel cells later.
            commit_selection(sim, app);
            sim.set_wind_tunnel(on);
        }
        Cmd::Preset(p) => {
            commit_selection(sim, app);
            clear_stroke_state(app);
            sim.apply_preset(p);
        }
        Cmd::SetResolution(i) => {
            // Commit (not abandon) the selection: set_resolution may
            // no-op (same size clicked) or the rebuild may clamp to the
            // same grid, and an abandoned session would leave the stamp
            // baked with no undo entry.
            commit_selection(sim, app);
            clear_stroke_state(app);
            sim.set_resolution(i);
        }
        Cmd::SetMargin(i) => {
            commit_selection(sim, app);
            clear_stroke_state(app);
            sim.set_margin_frac(MARGIN_CHOICES[i].1);
        }
        Cmd::SetParticles(nn) => sim.settings.particle_count = nn,
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
        Cmd::Undo => {
            // A floating selection must be resolved first: undoing under
            // it would desynchronize its background snapshot and session
            // tiles from the grid.
            commit_selection(sim, app);
            sim.undo_action();
        }
        Cmd::Redo => {
            commit_selection(sim, app);
            sim.redo_action();
        }
        Cmd::SaveScene(p) => {
            app.status = match sim.save_scene(&p) {
                Ok(()) => format!("Saved {}", p.display()),
                Err(e) => format!("Save failed: {e}"),
            };
        }
        Cmd::LoadScene(p) => {
            commit_selection(sim, app);
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
            // Canvas coordinates are visible-window relative; offset into
            // the full grid.
            let m = sim.margin() as f32;
            let a = [a[0] + m, a[1] + m];
            let b = [b[0] + m, b[1] + m];
            // Capture the pre-stroke contents of every tile this stamp can
            // touch before stamping (first touch wins, so tiles keep their
            // true pre-stroke data for the whole stroke).
            let bound =
                Geometry::capsule_bounds(a, b, r).clampped(sim.geo.w, sim.geo.h);
            capture_tiles(app, sim, bound);
            let ctx = app.brush_ctx();
            let rect = sim.geo.stamp_capsule(a, b, r, material, &ctx);
            app.pending_stroke_rect = app.pending_stroke_rect.union(rect);
        }
        Cmd::ShapeCommit { tool, a, b, r, material } => {
            let m = sim.margin() as f32;
            let a = [a[0] + m, a[1] + m];
            let b = [b[0] + m, b[1] + m];
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
            push_stroke_undo(sim, app);
        }
        Cmd::SetFanDir(d) => {
            app.fan_dir = d;
        }
        Cmd::SetFanPhase(p) => {
            app.fan_phase = p;
        }
        Cmd::SetSelectionFanPhysics { mult, turb } => {
            let mut changed = false;
            if let Some(sel) = app.selection.as_mut() {
                for (i, f) in sel.source.fan.iter_mut().enumerate() {
                    if sel.source.cell[i] != crate::geometry::CELL_INLET {
                        continue;
                    }
                    if let Some(mult) = mult {
                        let len = (f[0] * f[0] + f[1] * f[1]).sqrt();
                        if len > 1e-4 {
                            let k = mult / len;
                            f[0] *= k;
                            f[1] *= k;
                        } else {
                            f[0] = mult;
                            f[1] = 0.0;
                        }
                    }
                    if let Some(turb) = turb {
                        f[2] = turb;
                    }
                    changed = true;
                }
            }
            if changed {
                selection_update(sim, app);
            }
        }
        Cmd::SelectCut { a, b } => {
            commit_selection(sim, app);
            let m = sim.margin() as f32;
            // Clamp the marquee to the visible window: at margin 0 this
            // keeps the wind tunnel's edge columns out of the cut.
            let rect = GridRect {
                x0: (a[0].min(b[0]) + m).floor() as i32,
                y0: (a[1].min(b[1]) + m).floor() as i32,
                x1: (a[0].max(b[0]) + m).ceil() as i32,
                y1: (a[1].max(b[1]) + m).ceil() as i32,
            }
            .intersect(sim.vis_rect())
            .clampped(sim.geo.w, sim.geo.h);
            cut_rect_into_selection(sim, app, rect);
        }
        Cmd::SelectFanAt(p) => {
            commit_selection(sim, app);
            let m = sim.margin() as f32;
            let cx = (p[0] + m).floor() as i32;
            let cy = (p[1] + m).floor() as i32;
            let (gw, gh) = (sim.geo.w as i32, sim.geo.h as i32);
            if cx < 0 || cy < 0 || cx >= gw || cy >= gh {
                return;
            }
            if sim.geo.cell[(cy as usize) * sim.geo.w + cx as usize]
                != crate::geometry::CELL_INLET
            {
                return;
            }
            // Flood-fill the connected fan (4-neighbour) and select its
            // bounding box, so its physics can be retuned in place.
            let mut rect = GridRect { x0: cx, y0: cy, x1: cx + 1, y1: cy + 1 };
            let mut stack = vec![(cx, cy)];
            let mut seen = std::collections::HashSet::new();
            seen.insert((cx, cy));
            const FLOOD_CAP: usize = 300_000;
            while let Some((x, y)) = stack.pop() {
                rect = rect.union(GridRect { x0: x, y0: y, x1: x + 1, y1: y + 1 });
                for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                    if nx < 0 || ny < 0 || nx >= gw || ny >= gh {
                        continue;
                    }
                    if seen.len() >= FLOOD_CAP || seen.contains(&(nx, ny)) {
                        continue;
                    }
                    if sim.geo.cell[(ny as usize) * sim.geo.w + nx as usize]
                        == crate::geometry::CELL_INLET
                    {
                        seen.insert((nx, ny));
                        stack.push((nx, ny));
                    }
                }
            }
            let rect = rect.intersect(sim.vis_rect()).clampped(sim.geo.w, sim.geo.h);
            cut_rect_into_selection(sim, app, rect);
            if app.sel_has_fans {
                app.status =
                    "Fan selected — tune speed/gustiness in the panel, Enter when done."
                        .into();
            }
        }
        Cmd::SelectUpdate => selection_update(sim, app),
        Cmd::SelectCommit => commit_selection(sim, app),
        Cmd::SelectCancel => {
            if app.selection.take().is_some() {
                // Restore every captured tile: the grid returns exactly to
                // its pre-session state.
                let tiles = std::mem::take(&mut app.pending_stroke_tiles);
                for tile in tiles.values() {
                    sim.geo.restore(tile);
                }
                app.selection_bg = None;
                app.pending_stroke_rect = GridRect::empty();
                // Snapshots can contain tunnel edge cells; keep the
                // boundary consistent with the toggle.
                sim.reassert_tunnel();
            }
        }
        Cmd::SelectDelete => {
            if app.selection.take().is_some() {
                if let Some(bg) = app.selection_bg.take() {
                    sim.geo.restore(&bg);
                }
                push_stroke_undo(sim, app); // the cut area stays cleared
                sim.reassert_tunnel();
            }
        }
        Cmd::CopySelection => {
            if let Some(sel) = app.selection.as_ref() {
                match bake_selection(sel) {
                    Some(baked) => {
                        app.clipboard = Some(baked);
                        app.status = "Selection copied.".into();
                    }
                    None => {
                        app.status =
                            "Selection too large to copy — reduce its scale first.".into();
                    }
                }
            }
        }
        Cmd::PasteClipboard => {
            if let Some(clip) = app.clipboard.clone() {
                let (vw, vh) = sim.grid_size();
                start_selection(sim, app, clip, [vw as f32 * 0.5, vh as f32 * 0.5]);
                app.status = "Pasted — drag to place, Enter to apply.".into();
            }
        }
        Cmd::InsertStamp(region) => {
            let (vw, vh) = sim.grid_size();
            start_selection(sim, app, region, [vw as f32 * 0.5, vh as f32 * 0.5]);
            app.status =
                "Inserted — drag to place, rotate/scale in the panel, Enter to apply.".into();
        }
        Cmd::SetMapping(m) => {
            // Refit against the sim's authoritative VISIBLE size: on a
            // resolution-switch frame the UI computed this mapping from
            // last frame's dimensions. (The letterbox maps the visible
            // window, never the full margin-inclusive grid.)
            let (vw, vh) = sim.grid_size();
            sim.mapping = ViewportMapping::fit(m.vp_origin, m.vp_size, vw, vh);
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
        // Escape: drop a pending sketch first, then cancel the floating
        // selection, else cancel an in-progress drag (freehand strokes
        // just end — their paint is already down).
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.pending_sketch.is_some() {
                self.pending_sketch = None;
                self.drag = None;
            } else if self.selection.is_some() {
                self.drag = None;
                cmds.push(Cmd::SelectCancel);
            } else if let Some(drag) = self.drag.take() {
                if matches!(drag.tool, Tool::Brush | Tool::Eraser) {
                    cmds.push(Cmd::StrokeEnd);
                }
            }
        }

        // Enter commits the pending sketch. Also drop any in-flight
        // vertex drag so the mouse release can't seed a stray new one.
        if self.pending_sketch.is_some()
            && self.selection.is_none()
            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
        {
            self.commit_sketch(cmds);
            self.drag = None;
        }

        // Selection keys.
        if self.selection.is_some() {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::Enter) {
                    cmds.push(Cmd::SelectCommit);
                }
                if i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace) {
                    cmds.push(Cmd::SelectDelete);
                }
                if i.modifiers.command && i.key_pressed(egui::Key::C) {
                    cmds.push(Cmd::CopySelection);
                }
                let step = if i.modifiers.shift { 1.0 } else { 4.0 };
                let mut nudge = [0.0f32; 2];
                if i.key_pressed(egui::Key::ArrowLeft) {
                    nudge[0] -= step;
                }
                if i.key_pressed(egui::Key::ArrowRight) {
                    nudge[0] += step;
                }
                if i.key_pressed(egui::Key::ArrowUp) {
                    nudge[1] -= step;
                }
                if i.key_pressed(egui::Key::ArrowDown) {
                    nudge[1] += step;
                }
                if nudge != [0.0; 2] {
                    if let Some(sel) = self.selection.as_mut() {
                        sel.pos[0] += nudge[0];
                        sel.pos[1] += nudge[1];
                    }
                    cmds.push(Cmd::SelectUpdate);
                }
            });
        }
        // Paste only when no stroke is in flight: starting a selection
        // session mid-drag would conflate the two undo sessions. A
        // pending sketch commits first for the same reason.
        if self.drag.is_none()
            && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V))
        {
            self.commit_sketch(cmds);
            cmds.push(Cmd::PasteClipboard);
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
                if i.key_pressed(egui::Key::S) {
                    self.tool = Tool::Select;
                }
                if i.key_pressed(egui::Key::P) {
                    self.tool = Tool::Polyline;
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
                            // The file carries its own flow/viscosity.
                            self.fluid_preset_idx = None;
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
                    ui.menu_button("Domain margin", |ui| {
                        ui.label("Extra simulated area around the canvas;")
                            .on_hover_text("larger margins push boundary artifacts away");
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
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.side_panel_contents(ui, snap, cmds);
            });
        });
    }

    fn side_panel_contents(&mut self, ui: &mut egui::Ui, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        {
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

            if self.selection.is_some() {
                ui.add_space(6.0);
                ui.separator();
                ui.heading("Selection");
                let mut changed = false;
                {
                    let sel = self.selection.as_mut().unwrap();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut sel.angle_deg, -180.0..=180.0)
                                .text("rotate °"),
                        )
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut sel.scale, 0.25..=4.0)
                                .logarithmic(true)
                                .text("scale"),
                        )
                        .changed();
                    ui.horizontal(|ui| {
                        if ui.button("Flip H").clicked() {
                            sel.flip_h = !sel.flip_h;
                            changed = true;
                        }
                        if ui.button("Flip V").clicked() {
                            sel.flip_v = !sel.flip_v;
                            changed = true;
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Copy").clicked() {
                        cmds.push(Cmd::CopySelection);
                    }
                    if ui.button("Delete").clicked() {
                        cmds.push(Cmd::SelectDelete);
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("✔ Apply").clicked() {
                        cmds.push(Cmd::SelectCommit);
                    }
                    if ui.button("✖ Cancel").clicked() {
                        cmds.push(Cmd::SelectCancel);
                    }
                });
                if changed {
                    cmds.push(Cmd::SelectUpdate);
                }
                // Per-fan physics for fan cells inside the selection.
                if self.sel_has_fans {
                    ui.add_space(4.0);
                    ui.label("Fans in selection:");
                    if ui
                        .add(
                            egui::Slider::new(&mut self.sel_fan_mult, 0.2..=2.0)
                                .text("speed ×"),
                        )
                        .changed()
                    {
                        cmds.push(Cmd::SetSelectionFanPhysics {
                            mult: Some(self.sel_fan_mult),
                            turb: None,
                        });
                    }
                    if ui
                        .add(
                            egui::Slider::new(&mut self.sel_fan_turb, 0.0..=1.0)
                                .text("gustiness"),
                        )
                        .changed()
                    {
                        cmds.push(Cmd::SetSelectionFanPhysics {
                            mult: None,
                            turb: Some(self.sel_fan_turb),
                        });
                    }
                }
            }

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
            if self.material == Material::Fan {
                ui.add(
                    egui::Slider::new(&mut self.fan_speed_mult, 0.2..=2.0)
                        .text("fan speed ×"),
                )
                .on_hover_text("Multiplier on the global flow speed for newly painted fans");
                ui.add(
                    egui::Slider::new(&mut self.fan_gustiness, 0.0..=1.0)
                        .text("gustiness"),
                )
                .on_hover_text(
                    "Time-varying wander in the fan's direction and strength — \
                     0 is steady, 1 is a blustery day",
                );
            }

            ui.add_space(6.0);
            ui.separator();
            ui.heading("Brush");
            let ps = self.phys_cache;
            let radius_label = format!("radius ({})", fmt_len(ps.len_m(self.brush_radius)));
            ui.add(
                egui::Slider::new(&mut self.brush_radius, 1.0..=64.0).text(radius_label),
            );

            ui.add_space(6.0);
            ui.separator();
            ui.heading("Sketch");
            let thick_label = format!(
                "wall thickness ({})",
                fmt_len(ps.len_m(self.wall_thickness))
            );
            ui.add(
                egui::Slider::new(&mut self.wall_thickness, 1.0..=24.0)
                    .text(thick_label),
            )
            .on_hover_text("Lines, polylines and shape outlines commit at this thickness");
            ui.checkbox(&mut self.shape_filled, "Filled rect / ellipse")
                .on_hover_text("Off = SolidWorks-style outlines at the wall thickness");
            ui.horizontal(|ui| {
                ui.label("angle snap");
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
                    egui::Slider::new(&mut self.snap_spacing, 2.0..=50.0)
                        .text(spacing_label),
                );
            }
            ui.label(
                egui::RichText::new(
                    "Sketches stay editable (drag the handles) until \
                     Enter/right-click commits · Shift: angle-snap, squares, \
                     circles · Alt: from centre · Esc cancels",
                )
                .small()
                .weak(),
            );

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
                for p in Preset::ALL {
                    let short = match p {
                        Preset::Cylinder => "Cylinder",
                        Preset::Airfoil => "Airfoil",
                        Preset::Venturi => "Venturi",
                        Preset::Step => "Step",
                        Preset::Pinball => "Pinball",
                    };
                    if ui
                        .button(short)
                        .on_hover_text(format!("{} — replaces the scene", p.label()))
                        .clicked()
                    {
                        cmds.push(Cmd::Preset(p));
                    }
                }
            });

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
                        if ui.selectable_label(sel, p.name).on_hover_text(p.desc).clicked() {
                            self.fluid_preset_idx = Some(i);
                            self.fluid_name = p.name;
                            self.fluid_nu = p.nu;
                            self.fluid_rho = p.rho;
                            // Only touch the tunnel when it actually
                            // changes: SetWindTunnel commits any floating
                            // selection and rewrites the tunnel columns.
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
            let ps = self.phys_cache;
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
                if ui
                    .button(egui::RichText::new("Clear all").color(egui::Color32::from_rgb(255, 140, 120)))
                    .on_hover_text("Erase everything — geometry, fans, smoke and flow (not undoable)")
                    .clicked()
                {
                    cmds.push(Cmd::ClearAll);
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
                    row(
                        "Sim time",
                        fmt_time(self.stats_sim_steps as f32 * ps.dt),
                    );
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
                    ui.small("blue: clockwise · red: counter-clockwise");
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
                        "canvas {} x {} (sim {} x {}, +{} margin)   |   {:.0} MLUPS   |   Re ≈ {}",
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

        // Cursor coordinates for the status bar (visible-cell space).
        self.hover_cell = response.hover_pos().map(|p| to_cell(p));

        if response.drag_started() {
            if let Some(pos) = pointer {
                let cell = to_cell(pos);
                if let Some(kind) = Self::sketch_kind(self.tool) {
                    if erase {
                        // Right-click commits the pending sketch, CAD-style.
                        self.commit_sketch(cmds);
                    } else {
                        // Pressing a handle of the pending sketch edits
                        // that vertex; pressing elsewhere starts a new
                        // entity (committing the old one) or, for the
                        // polyline, appends a vertex on release.
                        let handle_r_cells = 8.0 * ppp / mapping.px_per_cell.max(1e-3);
                        let handle_hit = self.pending_sketch.as_ref().and_then(|sk| {
                            sk.handles().iter().position(|h| {
                                let dx = h[0] - cell[0];
                                let dy = h[1] - cell[1];
                                (dx * dx + dy * dy).sqrt() < handle_r_cells
                            })
                        });
                        let mut sketch_handle = None;
                        if let Some(h) = handle_hit {
                            let sk = self.pending_sketch.as_mut().unwrap();
                            sketch_handle = Some(sk.begin_handle_drag(h));
                        } else if kind != SketchKind::Polyline {
                            self.commit_sketch(cmds);
                            let a = self.snap_point(cell);
                            self.pending_sketch =
                                Some(PendingSketch { kind, verts: vec![a, a] });
                        }
                        self.drag = Some(DragState {
                            start_cell: self.snap_point(cell),
                            last_cell: cell,
                            erase: false,
                            tool: self.tool,
                            material: self.material,
                            radius: self.brush_radius,
                            fan_deferred: false,
                            sel_move: false,
                            sketch_handle,
                        });
                    }
                } else if self.tool == Tool::Select {
                    // Inside the floating selection: move it. Outside:
                    // commit it (if any) and rubber-band a new marquee.
                    let hit = self
                        .selection
                        .as_ref()
                        .map_or(false, |s| s.point_to_source(cell).is_some());
                    if !hit && self.selection.is_some() {
                        cmds.push(Cmd::SelectCommit);
                    }
                    self.drag = Some(DragState {
                        start_cell: cell,
                        last_cell: cell,
                        erase: false,
                        tool: Tool::Select,
                        material: self.material,
                        radius: self.brush_radius,
                        fan_deferred: false,
                        sel_move: hit,
                        sketch_handle: None,
                    });
                } else {
                    // Belt and braces: a painting drag must never start
                    // over an unresolved selection session.
                    if self.selection.is_some() {
                        cmds.push(Cmd::SelectCommit);
                    }
                    // Every new fan stroke gusts on its own schedule; the
                    // phase is pinned via a command so it survives other
                    // same-frame rolls (stamps read it at apply time).
                    if self.material == Material::Fan && !erase {
                        self.roll_fan_phase();
                        cmds.push(Cmd::SetFanPhase(self.fan_phase));
                    }
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
                        sel_move: false,
                        sketch_handle: None,
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
        }

        if response.dragged() {
            if let Some(pos) = pointer {
                let cell = to_cell(pos);
                if self.drag.is_some() {
                    let (a, radius, material, tool, deferred, start, sel_move) = {
                        let drag = self.drag.as_ref().unwrap();
                        (
                            drag.last_cell,
                            drag.radius,
                            drag.effective_material(),
                            drag.tool,
                            drag.fan_deferred,
                            drag.start_cell,
                            drag.sel_move,
                        )
                    };
                    if let Some(kind) = Self::sketch_kind(tool) {
                        let (shift, alt) = response
                            .ctx
                            .input(|i| (i.modifiers.shift, i.modifiers.alt));
                        let handle = self.drag.as_ref().unwrap().sketch_handle;
                        if let Some(idx) = handle {
                            // Dragging an existing vertex/handle.
                            let p = self.constrained_handle_pos(kind, idx, cell, shift);
                            if let Some(sk) = self.pending_sketch.as_mut() {
                                if idx < sk.verts.len() {
                                    sk.verts[idx] = p;
                                }
                            }
                        } else if kind != SketchKind::Polyline {
                            // Rubber-banding a new entity from its anchor.
                            let (ea, eb) =
                                self.effective_shape(tool, start, cell, shift, alt);
                            if let Some(sk) = self.pending_sketch.as_mut() {
                                sk.verts = vec![ea, eb];
                            }
                        }
                        self.drag.as_mut().unwrap().last_cell = cell;
                    } else if tool == Tool::Select {
                        self.drag.as_mut().unwrap().last_cell = cell;
                        if sel_move {
                            if let Some(sel) = self.selection.as_mut() {
                                sel.pos[0] += cell[0] - a[0];
                                sel.pos[1] += cell[1] - a[1];
                            }
                            cmds.push(Cmd::SelectUpdate);
                        }
                    } else if matches!(tool, Tool::Brush | Tool::Eraser) {
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
                    Tool::Select => {
                        if !drag.sel_move {
                            let w = (drag.last_cell[0] - drag.start_cell[0]).abs();
                            let h = (drag.last_cell[1] - drag.start_cell[1]).abs();
                            if w >= 2.0 && h >= 2.0 {
                                cmds.push(Cmd::SelectCut {
                                    a: drag.start_cell,
                                    b: drag.last_cell,
                                });
                            } else {
                                // A plain click: select the connected fan
                                // under the cursor, if any.
                                cmds.push(Cmd::SelectFanAt(drag.start_cell));
                            }
                        }
                    }
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
                    Tool::Polyline => {
                        if drag.sketch_handle.is_none() {
                            // A click (or drag-release) places a vertex,
                            // constrained relative to the previous one.
                            let shift = response.ctx.input(|i| i.modifiers.shift);
                            if self.pending_sketch.is_none() {
                                self.pending_sketch = Some(PendingSketch {
                                    kind: SketchKind::Polyline,
                                    verts: Vec::new(),
                                });
                            }
                            let prev = self
                                .pending_sketch
                                .as_ref()
                                .unwrap()
                                .verts
                                .last()
                                .copied();
                            let mut p = self.snap_point(drag.last_cell);
                            if let (Some(prev), true) = (prev, shift) {
                                p = self.angle_snap(prev, p);
                            }
                            let sk = self.pending_sketch.as_mut().unwrap();
                            if sk.verts.len() < 512 {
                                sk.verts.push(p);
                            }
                        }
                    }
                    Tool::Line | Tool::Rect | Tool::Ellipse => {
                        // The entity stays pending (editable handles);
                        // drop it if the initial drag was degenerate.
                        if drag.sketch_handle.is_none() {
                            if let Some(sk) = &self.pending_sketch {
                                if sk.verts.len() >= 2 {
                                    let d = [
                                        sk.verts[1][0] - sk.verts[0][0],
                                        sk.verts[1][1] - sk.verts[0][1],
                                    ];
                                    if (d[0] * d[0] + d[1] * d[1]).sqrt() < 1.0 {
                                        self.pending_sketch = None;
                                    }
                                }
                            }
                        }
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
            if matches!(self.tool, Tool::Brush | Tool::Eraser) {
                let r = self.brush_radius * mapping.px_per_cell / ppp;
                painter.circle_stroke(hover, r, stroke);
            }
        }

        // Floating-selection outline (rotated, dashed).
        if let Some(sel) = &self.selection {
            let corners = sel.corners();
            let pts: Vec<egui::Pos2> =
                corners.iter().map(|&c| cell_to_pos(c)).collect();
            let dash_stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 220, 255));
            for k in 0..4 {
                let seg = [pts[k], pts[(k + 1) % 4]];
                painter.extend(egui::Shape::dashed_line(&seg, dash_stroke, 6.0, 4.0));
            }
        }

        // Marquee preview while rubber-banding a new selection.
        if let Some(drag) = &self.drag {
            if drag.tool == Tool::Select && !drag.sel_move {
                let a = cell_to_pos(drag.start_cell);
                let b = cell_to_pos(drag.last_cell);
                let rect = egui::Rect::from_two_pos(a, b);
                let dash_stroke =
                    egui::Stroke::new(1.0, egui::Color32::from_white_alpha(200));
                let c = [
                    rect.left_top(),
                    rect.right_top(),
                    rect.right_bottom(),
                    rect.left_bottom(),
                ];
                for k in 0..4 {
                    let seg = [c[k], c[(k + 1) % 4]];
                    painter.extend(egui::Shape::dashed_line(&seg, dash_stroke, 5.0, 4.0));
                }
            }
        }

        // Dimension readout next to the cursor, CAD style.
        let dims_text = |painter: &egui::Painter, at: egui::Pos2, text: String| {
            painter.text(
                at + egui::vec2(16.0, -16.0),
                egui::Align2::LEFT_BOTTOM,
                text,
                egui::FontId::monospace(12.0),
                egui::Color32::from_rgb(160, 230, 255),
            );
        };

        // Pending sketch: CAD-style editable entity with vertex handles,
        // rendered at the true committed wall thickness.
        if let Some(sk) = &self.pending_sketch {
            let ps = self.phys_cache;
            let w_px = (self.wall_thickness * mapping.px_per_cell / ppp).max(1.5);
            let body_stroke = egui::Stroke::new(w_px, egui::Color32::from_white_alpha(70));
            let fill = egui::Color32::from_white_alpha(30);
            match sk.kind {
                SketchKind::Line => {
                    if sk.verts.len() >= 2 {
                        let a = cell_to_pos(sk.verts[0]);
                        let b = cell_to_pos(sk.verts[1]);
                        painter.line_segment([a, b], body_stroke);
                        painter.line_segment([a, b], stroke);
                        let dx = sk.verts[1][0] - sk.verts[0][0];
                        let dy = sk.verts[1][1] - sk.verts[0][1];
                        let len_c = (dx * dx + dy * dy).sqrt();
                        dims_text(
                            painter,
                            b,
                            format!(
                                "L {} ({:.0} c)  ∠ {:.0}°",
                                fmt_len(ps.len_m(len_c)),
                                len_c,
                                (-dy).atan2(dx).to_degrees()
                            ),
                        );
                    }
                }
                SketchKind::Rect => {
                    let a = cell_to_pos(sk.verts[0]);
                    let b = cell_to_pos(sk.verts[1]);
                    let rect = egui::Rect::from_two_pos(a, b);
                    if self.shape_filled {
                        painter.rect_filled(rect, 0.0, fill);
                        painter.rect_stroke(rect, 0.0, stroke);
                    } else {
                        painter.rect_stroke(
                            rect,
                            0.0,
                            egui::Stroke::new(w_px, egui::Color32::from_white_alpha(70)),
                        );
                        painter.rect_stroke(rect, 0.0, stroke);
                    }
                    let wc = (sk.verts[1][0] - sk.verts[0][0]).abs();
                    let hc = (sk.verts[1][1] - sk.verts[0][1]).abs();
                    dims_text(
                        painter,
                        b,
                        format!(
                            "{} × {}",
                            fmt_len(ps.len_m(wc)),
                            fmt_len(ps.len_m(hc))
                        ),
                    );
                }
                SketchKind::Ellipse => {
                    let a = cell_to_pos(sk.verts[0]);
                    let b = cell_to_pos(sk.verts[1]);
                    let rect = egui::Rect::from_two_pos(a, b);
                    let c = rect.center();
                    let rx = rect.width() * 0.5;
                    let ry = rect.height() * 0.5;
                    let pts: Vec<egui::Pos2> = (0..48)
                        .map(|i| {
                            let t = i as f32 / 48.0 * std::f32::consts::TAU;
                            egui::pos2(c.x + rx * t.cos(), c.y + ry * t.sin())
                        })
                        .collect();
                    if self.shape_filled {
                        painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
                    } else {
                        let mut ring = pts.clone();
                        ring.push(pts[0]);
                        painter.add(egui::Shape::line(ring.clone(), body_stroke));
                        painter.add(egui::Shape::line(ring, stroke));
                    }
                    let rxc = (sk.verts[1][0] - sk.verts[0][0]).abs() * 0.5;
                    let ryc = (sk.verts[1][1] - sk.verts[0][1]).abs() * 0.5;
                    dims_text(
                        painter,
                        b,
                        format!(
                            "r {} × {}",
                            fmt_len(ps.len_m(rxc)),
                            fmt_len(ps.len_m(ryc))
                        ),
                    );
                }
                SketchKind::Polyline => {
                    for seg in sk.verts.windows(2) {
                        let a = cell_to_pos(seg[0]);
                        let b = cell_to_pos(seg[1]);
                        painter.line_segment([a, b], body_stroke);
                        painter.line_segment([a, b], stroke);
                    }
                    // Rubber segment to the cursor.
                    if self.tool == Tool::Polyline && self.drag.is_none() {
                        if let (Some(hover), Some(&prev)) =
                            (self.hover_cell, sk.verts.last())
                        {
                            let shift = ui.input(|i| i.modifiers.shift);
                            let mut p = self.snap_point(hover);
                            if shift {
                                p = self.angle_snap(prev, p);
                            }
                            painter.line_segment(
                                [cell_to_pos(prev), cell_to_pos(p)],
                                egui::Stroke::new(
                                    1.0,
                                    egui::Color32::from_white_alpha(120),
                                ),
                            );
                            let dx = p[0] - prev[0];
                            let dy = p[1] - prev[1];
                            let len_c = (dx * dx + dy * dy).sqrt();
                            dims_text(
                                painter,
                                cell_to_pos(p),
                                format!(
                                    "L {}  ∠ {:.0}°  (Enter: finish)",
                                    fmt_len(ps.len_m(len_c)),
                                    (-dy).atan2(dx).to_degrees()
                                ),
                            );
                        }
                    }
                }
            }
            // Vertex handles: draggable squares, CAD-style.
            for h in sk.handles() {
                let p = cell_to_pos(h);
                let r = 4.0;
                let hrect =
                    egui::Rect::from_center_size(p, egui::vec2(2.0 * r, 2.0 * r));
                painter.rect_filled(hrect, 1.0, egui::Color32::from_rgb(40, 90, 120));
                painter.rect_stroke(
                    hrect,
                    1.0,
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(160, 230, 255)),
                );
            }
        }
    }

    fn generator_windows(&mut self, ctx: &egui::Context, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
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
                    self.commit_sketch(cmds); // don't orphan a pending sketch
                    cmds.push(Cmd::InsertStamp(stamp));
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
                ui.add(
                    egui::Slider::new(&mut p.fan_mult, 0.2..=2.0).text("chamber fan ×"),
                );
                // Expected jet speeds in real units, next to the engine's
                // actual exhaust velocity.
                let throat_sim =
                    self.phys_cache.u_phys(snap.flow * p.fan_mult * p.chamber_ratio);
                ui.label(format!("sim throat jet ≈ {}", fmt_speed(throat_sim)));
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
                    // Rescale the fan for the flow speed at insert time.
                    p.fan_mult = (0.27 / (snap.flow * p.chamber_ratio).max(1e-4))
                        .clamp(0.2, 2.0);
                    let stamp = gen::generate_nozzle(p);
                    self.commit_sketch(cmds); // don't orphan a pending sketch
                    cmds.push(Cmd::InsertStamp(stamp));
                }
            });
        self.show_nozzle_gen = show;
    }

    fn windows(&mut self, ctx: &egui::Context, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        self.generator_windows(ctx, snap, cmds);
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
                        ("B / L / R / E / P / X / S", "brush / line / rect / ellipse / polyline / eraser / select"),
                        ("[ / ]", "brush size"),
                        ("Right-drag", "erase with any tool"),
                        ("Shift", "angle-snapped lines · squares · circles"),
                        ("Alt", "rect/ellipse from centre"),
                        ("Enter / right-click", "commit the pending sketch"),
                        ("Esc", "cancel sketch / selection"),
                        ("Enter", "apply the floating selection"),
                        ("Del", "delete the selection"),
                        ("Arrows (+Shift)", "nudge the selection"),
                        ("Ctrl+C / Ctrl+V", "copy / paste selection"),
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
