//! GPU simulation engine: owns the wgpu resources, the CPU-side geometry
//! document and the undo stack. Lives inside egui-wgpu's
//! `CallbackResources`; the app mutates it during `update`, and the paint
//! callback encodes the compute + render work each frame.

use crate::geometry::{Geometry, UndoStack};
use std::sync::{mpsc, Arc};

pub const RESOLUTIONS: [(&str, usize, usize); 4] = [
    ("Low (960 x 480)", 960, 480),
    ("Medium (1440 x 720)", 1440, 720),
    ("High (1920 x 960)", 1920, 960),
    ("Ultra (2560 x 1280)", 2560, 1280),
];

pub const PARTICLE_CHOICES: [(&str, u32); 5] = [
    ("Off", 0),
    ("100 k", 100_000),
    ("500 k", 500_000),
    ("1 M", 1_000_000),
    ("2 M", 2_000_000),
];
pub const MAX_PARTICLES: u64 = 2_000_000;

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

/// Simulation settings mirrored by the UI.
pub struct Settings {
    pub paused: bool,
    pub wind_tunnel: bool,
    pub flow_speed: f32,   // lattice inlet speed
    pub viscosity: f32,    // lattice kinematic viscosity
    pub steps_per_frame: u32,
    pub dye_fade: f32,     // per-frame retention
    pub render_mode: RenderMode,
    pub particle_count: u32,
    pub boundary_tints: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            paused: false,
            wind_tunnel: true,
            flow_speed: 0.09,
            viscosity: 0.015,
            steps_per_frame: 8,
            dye_fade: 0.995,
            render_mode: RenderMode::Dye,
            particle_count: 500_000,
            boundary_tints: true,
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
    _pad0: f32,
    _pad1: f32,
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
    // Render bind groups keyed by which dye buffer is current.
    render_bind: [wgpu::BindGroup; 2],
    /// Which f/dye buffer holds the current state (0 = A, 1 = B).
    f_side: usize,
    dye_side: usize,
}

pub struct GpuSim {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,

    lbm_layout: wgpu::BindGroupLayout,
    dye_layout: wgpu::BindGroupLayout,
    part_layout: wgpu::BindGroupLayout,
    render_layout: wgpu::BindGroupLayout,

    collide_pipeline: wgpu::ComputePipeline,
    reset_pipeline: wgpu::ComputePipeline,
    advect_pipeline: wgpu::ComputePipeline,
    clear_dye_pipeline: wgpu::ComputePipeline,
    part_update_pipeline: wgpu::ComputePipeline,
    field_pipeline: wgpu::RenderPipeline,
    particle_pipeline: wgpu::RenderPipeline,
    field_pipeline_rgba: wgpu::RenderPipeline, // for PNG export

    sim_uniform: wgpu::Buffer,
    part_uniform: wgpu::Buffer,
    render_uniform: wgpu::Buffer,
    particles: wgpu::Buffer,
    particle_bind_group1: wgpu::BindGroup,

    bufs: GridBuffers,

    pub geo: Geometry,
    pub undo: UndoStack,
    pub settings: Settings,
    pub mapping: ViewportMapping,

    frame_counter: u32,
    pending_reset: bool,
    pending_clear_dye: bool,
    /// Steps actually encoded last frame (for stats/particle dt).
    pub steps_last_frame: u32,
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
        let (_, w, h) = RESOLUTIONS[res_index];

        let lbm_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lbm"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lbm.wgsl").into()),
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
            &dye_layout,
            &part_layout,
            &render_layout,
            &sim_uniform,
            &part_uniform,
            &render_uniform,
            &particles,
        );

        let mut sim = Self {
            device,
            queue,
            lbm_layout,
            dye_layout,
            part_layout,
            render_layout,
            collide_pipeline,
            reset_pipeline,
            advect_pipeline,
            clear_dye_pipeline,
            part_update_pipeline,
            field_pipeline,
            particle_pipeline,
            field_pipeline_rgba,
            sim_uniform,
            part_uniform,
            render_uniform,
            particles,
            particle_bind_group1,
            bufs,
            geo,
            undo: UndoStack::default(),
            settings: Settings::default(),
            mapping: ViewportMapping::default(),
            frame_counter: 0,
            pending_reset: true,
            pending_clear_dye: true,
            steps_last_frame: 0,
        };
        sim.geo.apply_wind_tunnel(true);
        sim.geo.stamp_preset(crate::geometry::Preset::Cylinder);
        sim
    }

    #[allow(clippy::too_many_arguments)]
    fn create_grid_buffers(
        device: &wgpu::Device,
        w: usize,
        h: usize,
        lbm_layout: &wgpu::BindGroupLayout,
        dye_layout: &wgpu::BindGroupLayout,
        part_layout: &wgpu::BindGroupLayout,
        render_layout: &wgpu::BindGroupLayout,
        sim_uniform: &wgpu::Buffer,
        part_uniform: &wgpu::Buffer,
        render_uniform: &wgpu::Buffer,
        particles: &wgpu::Buffer,
    ) -> GridBuffers {
        let n = (w * h) as u64;
        let mk = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let f_a = mk("f a", n * 9 * 4);
        let f_b = mk("f b", n * 9 * 4);
        let vel = mk("velocity", n * 8);
        let rho = mk("density", n * 4);
        let cell = mk("cell type", n * 4);
        let fan = mk("fan dir", n * 8);
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
            render_bind,
            f_side: 0,
            dye_side: 0,
        }
    }

    // --- Public control ----------------------------------------------

    pub fn grid_size(&self) -> (usize, usize) {
        (self.geo.w, self.geo.h)
    }

    /// Queue a full flow reset (populations to rest, dye cleared).
    pub fn reset_flow(&mut self) {
        self.pending_reset = true;
        self.pending_clear_dye = true;
    }

    pub fn clear_all(&mut self) {
        self.geo.clear();
        if self.settings.wind_tunnel {
            self.geo.apply_wind_tunnel(true);
        }
        self.undo.clear();
        self.reset_flow();
    }

    pub fn set_wind_tunnel(&mut self, on: bool) {
        self.settings.wind_tunnel = on;
        self.geo.apply_wind_tunnel(on);
    }

    pub fn apply_preset(&mut self, preset: crate::geometry::Preset) {
        self.geo.clear();
        self.settings.wind_tunnel = true;
        self.geo.apply_wind_tunnel(true);
        self.geo.stamp_preset(preset);
        self.undo.clear();
        self.reset_flow();
    }

    /// Switch grid resolution, resampling the current scene into it.
    pub fn set_resolution(&mut self, res_index: usize) {
        let (_, w, h) = RESOLUTIONS[res_index];
        if (w, h) == (self.geo.w, self.geo.h) {
            return;
        }
        // Strip tunnel edge cells first so they aren't smeared into ghost
        // inlet/outlet columns by the resample; they are re-applied on the
        // new grid below.
        if self.settings.wind_tunnel {
            self.geo.apply_wind_tunnel(false);
        }
        let mut new_geo = Geometry::new(w, h);
        new_geo.resample_from(&self.geo);
        if self.settings.wind_tunnel {
            new_geo.apply_wind_tunnel(true);
        }
        self.geo = new_geo;
        self.bufs = Self::create_grid_buffers(
            &self.device,
            w,
            h,
            &self.lbm_layout,
            &self.dye_layout,
            &self.part_layout,
            &self.render_layout,
            &self.sim_uniform,
            &self.part_uniform,
            &self.render_uniform,
            &self.particles,
        );
        self.undo.clear();
        self.clear_particles();
        self.reset_flow();
    }

    /// Zero the particle buffer so every slot respawns (positions are in
    /// grid cells and go stale when the grid changes).
    fn clear_particles(&self) {
        self.queue
            .write_buffer(&self.particles, 0, &vec![0u8; (MAX_PARTICLES * 16) as usize]);
    }

    pub fn undo_action(&mut self) {
        if let Some(e) = self.undo.pop_undo() {
            self.geo.restore(&e.before);
            self.reassert_tunnel();
            self.undo.push_redo(e);
        }
    }

    pub fn redo_action(&mut self) {
        if let Some(e) = self.undo.pop_redo() {
            self.geo.restore(&e.after);
            self.reassert_tunnel();
            self.undo.push_undo_back(e);
        }
    }

    /// Stroke snapshots can capture tunnel edge cells; after restoring one,
    /// re-assert the tunnel so the boundary matches the toggle.
    fn reassert_tunnel(&mut self) {
        if self.settings.wind_tunnel {
            self.geo.apply_wind_tunnel(true);
        }
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
            let off_fan = (a * 8) as u64;
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
        let steps = if self.settings.paused { 0 } else { self.settings.steps_per_frame.max(1) };
        self.steps_last_frame = steps;

        let params = SimParamsRaw {
            width: w as u32,
            height: h as u32,
            omega: 1.0 / (3.0 * self.settings.viscosity.max(0.004) + 0.5),
            inlet_speed: self.settings.flow_speed,
            dye_dt: steps as f32,
            dye_decay: if self.settings.paused { 1.0 } else { self.settings.dye_fade },
            _pad0: 0.0,
            _pad1: 0.0,
        };
        self.queue.write_buffer(&self.sim_uniform, 0, bytemuck::bytes_of(&params));

        self.frame_counter = self.frame_counter.wrapping_add(1);
        let part_params = PartParamsRaw {
            width: w as u32,
            height: h as u32,
            count: self.settings.particle_count,
            frame: self.frame_counter,
            dt: steps as f32,
            _pad0: 0.0,
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
            pass.set_pipeline(&self.reset_pipeline);
            for side in 0..2 {
                pass.set_bind_group(0, &self.bufs.lbm_bind[side], &[]);
                self.dispatch_grid(&mut pass, w, h);
            }
            self.bufs.f_side = 0;
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

        pass.set_pipeline(&self.collide_pipeline);
        for _ in 0..steps {
            pass.set_bind_group(0, &self.bufs.lbm_bind[self.bufs.f_side], &[]);
            self.dispatch_grid(&mut pass, w, h);
            self.bufs.f_side ^= 1;
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
    }

    /// Write the render uniform for the current viewport mapping.
    pub fn write_render_uniform(&self) {
        let p = RenderParamsRaw {
            width: self.geo.w as u32,
            height: self.geo.h as u32,
            mode: self.settings.render_mode as u32,
            flags: if self.settings.boundary_tints { 1 } else { 0 },
            vp_origin: self.mapping.vp_origin,
            vp_size: self.mapping.vp_size,
            lb_origin: self.mapping.lb_origin,
            px_per_cell: self.mapping.px_per_cell,
            inlet_speed: self.settings.flow_speed,
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

    /// Render the field at 1 px per cell into an offscreen texture and
    /// save it as a PNG. Blocks until the readback completes.
    pub fn export_png(&self, path: &std::path::Path) -> Result<(), String> {
        let (w, h) = (self.geo.w as u32, self.geo.h as u32);
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
            flags: if self.settings.boundary_tints { 1 } else { 0 },
            vp_origin: [0.0, 0.0],
            vp_size: [w as f32, h as f32],
            lb_origin: [0.0, 0.0],
            px_per_cell: 1.0,
            inlet_speed: self.settings.flow_speed,
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

    pub fn save_scene(&self, path: &std::path::Path) -> Result<(), String> {
        let scene = crate::geometry::SceneFile {
            version: crate::geometry::SCENE_VERSION,
            w: self.geo.w as u32,
            h: self.geo.h as u32,
            cell: self.geo.cell.clone(),
            fan: self.geo.fan.clone(),
            dye_src: self.geo.dye_src.clone(),
            wind_tunnel: self.settings.wind_tunnel,
            flow_speed: self.settings.flow_speed,
            viscosity: self.settings.viscosity,
        };
        let bytes = bincode::serialize(&scene).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }

    pub fn load_scene(&mut self, path: &std::path::Path) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let scene: crate::geometry::SceneFile =
            bincode::deserialize(&bytes).map_err(|e| e.to_string())?;
        if scene.version != crate::geometry::SCENE_VERSION {
            return Err(format!("unsupported scene version {}", scene.version));
        }
        let (sw, sh) = (scene.w as usize, scene.h as usize);
        if sw == 0
            || sh == 0
            || sw * sh != scene.cell.len()
            || scene.fan.len() != scene.cell.len()
            || scene.dye_src.len() != scene.cell.len()
        {
            return Err("corrupt scene file".into());
        }
        let mut loaded = Geometry {
            w: sw,
            h: sh,
            cell: scene.cell,
            fan: scene.fan,
            dye_src: scene.dye_src,
            dirty: None,
        };
        // Strip the saved tunnel edges before resampling so they aren't
        // smeared into ghost columns; the tunnel is re-applied below.
        if scene.wind_tunnel {
            loaded.apply_wind_tunnel(false);
        }
        // Resample into the current grid resolution.
        let mut geo = Geometry::new(self.geo.w, self.geo.h);
        geo.resample_from(&loaded);
        self.geo = geo;
        self.settings.wind_tunnel = scene.wind_tunnel;
        self.settings.flow_speed = scene.flow_speed;
        self.settings.viscosity = scene.viscosity;
        if self.settings.wind_tunnel {
            self.geo.apply_wind_tunnel(true);
        }
        self.undo.clear();
        self.clear_particles();
        self.reset_flow();
        Ok(())
    }

    /// Rough Reynolds number using a cylinder-preset-sized obstacle.
    pub fn reynolds_estimate(&self) -> u32 {
        let l = 0.16 * self.geo.h as f32;
        (self.settings.flow_speed * l / self.settings.viscosity.max(1e-5)) as u32
    }
}
