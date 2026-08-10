//! The left control panel: action row, tools, object/defaults dispatch,
//! sketch aids, generators, scene presets, view and physics sections.

use crate::app::{
    build_preset, fmt_len, fmt_speed, fmt_time, Cmd, FlowPaintApp, Gesture,
    ScenePreset, Tool, UiSnapshot, FLUID_PRESETS,
};
use crate::sim::{RenderMode, SolverMode, PARTICLE_CHOICES};
use eframe::egui;

impl FlowPaintApp {
    pub(in crate::app) fn side_panel(&mut self, ctx: &egui::Context, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
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
        ui.horizontal_wrapped(|ui| {
            for (m, label, tip) in [
                (
                    SolverMode::Lbm,
                    "Incompressible",
                    "Lattice-Boltzmann: viscous, low Mach — smoke, wakes, \
                     vortex streets",
                ),
                (
                    SolverMode::Euler,
                    "Compressible",
                    "Finite-volume Euler: real gas dynamics — shocks, \
                     expansion fans, choked nozzles (inviscid)",
                ),
            ] {
                if ui
                    .selectable_label(snap.solver == m, label)
                    .on_hover_text(tip)
                    .clicked()
                    && snap.solver != m
                {
                    cmds.push(Cmd::SetSolver(m));
                }
            }
        });
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
                        self.fluid_a = p.a;
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
        if snap.solver == SolverMode::Euler {
            let mut mach = snap.mach;
            let mach_label = format!(
                "inlet Mach ({})",
                fmt_speed(mach * self.fluid_a)
            );
            if ui
                .add(egui::Slider::new(&mut mach, 0.3..=3.0).text(mach_label))
                .changed()
            {
                cmds.push(Cmd::SetMach(mach));
            }
            ui.label(
                egui::RichText::new(format!(
                    "inviscid gas dynamics (γ = 1.4, a∞ = {}) — shocks and \
                     expansion fans are real; boundary layers are not",
                    fmt_speed(self.fluid_a)
                ))
                .small()
                .weak(),
            );
        } else {
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
}
