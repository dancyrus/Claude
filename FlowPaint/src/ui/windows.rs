//! The About and keyboard-shortcuts windows.

use crate::app::{FlowPaintApp, UiSnapshot};
use eframe::egui;

impl FlowPaintApp {
    pub(in crate::app) fn windows(&mut self, ctx: &egui::Context, snap: UiSnapshot) {
        self.generator_windows(ctx, snap);

        egui::Window::new("About FlowPaint V2")
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    "FlowPaint V2 solves the 2D Navier-Stokes equations in real \
                     time with a D2Q9 lattice-Boltzmann method in GPU compute \
                     shaders (wgpu: Vulkan / DX12 / Metal).",
                );
                ui.label(
                    "The Compressible solver mode switches to a finite-volume \
                     Euler method (MUSCL reconstruction + HLLC fluxes): real \
                     shocks, expansion fans and choked nozzles, inviscid.",
                );
                ui.label(
                    "Everything you sketch is a live object: select it any time \
                     to move, rotate, resize or retune its physics while the \
                     fluid reacts.",
                );
            });

        egui::Window::new("Keyboard shortcuts")
            .open(&mut self.show_shortcuts)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("shortcuts").striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("Space", "pause / resume"),
                        ("Ctrl+Z / Ctrl+Y", "undo / redo"),
                        (
                            "S / L / R / E / P / B",
                            "select / line / rect / ellipse / polyline / pencil",
                        ),
                        ("Del", "delete the selected object"),
                        ("Ctrl+D", "duplicate the selected object"),
                        ("Arrows (+Shift for fine)", "nudge the selected object"),
                        ("Shift while drawing", "angle-snapped lines · squares · circles"),
                        ("Alt while drawing", "rect/ellipse from centre"),
                        ("Enter / right-click", "finish the polyline"),
                        ("Esc", "cancel gesture / deselect"),
                    ] {
                        ui.label(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
            });
    }
}
