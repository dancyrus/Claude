//! The bottom of the window: a message line (hover-cell readout plus
//! the status message), then the status strip — live solver numbers in
//! monospace: grid, cell size, time step, elapsed sim time, inlet
//! speed, CFL, MLUPS and Reynolds.

use crate::app::{fmt_len, fmt_speed, fmt_time, FlowPaintApp};
use eframe::egui;

impl FlowPaintApp {
    pub(in crate::app) fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            // Message line.
            ui.horizontal(|ui| {
                if let Some(c) = self.hover_cell {
                    ui.monospace(format!("({:.0}, {:.0})", c[0], c[1]));
                    ui.separator();
                }
                ui.label(&self.status);
            });
            // Status strip.
            ui.horizontal(|ui| {
                let ps = self.phys_cache;
                let re = if self.stats_euler {
                    "Re ∞ (inviscid)".to_string()
                } else {
                    format!("Re ≈ {}", self.stats_re)
                };
                ui.monospace(format!(
                    "grid {}×{} (+{})  |  cell {}  |  dt {}  |  t {}  |  u∞ {}  |  CFL {:.2}  |  {} obj  |  {:.0} MLUPS  |  {}",
                    self.stats_grid.0,
                    self.stats_grid.1,
                    self.stats_margin,
                    fmt_len(ps.dx),
                    fmt_time(ps.dt),
                    fmt_time(self.sim_time_s as f32),
                    fmt_speed(self.stats_u_inf),
                    self.stats_cfl,
                    self.model.objects.len(),
                    self.stats_mlups,
                    re
                ));
            });
        });
    }
}
