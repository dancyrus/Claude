//! GPU simulation engine: owns the wgpu resources, the CPU-side geometry
//! document and the undo stack. Lives inside egui-wgpu's
//! `CallbackResources`; the app mutates it during `update`, and the paint
//! callback encodes the compute + render work each frame.

use crate::geometry::{Geometry, GridRect};
use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};

/// Visible-canvas resolutions; the simulated grid is larger by the margin.
pub const RESOLUTIONS: [(&str, usize, usize); 4] = [
    ("Low (960 x 480)", 960, 480),
    ("Medium (1440 x 720)", 1440, 720),
    ("High (1920 x 960)", 1920, 960),
    ("Ultra (2560 x 1280)", 2560, 1280),
];

/// Off-screen simulation margin around the visible canvas, as a fraction
/// of the visible height added on EACH side. A larger margin pushes the
/// domain boundaries (and their artifacts) away from what you see; the
/// outermost cells also get an absorbing sponge layer.
pub const MARGIN_CHOICES: [(&str, f32); 3] = [
    ("Small (+25 %)", 0.25),
    ("Medium (+50 %)", 0.5),
    ("Large (+100 %)", 1.0),
];
pub const DEFAULT_MARGIN_INDEX: usize = 1;

pub const PARTICLE_CHOICES: [(&str, u32); 4] = [
    ("100 k", 100_000),
    ("500 k", 500_000),
    ("1 M", 1_000_000),
    ("2 M", 2_000_000),
];
pub const MAX_PARTICLES: u64 = 2_000_000;

// --- Probes (plan v4.1, T2-B; folded into Settings at U3) --------------
//
// Persistent point probes. Track-era note: this state sat behind a
// process-wide `Mutex` (`sim::probes()`) while app.rs was frozen for
// Track 1; U3 folded it into `Settings.probes` with edits arriving
// through `Cmd` like every other setting (the T2-A precedent), and v8
// persists the probe positions. The sampling itself is sim machinery: a
// tiny per-frame GPU copy into a mapped-ring staging buffer, drained a
// couple of frames later without ever blocking a frame.

pub const MAX_PROBES: usize = 8;
/// Sample history cap per probe, in sampled frames; the oldest samples
/// drop. The plot panel states this cap, per the plan.
pub const PROBE_HISTORY_CAP: usize = 2048;

#[derive(Clone, Copy)]
pub struct ProbeSample {
    /// Lattice steps since the last flow reset (the x axis; the UI
    /// converts to seconds with the current Δt).
    pub steps: f32,
    /// Velocity-buffer value at the probe cell (render units: LBM
    /// cells/step, Euler u·dt).
    pub vel: [f32; 2],
    /// Central-difference curl of the velocity buffer, the same stencil
    /// as render.wgsl's vorticity view (render units).
    pub curl: f32,
    /// Density-buffer deviation from the reference 1.0.
    pub drho: f32,
    /// Smoke luminance at the probe, arbitrary units.
    pub dye: f32,
}

pub struct Probe {
    pub id: u32,
    /// Position in visible-canvas cells (margin-independent; rescaled
    /// with the grid on resolution changes, like sketch objects).
    pub pos: [f32; 2],
    pub samples: VecDeque<ProbeSample>,
}

/// Which quantity the plot panel draws (a UI preference).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ProbeQuantity {
    #[default]
    Speed,
    Vorticity,
    Pressure,
    Smoke,
}

impl ProbeQuantity {
    pub const ALL: [ProbeQuantity; 4] = [
        ProbeQuantity::Speed,
        ProbeQuantity::Vorticity,
        ProbeQuantity::Pressure,
        ProbeQuantity::Smoke,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            ProbeQuantity::Speed => "Speed",
            ProbeQuantity::Vorticity => "Vorticity",
            ProbeQuantity::Pressure => "Pressure",
            ProbeQuantity::Smoke => "Smoke",
        }
    }
}

/// The probe store: part of `Settings` since U3 (no globals). The UI
/// reads a per-frame snapshot and edits through `Cmd`; the sampling
/// machinery reads it directly (it owns `&mut GpuSim`).
pub struct ProbeSet {
    pub probes: Vec<Probe>,
    pub next_id: u32,
    pub show_plot: bool,
    pub quantity: ProbeQuantity,
}

impl Default for ProbeSet {
    fn default() -> Self {
        ProbeSet {
            probes: Vec::new(),
            next_id: 1,
            show_plot: false,
            quantity: ProbeQuantity::Speed,
        }
    }
}

/// One probe-readback staging buffer and where it is in the copy → map
/// → read round trip. Copies encode in frame N (submitted right after
/// `encode_compute` returns), the map request goes out in frame N+1,
/// and the data drains on whichever later frame the map completes.
struct ProbeStage {
    buf: wgpu::Buffer,
    state: ProbeStageState,
    /// Probe ids in slot order at copy time (the set can change before
    /// the data comes back).
    ids: Vec<u32>,
    /// `total_steps` when the copy was encoded.
    stamp: f32,
    /// Flow-reset generation at copy time; stale data is dropped.
    generation: u32,
}

enum ProbeStageState {
    Free,
    Copied,
    /// The map callback fills the slot (an mpsc receiver would make
    /// `GpuSim: !Sync`, which `CallbackResources` requires).
    Mapping(Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>),
}

/// Per-probe slot layout in a staging buffer: vel at c, c+1, c-1, c+W,
/// c-W (5 × 8 B), rho (4 B at +40), dye rgba (16 B at +48).
const PROBE_SLOT_BYTES: u64 = 64;

/// Which solver advances the flow.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SolverMode {
    /// D2Q9 lattice-Boltzmann: incompressible, viscous, low Mach.
    Lbm,
    /// Finite-volume compressible Euler (MUSCL + HLLC): shocks and
    /// expansion fans, inviscid.
    Euler,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    Dye = 0,
    Speed = 1,
    Vorticity = 2,
    Pressure = 3,
}

impl RenderMode {
    pub const ALL: [RenderMode; 4] =
        [RenderMode::Dye, RenderMode::Speed, RenderMode::Vorticity, RenderMode::Pressure];
    pub fn label(&self) -> &'static str {
        match self {
            RenderMode::Dye => "Smoke",
            RenderMode::Speed => "Speed",
            RenderMode::Vorticity => "Vorticity",
            RenderMode::Pressure => "Pressure",
        }
    }
}

// --- Color range (plan v4.1, T2-A) -----------------------------------
//
// The saturation point of the field color mapping, per render mode.
// Track-era note: this sat behind a process-wide Mutex while app.rs was
// frozen for Track 1; the track merge folded it into `Settings.ranges`
// with edits arriving through `Cmd` like every other setting.

/// How the color-mapping range for one render mode is chosen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RangeMode {
    /// The saturation point follows the inlet condition and the display
    /// gain — the pre-T2-A behavior, where the scale floats.
    Auto,
    /// The physical value that was on screen when the user locked is
    /// kept; the scale no longer follows the flow settings. After the
    /// capture this behaves exactly like `Manual`.
    Locked,
    /// The user typed the saturation value in physical units.
    Manual,
}

/// Which colormap paints a field view. The ramps live in render.wgsl
/// with CPU mirrors in app.rs (the legend bars); this only picks one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ColorMap {
    Inferno,
    Coolwarm,
}

impl ColorMap {
    /// The map a render mode is born with — the pre-T2-A binding, which
    /// is also what the shader draws when flags bit 1 is clear.
    pub fn default_for(mode: RenderMode) -> ColorMap {
        match mode {
            RenderMode::Dye | RenderMode::Speed => ColorMap::Inferno,
            RenderMode::Vorticity | RenderMode::Pressure => ColorMap::Coolwarm,
        }
    }
}

/// One render mode's range. The two values describe the same saturation
/// point: the physical value is authoritative for `Locked` and `Manual`,
/// and the UI rewrites the render-unit twin every frame (the physical
/// conversions live app-side, next to the rest of the unit handling);
/// under `Auto` the UI keeps both tracking the current settings.
#[derive(Clone, Copy)]
pub struct FieldRange {
    pub mode: RangeMode,
    /// Saturation point in render-buffer units (velocity-buffer units for
    /// Speed, their curl for Vorticity, density deviation for Pressure).
    /// This is what the shader mapping uses.
    pub sat_render: f32,
    /// The same point in physical units (m/s, 1/s, Pa) — what the UI
    /// shows and edits, and what a pinned range holds on to.
    pub sat_phys: f32,
    /// The colormap this view draws with (user-pickable, T2-A).
    pub map: ColorMap,
}

const fn field_range_default(map: ColorMap) -> FieldRange {
    FieldRange { mode: RangeMode::Auto, sat_render: 1.0, sat_phys: 0.0, map }
}

/// The color-range defaults, indexed by `RenderMode as usize`. The Dye
/// entry is unused — smoke is a passive tracer with no scale to lock.
pub const FIELD_RANGE_DEFAULTS: [FieldRange; 4] = [
    field_range_default(ColorMap::Inferno),  // Dye (unused)
    field_range_default(ColorMap::Inferno),  // Speed
    field_range_default(ColorMap::Coolwarm), // Vorticity
    field_range_default(ColorMap::Coolwarm), // Pressure
];

/// Simulation settings mirrored by the UI.
pub struct Settings {
    pub paused: bool,
    pub wind_tunnel: bool,
    pub solver: SolverMode,
    /// Euler mode: inlet Mach number (freestream speed / sound speed).
    pub mach: f32,
    pub flow_speed: f32,   // lattice inlet speed
    pub viscosity: f32,    // lattice kinematic viscosity
    pub steps_per_frame: u32,
    pub dye_fade: f32,     // per-frame retention
    pub render_mode: RenderMode,
    pub particle_count: u32,
    pub boundary_tints: bool,
    /// Gain on the speed/vorticity/pressure color mapping.
    pub display_gain: f32,
    /// Gain on smoke brightness in the Smoke view.
    pub smoke_gain: f32,
    /// Particle quad half-size in framebuffer pixels.
    pub particle_size: f32,
    /// Peak particle alpha.
    pub particle_brightness: f32,
    /// Absorbing-layer blend strength at the domain edge.
    pub sponge_strength: f32,
    /// Per-render-mode color range and colormap pick (T2-A), indexed by
    /// `RenderMode as usize`. The UI keeps the render/physical twins in
    /// sync every frame; edits arrive through `Cmd` like any setting.
    pub ranges: [FieldRange; 4],
    /// Persistent point probes (T2-B; folded here at U3, persisted v8+).
    pub probes: ProbeSet,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            paused: false,
            wind_tunnel: true,
            solver: SolverMode::Lbm,
            mach: 1.6,
            flow_speed: 0.09,
            viscosity: 0.015,
            steps_per_frame: 8,
            dye_fade: 0.995,
            render_mode: RenderMode::Dye,
            particle_count: 0, // tracers are opt-in
            boundary_tints: true,
            display_gain: 1.0,
            smoke_gain: 1.0,
            particle_size: 1.6,
            particle_brightness: 0.3,
            sponge_strength: 0.08,
            ranges: FIELD_RANGE_DEFAULTS,
            probes: ProbeSet::default(),
        }
    }
}

// --- Uniform layouts (match the WGSL structs) ------------------------

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SimParamsRaw {
    width: u32,
    height: u32,
    omega: f32,
    inlet_speed: f32,
    dye_dt: f32,
    dye_decay: f32,
    sponge_width: f32,
    sponge_strength: f32,
    free_u: [f32; 2],
    time: f32, // lattice steps elapsed (drives fan gusts)
    _pad1: f32,
}

// Matches EulerParams in euler.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EulerParamsRaw {
    width: u32,
    height: u32,
    gamma: f32,
    mach: f32,
    dt: f32,
    blend: f32,
    sponge_width: f32,
    sponge_strength: f32,
    free_u: [f32; 2],
    time: f32,
    write_render: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PartParamsRaw {
    width: u32,
    height: u32,
    count: u32,
    frame: u32,
    dt: f32,
    _pad0: f32,
    // Respawn window (full-grid cells): the visible area plus an
    // upstream band, so tracer density on screen doesn't drop as the
    // off-screen margin grows.
    spawn_min: [u32; 2],
    spawn_max: [u32; 2],
    _pad1: f32,
    _pad2: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RenderParamsRaw {
    width: u32,
    height: u32,
    mode: u32,
    flags: u32,
    vp_origin: [f32; 2],
    vp_size: [f32; 2],
    lb_origin: [f32; 2],
    px_per_cell: f32,
    inlet_speed: f32,
    vis_origin: [u32; 2],
    vis_size: [u32; 2],
    display_gain: f32,
    smoke_gain: f32,
    particle_size: f32,
    particle_brightness: f32,
}

/// Per-frame viewport mapping computed by the app from the canvas rect.
#[derive(Clone, Copy, Default)]
pub struct ViewportMapping {
    pub vp_origin: [f32; 2],  // framebuffer px
    pub vp_size: [f32; 2],
    pub lb_origin: [f32; 2],  // letterboxed grid origin, framebuffer px
    pub px_per_cell: f32,
}

impl ViewportMapping {
    /// Compute the letterbox mapping for a grid inside a viewport.
    pub fn fit(vp_origin: [f32; 2], vp_size: [f32; 2], gw: usize, gh: usize) -> Self {
        let sx = vp_size[0] / gw as f32;
        let sy = vp_size[1] / gh as f32;
        let s = sx.min(sy).max(1e-6);
        let used = [gw as f32 * s, gh as f32 * s];
        let lb = [
            vp_origin[0] + (vp_size[0] - used[0]) * 0.5,
            vp_origin[1] + (vp_size[1] - used[1]) * 0.5,
        ];
        Self { vp_origin, vp_size, lb_origin: lb, px_per_cell: s }
    }

    /// Framebuffer px -> grid cell coordinates.
    pub fn px_to_cell(&self, px: [f32; 2]) -> [f32; 2] {
        [
            (px[0] - self.lb_origin[0]) / self.px_per_cell,
            (px[1] - self.lb_origin[1]) / self.px_per_cell,
        ]
    }
}

// The unused buffer handles are kept for clarity of ownership; the bind
// groups hold their own references.
#[allow(dead_code)]
struct GridBuffers {
    f_a: wgpu::Buffer,
    f_b: wgpu::Buffer,
    // Euler conserved-state buffers (SSP-RK2 needs U^n, the stage value
    // and the result live at once, rotated cyclically).
    u_a: wgpu::Buffer,
    u_b: wgpu::Buffer,
    u_c: wgpu::Buffer,
    vel: wgpu::Buffer,
    rho: wgpu::Buffer,
    cell: wgpu::Buffer,
    fan: wgpu::Buffer,
    dye_a: wgpu::Buffer,
    dye_b: wgpu::Buffer,
    dye_src: wgpu::Buffer,
    // Ping-pong bind groups: [0] reads A writes B, [1] reads B writes A.
    lbm_bind: [wgpu::BindGroup; 2],
    dye_bind: [wgpu::BindGroup; 2],
    part_bind: wgpu::BindGroup,
    // Euler bind groups: [rotation][stage]. With the buffer triple
    // rotated as (a,b,c) -> (c,a,b) -> (b,c,a), stage 1 reads slot 0 and
    // writes slot 1; stage 2 reads slots 1 (src) + 0 (base) and writes
    // slot 2, which becomes the next rotation's slot 0.
    euler_bind: [[wgpu::BindGroup; 2]; 3],
    // Render bind groups keyed by which dye buffer is current.
    render_bind: [wgpu::BindGroup; 2],
    /// Which f/dye buffer holds the current state (0 = A, 1 = B).
    f_side: usize,
    dye_side: usize,
    /// Current Euler rotation index (0..3).
    euler_rot: usize,
}

pub struct GpuSim {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,

    lbm_layout: wgpu::BindGroupLayout,
    euler_layout: wgpu::BindGroupLayout,
    dye_layout: wgpu::BindGroupLayout,
    part_layout: wgpu::BindGroupLayout,
    render_layout: wgpu::BindGroupLayout,

    collide_pipeline: wgpu::ComputePipeline,
    reset_pipeline: wgpu::ComputePipeline,
    euler_step_pipeline: wgpu::ComputePipeline,
    euler_reset_pipeline: wgpu::ComputePipeline,
    advect_pipeline: wgpu::ComputePipeline,
    clear_dye_pipeline: wgpu::ComputePipeline,
    part_update_pipeline: wgpu::ComputePipeline,
    field_pipeline: wgpu::RenderPipeline,
    particle_pipeline: wgpu::RenderPipeline,
    field_pipeline_rgba: wgpu::RenderPipeline, // for PNG export

    sim_uniform: wgpu::Buffer,
    // Per-RK-stage Euler uniforms (blend / write_render differ).
    euler_uniform_s1: wgpu::Buffer,
    euler_uniform_s2: wgpu::Buffer,
    part_uniform: wgpu::Buffer,
    render_uniform: wgpu::Buffer,
    particles: wgpu::Buffer,
    particle_bind_group1: wgpu::BindGroup,

    bufs: GridBuffers,

    pub geo: Geometry,
    /// Visible-canvas size in cells; the full grid is vis + 2 * margin.
    vis_w: usize,
    vis_h: usize,
    margin: usize,
    margin_frac: f32,
    pub settings: Settings,
    pub mapping: ViewportMapping,

    frame_counter: u32,
    /// mach * euler_dt as of the last frame that wrote the velocity
    /// buffer (see render_inlet_speed).
    euler_render_ref: f32,
    /// Lattice time (steps) elapsed, fed to the fan-gust animation.
    lattice_time: f32,
    /// Total lattice steps since the last flow reset (never wraps; used
    /// for the physical sim-time readout).
    pub total_steps: f64,
    pending_reset: bool,
    pending_clear_dye: bool,
    /// Advance one frame's worth of steps despite `paused`; cleared by
    /// `encode_compute` after one frame.
    pub step_once: bool,
    /// Steps actually encoded last frame (for stats/particle dt).
    pub steps_last_frame: u32,
    /// Probe-readback staging ring (T2-B).
    probe_stages: Vec<ProbeStage>,
    /// Bumped on every flow reset so in-flight probe readbacks from the
    /// old flow are dropped instead of appended.
    probe_generation: u32,
}

/// Margin size in cells for a visible size and margin fraction, clamped so
/// the largest storage buffer (an f distribution buffer) stays within the
/// device's binding limits.
fn margin_cells(device: &wgpu::Device, vis_w: usize, vis_h: usize, frac: f32) -> usize {
    let mut margin = (vis_h as f32 * frac).round().max(0.0) as usize;
    let limits = device.limits();
    let cap = (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size);
    loop {
        let w = (vis_w + 2 * margin) as u64;
        let h = (vis_h + 2 * margin) as u64;
        if w * h * 9 * 4 <= cap || margin == 0 {
            break;
        }
        margin = margin.saturating_sub(32);
    }
    margin
}

fn storage_entry(binding: u32, read_only: bool, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl GpuSim {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        target_format: wgpu::TextureFormat,
        res_index: usize,
    ) -> Self {
        let (_, vis_w, vis_h) = RESOLUTIONS[res_index];
        let margin = margin_cells(
            &device,
            vis_w,
            vis_h,
            MARGIN_CHOICES[DEFAULT_MARGIN_INDEX].1,
        );
        let (w, h) = (vis_w + 2 * margin, vis_h + 2 * margin);

        let lbm_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lbm"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lbm.wgsl").into()),
        });
        let euler_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("euler"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/euler.wgsl").into()),
        });
        let dye_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("dye"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/dye.wgsl").into()),
        });
        let part_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/particles.wgsl").into()),
        });
        let render_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/render.wgsl").into()),
        });

        let c = wgpu::ShaderStages::COMPUTE;
        let lbm_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lbm layout"),
            entries: &[
                uniform_entry(0, c),
                storage_entry(1, true, c),
                storage_entry(2, false, c),
                storage_entry(3, true, c),
                storage_entry(4, true, c),
                storage_entry(5, false, c),
                storage_entry(6, false, c),
            ],
        });
        let euler_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("euler layout"),
            entries: &[
                uniform_entry(0, c),
                storage_entry(1, true, c),  // u_src
                storage_entry(2, true, c),  // u_base
                storage_entry(3, false, c), // u_dst
                storage_entry(4, true, c),  // cell
                storage_entry(5, true, c),  // fan
                storage_entry(6, false, c), // velocity
                storage_entry(7, false, c), // density
            ],
        });
        let dye_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dye layout"),
            entries: &[
                uniform_entry(0, c),
                storage_entry(1, true, c),
                storage_entry(2, false, c),
                storage_entry(3, true, c),
                storage_entry(4, true, c),
                storage_entry(5, true, c),
            ],
        });
        let part_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle layout"),
            entries: &[
                uniform_entry(0, c),
                storage_entry(1, false, c),
                storage_entry(2, true, c),
                storage_entry(3, true, c),
            ],
        });
        let vf = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render layout"),
            entries: &[
                uniform_entry(0, vf),
                storage_entry(1, true, vf),
                storage_entry(2, true, vf),
                storage_entry(3, true, vf),
                storage_entry(4, true, vf),
            ],
        });
        let particle_render_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("particle render layout"),
                entries: &[storage_entry(0, true, vf)],
            });

        let compute_pl = |layout: &wgpu::BindGroupLayout, label: &str| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[layout],
                push_constant_ranges: &[],
            })
        };
        let lbm_pl = compute_pl(&lbm_layout, "lbm pl");
        let euler_pl = compute_pl(&euler_layout, "euler pl");
        let dye_pl = compute_pl(&dye_layout, "dye pl");
        let part_pl = compute_pl(&part_layout, "part pl");

        let make_cp = |pl: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(pl),
                module,
                entry_point: entry,
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let collide_pipeline = make_cp(&lbm_pl, &lbm_module, "collide");
        let reset_pipeline = make_cp(&lbm_pl, &lbm_module, "reset_rest");
        let euler_step_pipeline = make_cp(&euler_pl, &euler_module, "euler_step");
        let euler_reset_pipeline = make_cp(&euler_pl, &euler_module, "euler_reset");
        let advect_pipeline = make_cp(&dye_pl, &dye_module, "advect");
        let clear_dye_pipeline = make_cp(&dye_pl, &dye_module, "clear_dye");
        let part_update_pipeline = make_cp(&part_pl, &part_module, "update");

        let render_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("field pl"),
            bind_group_layouts: &[&render_layout],
            push_constant_ranges: &[],
        });
        let particle_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle render pl"),
            bind_group_layouts: &[&render_layout, &particle_render_layout],
            push_constant_ranges: &[],
        });

        let make_field_pipeline = |format: wgpu::TextureFormat, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&render_pl),
                vertex: wgpu::VertexState {
                    module: &render_module,
                    entry_point: "vs_fullscreen",
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &render_module,
                    entry_point: "fs_field",
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let field_pipeline = make_field_pipeline(target_format, "field");
        let field_pipeline_rgba =
            make_field_pipeline(wgpu::TextureFormat::Rgba8UnormSrgb, "field export");

        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let particle_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles"),
            layout: Some(&particle_pl),
            vertex: wgpu::VertexState {
                module: &render_module,
                entry_point: "vs_particles",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_module,
                entry_point: "fs_particles",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(additive),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_usage = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let sim_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim uniform"),
            size: std::mem::size_of::<SimParamsRaw>() as u64,
            usage: uniform_usage,
            mapped_at_creation: false,
        });
        let mk_euler_uniform = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<EulerParamsRaw>() as u64,
                usage: uniform_usage,
                mapped_at_creation: false,
            })
        };
        let euler_uniform_s1 = mk_euler_uniform("euler uniform s1");
        let euler_uniform_s2 = mk_euler_uniform("euler uniform s2");
        let part_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("part uniform"),
            size: std::mem::size_of::<PartParamsRaw>() as u64,
            usage: uniform_usage,
            mapped_at_creation: false,
        });
        let render_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render uniform"),
            size: std::mem::size_of::<RenderParamsRaw>() as u64,
            usage: uniform_usage,
            mapped_at_creation: false,
        });

        let particles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles"),
            size: MAX_PARTICLES * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let particle_bind_group1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle render bind"),
            layout: &particle_render_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: particles.as_entire_binding(),
            }],
        });

        let geo = Geometry::new(w, h);
        let bufs = Self::create_grid_buffers(
            &device,
            w,
            h,
            &lbm_layout,
            &euler_layout,
            &dye_layout,
            &part_layout,
            &render_layout,
            &sim_uniform,
            &euler_uniform_s1,
            &euler_uniform_s2,
            &part_uniform,
            &render_uniform,
            &particles,
        );

        let probe_stages = (0..3)
            .map(|i| ProbeStage {
                buf: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("probe staging {i}")),
                    size: MAX_PROBES as u64 * PROBE_SLOT_BYTES,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: ProbeStageState::Free,
                ids: Vec::new(),
                stamp: 0.0,
                generation: 0,
            })
            .collect();

        let sim = Self {
            device,
            queue,
            lbm_layout,
            euler_layout,
            dye_layout,
            part_layout,
            render_layout,
            collide_pipeline,
            reset_pipeline,
            euler_step_pipeline,
            euler_reset_pipeline,
            advect_pipeline,
            clear_dye_pipeline,
            part_update_pipeline,
            field_pipeline,
            particle_pipeline,
            field_pipeline_rgba,
            sim_uniform,
            euler_uniform_s1,
            euler_uniform_s2,
            part_uniform,
            render_uniform,
            particles,
            particle_bind_group1,
            bufs,
            geo,
            vis_w,
            vis_h,
            margin,
            margin_frac: MARGIN_CHOICES[DEFAULT_MARGIN_INDEX].1,
            settings: Settings::default(),
            mapping: ViewportMapping::default(),
            frame_counter: 0,
            euler_render_ref: 0.1,
            lattice_time: 0.0,
            total_steps: 0.0,
            pending_reset: true,
            pending_clear_dye: true,
            step_once: false,
            steps_last_frame: 0,
            probe_stages,
            probe_generation: 0,
        };
        // Content (including tunnel bands) is projected from the sketch
        // model on the first frame.
        sim
    }

    #[allow(clippy::too_many_arguments)]
    fn create_grid_buffers(
        device: &wgpu::Device,
        w: usize,
        h: usize,
        lbm_layout: &wgpu::BindGroupLayout,
        euler_layout: &wgpu::BindGroupLayout,
        dye_layout: &wgpu::BindGroupLayout,
        part_layout: &wgpu::BindGroupLayout,
        render_layout: &wgpu::BindGroupLayout,
        sim_uniform: &wgpu::Buffer,
        euler_uniform_s1: &wgpu::Buffer,
        euler_uniform_s2: &wgpu::Buffer,
        part_uniform: &wgpu::Buffer,
        render_uniform: &wgpu::Buffer,
        particles: &wgpu::Buffer,
    ) -> GridBuffers {
        let n = (w * h) as u64;
        // COPY_SRC so the probe sampler (T2-B) can copy single cells out
        // of the field buffers; a usage flag costs nothing at rest.
        let mk = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let f_a = mk("f a", n * 9 * 4);
        let f_b = mk("f b", n * 9 * 4);
        let u_a = mk("euler u a", n * 16);
        let u_b = mk("euler u b", n * 16);
        let u_c = mk("euler u c", n * 16);
        let vel = mk("velocity", n * 8);
        let rho = mk("density", n * 4);
        let cell = mk("cell type", n * 4);
        let fan = mk("fan physics", n * 16);
        let dye_a = mk("dye a", n * 16);
        let dye_b = mk("dye b", n * 16);
        let dye_src = mk("dye src", n * 16);

        fn entry(binding: u32, buf: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry { binding, resource: buf.as_entire_binding() }
        }

        let lbm_bind_for = |fi: &wgpu::Buffer, fo: &wgpu::Buffer, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: lbm_layout,
                entries: &[
                    entry(0, sim_uniform),
                    entry(1, fi),
                    entry(2, fo),
                    entry(3, &cell),
                    entry(4, &fan),
                    entry(5, &vel),
                    entry(6, &rho),
                ],
            })
        };
        let lbm_bind = [lbm_bind_for(&f_a, &f_b, "lbm a->b"), lbm_bind_for(&f_b, &f_a, "lbm b->a")];

        let dye_bind_for = |di: &wgpu::Buffer, do_: &wgpu::Buffer, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: dye_layout,
                entries: &[
                    entry(0, sim_uniform),
                    entry(1, di),
                    entry(2, do_),
                    entry(3, &vel),
                    entry(4, &cell),
                    entry(5, &dye_src),
                ],
            })
        };
        let dye_bind =
            [dye_bind_for(&dye_a, &dye_b, "dye a->b"), dye_bind_for(&dye_b, &dye_a, "dye b->a")];

        // Euler bind groups: buffer-triple rotations (t0, t1, t2) with
        // stage 1 = (src t0, base t0, dst t1) and stage 2 =
        // (src t1, base t0, dst t2); the next rotation starts at t2.
        let euler_bind_for = |uniform: &wgpu::Buffer,
                              src: &wgpu::Buffer,
                              base: &wgpu::Buffer,
                              dst: &wgpu::Buffer,
                              label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: euler_layout,
                entries: &[
                    entry(0, uniform),
                    entry(1, src),
                    entry(2, base),
                    entry(3, dst),
                    entry(4, &cell),
                    entry(5, &fan),
                    entry(6, &vel),
                    entry(7, &rho),
                ],
            })
        };
        let rotations = [[&u_a, &u_b, &u_c], [&u_c, &u_a, &u_b], [&u_b, &u_c, &u_a]];
        let euler_bind = rotations.map(|[t0, t1, t2]| {
            [
                euler_bind_for(euler_uniform_s1, t0, t0, t1, "euler stage 1"),
                euler_bind_for(euler_uniform_s2, t1, t0, t2, "euler stage 2"),
            ]
        });

        let part_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle bind"),
            layout: part_layout,
            entries: &[
                entry(0, part_uniform),
                entry(1, particles),
                entry(2, &vel),
                entry(3, &cell),
            ],
        });

        let render_bind_for = |dye: &wgpu::Buffer, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: render_layout,
                entries: &[
                    entry(0, render_uniform),
                    entry(1, &vel),
                    entry(2, &rho),
                    entry(3, dye),
                    entry(4, &cell),
                ],
            })
        };
        let render_bind =
            [render_bind_for(&dye_a, "render dye a"), render_bind_for(&dye_b, "render dye b")];

        GridBuffers {
            f_a,
            f_b,
            u_a,
            u_b,
            u_c,
            vel,
            rho,
            cell,
            fan,
            dye_a,
            dye_b,
            dye_src,
            lbm_bind,
            dye_bind,
            part_bind,
            euler_bind,
            render_bind,
            f_side: 0,
            dye_side: 0,
            euler_rot: 0,
        }
    }

    // --- Public control ----------------------------------------------

    /// Visible-canvas size in cells (what painting and rendering map to).
    pub fn grid_size(&self) -> (usize, usize) {
        (self.vis_w, self.vis_h)
    }

    /// Full simulated size including the off-screen margin.
    pub fn full_size(&self) -> (usize, usize) {
        (self.geo.w, self.geo.h)
    }

    pub fn margin(&self) -> usize {
        self.margin
    }

    /// Queue a full flow reset (populations to the freestream, dye
    /// cleared, sim clock zeroed). Probe histories describe the old flow
    /// and the sim clock they are stamped with restarts, so they clear
    /// too (the probes themselves stay).
    pub fn reset_flow(&mut self) {
        self.pending_reset = true;
        self.pending_clear_dye = true;
        self.total_steps = 0.0;
        self.probe_generation = self.probe_generation.wrapping_add(1);
        for p in self.settings.probes.probes.iter_mut() {
            p.samples.clear();
        }
    }

    /// CFL-limited time step for the Euler solver (nondimensional).
    /// Budgeted for the unsplit 2D update, dt * (Sx + Sy) <= CFL, against
    /// the fastest state the solver DESIGN permits: fans boosted to 2x the
    /// inlet Mach (|u| up to 2*mach) plus post-shock sound speeds up to
    /// ~1.7 a_inf for M <= 3 — hence 2*mach + 3.5. States beyond that
    /// envelope are caught by the in-kernel guard instead.
    pub fn euler_dt(&self) -> f32 {
        0.35 / (2.0 * self.settings.mach.max(0.3) + 3.5)
    }

    /// Inlet-state CFL estimate for the status strip. Euler: the Courant
    /// number of the freestream state, dt * (|u_inlet| + a) / dx with
    /// a = 1 and dx = 1 — the field maximum can run higher through
    /// expansions (the dt budget in euler_dt covers that envelope). LBM
    /// has no acoustic CFL in the same sense; the analogous advective
    /// number is the inlet lattice speed in cells per step.
    pub fn cfl_estimate(&self) -> f32 {
        match self.settings.solver {
            SolverMode::Euler => self.euler_dt() * (self.settings.mach.max(0.0) + 1.0),
            SolverMode::Lbm => self.settings.flow_speed,
        }
    }

    /// Inlet-speed normalization for the renderer, in the same units as
    /// the velocity buffer (LBM: lattice cells/step; Euler: the velocity
    /// buffer stores u * dt, so the reference is mach * dt AS OF the last
    /// frame that actually wrote the buffer — changing Mach while paused
    /// must not rescale stale data).
    fn render_inlet_speed(&self) -> f32 {
        match self.settings.solver {
            SolverMode::Lbm => self.settings.flow_speed,
            SolverMode::Euler => self.euler_render_ref.max(1e-4),
        }
    }

    /// The flags word for the render uniform: bit 0 draws boundary
    /// tints, bit 1 tells the shader to swap the view's colormap away
    /// from its default binding (see ColorMap::default_for).
    fn render_flags(&self) -> u32 {
        let mut flags = if self.settings.boundary_tints { 1 } else { 0 };
        let mode = self.settings.render_mode;
        if self.settings.ranges[mode as usize].map != ColorMap::default_for(mode) {
            flags |= 2;
        }
        flags
    }

    /// The display gain the render uniform should carry. A pinned
    /// (Locked/Manual) range maps onto the one knob the shader exposes:
    /// every mapping in render.wgsl is linear in `display_gain`, so a
    /// fixed saturation point is a per-frame gain — each arm below sets
    /// the shader's clip point to `sat_render` by inverting the
    /// corresponding normalization. Auto passes the user's gain through.
    fn range_display_gain(&self) -> f32 {
        let mode = self.settings.render_mode;
        let fr = self.settings.ranges[mode as usize];
        if fr.mode == RangeMode::Auto {
            return self.settings.display_gain;
        }
        let sat = fr.sat_render.max(1e-9);
        let inlet = self.render_inlet_speed();
        match mode {
            RenderMode::Speed => (inlet * 1.6).max(1e-3) / sat,
            RenderMode::Vorticity => inlet.max(0.02) / (4.0 * sat),
            RenderMode::Pressure => 1.0 / (25.0 * sat),
            RenderMode::Dye => self.settings.display_gain,
        }
    }


    pub fn set_wind_tunnel(&mut self, on: bool) {
        self.settings.wind_tunnel = on;
        // The model rasterizer paints or clears the tunnel bands.
    }


    /// Switch grid resolution, resampling the current visible scene.
    pub fn set_resolution(&mut self, res_index: usize) {
        let (_, vis_w, vis_h) = RESOLUTIONS[res_index];
        if (vis_w, vis_h) == (self.vis_w, self.vis_h) {
            return;
        }
        let margin = margin_cells(&self.device, vis_w, vis_h, self.margin_frac);
        self.rebuild_grid(vis_w, vis_h, margin);
    }

    /// Change the off-screen margin, preserving the visible scene.
    pub fn set_margin_frac(&mut self, frac: f32) {
        self.margin_frac = frac;
        let margin = margin_cells(&self.device, self.vis_w, self.vis_h, frac);
        if margin == self.margin {
            return;
        }
        self.rebuild_grid(self.vis_w, self.vis_h, margin);
    }

    /// Rebuild the full grid at a new visible size and/or margin. The
    /// sketch model re-rasterizes the content afterwards, so no raster
    /// transfer happens here.
    fn rebuild_grid(&mut self, vis_w: usize, vis_h: usize, margin: usize) {
        // Probe positions are visible-canvas cells; scale them with the
        // grid the same way the sketch model rescales its objects.
        let scale = vis_w as f32 / self.vis_w.max(1) as f32;
        if (scale - 1.0).abs() > 1e-6 {
            for p in self.settings.probes.probes.iter_mut() {
                p.pos[0] *= scale;
                p.pos[1] *= scale;
            }
        }
        self.vis_w = vis_w;
        self.vis_h = vis_h;
        self.margin = margin;
        let (w, h) = (vis_w + 2 * margin, vis_h + 2 * margin);
        let mut geo = Geometry::new(w, h);
        geo.dirty = Some(GridRect::full(w, h));
        self.geo = geo;
        self.bufs = Self::create_grid_buffers(
            &self.device,
            w,
            h,
            &self.lbm_layout,
            &self.euler_layout,
            &self.dye_layout,
            &self.part_layout,
            &self.render_layout,
            &self.sim_uniform,
            &self.euler_uniform_s1,
            &self.euler_uniform_s2,
            &self.part_uniform,
            &self.render_uniform,
            &self.particles,
        );
        self.clear_particles();
        self.reset_flow();
    }

    /// Zero the particle buffer so every slot respawns (positions are in
    /// grid cells and go stale when the grid changes).
    fn clear_particles(&self) {
        self.queue
            .write_buffer(&self.particles, 0, &vec![0u8; (MAX_PARTICLES * 16) as usize]);
    }




    /// Upload any dirty geometry region to the GPU. Called after all edits
    /// for the frame have been applied.
    pub fn flush_geometry(&mut self) {
        let Some(rect) = self.geo.dirty.take() else { return };
        let rect = rect.clampped(self.geo.w, self.geo.h);
        if rect.is_empty() {
            return;
        }
        let w = self.geo.w;
        for y in rect.y0..rect.y1 {
            let row = (y as usize) * w;
            let a = row + rect.x0 as usize;
            let b = row + rect.x1 as usize;
            let off_cell = (a * 4) as u64;
            let off_fan = (a * 16) as u64;
            let off_src = (a * 16) as u64;
            self.queue.write_buffer(
                &self.bufs.cell,
                off_cell,
                bytemuck::cast_slice(&self.geo.cell[a..b]),
            );
            self.queue.write_buffer(
                &self.bufs.fan,
                off_fan,
                bytemuck::cast_slice(&self.geo.fan[a..b]),
            );
            self.queue.write_buffer(
                &self.bufs.dye_src,
                off_src,
                bytemuck::cast_slice(&self.geo.dye_src[a..b]),
            );
        }
    }

    // --- Frame encoding (called from the egui paint callback) --------

    fn dispatch_grid(&self, pass: &mut wgpu::ComputePass, w: usize, h: usize) {
        pass.dispatch_workgroups((w as u32 + 7) / 8, (h as u32 + 7) / 8, 1);
    }

    /// Encode this frame's compute work onto the provided encoder.
    pub fn encode_compute(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let (w, h) = (self.geo.w, self.geo.h);
        let steps = if !self.settings.paused || self.step_once {
            self.settings.steps_per_frame.max(1)
        } else {
            0
        };
        self.step_once = false;
        self.steps_last_frame = steps;

        let params = SimParamsRaw {
            width: w as u32,
            height: h as u32,
            omega: 1.0 / (3.0 * self.settings.viscosity.max(0.004) + 0.5),
            inlet_speed: self.settings.flow_speed,
            dye_dt: steps as f32,
            dye_decay: if self.settings.paused { 1.0 } else { self.settings.dye_fade },
            sponge_width: (self.margin.min(96)) as f32,
            sponge_strength: self.settings.sponge_strength,
            free_u: if self.settings.wind_tunnel {
                [self.settings.flow_speed, 0.0]
            } else {
                [0.0, 0.0]
            },
            time: self.lattice_time,
            _pad1: 0.0,
        };
        // Advance after building the params. The wrap period is the
        // common period of the shader's gust sinusoids (all exact
        // multiples of 2*pi/65536 per step), so wrapping is
        // phase-continuous, and 65536 is far below f32 precision loss.
        let time_now = self.lattice_time;
        self.lattice_time = (self.lattice_time + steps as f32) % 65536.0;
        self.total_steps += steps as f64;
        self.queue.write_buffer(&self.sim_uniform, 0, bytemuck::bytes_of(&params));

        let euler = self.settings.solver == SolverMode::Euler;
        if euler {
            let dt_e = self.euler_dt();
            // The velocity buffer only changes when steps run or a reset
            // rewrites it; keep the render normalization pinned to the
            // dt/mach it was written with.
            if steps > 0 || self.pending_reset {
                self.euler_render_ref = self.settings.mach * dt_e;
            }
            let free_u = if self.settings.wind_tunnel {
                [self.settings.mach, 0.0]
            } else {
                [0.0, 0.0]
            };
            let mut ep = EulerParamsRaw {
                width: w as u32,
                height: h as u32,
                gamma: 1.4,
                mach: self.settings.mach,
                dt: dt_e,
                blend: 0.0,
                sponge_width: (self.margin.min(96)) as f32,
                sponge_strength: self.settings.sponge_strength,
                free_u,
                time: time_now,
                write_render: 0.0,
            };
            self.queue
                .write_buffer(&self.euler_uniform_s1, 0, bytemuck::bytes_of(&ep));
            ep.blend = 0.5;
            ep.write_render = 1.0;
            self.queue
                .write_buffer(&self.euler_uniform_s2, 0, bytemuck::bytes_of(&ep));
        }

        self.frame_counter = self.frame_counter.wrapping_add(1);
        // Spawn tracers over the visible window plus an upstream band a
        // quarter of the visible width deep (bounded by the margin).
        let band = self.margin.min(self.vis_w / 4);
        let part_params = PartParamsRaw {
            width: w as u32,
            height: h as u32,
            count: self.settings.particle_count,
            frame: self.frame_counter,
            dt: steps as f32,
            _pad0: 0.0,
            spawn_min: [(self.margin - band) as u32, self.margin as u32],
            spawn_max: [
                (self.margin + self.vis_w) as u32,
                (self.margin + self.vis_h) as u32,
            ],
            _pad1: 0.0,
            _pad2: 0.0,
        };
        self.queue.write_buffer(&self.part_uniform, 0, bytemuck::bytes_of(&part_params));

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("flowpaint compute"),
            timestamp_writes: None,
        });

        if self.pending_reset {
            self.pending_reset = false;
            if euler {
                // The stage-1 bind groups' dst slots cover all three
                // state buffers across the rotations.
                pass.set_pipeline(&self.euler_reset_pipeline);
                for rot in 0..3 {
                    pass.set_bind_group(0, &self.bufs.euler_bind[rot][0], &[]);
                    self.dispatch_grid(&mut pass, w, h);
                }
                self.bufs.euler_rot = 0;
            } else {
                pass.set_pipeline(&self.reset_pipeline);
                for side in 0..2 {
                    pass.set_bind_group(0, &self.bufs.lbm_bind[side], &[]);
                    self.dispatch_grid(&mut pass, w, h);
                }
                self.bufs.f_side = 0;
            }
        }
        if self.pending_clear_dye {
            self.pending_clear_dye = false;
            pass.set_pipeline(&self.clear_dye_pipeline);
            for side in 0..2 {
                pass.set_bind_group(0, &self.bufs.dye_bind[side], &[]);
                self.dispatch_grid(&mut pass, w, h);
            }
            self.bufs.dye_side = 0;
        }

        if euler {
            pass.set_pipeline(&self.euler_step_pipeline);
            for _ in 0..steps {
                let rot = self.bufs.euler_rot;
                pass.set_bind_group(0, &self.bufs.euler_bind[rot][0], &[]);
                self.dispatch_grid(&mut pass, w, h);
                pass.set_bind_group(0, &self.bufs.euler_bind[rot][1], &[]);
                self.dispatch_grid(&mut pass, w, h);
                self.bufs.euler_rot = (rot + 1) % 3;
            }
        } else {
            pass.set_pipeline(&self.collide_pipeline);
            for _ in 0..steps {
                pass.set_bind_group(0, &self.bufs.lbm_bind[self.bufs.f_side], &[]);
                self.dispatch_grid(&mut pass, w, h);
                self.bufs.f_side ^= 1;
            }
        }

        // Dye advection once per frame (also while paused so painted smoke
        // sources appear immediately; dt is 0 then).
        pass.set_pipeline(&self.advect_pipeline);
        pass.set_bind_group(0, &self.bufs.dye_bind[self.bufs.dye_side], &[]);
        self.dispatch_grid(&mut pass, w, h);
        self.bufs.dye_side ^= 1;

        if self.settings.particle_count > 0 && !self.settings.paused {
            pass.set_pipeline(&self.part_update_pipeline);
            pass.set_bind_group(0, &self.bufs.part_bind, &[]);
            pass.dispatch_workgroups(self.settings.particle_count.div_ceil(256), 1, 1);
        }

        // Probe sampling encodes buffer copies, which cannot live inside
        // a compute pass.
        drop(pass);
        self.encode_probe_copies(encoder, steps);
    }

    /// Probe sampling (T2-B): drain any staging whose map completed,
    /// advance last frame's copies to a map request, and encode this
    /// frame's per-probe copies into a free staging. The round trip is
    /// copy (frame N) → map request (N+1) → read (N+2 or later); a busy
    /// GPU only stalls the ring, never the frame.
    fn encode_probe_copies(&mut self, encoder: &mut wgpu::CommandEncoder, steps: u32) {
        if self.probe_stages.iter().all(|s| matches!(s.state, ProbeStageState::Free))
            && (steps == 0 || self.settings.probes.probes.is_empty())
        {
            return;
        }
        // Pump map callbacks without waiting.
        let _ = self.device.poll(wgpu::Maintain::Poll);
        for st in &mut self.probe_stages {
            match &st.state {
                ProbeStageState::Mapping(slot) => {
                    let taken = slot.lock().unwrap().take();
                    match taken {
                        Some(Ok(())) => {
                            if st.generation == self.probe_generation {
                                let data = st.buf.slice(..).get_mapped_range();
                                let pr = &mut self.settings.probes;
                                for (slot, id) in st.ids.iter().enumerate() {
                                    let base = slot * PROBE_SLOT_BYTES as usize;
                                    let f = |off: usize| -> f32 {
                                        f32::from_le_bytes(
                                            data[base + off..base + off + 4]
                                                .try_into()
                                                .unwrap(),
                                        )
                                    };
                                    // Same stencil as render.wgsl's vorticity
                                    // view (slot layout: see PROBE_SLOT_BYTES).
                                    let curl =
                                        0.5 * ((f(12) - f(20)) - (f(24) - f(32)));
                                    let sample = ProbeSample {
                                        steps: st.stamp,
                                        vel: [f(0), f(4)],
                                        curl,
                                        drho: f(40) - 1.0,
                                        dye: 0.2126 * f(48)
                                            + 0.7152 * f(52)
                                            + 0.0722 * f(56),
                                    };
                                    if let Some(p) =
                                        pr.probes.iter_mut().find(|p| p.id == *id)
                                    {
                                        p.samples.push_back(sample);
                                        if p.samples.len() > PROBE_HISTORY_CAP {
                                            p.samples.pop_front();
                                        }
                                    }
                                }
                                drop(data);
                            }
                            st.buf.unmap();
                            st.state = ProbeStageState::Free;
                        }
                        Some(Err(_)) => {
                            st.state = ProbeStageState::Free;
                        }
                        None => {}
                    }
                }
                ProbeStageState::Copied => {
                    // The copies encoded last frame are submitted by now;
                    // the map may be requested.
                    let slot = Arc::new(Mutex::new(None));
                    let cb = Arc::clone(&slot);
                    st.buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                        *cb.lock().unwrap() = Some(r);
                    });
                    st.state = ProbeStageState::Mapping(slot);
                }
                ProbeStageState::Free => {}
            }
        }
        if steps == 0 {
            return; // paused: time does not advance, so no new sample
        }
        if self.settings.probes.probes.is_empty() {
            return;
        }
        let Some(stage_idx) = self
            .probe_stages
            .iter()
            .position(|s| matches!(s.state, ProbeStageState::Free))
        else {
            return; // ring full; skip this frame's sample
        };
        let w = self.geo.w as u64;
        let dye_buf =
            if self.bufs.dye_side == 0 { &self.bufs.dye_a } else { &self.bufs.dye_b };
        let mut ids = Vec::new();
        let pr = &self.settings.probes;
        for (slot, p) in pr.probes.iter().take(MAX_PROBES).enumerate() {
            // Clamp one cell in from the visible edge so the curl
            // stencil stays in bounds even with no margin.
            let x = (p.pos[0].floor() as i64).clamp(1, self.vis_w as i64 - 2)
                + self.margin as i64;
            let y = (p.pos[1].floor() as i64).clamp(1, self.vis_h as i64 - 2)
                + self.margin as i64;
            let c = y as u64 * w + x as u64;
            let st = &self.probe_stages[stage_idx];
            let dst = slot as u64 * PROBE_SLOT_BYTES;
            let vel = &self.bufs.vel;
            encoder.copy_buffer_to_buffer(vel, c * 8, &st.buf, dst, 8);
            encoder.copy_buffer_to_buffer(vel, (c + 1) * 8, &st.buf, dst + 8, 8);
            encoder.copy_buffer_to_buffer(vel, (c - 1) * 8, &st.buf, dst + 16, 8);
            encoder.copy_buffer_to_buffer(vel, (c + w) * 8, &st.buf, dst + 24, 8);
            encoder.copy_buffer_to_buffer(vel, (c - w) * 8, &st.buf, dst + 32, 8);
            encoder.copy_buffer_to_buffer(&self.bufs.rho, c * 4, &st.buf, dst + 40, 4);
            encoder.copy_buffer_to_buffer(dye_buf, c * 16, &st.buf, dst + 48, 16);
            ids.push(p.id);
        }
        let st = &mut self.probe_stages[stage_idx];
        st.ids = ids;
        st.stamp = self.total_steps as f32;
        st.generation = self.probe_generation;
        st.state = ProbeStageState::Copied;
    }

    /// Write the render uniform for the current viewport mapping.
    pub fn write_render_uniform(&self) {
        let p = RenderParamsRaw {
            width: self.geo.w as u32,
            height: self.geo.h as u32,
            mode: self.settings.render_mode as u32,
            flags: self.render_flags(),
            vp_origin: self.mapping.vp_origin,
            vp_size: self.mapping.vp_size,
            lb_origin: self.mapping.lb_origin,
            px_per_cell: self.mapping.px_per_cell,
            inlet_speed: self.render_inlet_speed(),
            vis_origin: [self.margin as u32, self.margin as u32],
            vis_size: [self.vis_w as u32, self.vis_h as u32],
            display_gain: self.range_display_gain(),
            smoke_gain: self.settings.smoke_gain,
            particle_size: self.settings.particle_size,
            particle_brightness: self.settings.particle_brightness,
        };
        self.queue.write_buffer(&self.render_uniform, 0, bytemuck::bytes_of(&p));
    }

    /// Draw the field (and particles) into the current render pass. The
    /// pass's viewport is the canvas rect.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'static>) {
        pass.set_pipeline(&self.field_pipeline);
        pass.set_bind_group(0, &self.bufs.render_bind[self.bufs.dye_side], &[]);
        pass.draw(0..3, 0..1);

        if self.settings.particle_count > 0 {
            pass.set_pipeline(&self.particle_pipeline);
            pass.set_bind_group(0, &self.bufs.render_bind[self.bufs.dye_side], &[]);
            pass.set_bind_group(1, &self.particle_bind_group1, &[]);
            pass.draw(0..self.settings.particle_count * 6, 0..1);
        }
    }

    // --- PNG export ---------------------------------------------------

    /// Render the visible window at 1 px per cell into an offscreen
    /// texture and save it as a PNG. Blocks until the readback completes.
    pub fn export_png(&self, path: &std::path::Path) -> Result<(), String> {
        let (w, h) = (self.vis_w as u32, self.vis_h as u32);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("export"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Temporarily aim the render uniform at the full offscreen target.
        let p = RenderParamsRaw {
            width: self.geo.w as u32,
            height: self.geo.h as u32,
            mode: self.settings.render_mode as u32,
            flags: self.render_flags(),
            vp_origin: [0.0, 0.0],
            vp_size: [w as f32, h as f32],
            lb_origin: [0.0, 0.0],
            px_per_cell: 1.0,
            inlet_speed: self.render_inlet_speed(),
            vis_origin: [self.margin as u32, self.margin as u32],
            vis_size: [self.vis_w as u32, self.vis_h as u32],
            // The same effective gain as the live view, so an exported
            // PNG of a locked range matches the screen.
            display_gain: self.range_display_gain(),
            smoke_gain: self.settings.smoke_gain,
            particle_size: self.settings.particle_size,
            particle_brightness: self.settings.particle_brightness,
        };
        self.queue.write_buffer(&self.render_uniform, 0, bytemuck::bytes_of(&p));

        let bytes_per_row = (w * 4 + 255) / 256 * 256;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("export readback"),
            size: (bytes_per_row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder =
            self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("export"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("export pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.field_pipeline_rgba);
            pass.set_bind_group(0, &self.bufs.render_bind[self.bufs.dye_side], &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("map failed: {e:?}"))?;

        let data = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let start = (y * bytes_per_row) as usize;
            pixels.extend_from_slice(&data[start..start + (w * 4) as usize]);
        }
        drop(data);
        readback.unmap();

        let img: image::RgbaImage =
            image::ImageBuffer::from_raw(w, h, pixels).ok_or("bad image buffer")?;
        img.save(path).map_err(|e| e.to_string())?;
        Ok(())
    }

    // --- Scene files --------------------------------------------------



    /// Rough Reynolds number using a cylinder-preset-sized obstacle.
    pub fn reynolds_estimate(&self) -> u32 {
        let l = 0.16 * self.vis_h as f32;
        (self.settings.flow_speed * l / self.settings.viscosity.max(1e-5)) as u32
    }
}
