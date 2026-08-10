//! The right-hand legend: flow numbers in physical units and the
//! color-scale bar for the current view.

use crate::app::{
    coolwarm_color, fmt_len, fmt_pressure, fmt_speed, fmt_time, inferno_color,
    FlowPaintApp, UiSnapshot,
};
use crate::sim::{RenderMode, SolverMode};
use eframe::egui;

impl FlowPaintApp {
    /// The right-hand legend: the important flow numbers in physical
    /// units, plus a color-scale bar for the current view.
    pub(in crate::app) fn legend_panel(&mut self, ctx: &egui::Context, snap: UiSnapshot) {
        if !self.show_legend {
            return;
        }
        let ps = self.phys_cache;
        let (_vw, vh) = self.stats_grid;
        egui::SidePanel::right("legend").default_width(200.0).show(ctx, |ui| {
            ui.label(super::theme::heading("Flow numbers"));
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
                    let euler = snap.solver == SolverMode::Euler;
                    let u_inf = if euler {
                        snap.mach * self.fluid_a
                    } else {
                        ps.u_phys(snap.flow)
                    };
                    row(
                        "Solver",
                        if euler { "Euler (compressible)" } else { "LBM (incompressible)" }
                            .to_string(),
                    );
                    row("Fluid", self.fluid_name.to_string());
                    if euler {
                        row("a∞", fmt_speed(self.fluid_a));
                        row("Mach M∞", format!("{:.2}", snap.mach));
                    } else {
                        row("ν", format!("{:.2e} m²/s", self.fluid_nu));
                    }
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
                    row("Inlet U∞", fmt_speed(u_inf));
                    row(
                        "Ref. length",
                        fmt_len(ps.len_m(0.16 * vh as f32)),
                    );
                    if euler {
                        row("Reynolds", "∞ (inviscid)".to_string());
                    } else {
                        row("Reynolds", format!("{}", self.stats_re));
                    }
                    row(
                        "Dyn. press.",
                        fmt_pressure(0.5 * self.fluid_rho * u_inf * u_inf),
                    );
                    row(
                        "Sim rate",
                        format!("{:.2}× real", self.stats_steps_per_s * ps.dt),
                    );
                    row("Sim time", fmt_time(self.sim_time_s as f32));
                });
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
                    // Inverts the shader normalization |vel| / (inlet * 1.6)
                    // — both solvers write `vel` in units where the inlet
                    // reference is their respective inflow speed.
                    let u_sat = if snap.solver == SolverMode::Euler {
                        snap.mach * self.fluid_a * 1.6 / gain
                    } else {
                        ps.u_phys(snap.flow * 1.6 / gain)
                    };
                    Self::colormap_bar(ui, |t| inferno_color(t));
                    ui.horizontal(|ui| {
                        super::theme::mono_small(ui, "0".into());
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| super::theme::mono_small(ui, format!("≥ {}", fmt_speed(u_sat))),
                        );
                    });
                }
                RenderMode::Vorticity => {
                    ui.label("Vorticity ω (curl)");
                    let inlet_render = if snap.solver == SolverMode::Euler {
                        snap.mach * snap.euler_dt
                    } else {
                        snap.flow
                    };
                    let w_sat = inlet_render.max(0.02) / (4.0 * gain) / ps.dt;
                    Self::colormap_bar(ui, |t| coolwarm_color(t * 2.0 - 1.0));
                    ui.horizontal(|ui| {
                        super::theme::mono_small(ui, format!("-{:.1} 1/s", w_sat));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| super::theme::mono_small(ui, format!("+{:.1} 1/s", w_sat)),
                        );
                    });
                    ui.small("red: clockwise · blue: counter-clockwise");
                }
                RenderMode::Pressure => {
                    ui.label("Pressure Δp (gauge)");
                    // Euler mode writes 1 + 0.1 * (p - p∞) into the density
                    // buffer (p nondimensionalized by ρ∞ a∞²).
                    let p_sat = if snap.solver == SolverMode::Euler {
                        (1.0 / (25.0 * gain)) / 0.1
                            * self.fluid_rho
                            * self.fluid_a
                            * self.fluid_a
                    } else {
                        ps.pressure_pa(1.0 / (25.0 * gain), self.fluid_rho)
                    };
                    Self::colormap_bar(ui, |t| coolwarm_color(t * 2.0 - 1.0));
                    ui.horizontal(|ui| {
                        super::theme::mono_small(ui, format!("-{}", fmt_pressure(p_sat)));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| super::theme::mono_small(ui, format!("+{}", fmt_pressure(p_sat))),
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
}
