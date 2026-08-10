//! The model tree: the scene's real contents as a selectable outline —
//! a domain root with the solver child, then one node per SketchObject,
//! TOPMOST FIRST (list order is z-order; later objects win overlaps).
//! Clicking writes the existing `FlowPaintApp::selected` set — plain
//! click selects, Ctrl toggles, Shift ranges from the last-clicked row.
//! Per-row eye/lock toggles manage `hidden`/`locked` (both persist).

use crate::app::{FlowPaintApp, Tool};
use crate::model::Shape;
use crate::sim::{probes, MAX_PROBES};
use eframe::egui;
use egui_phosphor::regular as ph;

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
        let domain_on = self.selected.is_empty();
        if ui.selectable_label(domain_on, "Domain").clicked() {
            self.finish_gesture();
            self.deselect_all();
        }
        ui.indent("tree_domain", |ui| {
            let solver = if self.stats_euler {
                "Solver — Euler (compressible)"
            } else {
                "Solver — LBM (incompressible)"
            };
            if ui.selectable_label(false, solver).clicked() {
                self.finish_gesture();
                self.deselect_all();
            }
        });

        ui.label(
            egui::RichText::new("Geometry (top first)")
                .small()
                .color(theme::INK_3),
        );
        // Snapshot rows first: the click/menu actions mutate the model,
        // so the row loop must not borrow it. Display order is REVERSED
        // model order — topmost (last rasterized, wins overlaps) first.
        let rows: Vec<(u64, String, bool, bool)> = self
            .model
            .objects
            .iter()
            .rev()
            .map(|o| {
                let kind = match &o.shape {
                    Shape::Line { .. } => "line",
                    Shape::Poly { closed: true, .. } => "polygon",
                    Shape::Poly { .. } => "polyline",
                    Shape::Rect { .. } => "rectangle",
                    Shape::Ellipse { .. } => "ellipse",
                    Shape::Stamp { .. } => "generated part",
                };
                (
                    o.id,
                    format!("{} #{:02} ({kind})", o.material.label(), o.id),
                    o.locked,
                    o.hidden,
                )
            })
            .collect();
        let display_ids: Vec<u64> = rows.iter().map(|r| r.0).collect();

        ui.indent("tree_objects", |ui| {
            for (id, label, locked, hidden) in rows {
                ui.horizontal(|ui| {
                    // Eye and lock toggles, undoable; hiding or locking
                    // also drops the object from the selection (it is no
                    // longer editable/visible).
                    let eye = if hidden { ph::EYE_SLASH } else { ph::EYE };
                    if ui
                        .small_button(eye)
                        .on_hover_text("Hidden objects aren't simulated")
                        .clicked()
                    {
                        self.toggle_flag(id, false);
                    }
                    let lock = if locked { ph::LOCK_SIMPLE } else { ph::LOCK_SIMPLE_OPEN };
                    if ui
                        .small_button(lock)
                        .on_hover_text(
                            "Locked objects can't be selected on the canvas or edited",
                        )
                        .clicked()
                    {
                        self.toggle_flag(id, true);
                    }
                    let mut text = egui::RichText::new(label);
                    if hidden {
                        text = text.weak();
                    }
                    let on = self.sel_contains(id);
                    let resp = ui.selectable_label(on, text);
                    if resp.clicked() {
                        self.finish_gesture();
                        let mods = ui.input(|i| i.modifiers);
                        if mods.shift {
                            // Range from the anchor row, display order.
                            let a = self
                                .tree_anchor
                                .and_then(|a| display_ids.iter().position(|&d| d == a));
                            let b = display_ids.iter().position(|&d| d == id);
                            if let (Some(a), Some(b)) = (a, b) {
                                let (lo, hi) = (a.min(b), a.max(b));
                                self.selected =
                                    display_ids[lo..=hi].to_vec();
                            } else {
                                self.select_only(id);
                                self.tree_anchor = Some(id);
                            }
                        } else if mods.command {
                            self.select_toggle(id);
                            self.tree_anchor = Some(id);
                        } else {
                            self.select_only(id);
                            self.tree_anchor = Some(id);
                        }
                    }
                    resp.context_menu(|ui| {
                        // Row actions target the clicked row's set: the
                        // whole selection when the row is part of it,
                        // else just this row.
                        if !self.sel_contains(id) {
                            self.select_only(id);
                            self.tree_anchor = Some(id);
                        }
                        if ui.button("Duplicate").clicked() {
                            self.finish_gesture();
                            self.duplicate_selected();
                            ui.close_menu();
                        }
                        if ui.button("Delete").clicked() {
                            self.delete_selected();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Bring to front").clicked() {
                            self.zorder_selected(2);
                            ui.close_menu();
                        }
                        if ui.button("Raise").clicked() {
                            self.zorder_selected(1);
                            ui.close_menu();
                        }
                        if ui.button("Lower").clicked() {
                            self.zorder_selected(-1);
                            ui.close_menu();
                        }
                        if ui.button("Send to back").clicked() {
                            self.zorder_selected(-2);
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .button(if locked { "Unlock" } else { "Lock" })
                            .clicked()
                        {
                            self.toggle_flag(id, true);
                            ui.close_menu();
                        }
                        if ui
                            .button(if hidden { "Show" } else { "Hide" })
                            .clicked()
                        {
                            self.toggle_flag(id, false);
                            ui.close_menu();
                        }
                    });
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

    /// Toggle one object's `locked` (true) or `hidden` (false) flag,
    /// undoably; engaging either drops the object from the selection.
    fn toggle_flag(&mut self, id: u64, lock: bool) {
        self.finish_gesture();
        let Some(i) = self.model.find(id) else { return };
        let before = self.model.objects[i].clone();
        let now_on = if lock {
            self.model.objects[i].locked = !before.locked;
            self.model.objects[i].locked
        } else {
            self.model.objects[i].hidden = !before.hidden;
            self.model.objects[i].hidden
        };
        self.model.record_modify(id, before);
        if now_on {
            self.deselect(id);
        }
    }
}
