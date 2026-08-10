//! The bottom status bar: cursor cell, status message, and live stats.

use crate::app::FlowPaintApp;
use eframe::egui;

impl FlowPaintApp {
    pub(in crate::app) fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(c) = self.hover_cell {
                    ui.monospace(format!("({:.0}, {:.0})", c[0], c[1]));
                    ui.separator();
                }
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let re = if self.stats_euler {
                        "Re ∞ (inviscid)".to_string()
                    } else {
                        format!("Re ≈ {}", self.stats_re)
                    };
                    ui.monospace(format!(
                        "{} objects   |   canvas {} x {} (sim {} x {}, +{} margin)   |   {:.0} MLUPS   |   {}",
                        self.model.objects.len(),
                        self.stats_grid.0,
                        self.stats_grid.1,
                        self.stats_full.0,
                        self.stats_full.1,
                        self.stats_margin,
                        self.stats_mlups,
                        re
                    ));
                });
            });
        });
    }
}
