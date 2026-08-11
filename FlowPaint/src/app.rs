//! The FlowPaint V2 application shell, rebuilt around a persistent
//! sketch-object model: everything you draw stays a live, selectable,
//! editable vector object; the solver grid is a continuous projection of
//! the model (see model.rs). Panels: tools, object properties, sketch
//! aids, generators, presets, view, physics — plus the legend.

use crate::model::{ObjMaterial, Shape, SketchModel, SketchObject};
use crate::sim::{
    ColorMap, EdgeBcs, EdgeKind, FieldRange, GpuSim, RangeMode, RenderMode,
    SolverMode, ViewportMapping, DEFAULT_MARGIN_INDEX, RESOLUTIONS,
};
use eframe::egui;
use serde::{Deserialize, Serialize};

/// The UI panels, one file per panel (see docs/flowpaint-ui-overhaul-plan-v3.md
/// phase 1). Declared as a child of this module via #[path] so the panel
/// code keeps access to the app state without widening its visibility.
#[path = "ui/mod.rs"]
mod ui;

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

/// Which ribbon tab is open (the ribbon itself lives in ui/ribbon.rs).
#[derive(Clone, Copy, PartialEq, Eq)]
enum RibbonTab {
    Home,
    Geometry,
    Physics,
    Study,
    Results,
}

impl RibbonTab {
    const ALL: [(RibbonTab, &'static str); 5] = [
        (RibbonTab::Home, "Home"),
        (RibbonTab::Geometry, "Geometry"),
        (RibbonTab::Physics, "Physics"),
        (RibbonTab::Study, "Study"),
        (RibbonTab::Results, "Results"),
    ];
}

/// A pending view-navigation request (ribbon buttons, shortcuts).
/// Consumed by the canvas, where the viewport geometry is known.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewRequest {
    /// Fit the whole domain in the window (and keep fitting on resize).
    Fit,
    /// One grid cell per framebuffer pixel.
    OneToOne,
    /// Zoom to the selected object's bounds.
    Selection,
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
    /// Moving the whole selection. `before` pairs (id, object at gesture
    /// start) for the one-group undo record; `last` is the previous
    /// effective pointer position.
    MoveSel { before: Vec<(u64, SketchObject)>, last: [f32; 2] },
    /// Dragging one vertex/corner handle (single selection only).
    HandleDrag { id: u64, idx: usize, before: SketchObject },
    /// Rubber-band selection: `base` is the selection at press (the
    /// prior selection when Shift was held — additive — else empty);
    /// `corner` trails the pointer. The selection is applied live each
    /// drag frame; Esc restores `base`.
    RubberBand {
        anchor: [f32; 2],
        corner: [f32; 2],
        base: Vec<u64>,
    },
    /// Gizmo rotation about `pivot` (world cells). `before` pairs the
    /// transform targets at gesture start; each drag frame restores
    /// them and applies the accumulated `total` afresh, so the gesture
    /// never compounds float error (U3).
    GizmoRotate {
        before: Vec<(u64, SketchObject)>,
        pivot: [f32; 2],
        start_ang: f32,
        total: f32,
    },
    /// Gizmo uniform scale about `pivot`; same restore-and-reapply
    /// scheme as GizmoRotate.
    GizmoScale {
        before: Vec<(u64, SketchObject)>,
        pivot: [f32; 2],
        start_dist: f32,
        total: f32,
    },
    /// Dragging the gizmo pivot marker (no model edit).
    GizmoPivot,
}

/// Object layout of scene files v3–v5, BEFORE lock/hide existed. bincode
/// is positional, so the live `SketchObject` cannot decode old files
/// directly; old payloads decode into this mirror and convert.
#[derive(Serialize, Deserialize)]
struct SketchObjectV5 {
    id: u64,
    shape: Shape,
    material: ObjMaterial,
    thickness: f32,
    filled: bool,
    fan_mult: f32,
    fan_gust: f32,
    fan_phase: f32,
    fan_angle: f32,
    smoke_rgb: [f32; 3],
}

impl From<SketchObjectV5> for SketchObjectV7 {
    fn from(o: SketchObjectV5) -> Self {
        SketchObjectV7 {
            id: o.id,
            shape: o.shape,
            material: o.material,
            thickness: o.thickness,
            filled: o.filled,
            fan_mult: o.fan_mult,
            fan_gust: o.fan_gust,
            fan_phase: o.fan_phase,
            fan_angle: o.fan_angle,
            smoke_rgb: o.smoke_rgb,
            locked: false,
            hidden: false,
        }
    }
}

/// Object layout of scene files v6–v7 (lock/hide present, BEFORE U3's
/// `parent` link). Decodes via this mirror; groups didn't exist yet, so
/// every pre-v8 object loads as a root.
#[derive(Serialize, Deserialize)]
struct SketchObjectV7 {
    id: u64,
    shape: Shape,
    material: ObjMaterial,
    thickness: f32,
    filled: bool,
    fan_mult: f32,
    fan_gust: f32,
    fan_phase: f32,
    fan_angle: f32,
    smoke_rgb: [f32; 3],
    locked: bool,
    hidden: bool,
}

impl From<SketchObjectV7> for SketchObject {
    fn from(o: SketchObjectV7) -> Self {
        SketchObject {
            id: o.id,
            shape: o.shape,
            material: o.material,
            thickness: o.thickness,
            filled: o.filled,
            fan_mult: o.fan_mult,
            fan_gust: o.fan_gust,
            fan_phase: o.fan_phase,
            fan_angle: o.fan_angle,
            smoke_rgb: o.smoke_rgb,
            locked: o.locked,
            hidden: o.hidden,
            parent: None,
        }
    }
}

/// Scene file (version 3): the vector model plus core settings —
/// resolution-independent by construction.
#[derive(Serialize, Deserialize)]
struct SceneV3 {
    version: u32,
    objects: Vec<SketchObjectV5>,
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

/// Scene file (versions 4 and 5 — identical layout). The v5 tag marks
/// files whose stamp objects carry a meaningful `smoke_rgb` (the plume
/// recolor); in older files that field is an unused default and gets
/// re-seeded from the baked stamp dye on load.
#[derive(Serialize, Deserialize)]
struct SceneV4 {
    version: u32,
    objects: Vec<SketchObjectV5>,
    wind_tunnel: bool,
    flow_speed: f32,
    viscosity: f32,
    steps_per_frame: u32,
    domain_width_m: f32,
    fluid_nu: f32,
    fluid_rho: f32,
    ref_width: u32,
    /// 0 = LBM (incompressible), 1 = Euler (compressible).
    solver: u32,
    mach: f32,
    fluid_a: f32,
}

/// Scene file (version 6): the v4/v5 settings layout, whose objects now
/// persist `locked` and `hidden` (U2). Objects decode via the pre-U3
/// mirror (no parent links yet).
#[derive(Serialize, Deserialize)]
struct SceneV6 {
    version: u32,
    objects: Vec<SketchObjectV7>,
    wind_tunnel: bool,
    flow_speed: f32,
    viscosity: f32,
    steps_per_frame: u32,
    domain_width_m: f32,
    fluid_nu: f32,
    fluid_rho: f32,
    ref_width: u32,
    /// 0 = LBM (incompressible), 1 = Euler (compressible).
    solver: u32,
    mach: f32,
    fluid_a: f32,
}

/// One render mode's persisted color scale (v7): everything needed to
/// restore a pinned range and colormap pick. `sat_render` is not saved —
/// the per-frame sync re-derives it from the physical value.
#[derive(Serialize, Deserialize, Clone, Copy)]
struct SceneRange {
    /// 0 = Auto, 1 = Locked, 2 = Manual (RangeMode).
    mode: u32,
    /// Saturation point in physical units (m/s, 1/s, Pa).
    sat_phys: f32,
    /// 0 = Inferno, 1 = Coolwarm (ColorMap).
    map: u32,
}

/// Scene file (version 7): the v6 layout plus the per-render-mode color
/// range and colormap picks (T2-A, persisted at the track merge — U5's
/// PNG export needs a locked range to survive a save/load).
#[derive(Serialize, Deserialize)]
struct SceneV7 {
    version: u32,
    objects: Vec<SketchObjectV7>,
    wind_tunnel: bool,
    flow_speed: f32,
    viscosity: f32,
    steps_per_frame: u32,
    domain_width_m: f32,
    fluid_nu: f32,
    fluid_rho: f32,
    ref_width: u32,
    /// 0 = LBM (incompressible), 1 = Euler (compressible).
    solver: u32,
    mach: f32,
    fluid_a: f32,
    /// Indexed by `RenderMode as usize`; the Dye entry is unused.
    ranges: [SceneRange; 4],
}

impl SceneV7 {
    /// A pre-v7 scene: same settings, default (Auto) color ranges.
    fn from_v6(s: SceneV6) -> Self {
        SceneV7 {
            version: s.version,
            objects: s.objects,
            wind_tunnel: s.wind_tunnel,
            flow_speed: s.flow_speed,
            viscosity: s.viscosity,
            steps_per_frame: s.steps_per_frame,
            domain_width_m: s.domain_width_m,
            fluid_nu: s.fluid_nu,
            fluid_rho: s.fluid_rho,
            ref_width: s.ref_width,
            solver: s.solver,
            mach: s.mach,
            fluid_a: s.fluid_a,
            ranges: crate::sim::FIELD_RANGE_DEFAULTS.map(SceneRange::from),
        }
    }
}

impl From<FieldRange> for SceneRange {
    fn from(fr: FieldRange) -> Self {
        SceneRange {
            mode: match fr.mode {
                RangeMode::Auto => 0,
                RangeMode::Locked => 1,
                RangeMode::Manual => 2,
            },
            sat_phys: fr.sat_phys,
            map: match fr.map {
                ColorMap::Inferno => 0,
                ColorMap::Coolwarm => 1,
            },
        }
    }
}

/// A persisted probe (v8+): id and position in visible cells. Sample
/// histories are runtime-only — they describe a flow that a load resets.
#[derive(Serialize, Deserialize, Clone, Copy)]
struct SceneProbe {
    id: u32,
    pos: [f32; 2],
}

/// Scene file (version 8, U3): the v7 settings layout with the live
/// object type — objects persist `parent` links and `Shape::Group`
/// nodes — plus the probe set (closing T2-B's persistence debt) and the
/// plot-panel preferences. Kept as a DECODE layout only: files written
/// on the U3 branch pre-merge carry it; the live format is v9, which
/// absorbed these fields and appended the edge kinds.
#[derive(Serialize, Deserialize)]
struct SceneV8 {
    version: u32,
    objects: Vec<SketchObject>,
    wind_tunnel: bool,
    flow_speed: f32,
    viscosity: f32,
    steps_per_frame: u32,
    domain_width_m: f32,
    fluid_nu: f32,
    fluid_rho: f32,
    ref_width: u32,
    /// 0 = LBM (incompressible), 1 = Euler (compressible).
    solver: u32,
    mach: f32,
    fluid_a: f32,
    /// Indexed by `RenderMode as usize`; the Dye entry is unused.
    ranges: [SceneRange; 4],
    probes: Vec<SceneProbe>,
    /// `ProbeQuantity` as its ALL-index (0 Speed … 3 Smoke).
    probe_quantity: u32,
    probe_show_plot: bool,
}

impl SceneV8 {
    /// A pre-v8 scene: same settings, root-only objects, no probes.
    fn from_v7(s: SceneV7) -> Self {
        SceneV8 {
            version: s.version,
            objects: s.objects.into_iter().map(Into::into).collect(),
            wind_tunnel: s.wind_tunnel,
            flow_speed: s.flow_speed,
            viscosity: s.viscosity,
            steps_per_frame: s.steps_per_frame,
            domain_width_m: s.domain_width_m,
            fluid_nu: s.fluid_nu,
            fluid_rho: s.fluid_rho,
            ref_width: s.ref_width,
            solver: s.solver,
            mach: s.mach,
            fluid_a: s.fluid_a,
            ranges: s.ranges,
            probes: Vec::new(),
            probe_quantity: 0,
            probe_show_plot: false,
        }
    }
}

/// Scene file (version 9): the v8 layout plus the four per-edge
/// boundary kinds appended (T2-C). This is the second track merge's
/// planned app.rs seam, resolved as designed: v9 ABSORBS v8 — U3's
/// parent links / group nodes / probe set sit above `edges`, so the
/// layout stays append-only over the whole v3→v9 lineage.
#[derive(Serialize, Deserialize)]
struct SceneV9 {
    version: u32,
    objects: Vec<SketchObject>,
    wind_tunnel: bool,
    flow_speed: f32,
    viscosity: f32,
    steps_per_frame: u32,
    domain_width_m: f32,
    fluid_nu: f32,
    fluid_rho: f32,
    ref_width: u32,
    /// 0 = LBM (incompressible), 1 = Euler (compressible).
    solver: u32,
    mach: f32,
    fluid_a: f32,
    /// Indexed by `RenderMode as usize`; the Dye entry is unused.
    ranges: [SceneRange; 4],
    probes: Vec<SceneProbe>,
    /// `ProbeQuantity` as its ALL-index (0 Speed … 3 Smoke).
    probe_quantity: u32,
    probe_show_plot: bool,
    /// Edge kinds in `EDGE_NAMES` order (left, right, top, bottom):
    /// 0 = far field, 1 = inlet, 2 = outlet, 3 = wall, 4 = RESERVED for
    /// periodic (blocked by the shader freeze; loads as far field).
    edges: [u32; 4],
}

impl SceneV9 {
    /// A v8 scene (written on the U3 branch pre-merge, or converted up
    /// from older): the edge set its `wind_tunnel` flag implies — the
    /// tunnel preset or all far field, exactly the legacy behavior.
    fn from_v8(s: SceneV8) -> Self {
        SceneV9 {
            version: s.version,
            objects: s.objects,
            wind_tunnel: s.wind_tunnel,
            flow_speed: s.flow_speed,
            viscosity: s.viscosity,
            steps_per_frame: s.steps_per_frame,
            domain_width_m: s.domain_width_m,
            fluid_nu: s.fluid_nu,
            fluid_rho: s.fluid_rho,
            ref_width: s.ref_width,
            solver: s.solver,
            mach: s.mach,
            fluid_a: s.fluid_a,
            ranges: s.ranges,
            probes: s.probes,
            probe_quantity: s.probe_quantity,
            probe_show_plot: s.probe_show_plot,
            edges: EdgeBcs::legacy(s.wind_tunnel).0.map(edge_kind_to_u32),
        }
    }
}

fn edge_kind_to_u32(k: EdgeKind) -> u32 {
    match k {
        EdgeKind::FarField => 0,
        EdgeKind::Inlet => 1,
        EdgeKind::Outlet => 2,
        EdgeKind::Wall => 3,
        EdgeKind::Periodic => 4,
    }
}

/// Unknown discriminants (a newer file) fall back to far field — the
/// only kind that is always safe. 4 stays mapped to the reserved
/// periodic variant, which paints nothing until it ships post-freeze.
fn edge_kind_from_u32(v: u32) -> EdgeKind {
    match v {
        1 => EdgeKind::Inlet,
        2 => EdgeKind::Outlet,
        3 => EdgeKind::Wall,
        4 => EdgeKind::Periodic,
        _ => EdgeKind::FarField,
    }
}

const SCENE_V3: u32 = 3;
const SCENE_V4: u32 = 4;
const SCENE_V5: u32 = 5;
const SCENE_V6: u32 = 6;
const SCENE_V7: u32 = 7;
/// U3's format, decode-only since the second track merge.
const SCENE_V8: u32 = 8;
/// Current (T2-C + U3 merged): v8 absorbed, edge kinds appended.
const SCENE_V9: u32 = 9;

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
    /// Physical sound speed [m/s] (anchors the compressible mode's units).
    a: f32,
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
        a: 343.0,
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
        a: 343.0,
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
        a: 343.0,
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
        a: 343.0,
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
        a: 1481.0,
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
        a: 1904.0,
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
        a: 343.0,
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
    /// Per-edge boundary kinds (T2-C).
    edges: EdgeBcs,
    tints: bool,
    mode: RenderMode,
    solver: SolverMode,
    mach: f32,
    /// CFL time step of the Euler solver (nondimensional).
    euler_dt: f32,
    /// Inlet-state CFL estimate (see GpuSim::cfl_estimate).
    cfl: f32,
    display_gain: f32,
    smoke_gain: f32,
    particle_size: f32,
    particle_brightness: f32,
    sponge_strength: f32,
    /// Color range + colormap per render mode, synced to this frame's
    /// physical scaling by `sync_color_ranges` before the panels draw
    /// and written back to `Settings` when commands apply (T2-A fold).
    ranges: [FieldRange; 4],
}

/// Per-frame read snapshot of the sim-owned probe store (T2-B fold):
/// rows for the tree and markers always; the full sample series only
/// while the plot panel is open (it is the one heavy clone).
#[derive(Default)]
struct ProbeUi {
    rows: Vec<(u32, [f32; 2])>,
    show_plot: bool,
    quantity: crate::sim::ProbeQuantity,
    series: Vec<(u32, Vec<crate::sim::ProbeSample>)>,
}

/// Commands for the sim (settings and file ops); the sketch model is
/// owned by the app and edited directly.
enum Cmd {
    TogglePause,
    /// Advance one frame (steps_per_frame solver steps), pausing first.
    StepOnce,
    ResetFlow,
    SetSolver(SolverMode),
    SetMach(f32),
    SetWindTunnel(bool),
    /// Set one edge's boundary kind (T2-C; index in `EDGE_NAMES` order).
    SetEdgeBc(usize, EdgeKind),
    /// Replace the whole edge set (scene load / dialog presets).
    SetEdgeBcs(EdgeBcs),
    SetResolution(usize),
    SetMarginFrac(f32),
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
    /// Color-range edits from the legend (T2-A), per render mode.
    SetRangeMode(RenderMode, RangeMode),
    SetRangeMax(RenderMode, f32),
    SetColorMap(RenderMode, ColorMap),
    /// Probe edits (T2-B fold): the store lives in `Settings.probes`;
    /// panels read the per-frame `ProbeUi` snapshot and write these.
    AddProbe([f32; 2]),
    RemoveProbe(u32),
    SetProbeQuantity(crate::sim::ProbeQuantity),
    SetProbePlot(bool),
    ClearProbeSamples,
    /// Replace the whole probe set (scene loads).
    LoadProbes {
        probes: Vec<(u32, [f32; 2])>,
        quantity: crate::sim::ProbeQuantity,
        show_plot: bool,
    },
    ExportPng(std::path::PathBuf),
    SetMapping(ViewportMapping),
}

pub struct FlowPaintApp {
    // Model + editing state.
    model: SketchModel,
    tool: Tool,
    /// The selection: an ORDERED SET of object ids — insertion order,
    /// no duplicates, last element = primary (anchor for handle grabs
    /// and the single-object inspector). ALL writes go through the
    /// select_* / deselect helpers so the invariant holds at every site.
    selected: Vec<u64>,
    /// Tree click anchor for Shift-range selection (a display-order row).
    tree_anchor: Option<u64>,
    /// Internal clipboard (Ctrl+C/V); `paste_gen` cascades repeated
    /// pastes by one offset step each, reset on copy.
    clipboard: Vec<SketchObject>,
    paste_gen: u32,
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
    /// Physical sound speed [m/s] of the current fluid.
    fluid_a: f32,
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
    ribbon_tab: RibbonTab,
    show_about: bool,
    show_shortcuts: bool,
    /// The per-edge boundary-conditions window (T2-C).
    show_edges: bool,
    show_legend: bool,
    res_index: usize,
    /// Tracer particles on/off; `particle_index` keeps the last count so
    /// rechecking restores it (the snap_enabled/snap_spacing pattern).
    particles_on: bool,
    particle_index: usize,
    /// Simulated margin on/off; `margin_index` keeps the last size.
    margin_on: bool,
    margin_index: usize,
    /// Staged inspector transform: (selection id set, rotation °,
    /// scale %). The fields hold cumulative deltas since the selection
    /// changed — not absolute object properties (see object_panel).
    inspector_stage: Option<(Vec<u64>, f32, f32)>,
    /// "Entered" group (double-click): clicks inside it select one
    /// level below it instead of the outermost group.
    entered_group: Option<u64>,
    /// Gizmo pivot override (world cells) and the selection set it was
    /// staged for — reverts to the selection-bounds centre when the
    /// selection changes (U3).
    gizmo_pivot: Option<[f32; 2]>,
    gizmo_pivot_sel: Vec<u64>,
    /// The next canvas click places a probe (armed from the tree).
    probe_arming: bool,
    /// Per-frame snapshot of the sim-owned probe state for the panels
    /// (T2-B fold: the store lives in `Settings`, edits go through
    /// `Cmd`; this is the read path).
    probe_ui: ProbeUi,
    /// The canvas view mapping pushed this frame — lets status.rs draw
    /// probe markers without owning the canvas.
    canvas_mapping: Option<ViewportMapping>,
    status: String,
    hover_cell: Option<[f32; 2]>,
    /// Set by ribbon buttons / shortcuts, consumed by the canvas next
    /// frame (the canvas knows the viewport geometry).
    view_request: Option<ViewRequest>,
    // Free view transform — a pure remap of px_per_cell/lb_origin. It
    // must never touch the grid, the margin, domain_width_m or
    // phys_cache; the simulation itself is zoom-agnostic.
    /// Zoom multiplier over the letterbox fit scale (1.0 = fit).
    view_zoom: f32,
    /// Grid-cell coordinate under the viewport centre.
    view_center: [f32; 2],
    /// While true the view re-fits on window resize; any manual zoom or
    /// pan clears it.
    view_fit: bool,
    /// px_per_cell of the mapping last pushed (status zoom readout).
    view_px_per_cell: f32,
    /// A space-held pan happened during the current Space hold, so the
    /// key release must not toggle pause.
    space_pan_suppress: bool,
    // Stats.
    stats_grid: (usize, usize),
    stats_full: (usize, usize),
    stats_margin: usize,
    stats_mlups: f32,
    stats_re: u32,
    stats_euler: bool,
    /// Inlet speed in m/s (mode-aware), for the status strip.
    stats_u_inf: f32,
    /// Inlet-state CFL estimate, for the status strip.
    stats_cfl: f32,
    stats_steps_per_s: f32,
    stats_sim_steps: f64,
    sim_time_s: f64,
    /// Frame-time measurement harness (`--bench`), None in normal runs.
    bench: Option<BenchState>,
}

/// Frame-time harness state (see the plan's working rules): loads the
/// Pinball preset in compressible mode, measures a fixed frame count and
/// exits, so pre- and post-overhaul frame times are comparable.
struct BenchState {
    frame: u32,
    samples: Vec<f32>,
    last: Option<std::time::Instant>,
}

/// Setup happens on frame 1; frames up to the warmup bound are excluded
/// (pipeline compilation, flow reset); the next BENCH_FRAMES frame times
/// are the measurement.
const BENCH_WARMUP: u32 = 10;
const BENCH_FRAMES: usize = 300;

impl FlowPaintApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::theme::apply(&cc.egui_ctx);
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
            selected: Vec::new(),
            tree_anchor: None,
            clipboard: Vec::new(),
            paste_gen: 0,
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
            fluid_a: 343.0,
            fluid_preset_idx: Some(2),
            phys_cache: PhysScale::default(),
            show_airfoil_gen: false,
            show_nozzle_gen: false,
            airfoil_params: crate::generators::AirfoilParams::default(),
            nozzle_params: crate::generators::NozzleParams::default(),
            nozzle_real_ve: None,
            nozzle_fan_auto: true,
            ribbon_tab: RibbonTab::Home,
            show_about: false,
            show_shortcuts: false,
            show_edges: false,
            show_legend: true,
            res_index,
            particles_on: false, // Settings::default starts with no tracers
            particle_index: 0,
            margin_on: true,
            margin_index: DEFAULT_MARGIN_INDEX,
            inspector_stage: None,
            entered_group: None,
            gizmo_pivot: None,
            gizmo_pivot_sel: Vec::new(),
            probe_arming: false,
            probe_ui: ProbeUi::default(),
            canvas_mapping: None,
            status: String::from(
                "Draw with the sketch tools; every object stays selectable and editable.",
            ),
            hover_cell: None,
            view_request: None,
            view_zoom: 1.0,
            view_center: [vw as f32 * 0.5, vh as f32 * 0.5],
            view_fit: true,
            view_px_per_cell: 1.0,
            space_pan_suppress: false,
            stats_grid: (vw, vh),
            stats_full: (0, 0),
            stats_margin: 0,
            stats_mlups: 0.0,
            stats_re: 0,
            stats_euler: false,
            stats_u_inf: 0.0,
            stats_cfl: 0.0,
            bench: if std::env::args().any(|a| a == "--bench") {
                Some(BenchState { frame: 0, samples: Vec::new(), last: None })
            } else {
                None
            },
            stats_steps_per_s: 0.0,
            stats_sim_steps: 0.0,
            sim_time_s: 0.0,
        }
    }

    fn phys_scale(&self, snap: &UiSnapshot) -> PhysScale {
        let vis_w = self.stats_grid.0.max(1);
        let dx = self.domain_width_m / vis_w as f32;
        let dt = match snap.solver {
            // LBM: the lattice viscosity fixes the time step.
            SolverMode::Lbm => {
                snap.visc.max(1e-5) * dx * dx / self.fluid_nu.max(1e-12)
            }
            // Euler: the (nondimensional) sound speed fixes it instead —
            // one nondim time unit is dx / a_phys seconds.
            SolverMode::Euler => snap.euler_dt * dx / self.fluid_a.max(1.0),
        };
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
            locked: false,
            hidden: false,
            parent: None,
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
        // The rasterizer recolors fan-cell dye with the object's smoke
        // color; start it at the baked plume color so the insert looks
        // exactly like the dialog preview and the picker tells the truth.
        if let Some(rgb) = obj.stamp_plume_rgb() {
            obj.smoke_rgb = rgb;
        }
        self.model.add(obj.clone());
        self.select_only(obj.id);
        self.tool = Tool::Select;
        self.status =
            "Inserted — drag to place, rotate/scale in the Object panel.".into();
    }

    /// Insert a generated nozzle. With the chamber fan on, the insert
    /// is an ENGINE GROUP: the bell stamp plus a real Fan rect child
    /// across the chamber entrance — the parent link is what makes it
    /// an engine (the pre-U3 build baked `CELL_INLET` cells into the
    /// stamp and the inspector re-detected them by raster scanning).
    /// One `add_many` = one undo entry for the whole insert.
    pub(in crate::app) fn insert_nozzle_object(
        &mut self,
        params: crate::generators::NozzleParams,
    ) {
        let walls = crate::generators::generate_nozzle(&params);
        if !params.chamber_fan {
            self.insert_stamp_object(walls);
            return;
        }
        self.finish_gesture();
        let (vw, vh) = self.stats_grid;
        let c = [vw as f32 * 0.5, vh as f32 * 0.5];
        let gid = self.model.fresh_id();
        let group = SketchObject {
            id: gid,
            shape: Shape::Group { t: [0.0, 0.0], rot: 0.0, scale: 1.0 },
            material: ObjMaterial::Wall,
            thickness: 1.0,
            filled: false,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        };
        let mut bell = self.new_object(Shape::Stamp {
            raster: walls,
            c,
            scale: 1.0,
            angle: 0.0,
        });
        bell.material = ObjMaterial::Wall;
        bell.fan_mult = 1.0;
        bell.fan_gust = 0.0;
        bell.parent = Some(gid);
        let (off, half) = crate::generators::nozzle_fan_layout(&params);
        let mut fan = self.new_object(Shape::Rect {
            c: [c[0] + off[0], c[1] + off[1]],
            half,
            angle: 0.0,
        });
        fan.material = ObjMaterial::Fan;
        fan.filled = true;
        fan.thickness = 1.0;
        fan.fan_mult = params.fan_mult.clamp(0.2, 2.0);
        fan.fan_gust = 0.0;
        fan.fan_angle = 0.0; // blows +x; aim by rotating the group
        // The chamber plume color the baked stamps used.
        fan.smoke_rgb = [0.95, 0.85, 0.55];
        fan.parent = Some(gid);
        // Group node first so the children's damage marks resolve
        // their chain during the add.
        self.model.add_many(vec![group, fan, bell]);
        self.select_only(gid);
        self.tool = Tool::Select;
        self.status =
            "Engine inserted — drag to place; aim it with Rotate.".into();
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

    // --- Selection: the ordered set and its ONLY writers ---------------
    // Invariant: `selected` holds distinct ids in insertion order; the
    // last id is the primary. Every writer site in the app funnels
    // through these five (plus `selected.clear()` via deselect_all).

    pub(in crate::app) fn sel_contains(&self, id: u64) -> bool {
        self.selected.contains(&id)
    }

    /// The primary selected id (only meaningful single-object contexts
    /// check it; most readers iterate the whole set).
    pub(in crate::app) fn primary_sel(&self) -> Option<u64> {
        self.selected.last().copied()
    }

    /// The single selected id, when exactly one object is selected.
    pub(in crate::app) fn single_sel(&self) -> Option<u64> {
        match self.selected.as_slice() {
            [id] => Some(*id),
            _ => None,
        }
    }

    pub(in crate::app) fn select_only(&mut self, id: u64) {
        self.selected.clear();
        self.selected.push(id);
    }

    pub(in crate::app) fn select_add(&mut self, id: u64) {
        if !self.selected.contains(&id) {
            self.selected.push(id);
        }
    }

    /// Shift-click semantics: in the set → drop it, else append it.
    pub(in crate::app) fn select_toggle(&mut self, id: u64) {
        if let Some(i) = self.selected.iter().position(|&s| s == id) {
            self.selected.remove(i);
        } else {
            self.selected.push(id);
        }
    }

    pub(in crate::app) fn deselect(&mut self, id: u64) {
        self.selected.retain(|&s| s != id);
    }

    pub(in crate::app) fn deselect_all(&mut self) {
        self.selected.clear();
    }

    /// Drop ids that no longer resolve (undo/redo, loads); called once
    /// per frame before the UI reads the selection.
    fn prune_selection(&mut self) {
        let model = &self.model;
        self.selected.retain(|&id| model.find(id).is_some());
        if let Some(a) = self.tree_anchor {
            if model.find(a).is_none() {
                self.tree_anchor = None;
            }
        }
    }

    /// Selected ids that may be edited (locked objects can sit in the
    /// selection via the tree but must not be moved/deleted/retuned).
    /// Lock state is EFFECTIVE — a locked ancestor group locks the
    /// whole subtree.
    pub(in crate::app) fn editable_selection(&self) -> Vec<u64> {
        self.selected
            .iter()
            .copied()
            .filter(|&id| self.model.find(id).is_some() && !self.model.eff_locked(id))
            .collect()
    }

    /// The editable selection reduced to its OUTERMOST members: when a
    /// group and one of its descendants are both selected, only the
    /// group transforms — the descendant follows through composition
    /// (transforming both would apply the edit twice).
    pub(in crate::app) fn transform_targets(&self) -> Vec<u64> {
        let sel = self.editable_selection();
        sel.iter()
            .copied()
            .filter(|&id| {
                !sel.iter()
                    .any(|&other| other != id && self.model.is_descendant(id, other))
            })
            .collect()
    }

    /// Expand a set of ids to include every descendant (dedup'd, model
    /// order): deletes, copies and z-order moves act on whole subtrees.
    pub(in crate::app) fn expand_subtrees(&self, ids: &[u64]) -> Vec<u64> {
        self.model
            .objects
            .iter()
            .filter(|o| {
                ids.contains(&o.id)
                    || ids.iter().any(|&a| self.model.is_descendant(o.id, a))
            })
            .map(|o| o.id)
            .collect()
    }

    /// World-space AABB of the whole selection (the gizmo box and the
    /// common transform pivot).
    pub(in crate::app) fn selection_world_bounds(&self) -> Option<crate::geometry::GridRect> {
        let mut acc: Option<crate::geometry::GridRect> = None;
        for &id in &self.selected {
            if let Some(b) = self.model.world_bounds(id) {
                acc = Some(match acc {
                    Some(u) => u.union(b),
                    None => b,
                });
            }
        }
        acc
    }

    /// The pivot the selection transforms about: the dragged gizmo
    /// pivot while it belongs to this selection, else the selection
    /// bounds centre.
    pub(in crate::app) fn transform_pivot(&mut self) -> Option<[f32; 2]> {
        if self.gizmo_pivot_sel != self.selected {
            self.gizmo_pivot = None;
            self.gizmo_pivot_sel = self.selected.clone();
        }
        if let Some(p) = self.gizmo_pivot {
            return Some(p);
        }
        let b = self.selection_world_bounds()?;
        Some([(b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5])
    }

    /// Apply one edit to every editable selected object as ONE coalesced
    /// undo entry (multi-object panel widgets).
    pub(in crate::app) fn edit_selection(&mut self, f: impl Fn(&mut SketchObject)) {
        let ids = self.editable_selection();
        let mut pairs = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(i) = self.model.find(id) {
                let before = self.model.objects[i].clone();
                f(&mut self.model.objects[i]);
                pairs.push((id, before));
            }
        }
        self.model.record_modify_many_coalesced(&pairs);
    }

    /// Delete the editable selection — whole subtrees, one undo entry.
    /// Children are removed before their ancestors so each removal's
    /// damage region resolves through a still-intact chain.
    pub(in crate::app) fn delete_selected(&mut self) {
        self.finish_gesture();
        let mut ids = self.expand_subtrees(&self.editable_selection());
        if ids.is_empty() {
            return;
        }
        ids.sort_by_key(|&id| std::cmp::Reverse(self.model.depth(id)));
        for id in &ids {
            self.deselect(*id);
        }
        let n = ids.len();
        self.model.remove_many(&ids);
        if n > 1 {
            self.status = format!("Deleted {n} objects.");
        }
    }

    /// Group the selection's outermost members under a new group node;
    /// the group becomes the selection (Ctrl+G).
    pub(in crate::app) fn group_selected(&mut self) {
        self.finish_gesture();
        let targets = self.transform_targets();
        if targets.is_empty() {
            return;
        }
        if let Some(gid) = self.model.group_objects(&targets) {
            self.select_only(gid);
            self.entered_group = None;
            self.status = format!("Grouped {} object(s).", targets.len());
        }
    }

    /// Dissolve every selected group one level; the freed children
    /// become the selection (Ctrl+Shift+G).
    pub(in crate::app) fn ungroup_selected(&mut self) {
        self.finish_gesture();
        let groups: Vec<u64> = self
            .editable_selection()
            .into_iter()
            .filter(|&id| {
                self.model
                    .find(id)
                    .map(|i| matches!(self.model.objects[i].shape, Shape::Group { .. }))
                    .unwrap_or(false)
            })
            .collect();
        if groups.is_empty() {
            return;
        }
        self.deselect_all();
        self.entered_group = None;
        let mut n = 0;
        for gid in groups {
            let children = self.model.children_of(gid);
            if self.model.ungroup(gid) {
                n += 1;
                for c in children {
                    self.select_add(c);
                }
            }
        }
        self.status = if n == 1 {
            "Ungrouped.".into()
        } else {
            format!("Ungrouped {n} groups.")
        };
    }

    // --- Clipboard ------------------------------------------------------

    /// Marker written to the SYSTEM clipboard on copy: egui swallows
    /// Ctrl+V entirely and emits a Paste event only when the system
    /// clipboard holds non-empty text, so the internal object clipboard
    /// must leave this breadcrumb for the shortcut to fire at all.
    pub(in crate::app) const CLIPBOARD_MARKER: &'static str = "flowpaint/objects";

    pub(in crate::app) fn copy_selected(&mut self, ctx: &egui::Context) {
        // Clipboard keeps model (z) order so a paste preserves stacking.
        // Selecting a group copies its whole subtree; the copied roots
        // are FLATTENED to world space (their ancestor transforms baked
        // in), so a paste lands at the copied world position no matter
        // what happened to the original group in the meantime.
        let ids = self.expand_subtrees(&self.selected);
        let objs: Vec<SketchObject> = self
            .model
            .objects
            .iter()
            .filter(|o| ids.contains(&o.id))
            .map(|o| {
                let mut c = o.clone();
                if !ids.contains(&c.parent.unwrap_or(0)) || c.parent.is_none() {
                    c.apply_sim(self.model.parent_abs(c.id));
                    c.parent = None;
                }
                c
            })
            .collect();
        if !objs.is_empty() {
            self.paste_gen = 0;
            self.status = format!("Copied {} object(s).", objs.len());
            self.clipboard = objs;
            ctx.output_mut(|o| o.copied_text = Self::CLIPBOARD_MARKER.into());
        }
    }

    /// Paste the clipboard: fresh ids, one undo entry, pasted set
    /// selected. `in_place` pastes at the exact copied coordinates;
    /// otherwise repeated pastes cascade by one offset step each.
    pub(in crate::app) fn paste_clipboard(&mut self, in_place: bool) {
        if self.clipboard.is_empty() {
            return;
        }
        self.finish_gesture();
        let offset = if in_place {
            [0.0, 0.0]
        } else {
            self.paste_gen += 1;
            let d = 16.0 * self.paste_gen as f32;
            [d, d]
        };
        // Fresh ids minted first, then parent links remapped WITHIN the
        // pasted set (children can precede their group node in z-order,
        // so the mint must be a separate pass). Clipboard roots are
        // world-space with parent None; only roots translate — their
        // subtrees follow through composition.
        let src_list = self.clipboard.clone();
        let mut id_map: std::collections::HashMap<u64, u64> = Default::default();
        for src in &src_list {
            id_map.insert(src.id, self.model.fresh_id());
        }
        let mut copies = Vec::with_capacity(src_list.len());
        self.deselect_all();
        for src in &src_list {
            let mut copy = src.clone();
            copy.id = id_map[&src.id];
            match copy.parent.and_then(|p| id_map.get(&p).copied()) {
                Some(np) => copy.parent = Some(np),
                None => {
                    copy.parent = None;
                    copy.translate(offset);
                    self.select_add(copy.id);
                }
            }
            copies.push(copy);
        }
        let n = copies.len();
        self.model.add_many(copies);
        self.status = format!("Pasted {n} object(s).");
    }

    // --- Z-order --------------------------------------------------------
    // model.objects order IS the paint/rasterize order: later wins
    // overlaps, so these are functional, not cosmetic.

    /// Reorder the selection within the object list. `dir`: +1 raise,
    /// -1 lower, +2 to front, -2 to back — relative order of the
    /// selected objects is preserved in all four.
    pub(in crate::app) fn zorder_selected(&mut self, dir: i8) {
        if self.selected.is_empty() {
            return;
        }
        self.finish_gesture();
        // Whole subtrees move together — raising a group means raising
        // what it contains (the node itself never rasterizes).
        let expanded = self.expand_subtrees(&self.selected);
        let in_sel: Vec<bool> = self
            .model
            .objects
            .iter()
            .map(|o| expanded.contains(&o.id))
            .collect();
        let ids: Vec<u64> = self.model.objects.iter().map(|o| o.id).collect();
        let n = ids.len();
        let mut order: Vec<usize> = (0..n).collect();
        match dir {
            2 => {
                order = (0..n).filter(|&i| !in_sel[i]).collect();
                order.extend((0..n).filter(|&i| in_sel[i]));
            }
            -2 => {
                order = (0..n).filter(|&i| in_sel[i]).collect();
                order.extend((0..n).filter(|&i| !in_sel[i]));
            }
            1 => {
                // Bubble each selected run up one slot, topmost first.
                for i in (0..n.saturating_sub(1)).rev() {
                    if in_sel[order[i]] && !in_sel[order[i + 1]] {
                        order.swap(i, i + 1);
                    }
                }
            }
            _ => {
                for i in 1..n {
                    if in_sel[order[i]] && !in_sel[order[i - 1]] {
                        order.swap(i, i - 1);
                    }
                }
            }
        }
        self.model.reorder(order.into_iter().map(|i| ids[i]).collect());
    }
}

impl eframe::App for FlowPaintApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        ctx.request_repaint();

        let mut cmds: Vec<Cmd> = Vec::new();

        if self.bench.is_some() {
            self.bench_tick(ctx, &mut cmds);
        }

        let mut snapshot = {
            let Some(rs) = frame.wgpu_render_state() else { return };
            let renderer = rs.renderer.read();
            let Some(sim) = renderer.callback_resources.get::<GpuSim>() else { return };
            // Probe read path (T2-B fold): rows always; the sample
            // series — the one heavy clone — only while the plot shows.
            let pr = &sim.settings.probes;
            self.probe_ui = ProbeUi {
                rows: pr.probes.iter().map(|p| (p.id, p.pos)).collect(),
                show_plot: pr.show_plot,
                quantity: pr.quantity,
                series: if pr.show_plot {
                    pr.probes
                        .iter()
                        .map(|p| (p.id, p.samples.iter().copied().collect()))
                        .collect()
                } else {
                    Vec::new()
                },
            };
            UiSnapshot {
                flow: sim.settings.flow_speed,
                visc: sim.settings.viscosity,
                steps: sim.settings.steps_per_frame,
                fade: sim.settings.dye_fade,
                paused: sim.settings.paused,
                tunnel: sim.settings.wind_tunnel,
                edges: sim.settings.edges,
                tints: sim.settings.boundary_tints,
                mode: sim.settings.render_mode,
                solver: sim.settings.solver,
                mach: sim.settings.mach,
                euler_dt: sim.euler_dt(),
                cfl: sim.cfl_estimate(),
                display_gain: sim.settings.display_gain,
                smoke_gain: sim.settings.smoke_gain,
                particle_size: sim.settings.particle_size,
                particle_brightness: sim.settings.particle_brightness,
                sponge_strength: sim.settings.sponge_strength,
                ranges: sim.settings.ranges,
            }
        };
        self.phys_cache = self.phys_scale(&snapshot);
        // Reconcile the color ranges with this frame's physical scaling
        // before any panel reads them; the synced twins go back into
        // `Settings` below, right before the frame's commands apply.
        self.sync_color_ranges(&mut snapshot);
        self.stats_euler = snapshot.solver == SolverMode::Euler;
        self.stats_u_inf = if self.stats_euler {
            snapshot.mach * self.fluid_a
        } else {
            self.phys_cache.u_phys(snapshot.flow)
        };
        self.stats_cfl = snapshot.cfl;

        self.prune_selection();
        self.keyboard(ctx, &mut cmds);
        ui::draw(self, ctx, snapshot, &mut cmds);

        // Apply sim commands, project the model, upload.
        let Some(rs) = frame.wgpu_render_state() else { return };
        let mut renderer = rs.renderer.write();
        let Some(sim) = renderer.callback_resources.get_mut::<GpuSim>() else { return };

        // The synced range twins first, so a `SetRangeMode(_, Locked)`
        // arriving this frame pins exactly the value that was on screen.
        sim.settings.ranges = snapshot.ranges;
        for cmd in cmds {
            apply_cmd(sim, cmd, self);
        }
        if let Some(region) = self.model.take_dirty() {
            // The wind-tunnel preset keeps the rasterizer's own band
            // painting (byte-identical to pre-T2-C scenes); custom edge
            // sets paint their bands after the object pass instead.
            let (margin, tunnel_bands) =
                (sim.margin(), sim.settings.edges.is_tunnel_preset());
            self.model
                .rasterize_region(&mut sim.geo, region, margin, tunnel_bands);
            sim.apply_edge_bcs(region);
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
        Cmd::StepOnce => {
            // Pausing first means Step while running reads as "pause,
            // then advance one more frame".
            sim.settings.paused = true;
            sim.step_once = true;
        }
        Cmd::ResetFlow => sim.reset_flow(),
        Cmd::SetSolver(m) => {
            if sim.settings.solver != m {
                sim.settings.solver = m;
                // The two solvers keep separate state; start the new one
                // from the freestream.
                sim.reset_flow();
            }
        }
        Cmd::SetMach(v) => sim.settings.mach = v,
        Cmd::SetWindTunnel(on) => {
            sim.set_wind_tunnel(on);
            app.model.mark_all_dirty();
        }
        Cmd::SetEdgeBc(i, k) => {
            if let Some(slot) = sim.settings.edges.0.get_mut(i) {
                *slot = k;
                // The bands live in the geometry layers; repaint them.
                app.model.mark_all_dirty();
            }
        }
        Cmd::SetEdgeBcs(e) => {
            sim.settings.edges = e;
            app.model.mark_all_dirty();
        }
        Cmd::SetResolution(i) => {
            let old_w = sim.grid_size().0;
            sim.set_resolution(i);
            let new_w = sim.grid_size().0;
            if new_w != old_w && old_w > 0 {
                let f = new_w as f32 / old_w as f32;
                app.model.rescale_all(f);
                // The view centre is a grid-cell coordinate; rescale it
                // with the model so a non-fit view keeps looking at the
                // same world region (view_fit handles the rest).
                app.view_center[0] *= f;
                app.view_center[1] *= f;
            }
            app.model.mark_all_dirty();
            app.stats_grid = sim.grid_size();
        }
        Cmd::SetMarginFrac(frac) => {
            sim.set_margin_frac(frac);
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
        Cmd::SetRangeMode(m, v) => sim.settings.ranges[m as usize].mode = v,
        Cmd::SetRangeMax(m, v) => sim.settings.ranges[m as usize].sat_phys = v.max(1e-6),
        Cmd::SetColorMap(m, v) => sim.settings.ranges[m as usize].map = v,
        Cmd::AddProbe(pos) => {
            let pr = &mut sim.settings.probes;
            if pr.probes.len() < crate::sim::MAX_PROBES {
                let id = pr.next_id;
                pr.next_id += 1;
                pr.probes.push(crate::sim::Probe {
                    id,
                    pos,
                    samples: Default::default(),
                });
                pr.show_plot = true;
                app.status = format!("Probe P{id} placed.");
            }
        }
        Cmd::RemoveProbe(id) => {
            let pr = &mut sim.settings.probes;
            pr.probes.retain(|p| p.id != id);
            if pr.probes.is_empty() {
                pr.show_plot = false;
            }
        }
        Cmd::SetProbeQuantity(q) => sim.settings.probes.quantity = q,
        Cmd::SetProbePlot(on) => sim.settings.probes.show_plot = on,
        Cmd::ClearProbeSamples => {
            for p in &mut sim.settings.probes.probes {
                p.samples.clear();
            }
        }
        Cmd::LoadProbes { probes, quantity, show_plot } => {
            let pr = &mut sim.settings.probes;
            pr.next_id = probes.iter().map(|&(id, _)| id).max().unwrap_or(0) + 1;
            pr.probes = probes
                .into_iter()
                .map(|(id, pos)| crate::sim::Probe {
                    id,
                    pos,
                    samples: Default::default(),
                })
                .collect();
            pr.quantity = quantity;
            pr.show_plot = show_plot;
        }
        Cmd::ExportPng(p) => {
            app.status = match sim.export_png(&p) {
                Ok(()) => format!("Exported {}", p.display()),
                Err(e) => format!("Export failed: {e}"),
            };
        }
        Cmd::SetMapping(m) => {
            // Passthrough: the canvas owns the view transform (free zoom
            // and pan). Only a finite-value guard here — a NaN mapping
            // must never reach the render uniform.
            let finite = m.px_per_cell.is_finite()
                && m.px_per_cell > 0.0
                && m.vp_origin.iter().all(|v| v.is_finite())
                && m.vp_size.iter().all(|v| v.is_finite())
                && m.lb_origin.iter().all(|v| v.is_finite());
            if finite {
                sim.mapping = m;
                sim.write_render_uniform();
            }
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
        locked: false,
        hidden: false,
        parent: None,
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
            if self.probe_arming {
                self.probe_arming = false;
                self.status = "Probe placement cancelled.".into();
            } else {
            match std::mem::replace(&mut self.gesture, Gesture::None) {
                Gesture::DrawShape { id, .. }
                | Gesture::DrawPoly { id }
                | Gesture::DrawPencil { id } => {
                    self.model.cancel_last_add(id);
                    self.deselect(id);
                }
                Gesture::MoveSel { before, .. } => {
                    // Revert the in-flight move on every member.
                    self.restore_before(&before);
                }
                Gesture::HandleDrag { id, before, .. } => {
                    self.restore_before(&[(id, before)]);
                }
                Gesture::GizmoRotate { before, .. } | Gesture::GizmoScale { before, .. } => {
                    self.restore_before(&before);
                }
                Gesture::GizmoPivot => {}
                Gesture::RubberBand { base, .. } => self.selected = base,
                Gesture::None => {
                    // First Esc leaves an entered group, second clears
                    // the selection.
                    if self.entered_group.take().is_none() {
                        self.deselect_all();
                    }
                }
            }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            if matches!(self.gesture, Gesture::None) {
                // (An active gesture must not be deleted out from under.)
                self.delete_selected();
            }
        }
        // Select all TOP-LEVEL objects (group members follow their
        // group); locked or hidden ones stay out. Esc above clears.
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
            self.finish_gesture();
            self.selected = self
                .model
                .objects
                .iter()
                .filter(|o| o.parent.is_none() && !o.locked && !o.hidden)
                .map(|o| o.id)
                .collect();
        }
        // Group / ungroup (U3).
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::G)) {
            if ctx.input(|i| i.modifiers.shift) {
                self.ungroup_selected();
            } else {
                self.group_selected();
            }
        }
        // Clipboard. egui swallows Ctrl+C/Ctrl+V into Copy/Paste EVENTS
        // (they never arrive as key_pressed), so match the events.
        let (copy, paste, shift_now) = ctx.input(|i| {
            let mut c = false;
            let mut p = false;
            for e in &i.events {
                match e {
                    egui::Event::Copy => c = true,
                    // Only our own breadcrumb pastes objects; foreign
                    // clipboard text must not trigger an object paste.
                    egui::Event::Paste(s) if s == Self::CLIPBOARD_MARKER => {
                        p = true;
                    }
                    _ => {}
                }
            }
            (c, p, i.modifiers.shift)
        });
        if copy {
            self.copy_selected(ctx);
        }
        if paste {
            // Ctrl+Shift+V pastes in place.
            self.paste_clipboard(shift_now);
        }
        // Z-order: Ctrl+]/[ raise/lower, +Shift to front/back.
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::CloseBracket)) {
            let dir = if ctx.input(|i| i.modifiers.shift) { 2 } else { 1 };
            self.zorder_selected(dir);
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::OpenBracket)) {
            let dir = if ctx.input(|i| i.modifiers.shift) { -2 } else { -1 };
            self.zorder_selected(dir);
        }
        // Space pauses on RELEASE, not press: while held it is the pan
        // modifier on the canvas, and a hold that panned must not also
        // toggle the sim (the canvas sets space_pan_suppress).
        if ctx.input(|i| i.key_released(egui::Key::Space)) {
            if !self.space_pan_suppress {
                cmds.push(Cmd::TogglePause);
            }
            self.space_pan_suppress = false;
        }
        // View navigation; consumed by the canvas, which knows the
        // viewport geometry.
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num0)) {
            self.view_request = Some(ViewRequest::Fit);
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num1)) {
            self.view_request = Some(ViewRequest::OneToOne);
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num2)) {
            self.view_request = Some(ViewRequest::Selection);
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
            self.deselect_all();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Y)) {
            self.finish_gesture();
            self.model.redo();
            self.deselect_all();
        }
        if matches!(self.gesture, Gesture::None) {
            let mut switch = None;
            ctx.input(|i| {
                // Bare keys only: Ctrl/Cmd/Alt shortcuts (Ctrl+S, Ctrl+D…)
                // must not also switch tools.
                if i.modifiers.command || i.modifiers.alt {
                    return;
                }
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
        // Arrow-key nudge for the whole selection: 1 cell, Shift for a
        // coarse 8-cell step. One (coalesced) undo entry per selection.
        if !self.selected.is_empty() && matches!(self.gesture, Gesture::None) {
            let step = if ctx.input(|i| i.modifiers.shift) { 8.0 } else { 1.0 };
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
                self.transform_selection_world(|m, id| m.translate_world(id, d));
            }
        }
    }

    /// Apply a world-space transform to the selection's OUTERMOST
    /// editable members as ONE coalesced undo entry (arrow nudges, the
    /// inspector's rotate/scale/centre fields). The closure gets the
    /// model so it can use the world-space ops, which damage-mark
    /// through the ancestor chain themselves.
    pub(in crate::app) fn transform_selection_world(
        &mut self,
        f: impl Fn(&mut SketchModel, u64),
    ) {
        let ids = self.transform_targets();
        let mut pairs = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(i) = self.model.find(id) {
                let before = self.model.objects[i].clone();
                f(&mut self.model, id);
                pairs.push((id, before));
            }
        }
        self.model.record_modify_many_coalesced(&pairs);
    }

    /// Duplicate the whole selection (subtrees included) — one undo
    /// entry; the copies become the selection. Copied roots stay
    /// siblings of their originals (same parent), offset by one step in
    /// WORLD space; members keep their remapped hierarchy.
    fn duplicate_selected(&mut self) {
        self.finish_gesture();
        let ids = self.expand_subtrees(&self.selected);
        let src: Vec<SketchObject> = self
            .model
            .objects
            .iter()
            .filter(|o| ids.contains(&o.id))
            .cloned()
            .collect();
        if src.is_empty() {
            return;
        }
        let mut id_map: std::collections::HashMap<u64, u64> = Default::default();
        for s in &src {
            id_map.insert(s.id, self.model.fresh_id());
        }
        let mut copies = Vec::with_capacity(src.len());
        self.deselect_all();
        for mut copy in src {
            copy.id = id_map[&copy.id];
            match copy.parent.and_then(|p| id_map.get(&p).copied()) {
                Some(np) => copy.parent = Some(np),
                None => {
                    // Forest root: offset one paste step in world space,
                    // converted into its (kept) parent's space.
                    let dl = self
                        .model
                        .abs_of(copy.parent)
                        .inverse()
                        .apply_vec([16.0, 16.0]);
                    copy.translate(dl);
                    self.select_add(copy.id);
                }
            }
            copies.push(copy);
        }
        let n = copies.len();
        self.model.add_many(copies);
        self.status = if n == 1 {
            "Duplicated.".into()
        } else {
            format!("Duplicated {n} objects.")
        };
    }

    /// One harness step per frame: time the frame, set up the scene on
    /// the first call, and print the stats + quit when done.
    fn bench_tick(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        let now = std::time::Instant::now();
        let (frame, done) = {
            let b = self.bench.as_mut().expect("bench_tick without bench");
            if let Some(last) = b.last {
                if b.frame > BENCH_WARMUP && b.samples.len() < BENCH_FRAMES {
                    b.samples.push(now.duration_since(last).as_secs_f32() * 1e3);
                }
            }
            b.last = Some(now);
            b.frame += 1;
            (b.frame, b.samples.len() >= BENCH_FRAMES)
        };
        if frame == 1 {
            // Deterministic scene: Pinball preset, compressible mode, at
            // whatever grid the app started with (the default).
            let (vw, vh) = self.stats_grid;
            let objs = build_preset(ScenePreset::Pinball, &mut self.model, vw, vh);
            self.model.replace_all(objs);
            cmds.push(Cmd::SetSolver(SolverMode::Euler));
            cmds.push(Cmd::ResetFlow);
        }
        if done {
            let mut s = std::mem::take(&mut self.bench.as_mut().unwrap().samples);
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mean = s.iter().sum::<f32>() / s.len() as f32;
            let p99 = s[((s.len() as f32 * 0.99).ceil() as usize).clamp(1, s.len()) - 1];
            println!(
                "bench: {} frames  mean {:.2} ms  p99 {:.2} ms  min {:.2} ms  max {:.2} ms",
                s.len(),
                mean,
                p99,
                s[0],
                s[s.len() - 1]
            );
            self.bench = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Mutate an object mid-gesture: no undo record (that lands when the
    /// gesture finishes), just world-footprint damage marking (the
    /// object may live inside a transformed group).
    fn mutate_live(&mut self, id: u64, f: impl FnOnce(&mut SketchObject)) {
        if self.model.find(id).is_some() {
            self.model.mark_world_dirty(id);
            if let Some(i) = self.model.find(id) {
                f(&mut self.model.objects[i]);
            }
            self.model.mark_world_dirty(id);
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
                    self.deselect(id);
                } else {
                    self.model.finalize_last_add(id);
                    self.select_only(id);
                }
            }
            Gesture::DrawShape { id, .. } => {
                // A click without a drag: don't leave a speck behind.
                let degenerate = self
                    .model
                    .find(id)
                    .map(|i| match &self.model.objects[i].shape {
                        Shape::Line { a, b } => Self::dist(*a, *b) < 1.5,
                        Shape::Rect { half, .. } | Shape::Ellipse { r: half, .. } => {
                            half[0] < 1.0 && half[1] < 1.0
                        }
                        _ => false,
                    })
                    .unwrap_or(true);
                if degenerate {
                    self.model.cancel_last_add(id);
                    self.deselect(id);
                } else {
                    self.model.finalize_last_add(id);
                    self.select_only(id);
                }
            }
            Gesture::DrawPencil { id } => {
                // Simplify the freehand stroke into a clean polyline.
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
                    self.deselect(id);
                } else {
                    self.model.finalize_last_add(id);
                    self.select_only(id);
                }
            }
            Gesture::MoveSel { before, .. } => {
                // One undo entry for the whole selection's move.
                self.model.record_modify_many(&before);
            }
            Gesture::HandleDrag { id, before, .. } => {
                self.model.record_modify(id, before);
            }
            // One undo entry for the whole gizmo transform.
            Gesture::GizmoRotate { before, .. } | Gesture::GizmoScale { before, .. } => {
                self.model.record_modify_many(&before);
            }
            Gesture::GizmoPivot => {}
            // Selection was applied live; nothing to finalize.
            Gesture::RubberBand { .. } => {}
            Gesture::None => {}
        }
    }

    /// Restore a gesture's `before` set verbatim (Esc mid-gizmo), with
    /// world-footprint damage marks on both states.
    pub(in crate::app) fn restore_before(&mut self, before: &[(u64, SketchObject)]) {
        for (id, b) in before {
            self.model.mark_world_dirty(*id);
            if let Some(i) = self.model.find(*id) {
                self.model.objects[i] = b.clone();
            }
            self.model.mark_world_dirty(*id);
        }
    }

    fn save_scene(&mut self, path: &std::path::Path, snap: UiSnapshot) {
        // Commit any in-flight gesture so the file doesn't capture a
        // polyline's cursor-tracking rubber vertex.
        self.finish_gesture();
        let scene = SceneV9 {
            version: SCENE_V9,
            objects: self.model.objects.clone(),
            wind_tunnel: snap.tunnel,
            flow_speed: snap.flow,
            viscosity: snap.visc,
            steps_per_frame: snap.steps,
            domain_width_m: self.domain_width_m,
            fluid_nu: self.fluid_nu,
            fluid_rho: self.fluid_rho,
            ref_width: self.stats_grid.0 as u32,
            solver: match snap.solver {
                SolverMode::Lbm => 0,
                SolverMode::Euler => 1,
            },
            mach: snap.mach,
            fluid_a: self.fluid_a,
            ranges: snap.ranges.map(SceneRange::from),
            probes: self
                .probe_ui
                .rows
                .iter()
                .map(|&(id, pos)| SceneProbe { id, pos })
                .collect(),
            probe_quantity: crate::sim::ProbeQuantity::ALL
                .iter()
                .position(|&q| q == self.probe_ui.quantity)
                .unwrap_or(0) as u32,
            probe_show_plot: self.probe_ui.show_plot,
            edges: snap.edges.0.map(edge_kind_to_u32),
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
        let version = if bytes.len() >= 4 {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            0
        };
        if !(SCENE_V3..=SCENE_V9).contains(&version) {
            self.status =
                "Load failed: not a FlowPaint V2 scene (older .flow files aren't supported)"
                    .into();
            return;
        }
        // A v3 file is a v4 file without the solver fields; v5 shares
        // the v4 layout; v6 appends per-object lock/hide, so pre-v6
        // objects decode via the SketchObjectV5 mirror; v7 appends the
        // color ranges; v8 (U3) appends parent links / group nodes and
        // the probe set — pre-v8 objects decode via the SketchObjectV7
        // mirror; v9 (T2-C, current) appends the edge kinds, derived
        // from wind_tunnel for anything older. Everything funnels
        // upward: … → v6 → v7 → v8 → v9.
        let decoded = if version >= SCENE_V9 {
            bincode::deserialize::<SceneV9>(&bytes)
        } else if version >= SCENE_V8 {
            bincode::deserialize::<SceneV8>(&bytes).map(SceneV9::from_v8)
        } else if version >= SCENE_V7 {
            bincode::deserialize::<SceneV7>(&bytes)
                .map(|s| SceneV9::from_v8(SceneV8::from_v7(s)))
        } else if version >= SCENE_V6 {
            bincode::deserialize::<SceneV6>(&bytes)
                .map(|s| SceneV9::from_v8(SceneV8::from_v7(SceneV7::from_v6(s))))
        } else if version >= SCENE_V4 {
            bincode::deserialize::<SceneV4>(&bytes).map(|s| {
                SceneV9::from_v8(SceneV8::from_v7(SceneV7::from_v6(SceneV6 {
                    version: s.version,
                    objects: s.objects.into_iter().map(Into::into).collect(),
                    wind_tunnel: s.wind_tunnel,
                    flow_speed: s.flow_speed,
                    viscosity: s.viscosity,
                    steps_per_frame: s.steps_per_frame,
                    domain_width_m: s.domain_width_m,
                    fluid_nu: s.fluid_nu,
                    fluid_rho: s.fluid_rho,
                    ref_width: s.ref_width,
                    solver: s.solver,
                    mach: s.mach,
                    fluid_a: s.fluid_a,
                })))
            })
        } else {
            bincode::deserialize::<SceneV3>(&bytes).map(|s| {
                SceneV9::from_v8(SceneV8::from_v7(SceneV7::from_v6(SceneV6 {
                    version: s.version,
                    objects: s.objects.into_iter().map(Into::into).collect(),
                    wind_tunnel: s.wind_tunnel,
                    flow_speed: s.flow_speed,
                    viscosity: s.viscosity,
                    steps_per_frame: s.steps_per_frame,
                    domain_width_m: s.domain_width_m,
                    fluid_nu: s.fluid_nu,
                    fluid_rho: s.fluid_rho,
                    ref_width: s.ref_width,
                    solver: 0,
                    mach: 1.6,
                    fluid_a: 343.0,
                })))
            })
        };
        match decoded {
            Ok(scene) => {
                self.finish_gesture();
                self.deselect_all();
                let mut objects = scene.objects;
                // Drop payloads a corrupt or crafted file could smuggle in
                // that the rasterizer / bounds math can't survive.
                let before_n = objects.len();
                objects.retain(object_is_sane);
                let dropped = before_n - objects.len();
                sanitize_parents(&mut objects);
                // Pre-v5 files predate the recolorable stamp plume: their
                // stamps' smoke_rgb is an unused default, so seed it from
                // the baked fan dye to keep old scenes' plume colors.
                if version < SCENE_V5 {
                    for o in &mut objects {
                        if let Some(rgb) = o.stamp_plume_rgb() {
                            o.smoke_rgb = rgb;
                        }
                    }
                }
                // Rescale into the current grid width (probe positions
                // are visible-cell coordinates like everything else).
                let cur_w = self.stats_grid.0 as f32;
                let f = cur_w / (scene.ref_width.max(1) as f32);
                let mut probes = scene.probes;
                probes.retain(|p| p.pos[0].is_finite() && p.pos[1].is_finite());
                probes.truncate(crate::sim::MAX_PROBES);
                if (f - 1.0).abs() > 1e-3 {
                    for o in &mut objects {
                        o.rescale_all(f);
                    }
                    for p in &mut probes {
                        p.pos[0] *= f;
                        p.pos[1] *= f;
                    }
                }
                self.model.replace_all(objects);
                self.entered_group = None;
                cmds.push(Cmd::LoadProbes {
                    probes: probes.iter().map(|p| (p.id, p.pos)).collect(),
                    quantity: crate::sim::ProbeQuantity::ALL
                        [scene.probe_quantity.min(3) as usize],
                    show_plot: scene.probe_show_plot && !probes.is_empty(),
                });
                self.domain_width_m = sane_f32(scene.domain_width_m, 0.01, 10_000.0, 1.0);
                self.fluid_nu = sane_f32(scene.fluid_nu, 1e-9, 1.0, 1.5e-5);
                self.fluid_rho = sane_f32(scene.fluid_rho, 1e-3, 1e5, 1.2);
                self.fluid_a = sane_f32(scene.fluid_a, 1.0, 10_000.0, 343.0);
                self.fluid_preset_idx = None;
                self.fluid_name = "custom (loaded)";
                if scene.flow_speed > 0.0 {
                    cmds.push(Cmd::SetFlowSpeed(sane_f32(
                        scene.flow_speed,
                        0.01,
                        0.2,
                        0.09,
                    )));
                    cmds.push(Cmd::SetViscosity(sane_f32(
                        scene.viscosity,
                        0.004,
                        0.2,
                        0.015,
                    )));
                    cmds.push(Cmd::SetSteps(scene.steps_per_frame.clamp(1, 64)));
                }
                cmds.push(Cmd::SetSolver(if scene.solver == 1 {
                    SolverMode::Euler
                } else {
                    SolverMode::Lbm
                }));
                cmds.push(Cmd::SetMach(sane_f32(scene.mach, 0.3, 3.0, 1.6)));
                cmds.push(Cmd::SetWindTunnel(scene.wind_tunnel));
                // After the tunnel preset re-arm: the saved edge set
                // wins (v9; legacy-derived for older files, where it
                // matches the preset exactly).
                cmds.push(Cmd::SetEdgeBcs(EdgeBcs(
                    scene.edges.map(edge_kind_from_u32),
                )));
                // Color ranges (v7; defaults for older files). Dye has
                // no scale, so its entry stays untouched.
                for mode in [RenderMode::Speed, RenderMode::Vorticity, RenderMode::Pressure] {
                    let r = scene.ranges[mode as usize];
                    cmds.push(Cmd::SetRangeMode(
                        mode,
                        match r.mode {
                            1 => RangeMode::Locked,
                            2 => RangeMode::Manual,
                            _ => RangeMode::Auto,
                        },
                    ));
                    cmds.push(Cmd::SetRangeMax(mode, sane_f32(r.sat_phys, 1e-6, 1e9, 1.0)));
                    cmds.push(Cmd::SetColorMap(
                        mode,
                        if r.map == 1 { ColorMap::Coolwarm } else { ColorMap::Inferno },
                    ));
                }
                cmds.push(Cmd::ResetFlow);
                self.status = if dropped > 0 {
                    format!(
                        "Loaded {} ({dropped} invalid object(s) dropped)",
                        path.display()
                    )
                } else {
                    format!("Loaded {}", path.display())
                };
            }
            Err(e) => self.status = format!("Load failed: {e}"),
        }
    }


}

/// Clamp a file-borne float, replacing NaN/inf (which pass through
/// `clamp` unchanged) with a default.
fn sane_f32(v: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        default
    }
}

/// Structural validation for objects decoded from a scene file: finite
/// coordinates everywhere, and stamp rasters whose layer lengths match
/// their rect (the rasterizer indexes them unchecked).
fn object_is_sane(o: &SketchObject) -> bool {
    let finite2 = |p: &[f32; 2]| p[0].is_finite() && p[1].is_finite();
    if !o.thickness.is_finite() || !(0.0..=1000.0).contains(&o.thickness) {
        return false;
    }
    if !(o.fan_mult.is_finite() && o.fan_gust.is_finite() && o.fan_angle.is_finite()) {
        return false;
    }
    match &o.shape {
        Shape::Line { a, b } => finite2(a) && finite2(b),
        Shape::Poly { pts, .. } => {
            !pts.is_empty() && pts.len() <= 100_000 && pts.iter().all(finite2)
        }
        Shape::Rect { c, half, angle } | Shape::Ellipse { c, r: half, angle } => {
            finite2(c) && finite2(half) && angle.is_finite()
        }
        Shape::Stamp { raster, c, scale, angle } => {
            let (x0, y0, x1, y1) = raster.rect;
            let w = x1.saturating_sub(x0);
            let h = y1.saturating_sub(y0);
            if !(1..=8192).contains(&w) || !(1..=8192).contains(&h) {
                return false;
            }
            let n = w as usize * h as usize;
            n <= 32_000_000
                && raster.cell.len() == n
                && raster.fan.len() == n
                && raster.dye_src.len() == n
                && finite2(c)
                && scale.is_finite()
                && (1e-3..=1e3).contains(scale)
                && angle.is_finite()
        }
        Shape::Group { t, rot, scale } => {
            finite2(t) && rot.is_finite() && (1e-3..=1e3).contains(scale)
        }
    }
}

/// Repair the parent links of a loaded object set: a dangling parent, a
/// parent that is not a group, or a parent CYCLE (a crafted file can
/// write one; the live model can't — reparent refuses them) resolves to
/// the root rather than poisoning every ancestor walk.
fn sanitize_parents(objects: &mut [SketchObject]) {
    use std::collections::HashMap;
    let groups: HashMap<u64, usize> = objects
        .iter()
        .enumerate()
        .filter(|(_, o)| matches!(o.shape, Shape::Group { .. }))
        .map(|(i, o)| (o.id, i))
        .collect();
    for i in 0..objects.len() {
        if let Some(p) = objects[i].parent {
            if !groups.contains_key(&p) || p == objects[i].id {
                objects[i].parent = None;
            }
        }
    }
    // Break cycles: walk each chain with a visited set; on revisiting a
    // node, detach the walk's starting object.
    for i in 0..objects.len() {
        let mut seen = vec![objects[i].id];
        let mut cur = objects[i].parent;
        while let Some(pid) = cur {
            if seen.contains(&pid) {
                objects[i].parent = None;
                break;
            }
            seen.push(pid);
            cur = groups.get(&pid).and_then(|&pi| objects[pi].parent);
        }
    }
}

// --- The wgpu paint callback ------------------------------------------

#[cfg(test)]
mod scene_tests {
    use super::*;

    fn v5_obj(id: u64) -> SketchObjectV5 {
        SketchObjectV5 {
            id,
            shape: Shape::Ellipse { c: [10.0, 20.0], r: [5.0, 5.0], angle: 0.3 },
            material: ObjMaterial::Fan,
            thickness: 6.0,
            filled: true,
            fan_mult: 1.5,
            fan_gust: 0.2,
            fan_phase: 0.1,
            fan_angle: 0.4,
            smoke_rgb: [0.1, 0.2, 0.3],
        }
    }

    /// A v5-layout file (written by the pre-U2 code) still decodes, and
    /// its objects convert with lock/hide off.
    #[test]
    fn v5_bytes_decode_and_convert() {
        let scene = SceneV4 {
            version: SCENE_V5,
            objects: vec![v5_obj(7)],
            wind_tunnel: true,
            flow_speed: 0.09,
            viscosity: 0.015,
            steps_per_frame: 8,
            domain_width_m: 1.0,
            fluid_nu: 1.5e-5,
            fluid_rho: 1.2,
            ref_width: 1920,
            solver: 1,
            mach: 1.6,
            fluid_a: 343.0,
        };
        let bytes = bincode::serialize(&scene).unwrap();
        // The version peek the loader does.
        let version = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(version, SCENE_V5);
        let back = bincode::deserialize::<SceneV4>(&bytes).unwrap();
        let v7: SketchObjectV7 = back.objects.into_iter().next().unwrap().into();
        let obj: SketchObject = v7.into();
        assert_eq!(obj.id, 7);
        assert!(!obj.locked && !obj.hidden);
        assert_eq!(obj.parent, None);
        assert_eq!(obj.fan_mult, 1.5);
    }

    /// v6 round-trips lock/hide, and its version peek reads 6.
    #[test]
    fn v6_roundtrip_persists_lock_hide() {
        let mut obj: SketchObjectV7 = v5_obj(3).into();
        obj.locked = true;
        obj.hidden = true;
        let scene = SceneV6 {
            version: SCENE_V6,
            objects: vec![obj],
            wind_tunnel: false,
            flow_speed: 0.05,
            viscosity: 0.02,
            steps_per_frame: 8,
            domain_width_m: 2.0,
            fluid_nu: 1.0e-6,
            fluid_rho: 998.0,
            ref_width: 1280,
            solver: 0,
            mach: 1.6,
            fluid_a: 1481.0,
        };
        let bytes = bincode::serialize(&scene).unwrap();
        let version = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(version, SCENE_V6);
        let back = bincode::deserialize::<SceneV6>(&bytes).unwrap();
        assert!(back.objects[0].locked && back.objects[0].hidden);
    }

    /// v7 round-trips the color ranges: a locked physical value and a
    /// non-default colormap pick survive save/load (U5 depends on this).
    #[test]
    fn v7_roundtrip_persists_ranges() {
        let mut ranges = crate::sim::FIELD_RANGE_DEFAULTS.map(SceneRange::from);
        ranges[RenderMode::Speed as usize] =
            SceneRange { mode: 1, sat_phys: 12.5, map: 1 };
        let scene = SceneV7 {
            version: SCENE_V7,
            objects: vec![v5_obj(4).into()],
            wind_tunnel: true,
            flow_speed: 0.09,
            viscosity: 0.015,
            steps_per_frame: 8,
            domain_width_m: 1.0,
            fluid_nu: 1.5e-5,
            fluid_rho: 1.2,
            ref_width: 1920,
            solver: 0,
            mach: 1.6,
            fluid_a: 343.0,
            ranges,
        };
        let bytes = bincode::serialize(&scene).unwrap();
        let version = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(version, SCENE_V7);
        let back = bincode::deserialize::<SceneV7>(&bytes).unwrap();
        let r = back.ranges[RenderMode::Speed as usize];
        assert_eq!(r.mode, 1);
        assert_eq!(r.sat_phys, 12.5);
        assert_eq!(r.map, 1);
    }

    /// A v6 file (written by the U2-era code) still decodes through the
    /// v6 arm and comes out with default (Auto) color ranges.
    #[test]
    fn v6_bytes_decode_with_default_ranges() {
        let scene = SceneV6 {
            version: SCENE_V6,
            objects: vec![v5_obj(9).into()],
            wind_tunnel: true,
            flow_speed: 0.09,
            viscosity: 0.015,
            steps_per_frame: 8,
            domain_width_m: 1.0,
            fluid_nu: 1.5e-5,
            fluid_rho: 1.2,
            ref_width: 1920,
            solver: 0,
            mach: 1.6,
            fluid_a: 343.0,
        };
        let bytes = bincode::serialize(&scene).unwrap();
        let back = SceneV7::from_v6(bincode::deserialize::<SceneV6>(&bytes).unwrap());
        for (i, r) in back.ranges.iter().enumerate() {
            assert_eq!(r.mode, 0, "range {i} should load as Auto");
        }
        // Default map bindings: Speed inferno, Vorticity/Pressure coolwarm.
        assert_eq!(back.ranges[RenderMode::Speed as usize].map, 0);
        assert_eq!(back.ranges[RenderMode::Vorticity as usize].map, 1);
        assert_eq!(back.ranges[RenderMode::Pressure as usize].map, 1);
    }

    /// The canonical test group node.
    fn group_obj(id: u64) -> SketchObject {
        SketchObject {
            id,
            shape: Shape::Group { t: [3.0, 4.0], rot: 0.5, scale: 2.0 },
            material: ObjMaterial::Wall,
            thickness: 1.0,
            filled: false,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        }
    }

    /// A locked Speed range, for asserting range survival across
    /// version conversions.
    fn locked_ranges() -> [SceneRange; 4] {
        let mut ranges = crate::sim::FIELD_RANGE_DEFAULTS.map(SceneRange::from);
        ranges[RenderMode::Speed as usize] =
            SceneRange { mode: 1, sat_phys: 12.5, map: 1 };
        ranges
    }

    /// A v7 FIXTURE (track-merge era bytes) loads through the full
    /// v7 → v8 → v9 chain: root-only objects, no probes, color ranges
    /// kept, edges derived from `wind_tunnel` (the tunnel preset here).
    #[test]
    fn v7_fixture_converts_to_v9() {
        let scene = SceneV7 {
            version: SCENE_V7,
            objects: vec![v5_obj(11).into()],
            wind_tunnel: true,
            flow_speed: 0.09,
            viscosity: 0.015,
            steps_per_frame: 8,
            domain_width_m: 1.0,
            fluid_nu: 1.5e-5,
            fluid_rho: 1.2,
            ref_width: 1920,
            solver: 0,
            mach: 1.6,
            fluid_a: 343.0,
            ranges: locked_ranges(),
        };
        let bytes = bincode::serialize(&scene).unwrap();
        assert_eq!(
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            SCENE_V7
        );
        let back = SceneV9::from_v8(SceneV8::from_v7(
            bincode::deserialize::<SceneV7>(&bytes).unwrap(),
        ));
        assert_eq!(back.objects[0].parent, None);
        assert!(back.probes.is_empty());
        assert!(!back.probe_show_plot);
        // The locked color range survives the two conversions …
        let r = back.ranges[RenderMode::Speed as usize];
        assert_eq!((r.mode, r.sat_phys, r.map), (1, 12.5, 1));
        // … and the edge set is the one the tunnel flag implies.
        assert_eq!(
            EdgeBcs(back.edges.map(edge_kind_from_u32)),
            EdgeBcs::WIND_TUNNEL
        );
    }

    /// v8 round-trips the U3 payload: a Group node, a parent link, and
    /// the probe set with the plot preferences.
    #[test]
    fn v8_roundtrip_persists_groups_and_probes() {
        let group = SketchObject {
            id: 20,
            shape: Shape::Group { t: [3.0, 4.0], rot: 0.5, scale: 2.0 },
            material: ObjMaterial::Wall,
            thickness: 1.0,
            filled: false,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        };
        let mut child: SketchObject = SketchObjectV7::from(v5_obj(21)).into();
        child.parent = Some(20);
        let scene = SceneV8 {
            version: SCENE_V8,
            objects: vec![child, group],
            wind_tunnel: true,
            flow_speed: 0.09,
            viscosity: 0.015,
            steps_per_frame: 8,
            domain_width_m: 1.0,
            fluid_nu: 1.5e-5,
            fluid_rho: 1.2,
            ref_width: 1920,
            solver: 1,
            mach: 1.6,
            fluid_a: 343.0,
            ranges: crate::sim::FIELD_RANGE_DEFAULTS.map(SceneRange::from),
            probes: vec![SceneProbe { id: 3, pos: [120.0, 240.0] }],
            probe_quantity: 2,
            probe_show_plot: true,
        };
        let bytes = bincode::serialize(&scene).unwrap();
        let version = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(version, SCENE_V8);
        let back = bincode::deserialize::<SceneV8>(&bytes).unwrap();
        assert_eq!(back.objects[0].parent, Some(20));
        assert!(matches!(
            back.objects[1].shape,
            Shape::Group { t: [3.0, 4.0], rot, scale } if rot == 0.5 && scale == 2.0
        ));
        assert_eq!(back.probes.len(), 1);
        assert_eq!(back.probes[0].pos, [120.0, 240.0]);
        assert_eq!(back.probe_quantity, 2);
        assert!(back.probe_show_plot);
    }

    /// Loader repair: a crafted file with a parent cycle or a dangling
    /// parent gets detached to the root instead of poisoning every
    /// ancestor walk.
    #[test]
    fn sanitize_parents_breaks_cycles_and_dangles() {
        let g = |id: u64, parent: Option<u64>| SketchObject {
            id,
            shape: Shape::Group { t: [0.0, 0.0], rot: 0.0, scale: 1.0 },
            material: ObjMaterial::Wall,
            thickness: 1.0,
            filled: false,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent,
        };
        // 1 → 2 → 1 is a cycle; 3 dangles; 4 parents a non-group leaf.
        let mut leaf: SketchObject = SketchObjectV7::from(v5_obj(5)).into();
        leaf.parent = Some(99); // dangling
        let mut leaf2: SketchObject = SketchObjectV7::from(v5_obj(6)).into();
        leaf2.parent = Some(5); // a leaf, not a group
        let mut objects = vec![g(1, Some(2)), g(2, Some(1)), g(3, Some(42)), leaf, leaf2];
        sanitize_parents(&mut objects);
        // The cycle is broken (at least one of the pair detached) …
        let p1 = objects[0].parent;
        let p2 = objects[1].parent;
        assert!(p1.is_none() || p2.is_none());
        // … and every remaining chain terminates.
        for o in &objects {
            let mut cur = o.parent;
            let mut hops = 0;
            while let Some(p) = cur {
                cur = objects.iter().find(|x| x.id == p).and_then(|x| x.parent);
                hops += 1;
                assert!(hops < 16, "unterminated chain from #{}", o.id);
            }
        }
        assert_eq!(objects[2].parent, None); // dangling cleared
        assert_eq!(objects[4].parent, None); // non-group parent cleared
    }

    /// v9 round-trips the WHOLE merged payload: a group node, a parent
    /// link, the probe set with plot preferences, a locked color range,
    /// and the per-edge boundary kinds — the save-and-reload test of
    /// the second track merge.
    #[test]
    fn v9_roundtrip_persists_groups_probes_ranges_edges() {
        let mut child: SketchObject = SketchObjectV7::from(v5_obj(21)).into();
        child.parent = Some(20);
        let scene = SceneV9 {
            version: SCENE_V9,
            objects: vec![child, group_obj(20)],
            wind_tunnel: false,
            flow_speed: 0.09,
            viscosity: 0.015,
            steps_per_frame: 8,
            domain_width_m: 1.0,
            fluid_nu: 1.5e-5,
            fluid_rho: 1.2,
            ref_width: 1920,
            solver: 0,
            mach: 1.6,
            fluid_a: 343.0,
            ranges: locked_ranges(),
            probes: vec![SceneProbe { id: 3, pos: [120.0, 240.0] }],
            probe_quantity: 2,
            probe_show_plot: true,
            edges: [1, 2, 3, 0], // inlet, outlet, wall, far field
        };
        let bytes = bincode::serialize(&scene).unwrap();
        let version = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(version, SCENE_V9);
        let back = bincode::deserialize::<SceneV9>(&bytes).unwrap();
        assert_eq!(back.objects[0].parent, Some(20));
        assert!(matches!(
            back.objects[1].shape,
            Shape::Group { t: [3.0, 4.0], rot, scale } if rot == 0.5 && scale == 2.0
        ));
        assert_eq!(back.probes.len(), 1);
        assert_eq!(back.probes[0].pos, [120.0, 240.0]);
        assert_eq!(back.probe_quantity, 2);
        assert!(back.probe_show_plot);
        let r = back.ranges[RenderMode::Speed as usize];
        assert_eq!((r.mode, r.sat_phys, r.map), (1, 12.5, 1));
        assert_eq!(back.edges, [1, 2, 3, 0]);
        let decoded = EdgeBcs(back.edges.map(edge_kind_from_u32));
        assert_eq!(decoded.0[0], EdgeKind::Inlet);
        assert_eq!(decoded.0[2], EdgeKind::Wall);
    }

    /// A v8 FIXTURE (bytes written on the U3 branch pre-merge) loads
    /// through `from_v8`: groups, probes and ranges survive; the edge
    /// set defaults from `wind_tunnel` — all far field for an open
    /// scene, the tunnel preset for a tunnel one (the legacy bands).
    #[test]
    fn v8_fixture_converts_to_v9_with_default_edges() {
        for (tunnel, expect) in [(false, EdgeBcs::OPEN), (true, EdgeBcs::WIND_TUNNEL)] {
            let mut child: SketchObject = SketchObjectV7::from(v5_obj(21)).into();
            child.parent = Some(20);
            let scene = SceneV8 {
                version: SCENE_V8,
                objects: vec![child, group_obj(20)],
                wind_tunnel: tunnel,
                flow_speed: 0.09,
                viscosity: 0.015,
                steps_per_frame: 8,
                domain_width_m: 1.0,
                fluid_nu: 1.5e-5,
                fluid_rho: 1.2,
                ref_width: 1920,
                solver: 1,
                mach: 1.6,
                fluid_a: 343.0,
                ranges: locked_ranges(),
                probes: vec![SceneProbe { id: 3, pos: [120.0, 240.0] }],
                probe_quantity: 2,
                probe_show_plot: true,
            };
            let bytes = bincode::serialize(&scene).unwrap();
            assert_eq!(
                u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                SCENE_V8
            );
            let back =
                SceneV9::from_v8(bincode::deserialize::<SceneV8>(&bytes).unwrap());
            assert_eq!(back.objects[0].parent, Some(20));
            assert!(matches!(back.objects[1].shape, Shape::Group { .. }));
            assert_eq!(back.probes.len(), 1);
            assert_eq!(back.probe_quantity, 2);
            assert!(back.probe_show_plot);
            let r = back.ranges[RenderMode::Speed as usize];
            assert_eq!((r.mode, r.sat_phys, r.map), (1, 12.5, 1));
            assert_eq!(EdgeBcs(back.edges.map(edge_kind_from_u32)), expect);
        }
    }

    /// A pre-v9 file's edge set derives from its wind_tunnel flag: the
    /// tunnel preset for true, all far field for false — so legacy
    /// scenes keep their exact painted bands (or lack of them).
    #[test]
    fn pre_v9_edges_derive_from_wind_tunnel() {
        for (tunnel, expect) in [(true, EdgeBcs::WIND_TUNNEL), (false, EdgeBcs::OPEN)] {
            let scene = SceneV7 {
                version: SCENE_V7,
                objects: vec![],
                wind_tunnel: tunnel,
                flow_speed: 0.09,
                viscosity: 0.015,
                steps_per_frame: 8,
                domain_width_m: 1.0,
                fluid_nu: 1.5e-5,
                fluid_rho: 1.2,
                ref_width: 1920,
                solver: 0,
                mach: 1.6,
                fluid_a: 343.0,
                ranges: crate::sim::FIELD_RANGE_DEFAULTS.map(SceneRange::from),
            };
            let v9 = SceneV9::from_v8(SceneV8::from_v7(scene));
            assert_eq!(EdgeBcs(v9.edges.map(edge_kind_from_u32)), expect);
        }
    }

    /// The reserved periodic discriminant (4) and unknown future values
    /// decode without exploding: 4 keeps its reserved variant, anything
    /// newer falls back to far field.
    #[test]
    fn reserved_and_unknown_edge_kinds_decode_safely() {
        assert_eq!(edge_kind_from_u32(4), EdgeKind::Periodic);
        assert_eq!(edge_kind_from_u32(99), EdgeKind::FarField);
        // Reserved discriminant survives a save round-trip unchanged.
        assert_eq!(edge_kind_to_u32(edge_kind_from_u32(4)), 4);
    }
}
