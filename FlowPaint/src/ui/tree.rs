//! The model tree: the scene's real contents as a selectable outline —
//! a domain root with the solver child, then one node per SketchObject.
//! Clicking writes the existing `FlowPaintApp::selected`; the tree is a
//! new reader and writer of existing selection state, not new state.

use crate::app::FlowPaintApp;
use crate::model::Shape;
use eframe::egui;

use super::theme;

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
    }
}
