//! The airfoil and rocket-nozzle generator dialog windows.

use crate::app::{FlowPaintApp, UiSnapshot};
use crate::sim::SolverMode;
use eframe::egui;

use super::units::{fmt_angle, fmt_factor, fmt_len, fmt_speed};

impl FlowPaintApp {
    pub(in crate::app) fn generator_windows(&mut self, ctx: &egui::Context, snap: UiSnapshot) {
        use crate::generators as gen;

        let mut show = self.show_airfoil_gen;
        egui::Window::new("Airfoil generator")
            .open(&mut show)
            .resizable(false)
            .show(ctx, |ui| {
                let p = &mut self.airfoil_params;
                egui::ComboBox::from_label("Famous airfoils")
                    .selected_text("Choose a preset…")
                    .show_ui(ui, |ui| {
                        for (name, m, cp, t, aoa) in gen::AIRFOIL_PRESETS {
                            if ui.selectable_label(false, name).clicked() {
                                p.camber = m;
                                p.camber_pos = if cp > 0.0 { cp } else { 40.0 };
                                p.thickness = t;
                                p.aoa_deg = aoa;
                            }
                        }
                    });
                dv_row(ui, "camber", egui::DragValue::new(&mut p.camber).range(0.0..=9.0).speed(0.1).suffix(" %"));
                dv_row(ui, "camber position", egui::DragValue::new(&mut p.camber_pos).range(15.0..=70.0).speed(0.5).suffix(" %"));
                dv_row(ui, "thickness", egui::DragValue::new(&mut p.thickness).range(4.0..=24.0).speed(0.1).suffix(" %"));
                dv_row(ui, "angle of attack", egui::DragValue::new(&mut p.aoa_deg).range(-15.0..=20.0).speed(0.1).suffix("°"));
                dv_row(ui, "chord", egui::DragValue::new(&mut p.chord_cells).range(60.0..=600.0).speed(1.0).suffix(" cells"));
                super::theme::derived(
                    ui,
                    format!("chord = {}", fmt_len(self.phys_cache.len_m(p.chord_cells))),
                );
                let pos_digit = if p.camber > 0.0 {
                    (p.camber_pos / 10.0).round()
                } else {
                    0.0 // symmetric airfoils are NACA 00xx
                };
                ui.monospace(format!(
                    "≈ NACA {:.0}{:.0}{:02.0} at {}",
                    p.camber,
                    pos_digit,
                    p.thickness,
                    fmt_angle(p.aoa_deg)
                ));
                if ui.button("Insert into scene").clicked() {
                    let stamp = gen::generate_airfoil(p);
                    self.insert_stamp_object(stamp);
                }
            });
        self.show_airfoil_gen = show;

        let mut show = self.show_nozzle_gen;
        egui::Window::new("Rocket nozzle generator")
            .open(&mut show)
            .resizable(false)
            .show(ctx, |ui| {
                let p = &mut self.nozzle_params;
                egui::ComboBox::from_label("Famous engines")
                    .selected_text("Choose a preset…")
                    .show_ui(ui, |ui| {
                        for (name, eps, contour, ve) in gen::NOZZLE_PRESETS {
                            if ui.selectable_label(false, name).clicked() {
                                // Planar 2D analogue of an axisymmetric area
                                // ratio: width ratio = sqrt(eps).
                                p.exit_ratio = eps.sqrt().clamp(1.2, 20.0);
                                p.contour = contour;
                                p.div_ratio =
                                    (1.5 * (p.exit_ratio - 1.0)).clamp(2.0, 16.0);
                                // Scale the chamber fan for the active
                                // solver (see nozzle_auto_fan_mult).
                                p.fan_mult =
                                    nozzle_auto_fan_mult(&snap, p.chamber_ratio);
                                self.nozzle_fan_auto = true;
                                self.nozzle_real_ve = Some(ve);
                            }
                        }
                    });
                dv_row(ui, "throat width", egui::DragValue::new(&mut p.throat_cells).range(12.0..=100.0).speed(0.5).suffix(" cells"));
                super::theme::derived(
                    ui,
                    format!("throat = {}", fmt_len(self.phys_cache.len_m(p.throat_cells))),
                );
                dv_row(ui, "exit / throat width", egui::DragValue::new(&mut p.exit_ratio).range(1.2..=20.0).speed(0.05));
                dv_row(ui, "chamber / throat width", egui::DragValue::new(&mut p.chamber_ratio).range(1.5..=4.0).speed(0.02));
                dv_row(ui, "converging length / throat", egui::DragValue::new(&mut p.conv_ratio).range(1.0..=4.0).speed(0.02));
                dv_row(ui, "bell length / throat", egui::DragValue::new(&mut p.div_ratio).range(2.0..=16.0).speed(0.05));
                dv_row(ui, "wall", egui::DragValue::new(&mut p.wall_cells).range(3.0..=12.0).speed(0.1).suffix(" cells"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut p.contour, gen::NozzleContour::Bell, "Bell");
                    ui.radio_value(&mut p.contour, gen::NozzleContour::Conical, "Conical (15°-style)");
                });
                ui.checkbox(&mut p.chamber_fan, "Fan in the chamber (self-powered)");
                // Track the preset formula live (it depends on the current
                // flow speed / Mach) until the user overrides the slider —
                // what the dialog shows is exactly what Insert stamps.
                if self.nozzle_fan_auto {
                    p.fan_mult = nozzle_auto_fan_mult(&snap, p.chamber_ratio);
                }
                let fan_resp = ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::DragValue::new(&mut p.fan_mult)
                            .range(0.2..=2.0)
                            .speed(0.01)
                            .suffix(" ×"),
                    );
                    ui.label("chamber fan");
                    if self.nozzle_fan_auto {
                        ui.label(
                            egui::RichText::new("(auto)")
                                .small()
                                .color(super::theme::INK_3),
                        );
                    }
                    resp
                });
                if fan_resp.inner.changed() {
                    self.nozzle_fan_auto = false;
                }
                // Expected jet speeds in real units, next to the engine's
                // actual exhaust velocity.
                let euler_mode = snap.solver == SolverMode::Euler;
                let (throat_sim, capped) = if euler_mode {
                    // Continuity estimate of the throat speed from the
                    // chamber feed; the bell accelerates further on its own.
                    let m_throat = snap.mach * p.fan_mult * p.chamber_ratio;
                    (m_throat * self.fluid_a, false)
                } else {
                    // The LBM solver clamps lattice speed at 0.3, so the
                    // readout is capped the same way.
                    let throat_lattice = snap.flow * p.fan_mult * p.chamber_ratio;
                    (
                        self.phys_cache.u_phys(throat_lattice.min(0.3)),
                        throat_lattice > 0.3,
                    )
                };
                ui.monospace(format!(
                    "sim throat jet ≈ {}{}",
                    fmt_speed(throat_sim),
                    if capped { " (speed-capped)" } else { "" }
                ));
                if let Some(ve) = self.nozzle_real_ve {
                    let factor = ve / throat_sim.max(1e-6);
                    if euler_mode {
                        super::theme::mono_small(
                            ui,
                            format!("real engine exhaust ≈ {}", fmt_speed(ve)),
                        );
                        ui.label(
                            egui::RichText::new(
                                "In compressible mode the bell itself accelerates \
                                 the jet through the sonic throat; expect a \
                                 supersonic plume.",
                            )
                            .small()
                            .weak(),
                        );
                    } else {
                        super::theme::mono_small(
                            ui,
                            format!(
                                "real engine exhaust ≈ {} (~{} faster)",
                                fmt_speed(ve),
                                fmt_factor(factor)
                            ),
                        );
                        ui.label(
                            egui::RichText::new(
                                "The incompressible solver caps jet speed, so \
                                 this is a scaled approximation.",
                            )
                            .small()
                            .weak(),
                        );
                    }
                }
                let solver_note = if euler_mode {
                    "Compressible Euler mode: choked flow, expansion fans and \
                     shocks are real gas dynamics (inviscid)."
                } else {
                    "Note: this solver is incompressible (low Mach), so you get \
                     the shape and a jet, not real choked-nozzle gas dynamics. \
                     Switch Physics → Compressible for the real thing."
                };
                ui.label(egui::RichText::new(solver_note).small().weak());
                if ui.button("Insert into scene").clicked() {
                    let stamp = gen::generate_nozzle(p);
                    self.insert_stamp_object(stamp);
                }
            });
        self.show_nozzle_gen = show;
    }
}
/// Auto chamber-fan multiplier for the nozzle generator, per solver.
fn nozzle_auto_fan_mult(snap: &UiSnapshot, chamber_ratio: f32) -> f32 {
    match snap.solver {
        // Scale so the throat jet approximates the engine's exhaust as
        // closely as the LBM speed cap (0.3 lattice) allows.
        SolverMode::Lbm => {
            (0.27 / (snap.flow * chamber_ratio).max(1e-4)).clamp(0.2, 2.0)
        }
        // Compressible: feed the chamber at ~Mach 0.3 like a real engine —
        // the converging-diverging geometry does the accelerating.
        SolverMode::Euler => (0.3 / snap.mach.max(0.1)).clamp(0.2, 2.0),
    }
}

/// A dialog control row: DragValue box first, label after (canonical
/// value in the box, unit in the suffix).
fn dv_row(ui: &mut egui::Ui, label: &str, dv: egui::DragValue<'_>) -> egui::Response {
    ui.horizontal(|ui| {
        let resp = ui.add(dv);
        ui.label(label);
        resp
    })
    .inner
}
