//! The bottom of the window: a message line (hover-cell readout plus
//! the status message), then the status strip — live solver numbers in
//! monospace. At narrow widths whole trailing fields are dropped rather
//! than clipping text mid-word; the fields are ordered so the physics
//! numbers survive longest.
//!
//! T2-B added the probe UI that has no canvas of its own: the probe
//! markers drawn over the canvas and the probe plot panel that stacks
//! above the status strip. Since U3 the probe store lives in
//! `Settings.probes` — panels read the app's per-frame `ProbeUi`
//! snapshot and edit through `Cmd` (placement clicks moved into the
//! canvas, which U3 owns).

use crate::app::{Cmd, FlowPaintApp};
use crate::sim::{ProbeQuantity, ProbeSample, RenderMode, PROBE_HISTORY_CAP};
use eframe::egui;

use super::theme;
use super::units::{
    self, fmt_cfl, fmt_len, fmt_omega, fmt_pressure, fmt_speed, fmt_time, fmt_zoom,
};

impl FlowPaintApp {
    pub(in crate::app) fn status_bar(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            // Message line.
            ui.horizontal(|ui| {
                if let Some(c) = self.hover_cell {
                    ui.monospace(format!("({:.0}, {:.0})", c[0], c[1]));
                    ui.separator();
                }
                ui.label(&self.status);
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if !self.probe_ui.rows.is_empty() {
                            let on = self.probe_ui.show_plot;
                            if theme::toggle(ui, on, "Probe plot").clicked() {
                                cmds.push(Cmd::SetProbePlot(!on));
                            }
                        }
                    },
                );
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
        self.probe_plot_panel(ctx, cmds);
        self.probe_markers(ctx);
    }

    // --- Probes (plan v4.1, T2-B; store folded into Settings at U3) ---

    /// Draw a marker and label over the canvas for every probe, using
    /// the view mapping the canvas pushed this frame.
    fn probe_markers(&self, ctx: &egui::Context) {
        let Some(v) = self.canvas_mapping else { return };
        if self.probe_ui.rows.is_empty() {
            return;
        }
        let ppp = ctx.pixels_per_point();
        let canvas = egui::Rect::from_min_size(
            egui::pos2(v.vp_origin[0] / ppp, v.vp_origin[1] / ppp),
            egui::vec2(v.vp_size[0] / ppp, v.vp_size[1] / ppp),
        );
        let painter = ctx
            .layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("probe_markers"),
            ))
            .with_clip_rect(canvas);
        for (i, (id, pos)) in self.probe_ui.rows.iter().enumerate() {
            let color = theme::PROBE_COLORS[i % theme::PROBE_COLORS.len()];
            let at = egui::pos2(
                (v.lb_origin[0] + pos[0] * v.px_per_cell) / ppp,
                (v.lb_origin[1] + pos[1] * v.px_per_cell) / ppp,
            );
            painter.circle_stroke(at, 5.0, egui::Stroke::new(1.5, color));
            painter.circle_filled(at, 1.5, color);
            painter.text(
                at + egui::vec2(6.0, -6.0),
                egui::Align2::LEFT_BOTTOM,
                format!("P{id}"),
                egui::FontId::proportional(10.0),
                color,
            );
        }
    }

    /// Physical value of one probe sample for a plot quantity: the raw
    /// sample magnitude times `units::field_phys_per_render` — THE home
    /// for inverting the shader normalizations (queue item 5 unified
    /// this with the legend's copy; this file's old copy carried the
    /// "unify at the track merge" note).
    fn probe_value(&self, q: ProbeQuantity, s: &ProbeSample) -> f32 {
        let f = |mode: RenderMode| {
            units::field_phys_per_render(
                mode,
                self.phys_cache,
                self.stats_euler,
                self.fluid_a,
                self.fluid_rho,
            )
        };
        match q {
            ProbeQuantity::Speed => {
                (s.vel[0] * s.vel[0] + s.vel[1] * s.vel[1]).sqrt() * f(RenderMode::Speed)
            }
            ProbeQuantity::Vorticity => s.curl * f(RenderMode::Vorticity),
            ProbeQuantity::Pressure => s.drho * f(RenderMode::Pressure),
            // Smoke is a passive tracer (factor 1); keep the direct read.
            ProbeQuantity::Smoke => s.dye,
        }
    }

    fn fmt_probe_value(q: ProbeQuantity, v: f32) -> String {
        match q {
            ProbeQuantity::Speed => fmt_speed(v),
            ProbeQuantity::Vorticity => fmt_omega(v),
            ProbeQuantity::Pressure => fmt_pressure(v),
            // Smoke is a passive tracer — arbitrary units, dimensionless.
            ProbeQuantity::Smoke => format!("{v:.2}"),
        }
    }

    /// The probe plot: every probe's history of the chosen quantity
    /// against sim time, physical units, drawn with the painter (no
    /// plotting dependency). For a compressible run this is what says
    /// whether the flow has settled, which smoke cannot show.
    fn probe_plot_panel(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        if !self.probe_ui.show_plot || self.probe_ui.rows.is_empty() {
            return;
        }
        let mut set_quantity: Option<ProbeQuantity> = None;
        let mut clear = false;
        let mut close = false;
        let quantity = self.probe_ui.quantity;
        let dt = self.phys_cache.dt;
        let series: Vec<(u32, Vec<[f32; 2]>)> = self
            .probe_ui
            .series
            .iter()
            .map(|(id, samples)| {
                (
                    *id,
                    samples
                        .iter()
                        .map(|s| [s.steps * dt, self.probe_value(quantity, s)])
                        .collect(),
                )
            })
            .collect();
        egui::TopBottomPanel::bottom("probe_plot").exact_height(150.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Probe plot").color(theme::INK));
                for q in ProbeQuantity::ALL {
                    if theme::toggle(ui, quantity == q, q.label()).clicked() {
                        set_quantity = Some(q);
                    }
                }
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui
                            .small_button("✕")
                            .on_hover_text("Hide the plot; the probes stay")
                            .clicked()
                        {
                            close = true;
                        }
                        if ui
                            .small_button("Clear")
                            .on_hover_text("Drop the collected history")
                            .clicked()
                        {
                            clear = true;
                        }
                        // The plan requires the cap to be stated.
                        theme::derived(
                            ui,
                            format!(
                                "history: last {PROBE_HISTORY_CAP} frames — oldest drop"
                            ),
                        );
                    },
                );
            });
            let (rect, _) =
                ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, theme::RADIUS, theme::FIELD_BG);
            let inner = rect.shrink(6.0);

            let mut xmin = f32::MAX;
            let mut xmax = f32::MIN;
            let mut ymin = f32::MAX;
            let mut ymax = f32::MIN;
            let mut any = false;
            for (_, pts) in &series {
                for p in pts {
                    any = true;
                    xmin = xmin.min(p[0]);
                    xmax = xmax.max(p[0]);
                    ymin = ymin.min(p[1]);
                    ymax = ymax.max(p[1]);
                }
            }
            if !any {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Run the flow to collect samples",
                    egui::FontId::proportional(12.0),
                    theme::INK_3,
                );
                return;
            }
            // Pad the y range so flat lines stay visible.
            let pad = ((ymax - ymin) * 0.05).max(ymax.abs().max(ymin.abs()) * 1e-3 + 1e-12);
            ymin -= pad;
            ymax += pad;
            let xspan = (xmax - xmin).max(1e-12);
            let yspan = (ymax - ymin).max(1e-12);
            let map = |p: [f32; 2]| {
                egui::pos2(
                    inner.left() + (p[0] - xmin) / xspan * inner.width(),
                    inner.bottom() - (p[1] - ymin) / yspan * inner.height(),
                )
            };
            if ymin < 0.0 && ymax > 0.0 {
                let y0 = map([xmin, 0.0]).y;
                painter.line_segment(
                    [egui::pos2(inner.left(), y0), egui::pos2(inner.right(), y0)],
                    egui::Stroke::new(1.0, theme::LINE),
                );
            }
            for (i, (_, pts)) in series.iter().enumerate() {
                if pts.len() < 2 {
                    continue;
                }
                let color = theme::PROBE_COLORS[i % theme::PROBE_COLORS.len()];
                let line: Vec<egui::Pos2> = pts.iter().map(|p| map(*p)).collect();
                painter.add(egui::Shape::line(line, egui::Stroke::new(1.5, color)));
            }
            let mono = egui::FontId::monospace(10.0);
            painter.text(
                inner.left_top(),
                egui::Align2::LEFT_TOP,
                Self::fmt_probe_value(quantity, ymax),
                mono.clone(),
                theme::INK_3,
            );
            painter.text(
                inner.left_bottom(),
                egui::Align2::LEFT_BOTTOM,
                Self::fmt_probe_value(quantity, ymin),
                mono.clone(),
                theme::INK_3,
            );
            painter.text(
                inner.right_bottom(),
                egui::Align2::RIGHT_BOTTOM,
                format!("t {} – {}", fmt_time(xmin), fmt_time(xmax)),
                mono.clone(),
                theme::INK_3,
            );
            // Legend chips, top-right.
            let mut at = inner.right_top();
            for (i, (id, _)) in series.iter().enumerate() {
                let color = theme::PROBE_COLORS[i % theme::PROBE_COLORS.len()];
                let r = painter.text(
                    at,
                    egui::Align2::RIGHT_TOP,
                    format!("P{id}"),
                    mono.clone(),
                    color,
                );
                at.x = r.left() - 8.0;
            }
        });
        if let Some(q) = set_quantity {
            cmds.push(Cmd::SetProbeQuantity(q));
        }
        if clear {
            cmds.push(Cmd::ClearProbeSamples);
        }
        if close {
            cmds.push(Cmd::SetProbePlot(false));
        }
    }
}
