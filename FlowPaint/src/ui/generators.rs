//! The airfoil and rocket-nozzle generator dialog windows.

use crate::app::{fmt_speed, FlowPaintApp, UiSnapshot};
use crate::sim::SolverMode;
use eframe::egui;

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
                ui.add(egui::Slider::new(&mut p.camber, 0.0..=9.0).text("camber %"));
                ui.add(
                    egui::Slider::new(&mut p.camber_pos, 15.0..=70.0)
                        .text("camber position %"),
                );
                ui.add(egui::Slider::new(&mut p.thickness, 4.0..=24.0).text("thickness %"));
                ui.add(egui::Slider::new(&mut p.aoa_deg, -15.0..=20.0).text("angle of attack °"));
                ui.add(
                    egui::Slider::new(&mut p.chord_cells, 60.0..=600.0)
                        .text("chord (cells)"),
                );
                let pos_digit = if p.camber > 0.0 {
                    (p.camber_pos / 10.0).round()
                } else {
                    0.0 // symmetric airfoils are NACA 00xx
                };
                ui.monospace(format!(
                    "≈ NACA {:.0}{:.0}{:02.0} at {:.1}°",
                    p.camber, pos_digit, p.thickness, p.aoa_deg
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
                ui.add(
                    egui::Slider::new(&mut p.throat_cells, 12.0..=100.0)
                        .text("throat width (cells)"),
                );
                ui.add(
                    egui::Slider::new(&mut p.exit_ratio, 1.2..=20.0)
                        .text("exit / throat width"),
                );
                ui.add(
                    egui::Slider::new(&mut p.chamber_ratio, 1.5..=4.0)
                        .text("chamber / throat width"),
                );
                ui.add(
                    egui::Slider::new(&mut p.conv_ratio, 1.0..=4.0)
                        .text("converging length / throat"),
                );
                ui.add(
                    egui::Slider::new(&mut p.div_ratio, 2.0..=16.0)
                        .text("bell length / throat"),
                );
                ui.add(egui::Slider::new(&mut p.wall_cells, 3.0..=12.0).text("wall (cells)"));
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
                if ui
                    .add(
                        egui::Slider::new(&mut p.fan_mult, 0.2..=2.0)
                            .text("chamber fan ×"),
                    )
                    .changed()
                {
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
                            format!("real engine exhaust ≈ {:.0} m/s", ve),
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
                                "real engine exhaust ≈ {:.0} m/s (~{:.0}× faster)",
                                ve, factor
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
