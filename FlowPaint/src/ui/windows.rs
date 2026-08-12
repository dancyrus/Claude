//! The About, keyboard-shortcuts and domain-boundaries windows.

use crate::app::{Cmd, FlowPaintApp, UiSnapshot};
use crate::sim::{EdgeBcs, EdgeKind, EDGE_NAMES};
use eframe::egui;

use super::theme;

impl FlowPaintApp {
    pub(in crate::app) fn windows(
        &mut self,
        ctx: &egui::Context,
        snap: UiSnapshot,
        cmds: &mut Vec<Cmd>,
    ) {
        self.generator_windows(ctx, snap);
        self.edges_window(ctx, snap, cmds);

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
                        ("Space (release)", "pause / resume"),
                        ("Scroll / pinch", "zoom at the cursor"),
                        ("Middle-drag / Space+drag", "pan the view"),
                        ("Ctrl+0", "fit the domain to the window"),
                        ("Ctrl+1", "1:1 — one cell per pixel"),
                        ("Ctrl+2", "zoom to the selection"),
                        ("Ctrl+Z / Ctrl+Y", "undo / redo"),
                        ("Ctrl+E / Ctrl+Shift+E", "quick export PNG: canvas / annotated"),
                        (
                            "S / L / R / E / P / B",
                            "select / line / rect / ellipse / polyline / pencil",
                        ),
                        ("A / C", "arc (two ends, then bulge) / spline"),
                        ("F / X / M", "fill (paint bucket) / eraser / measure"),
                        ("Ctrl while drawing", "suspend object snaps"),
                        ("Shift+click", "add to / remove from the selection"),
                        ("Drag on empty space", "rubber-band select (Shift adds)"),
                        ("Ctrl+A", "select all"),
                        ("Del", "delete the selection"),
                        ("Ctrl+D", "duplicate the selection"),
                        ("Ctrl+C / Ctrl+V", "copy / paste (Shift+V in place)"),
                        ("Ctrl+G / Ctrl+Shift+G", "group / ungroup the selection"),
                        ("Double-click a member", "enter its group (Esc leaves)"),
                        ("Ctrl+] / Ctrl+[", "raise / lower (+Shift: front / back)"),
                        ("Arrows (+Shift for coarse)", "nudge the selection"),
                        ("Shift on the rotate handle", "snap to the angle-snap step"),
                        ("Shift while drawing", "angle-snapped lines · squares · circles"),
                        ("Alt while drawing", "rect/ellipse from centre"),
                        ("Enter / right-click", "finish the polyline / spline / arc"),
                        ("Esc", "cancel gesture / leave group / deselect"),
                    ] {
                        ui.label(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
            });
    }

    /// The per-edge boundary-conditions window (T2-C). Lives here rather
    /// than in the ribbon so it stays up across tab switches, like the
    /// generator dialogs.
    fn edges_window(&mut self, ctx: &egui::Context, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        let mut open = self.show_edges;
        egui::Window::new("Domain boundaries")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    "What each edge of the domain does. Far field is the \
                     open edge every scene had before: nothing painted, \
                     with the absorbing margin around the canvas.",
                );
                ui.add_space(4.0);
                egui::Grid::new("edge_bc_grid").show(ui, |ui| {
                    for (i, name) in EDGE_NAMES.iter().enumerate() {
                        let cur = snap.edges.0[i];
                        // Periodic is a property of an opposite PAIR
                        // (left–right or top–bottom): picking it sets
                        // both edges, leaving it clears both, so the
                        // solver never sees an unpaired periodic edge.
                        let opp = i ^ 1;
                        ui.label(*name);
                        egui::ComboBox::from_id_salt(("edge_bc", i))
                            .width(110.0)
                            .selected_text(cur.label())
                            .show_ui(ui, |ui| {
                                for k in [
                                    EdgeKind::FarField,
                                    EdgeKind::Inlet,
                                    EdgeKind::Outlet,
                                    EdgeKind::Wall,
                                ] {
                                    if theme::toggle(ui, cur == k, k.label())
                                        .clicked()
                                        && cur != k
                                    {
                                        cmds.push(Cmd::SetEdgeBc(i, k));
                                        if cur == EdgeKind::Periodic
                                            && snap.edges.0[opp]
                                                == EdgeKind::Periodic
                                        {
                                            cmds.push(Cmd::SetEdgeBc(
                                                opp,
                                                EdgeKind::FarField,
                                            ));
                                        }
                                    }
                                }
                                if theme::toggle(
                                    ui,
                                    cur == EdgeKind::Periodic,
                                    EdgeKind::Periodic.label(),
                                )
                                .on_hover_text(
                                    "Flow that goes out through this edge \
                                     comes in again through the opposite \
                                     edge. The two edges of the axis \
                                     always change together, as one pair.",
                                )
                                .clicked()
                                    && cur != EdgeKind::Periodic
                                {
                                    cmds.push(Cmd::SetEdgeBc(
                                        i,
                                        EdgeKind::Periodic,
                                    ));
                                    cmds.push(Cmd::SetEdgeBc(
                                        opp,
                                        EdgeKind::Periodic,
                                    ));
                                }
                            });
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                // Decision T2-C: never ship a wall that silently is not
                // one — any wall edge turns the absorbing sponge off.
                if snap.edges.disables_sponge() {
                    ui.colored_label(
                        theme::WARN,
                        "Wall edge set: the absorbing sponge layer is off \
                         (a sponged wall is not a wall).",
                    );
                }
                // Wrap is per axis (EdgeBcs::wrap_bits); the sponge is
                // excluded on a wrapped axis in the kernels.
                let wrap = snap.edges.wrap_bits();
                if wrap != 0 {
                    theme::derived(
                        ui,
                        match wrap {
                            1 => "Periodic pair: left–right. The sponge does \
                                  not apply on that axis."
                                .into(),
                            2 => "Periodic pair: top–bottom. The sponge does \
                                  not apply on that axis."
                                .into(),
                            _ => "Periodic pairs: left–right and top–bottom. \
                                  The flow wraps on both axes."
                                .into(),
                        },
                    );
                }
                theme::derived(
                    ui,
                    if snap.tunnel {
                        "Far-field edges carry the wind-tunnel freestream \
                         (left to right)."
                            .into()
                    } else {
                        "Far-field edges are still (wind tunnel off).".into()
                    },
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("Wind tunnel preset")
                        .on_hover_text(
                            "Left inlet, right outlet, top and bottom far \
                             field, freestream on — the classic tunnel",
                        )
                        .clicked()
                    {
                        cmds.push(Cmd::SetWindTunnel(true));
                    }
                    if ui
                        .button("All far field")
                        .on_hover_text(
                            "Every edge open; the freestream keeps its \
                             current on/off state",
                        )
                        .clicked()
                    {
                        cmds.push(Cmd::SetEdgeBcs(EdgeBcs::OPEN));
                    }
                });
            });
        self.show_edges = open;
    }
}
