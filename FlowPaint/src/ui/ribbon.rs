//! The ribbon: a tab strip over a fixed-height body of task-grouped
//! controls (phase 3b — the pre-ribbon control column, redistributed).
//! Every button is a phosphor icon over a text label; value controls
//! use compact label + widget rows, mockup-style.

use crate::app::{build_preset, Cmd, FlowPaintApp, RibbonTab, ScenePreset, Tool, UiSnapshot, ViewRequest, FLUID_PRESETS};
use crate::sim::{RangeMode, RenderMode, SolverMode, PARTICLE_CHOICES};
use eframe::egui;

use super::units::{fmt_density, fmt_len, fmt_speed, fmt_time};
use egui_phosphor::regular as ph;

use super::theme;

impl FlowPaintApp {
    pub(in crate::app) fn ribbon(
        &mut self,
        ctx: &egui::Context,
        snap: UiSnapshot,
        cmds: &mut Vec<Cmd>,
    ) {
        egui::TopBottomPanel::top("ribbon_tabs")
            .exact_height(theme::dim::RIBBON_TABS_HEIGHT)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in RibbonTab::ALL {
                        if theme::toggle(ui, self.ribbon_tab == tab, label).clicked() {
                            self.ribbon_tab = tab;
                        }
                    }
                });
            });

        egui::TopBottomPanel::top("ribbon_body")
            .exact_height(theme::dim::RIBBON_HEIGHT)
            .show(ctx, |ui| {
                // Compact metrics inside the ribbon body only.
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.spacing_mut().slider_width = 70.0;
                ui.horizontal(|ui| match self.ribbon_tab {
                    RibbonTab::Home => self.ribbon_home(ui, snap, cmds),
                    RibbonTab::Geometry => self.ribbon_geometry(ui),
                    RibbonTab::Physics => self.ribbon_physics(ui, snap, cmds),
                    RibbonTab::Study => self.ribbon_study(ui, cmds),
                    RibbonTab::Results => self.ribbon_results(ui, snap, cmds),
                });
            });
    }

    // --- Tabs ---------------------------------------------------------

    fn ribbon_home(&mut self, ui: &mut egui::Ui, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        group(ui, "Run", |ui| {
            let (icon, label) = if snap.paused {
                (ph::PLAY, "Resume")
            } else {
                (ph::PAUSE, "Pause")
            };
            if theme::ribbon_button(ui, false, false, icon, label).clicked() {
                cmds.push(Cmd::TogglePause);
            }
            if theme::ribbon_button(ui, false, false, ph::SKIP_FORWARD, "Step")
                .on_hover_text("Advance the simulation by one frame.")
                .clicked()
            {
                cmds.push(Cmd::StepOnce);
            }
        });
        group(ui, "Scene", |ui| {
            if theme::ribbon_button(ui, false, false, ph::ARROW_COUNTER_CLOCKWISE, "Reset flow")
                .on_hover_text("Restart the flow from the freestream; the sketch stays")
                .clicked()
            {
                cmds.push(Cmd::ResetFlow);
            }
            if theme::ribbon_button(ui, false, true, ph::TRASH, "Clear all")
                .on_hover_text("Remove every object (undoable) and reset the flow")
                .clicked()
            {
                self.finish_gesture();
                self.deselect_all();
                self.model.replace_all(Vec::new());
                cmds.push(Cmd::ResetFlow);
            }
        });
        group(ui, "History", |ui| {
            ui.add_enabled_ui(self.model.can_undo(), |ui| {
                if theme::ribbon_button(ui, false, false, ph::ARROW_U_UP_LEFT, "Undo").clicked() {
                    self.finish_gesture();
                    self.model.undo();
                    self.deselect_all();
                }
            });
            ui.add_enabled_ui(self.model.can_redo(), |ui| {
                if theme::ribbon_button(ui, false, false, ph::ARROW_U_UP_RIGHT, "Redo").clicked() {
                    self.finish_gesture();
                    self.model.redo();
                    self.deselect_all();
                }
            });
        });
    }

    fn ribbon_geometry(&mut self, ui: &mut egui::Ui) {
        group(ui, "Sketch tools", |ui| {
            for (tool, label, key) in Tool::ALL {
                let icon = match tool {
                    Tool::Select => ph::CURSOR,
                    Tool::Line => ph::LINE_SEGMENT,
                    Tool::Rect => ph::RECTANGLE,
                    Tool::Ellipse => ph::CIRCLE,
                    Tool::Polyline => ph::POLYGON,
                    Tool::Pencil => ph::PENCIL_SIMPLE,
                };
                let on = self.tool == tool;
                if theme::ribbon_button(ui, on, false, icon, label)
                    .on_hover_text(format!("Key: {key}"))
                    .clicked()
                    && !on
                {
                    self.finish_gesture();
                    self.tool = tool;
                }
            }
        });
        group(ui, "Sketch aids", |ui| {
            ui.vertical(|ui| {
                row(ui, "angle snap", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.snap_angle_deg)
                            .range(1.0..=90.0)
                            .speed(0.5)
                            .suffix("°"),
                    )
                    .on_hover_text("Hold Shift while you draw to snap the angle");
                    for a in [15.0f32, 30.0, 45.0, 90.0] {
                        if ui.small_button(format!("{a}°")).clicked() {
                            self.snap_angle_deg = a;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.snap_enabled, "Snap to grid");
                    if self.snap_enabled {
                        let spacing_m =
                            fmt_len(self.phys_cache.len_m(self.snap_spacing));
                        ui.add(
                            egui::Slider::new(&mut self.snap_spacing, 2.0..=50.0),
                        )
                        .on_hover_text(format!("Grid spacing: {spacing_m}"));
                    }
                });
            });
        });
    }

    fn ribbon_physics(&mut self, ui: &mut egui::Ui, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        group(ui, "Solver", |ui| {
            for (m, icon, label, tip) in [
                (
                    SolverMode::Lbm,
                    ph::WAVES,
                    "Incompressible",
                    "Lattice-Boltzmann: viscous, low Mach — smoke, wakes, \
                     vortex streets",
                ),
                (
                    SolverMode::Euler,
                    ph::WAVE_SAWTOOTH,
                    "Compressible",
                    "Finite-volume Euler: real gas dynamics — shocks, \
                     expansion fans, choked nozzles (inviscid)",
                ),
            ] {
                if theme::ribbon_button(ui, snap.solver == m, false, icon, label)
                    .on_hover_text(tip)
                    .clicked()
                    && snap.solver != m
                {
                    cmds.push(Cmd::SetSolver(m));
                }
            }
        });
        group(ui, "Fluid", |ui| {
            ui.vertical(|ui| {
                ui.set_width(150.0);
                let combo_label = match self.fluid_preset_idx {
                    Some(i) => FLUID_PRESETS[i].name,
                    None => "Custom",
                };
                egui::ComboBox::from_id_salt("ribbon_fluid")
                    .width(146.0)
                    .selected_text(combo_label)
                    .show_ui(ui, |ui| {
                        for (i, p) in FLUID_PRESETS.iter().enumerate() {
                            let sel = self.fluid_preset_idx == Some(i);
                            if theme::toggle(ui, sel, p.name)
                                .on_hover_text(p.desc)
                                .clicked()
                            {
                                self.fluid_preset_idx = Some(i);
                                self.fluid_name = p.name;
                                self.fluid_nu = p.nu;
                                self.fluid_rho = p.rho;
                                self.fluid_a = p.a;
                                if p.tunnel != snap.tunnel {
                                    cmds.push(Cmd::SetWindTunnel(p.tunnel));
                                }
                                cmds.push(Cmd::SetFlowSpeed(p.flow));
                                cmds.push(Cmd::SetViscosity(p.visc));
                                // Presets own the sub-step count too, so
                                // e.g. Supersonic's 16 steps don't leak
                                // into the next regime.
                                cmds.push(Cmd::SetSteps(p.steps.unwrap_or(8)));
                                self.status = format!("Fluid preset: {}", p.name);
                            }
                        }
                    });
                theme::derived(ui, format!("a∞ = {}", fmt_speed(self.fluid_a)));
                theme::derived(ui, format!("ρ  = {}", fmt_density(self.fluid_rho)));
            });
        });
        group(ui, "Inlet condition", |ui| {
            ui.vertical(|ui| {
                // Fixed width so the LBM/Euler swap can't change the
                // group's footprint.
                ui.set_width(180.0);
                if snap.solver == SolverMode::Euler {
                    let mut mach = snap.mach;
                    row(ui, "inlet Mach", |ui| {
                        if ui
                            .add(
                                egui::DragValue::new(&mut mach)
                                    .range(0.3..=3.0)
                                    .speed(0.01)
                                    .fixed_decimals(3)
                                    .suffix(" M"),
                            )
                            .changed()
                        {
                            cmds.push(Cmd::SetMach(mach));
                        }
                    });
                    theme::derived(
                        ui,
                        format!("u = M · a = {}", fmt_speed(mach * self.fluid_a)),
                    );
                    theme::derived(ui, format!("(a∞ = {})", fmt_speed(self.fluid_a)));
                    theme::derived(ui, "Re = ∞ (inviscid)".to_string());
                } else {
                    let ps = self.phys_cache;
                    let mut flow = snap.flow;
                    let mut visc = snap.visc;
                    row(ui, "flow speed", |ui| {
                        if ui
                            .add(
                                egui::DragValue::new(&mut flow)
                                    .range(0.02..=0.14)
                                    .speed(0.001)
                                    .fixed_decimals(3),
                            )
                            .on_hover_text("Inlet speed, lattice units")
                            .changed()
                        {
                            self.fluid_preset_idx = None;
                            cmds.push(Cmd::SetFlowSpeed(flow));
                        }
                    });
                    theme::derived(ui, format!("= {}", fmt_speed(ps.u_phys(flow))));
                    // T2-B: viscosity in either direction — set ν and
                    // read Re, or set Re and let ν follow. The label
                    // cell holds the direction toggle; the dependent
                    // value moves to the derived line below.
                    let re_id = ui.id().with("re_input_mode");
                    let mut re_mode: bool = ui
                        .data_mut(|d| *d.get_persisted_mut_or_insert_with(re_id, || false));
                    let l_ref = 0.16 * self.stats_grid.1.max(1) as f32;
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(66.0, 18.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if theme::toggle(ui, re_mode, "Re")
                                    .on_hover_text(
                                        "Set the Reynolds number; viscosity follows",
                                    )
                                    .clicked()
                                {
                                    re_mode = true;
                                }
                                if theme::toggle(ui, !re_mode, "ν")
                                    .on_hover_text("Set the viscosity; Reynolds follows")
                                    .clicked()
                                {
                                    re_mode = false;
                                }
                            },
                        );
                        if re_mode {
                            let mut re = flow * l_ref / visc.max(1e-5);
                            let (re_min, re_max) =
                                (flow * l_ref / 0.08, flow * l_ref / 0.005);
                            let drag_speed = (re * 0.01).max(1.0);
                            if ui
                                .add(
                                    egui::DragValue::new(&mut re)
                                        .range(re_min..=re_max)
                                        .speed(drag_speed)
                                        .fixed_decimals(0),
                                )
                                .on_hover_text(
                                    "Reynolds number at the reference length \
                                     (0.16 × domain height)",
                                )
                                .changed()
                            {
                                let v =
                                    (flow * l_ref / re.max(1.0)).clamp(0.005, 0.08);
                                self.fluid_preset_idx = None;
                                cmds.push(Cmd::SetViscosity(v));
                            }
                        } else if ui
                            .add(
                                egui::DragValue::new(&mut visc)
                                    .range(0.005..=0.08)
                                    .speed(0.0005),
                            )
                            .on_hover_text("Lattice kinematic viscosity")
                            .changed()
                        {
                            self.fluid_preset_idx = None;
                            cmds.push(Cmd::SetViscosity(visc));
                        }
                    });
                    ui.data_mut(|d| d.insert_persisted(re_id, re_mode));
                    theme::derived(
                        ui,
                        if re_mode {
                            format!("ν = {:.4} · Δt = {}", visc, fmt_time(ps.dt))
                        } else {
                            format!("Re ≈ {} · Δt = {}", self.stats_re, fmt_time(ps.dt))
                        },
                    );
                }
            });
        });
        group(ui, "Integration", |ui| {
            ui.vertical(|ui| {
                let mut steps = snap.steps;
                let mut fade = snap.fade;
                let mut tunnel = snap.tunnel;
                row(ui, "steps / frame", |ui| {
                    if ui
                        .add(egui::DragValue::new(&mut steps).range(1..=32))
                        .changed()
                    {
                        self.fluid_preset_idx = None;
                        cmds.push(Cmd::SetSteps(steps));
                    }
                });
                row(ui, "persistence", |ui| {
                    if ui
                        .add(egui::Slider::new(&mut fade, 0.985..=1.0))
                        .on_hover_text("Smoke retention per frame")
                        .changed()
                    {
                        cmds.push(Cmd::SetDyeFade(fade));
                    }
                });
                if ui
                    .checkbox(&mut tunnel, "Wind tunnel")
                    .on_hover_text("Freestream inflow, left to right")
                    .changed()
                {
                    self.fluid_preset_idx = None;
                    cmds.push(Cmd::SetWindTunnel(tunnel));
                }
            });
        });
        group(ui, "Domain", |ui| {
            ui.vertical(|ui| {
                row(ui, "width", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.domain_width_m)
                            .range(0.05..=100.0)
                            .speed(0.01)
                            .suffix(" m"),
                    )
                    .on_hover_text(
                        "Physical size the canvas represents; anchors every \
                         unit readout (cell size, time step, speeds, pressures)",
                    );
                });
                theme::derived(ui, format!("cell = {}", fmt_len(self.phys_cache.dx)));
            });
        });
    }

    fn ribbon_study(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        group(ui, "Generators", |ui| {
            if theme::ribbon_button(ui, self.show_airfoil_gen, false, ph::AIRPLANE_TILT, "Airfoil…")
                .clicked()
            {
                self.show_airfoil_gen = !self.show_airfoil_gen;
            }
            if theme::ribbon_button(ui, self.show_nozzle_gen, false, ph::ROCKET_LAUNCH, "Nozzle…")
                .clicked()
            {
                self.show_nozzle_gen = !self.show_nozzle_gen;
            }
        });
        group(ui, "Scene presets", |ui| {
            for (p, short, desc) in ScenePreset::ALL {
                let icon = match p {
                    ScenePreset::Cylinder => ph::CIRCLE,
                    ScenePreset::Airfoil => ph::AIRPLANE,
                    ScenePreset::Venturi => ph::FUNNEL,
                    ScenePreset::Step => ph::STEPS,
                    ScenePreset::Pinball => ph::CIRCLES_THREE,
                };
                if theme::ribbon_button(ui, false, false, icon, short)
                    .on_hover_text(format!("{desc} — replaces the scene"))
                    .clicked()
                {
                    self.finish_gesture();
                    self.deselect_all();
                    let (vw, vh) = self.stats_grid;
                    let objs = build_preset(p, &mut self.model, vw, vh);
                    self.model.replace_all(objs);
                    cmds.push(Cmd::ResetFlow);
                    self.status = format!("Scene preset: {short} (editable objects)");
                }
            }
        });
    }

    fn ribbon_results(&mut self, ui: &mut egui::Ui, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        group(ui, "Field", |ui| {
            for m in RenderMode::ALL {
                let icon = match m {
                    RenderMode::Dye => ph::CLOUD,
                    RenderMode::Speed => ph::GAUGE,
                    RenderMode::Vorticity => ph::TORNADO,
                    RenderMode::Pressure => ph::CIRCLE_HALF,
                };
                if theme::ribbon_button(ui, snap.mode == m, false, icon, m.label()).clicked() {
                    cmds.push(Cmd::SetRenderMode(m));
                }
            }
        });
        group(ui, "Display", |ui| {
            ui.vertical(|ui| {
                let mut tints = snap.tints;
                ui.checkbox(&mut self.show_legend, "Show legend");
                if ui.checkbox(&mut tints, "Highlight fans & drains").changed() {
                    cmds.push(Cmd::SetBoundaryTints(tints));
                }
            });
        });
        group(ui, "Particles", |ui| {
            ui.vertical(|ui| {
                if ui
                    .checkbox(&mut self.particles_on, "Particles")
                    .on_hover_text("Show tracer particles.")
                    .changed()
                {
                    let count = if self.particles_on {
                        PARTICLE_CHOICES[self.particle_index].1
                    } else {
                        0
                    };
                    cmds.push(Cmd::SetParticles(count));
                }
                // The count keeps its last value while unchecked, so
                // rechecking restores it.
                ui.add_enabled_ui(self.particles_on, |ui| {
                    row(ui, "count", |ui| {
                        egui::ComboBox::from_id_salt("ribbon_particles")
                            .width(80.0)
                            .selected_text(PARTICLE_CHOICES[self.particle_index].0)
                            .show_ui(ui, |ui| {
                                for (i, (label, count)) in
                                    PARTICLE_CHOICES.iter().enumerate()
                                {
                                    if theme::toggle(ui, i == self.particle_index, *label)
                                        .clicked()
                                    {
                                        self.particle_index = i;
                                        cmds.push(Cmd::SetParticles(*count));
                                    }
                                }
                            });
                    });
                });
                let mut psize = snap.particle_size;
                row(ui, "size", |ui| {
                    if ui.add(egui::Slider::new(&mut psize, 0.8..=5.0)).changed() {
                        cmds.push(Cmd::SetParticleSize(psize));
                    }
                });
                let mut pbright = snap.particle_brightness;
                row(ui, "brightness", |ui| {
                    if ui.add(egui::Slider::new(&mut pbright, 0.05..=1.0)).changed() {
                        cmds.push(Cmd::SetParticleBrightness(pbright));
                    }
                });
            });
        });
        group(ui, "Mapping", |ui| {
            ui.vertical(|ui| {
                let range_auto = snap.ranges[snap.mode as usize].mode == RangeMode::Auto;
                let mut gain = snap.display_gain;
                row(ui, "display gain", |ui| {
                    if ui
                        .add_enabled(
                            range_auto || snap.mode == RenderMode::Dye,
                            egui::Slider::new(&mut gain, 0.25..=4.0).logarithmic(true),
                        )
                        .on_hover_text("Scales the speed/vorticity/pressure color mapping")
                        .on_disabled_hover_text(
                            "The color range is pinned; set the range to Auto \
                             to use the gain",
                        )
                        .changed()
                    {
                        cmds.push(Cmd::SetDisplayGain(gain));
                    }
                });
                let mut sgain = snap.smoke_gain;
                row(ui, "smoke bright", |ui| {
                    if ui.add(egui::Slider::new(&mut sgain, 0.25..=3.0)).changed() {
                        cmds.push(Cmd::SetSmokeGain(sgain));
                    }
                });
                let mut sponge = snap.sponge_strength;
                row(ui, "edge damping", |ui| {
                    if ui
                        .add(
                            egui::DragValue::new(&mut sponge)
                                .range(0.0..=0.3)
                                .speed(0.002),
                        )
                        .on_hover_text(
                            "Absorbing sponge at the domain edge (needs a \
                             margin); kills reflections of pressure waves",
                        )
                        .changed()
                    {
                        cmds.push(Cmd::SetSpongeStrength(sponge));
                    }
                });
            });
        });
        // Requests only — the canvas consumes them, where the viewport
        // geometry is known (Half B of U1).
        group(ui, "View", |ui| {
            if theme::ribbon_button(ui, false, false, ph::FRAME_CORNERS, "Fit")
                .on_hover_text("Fit the domain in the window. Shortcut: Ctrl+0.")
                .clicked()
            {
                self.view_request = Some(ViewRequest::Fit);
            }
            if theme::ribbon_button(ui, false, false, ph::NUMBER_SQUARE_ONE, "1:1")
                .on_hover_text("Show one grid cell per pixel. Shortcut: Ctrl+1.")
                .clicked()
            {
                self.view_request = Some(ViewRequest::OneToOne);
            }
            ui.add_enabled_ui(!self.selected.is_empty(), |ui| {
                if theme::ribbon_button(ui, false, false, ph::SELECTION, "Selection")
                    .on_hover_text("Zoom to the selection. Shortcut: Ctrl+2.")
                    .clicked()
                {
                    self.view_request = Some(ViewRequest::Selection);
                }
            });
        });
    }
}

// --- Group / row scaffolding ------------------------------------------

/// One ribbon group: content over a small centered caption, followed by
/// a vertical rule. The caption width follows the content.
fn group(ui: &mut egui::Ui, caption: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        let inner = ui.horizontal(|ui| {
            ui.set_height(64.0);
            content(ui);
        });
        let w = inner.response.rect.width().max(52.0);
        ui.allocate_ui_with_layout(
            egui::vec2(w, 14.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(caption)
                        .small()
                        .color(theme::INK_3),
                );
            },
        );
    });
    ui.separator();
}

/// A compact labeled control row (the mockup's `.fld`): a right-aligned
/// caption cell, then the widget.
fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(66.0, 16.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(label)
                        .small()
                        .color(theme::INK_3),
                );
            },
        );
        add(ui);
    });
}
