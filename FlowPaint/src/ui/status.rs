//! The bottom of the window: a message line (hover-cell readout plus
//! the status message), then the status strip — live solver numbers in
//! monospace. At narrow widths whole trailing fields are dropped rather
//! than clipping text mid-word; the fields are ordered so the physics
//! numbers survive longest.

use crate::app::{FlowPaintApp};
use eframe::egui;

use super::units::{fmt_cfl, fmt_len, fmt_speed, fmt_time, fmt_zoom};

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
            // Status strip. "CFL (inlet)" because sim.rs computes the
            // inlet-state Courant estimate, not the field maximum.
            ui.horizontal(|ui| {
                let ps = self.phys_cache;
                let re = if self.stats_euler {
                    "Re ∞ (inviscid)".to_string()
                } else {
                    format!("Re ≈ {}", self.stats_re)
                };
                let segments: Vec<String> = vec![
                    format!(
                        "grid {}×{} (+{})",
                        self.stats_grid.0, self.stats_grid.1, self.stats_margin
                    ),
                    format!("cell {}", fmt_len(ps.dx)),
                    format!("dt {}", fmt_time(ps.dt)),
                    format!("t {}", fmt_time(self.sim_time_s as f32)),
                    format!("zoom {}", fmt_zoom(self.view_px_per_cell)),
                    format!("u∞ {}", fmt_speed(self.stats_u_inf)),
                    format!("CFL (inlet) {}", fmt_cfl(self.stats_cfl)),
                    format!("{} obj", self.model.objects.len()),
                    format!("{:.0} MLUPS", self.stats_mlups),
                    re,
                ];
                let sep = "  |  ";
                let font = egui::TextStyle::Monospace.resolve(ui.style());
                let avail = ui.available_width();
                let fits = |n: usize, ui: &egui::Ui| {
                    let text = segments[..n].join(sep);
                    ui.fonts(|f| {
                        f.layout_no_wrap(text, font.clone(), egui::Color32::WHITE)
                            .size()
                            .x
                    }) <= avail
                };
                let mut n = segments.len();
                while n > 1 && !fits(n, ui) {
                    n -= 1;
                }
                ui.monospace(segments[..n].join(sep));
            });
        });
    }
}
