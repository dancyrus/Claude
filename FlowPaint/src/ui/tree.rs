//! The model tree: the scene's real contents as a selectable outline —
//! a domain root with the solver child, then one node per SketchObject.
//! Clicking writes the existing `FlowPaintApp::selected`; the tree is a
//! new reader and writer of existing selection state, not new state.

use crate::app::{FlowPaintApp, Tool};
use crate::model::Shape;
use crate::sim::{probes, MAX_PROBES};
use eframe::egui;

use super::theme;
use super::units::fmt_len;

impl FlowPaintApp {
    pub(in crate::app) fn tree_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("model_tree")
            .resizable(true)
            .default_width(theme::dim::TREE_WIDTH)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::heading("Model tree"));
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} objects",
                                    self.model.objects.len()
                                ))
                                .small()
                                .color(theme::INK_3),
                            );
                        },
                    );
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.tree_contents(ui));
            });
    }

    fn tree_contents(&mut self, ui: &mut egui::Ui) {
        // Domain root and the solver child select "nothing" — the
        // settings panel then shows the domain/defaults block.
        let domain_on = self.selected.is_none();
        if ui.selectable_label(domain_on, "Domain").clicked() {
            self.finish_gesture();
            self.selected = None;
        }
        ui.indent("tree_domain", |ui| {
            let solver = if self.stats_euler {
                "Solver — Euler (compressible)"
            } else {
                "Solver — LBM (incompressible)"
            };
            if ui.selectable_label(false, solver).clicked() {
                self.finish_gesture();
                self.selected = None;
            }
        });

        ui.label(
            egui::RichText::new("Geometry")
                .small()
                .color(theme::INK_3),
        );
        // Snapshot (id, label) pairs first: the context-menu actions
        // mutate the model, so the row loop must not borrow it.
        let rows: Vec<(u64, String)> = self
            .model
            .objects
            .iter()
            .map(|o| {
                let kind = match &o.shape {
                    Shape::Line { .. } => "line",
                    Shape::Poly { closed: true, .. } => "polygon",
                    Shape::Poly { .. } => "polyline",
                    Shape::Rect { .. } => "rectangle",
                    Shape::Ellipse { .. } => "ellipse",
                    Shape::Stamp { .. } => "generated part",
                };
                (o.id, format!("{} #{:02} ({kind})", o.material.label(), o.id))
            })
            .collect();

        ui.indent("tree_objects", |ui| {
            for (id, label) in rows {
                let on = self.selected == Some(id);
                let resp = ui.selectable_label(on, label);
                if resp.clicked() {
                    self.finish_gesture();
                    self.selected = Some(id);
                }
                resp.context_menu(|ui| {
                    if ui.button("Duplicate").clicked() {
                        self.finish_gesture();
                        self.selected = Some(id);
                        self.duplicate_selected();
                        ui.close_menu();
                    }
                    if ui.button("Delete").clicked() {
                        self.finish_gesture();
                        if self.selected == Some(id) {
                            self.selected = None;
                        }
                        self.model.remove(id);
                        ui.close_menu();
                    }
                });
            }
        });

        self.tree_probes(ui);
    }

    /// The probe section (plan v4.1, T2-B): persistent point probes as
    /// tree entries with a delete action, plus the place-probe arming
    /// button. Probes are not sketch objects — they live in the shared
    /// probe store, not the model — so this section reads and writes
    /// that store directly.
    fn tree_probes(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Probes")
                .small()
                .color(theme::INK_3),
        );
        // Snapshot rows first: the delete action mutates the store, so
        // the row loop must not hold its lock.
        let (rows, count, arming) = {
            let pr = probes().lock().unwrap();
            let ps = self.phys_cache;
            let rows: Vec<(u32, String, String)> = pr
                .probes
                .iter()
                .map(|p| {
                    (
                        p.id,
                        format!("Probe P{} ({:.0}, {:.0})", p.id, p.pos[0], p.pos[1]),
                        format!(
                            "at {} , {} from the top-left corner",
                            fmt_len(ps.len_m(p.pos[0])),
                            fmt_len(ps.len_m(p.pos[1]))
                        ),
                    )
                })
                .collect();
            (rows, pr.probes.len(), pr.arming)
        };
        let mut remove: Option<u32> = None;
        ui.indent("tree_probes", |ui| {
            for (id, label, hover) in &rows {
                let resp = ui.selectable_label(false, label).on_hover_text(hover);
                resp.context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        remove = Some(*id);
                        ui.close_menu();
                    }
                });
            }
            let full = count >= MAX_PROBES;
            let resp = ui.add_enabled(
                !full && !arming,
                egui::Button::new(if arming { "Click the canvas…" } else { "+ Add probe" })
                    .small(),
            );
            let resp = resp
                .on_hover_text("Place a probe with one canvas click (Esc cancels)")
                .on_disabled_hover_text(if full {
                    "8 probes maximum"
                } else {
                    "Click the canvas to place the probe (Esc cancels)"
                });
            if resp.clicked() {
                probes().lock().unwrap().arming = true;
                // Placing a probe must not also draw a shape.
                self.finish_gesture();
                self.tool = Tool::Select;
                self.status =
                    "Click the canvas to place the probe (Esc cancels).".into();
            }
        });
        if let Some(id) = remove {
            let mut pr = probes().lock().unwrap();
            pr.probes.retain(|p| p.id != id);
            if pr.probes.is_empty() {
                pr.show_plot = false;
            }
        }
    }
}
