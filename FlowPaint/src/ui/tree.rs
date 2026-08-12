//! The model tree: the scene's real contents as a selectable outline —
//! a domain root with the solver child, then one node per SketchObject,
//! TOPMOST FIRST (list order is z-order; later objects win overlaps),
//! nested under their group nodes (U3). Clicking writes the existing
//! `FlowPaintApp::selected` set — plain click selects, Ctrl toggles,
//! Shift ranges from the last-clicked row (display order). Per-row
//! eye/lock toggles manage `hidden`/`locked` (both persist; a group's
//! flag governs its whole subtree). Rows drag-to-reparent: drop a row
//! on a group to move it inside, on a leaf to make it that leaf's
//! sibling, on the Geometry header to move it to the root — a group can
//! never be dropped into its own subtree (the model refuses cycles).

use crate::app::{Cmd, FlowPaintApp, Tool};
use crate::model::Shape;
use crate::sim::MAX_PROBES;
use eframe::egui;
use egui_phosphor::regular as ph;

use super::theme;
use super::units::fmt_len;

/// One display row, TOPMOST FIRST within each nesting level.
struct TreeRow {
    id: u64,
    label: String,
    locked: bool,
    hidden: bool,
    depth: usize,
    is_group: bool,
}

impl FlowPaintApp {
    pub(in crate::app) fn tree_panel(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
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
                    .show(ui, |ui| self.tree_contents(ui, cmds));
            });
    }

    /// Build the nested display list: roots topmost-first, each group's
    /// children topmost-first beneath it.
    fn collect_rows(&self, parent: Option<u64>, depth: usize, out: &mut Vec<TreeRow>) {
        if depth > 64 {
            return; // corrupt-file cycle guard; loads also sanitize
        }
        for o in self.model.objects.iter().rev() {
            if o.parent != parent {
                continue;
            }
            let (label, is_group) = match &o.shape {
                Shape::Group { .. } => (
                    format!(
                        "Group #{:02} ({} objects)",
                        o.id,
                        self.model.subtree_ids(o.id).len() - 1
                    ),
                    true,
                ),
                s => {
                    let kind = match s {
                        Shape::Line { .. } => "line",
                        Shape::Poly { closed: true, .. } => "polygon",
                        Shape::Poly { .. } => "polyline",
                        Shape::Rect { .. } => "rectangle",
                        Shape::Ellipse { .. } => "ellipse",
                        Shape::Stamp { .. } => "generated part",
                        Shape::Group { .. } => unreachable!(),
                        Shape::Arc { .. } => "arc",
                        Shape::Spline { closed: true, .. } => "closed spline",
                        Shape::Spline { .. } => "spline",
                    };
                    (format!("{} #{:02} ({kind})", o.material.label(), o.id), false)
                }
            };
            out.push(TreeRow {
                id: o.id,
                label,
                locked: o.locked,
                hidden: o.hidden,
                depth,
                is_group,
            });
            if matches!(o.shape, Shape::Group { .. }) {
                self.collect_rows(Some(o.id), depth + 1, out);
            }
        }
    }

    fn tree_contents(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
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

        let header = ui.label(
            egui::RichText::new("Geometry (top first)")
                .small()
                .color(theme::INK_3),
        );
        // Dropping a row on the header moves it to the root level.
        if let Some(dragged) = header.dnd_release_payload::<u64>() {
            self.reparent_row(*dragged, None);
        }

        // Snapshot rows first: the click/menu actions mutate the model,
        // so the row loop must not borrow it.
        let mut rows: Vec<TreeRow> = Vec::new();
        self.collect_rows(None, 0, &mut rows);
        let display_ids: Vec<u64> = rows.iter().map(|r| r.id).collect();

        ui.indent("tree_objects", |ui| {
            for row in rows {
                self.tree_row(ui, row, &display_ids);
            }
        });

        self.tree_probes(ui, cmds);
    }

    /// One object row: eye/lock toggles, the drag-source label with
    /// selection semantics, the context menu, and drop handling.
    fn tree_row(&mut self, ui: &mut egui::Ui, row: TreeRow, display_ids: &[u64]) {
        let TreeRow { id, label, locked, hidden, depth, is_group } = row;
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 12.0);
            // Eye and lock toggles, undoable; hiding or locking also
            // drops the object from the selection. On a group they
            // govern the whole subtree (effective flags).
            let eye = if hidden { ph::EYE_SLASH } else { ph::EYE };
            if ui
                .small_button(eye)
                .on_hover_text(if is_group {
                    "Hidden groups (and everything inside) aren't simulated"
                } else {
                    "Hidden objects aren't simulated"
                })
                .clicked()
            {
                self.toggle_flag(id, false);
            }
            let lock = if locked { ph::LOCK_SIMPLE } else { ph::LOCK_SIMPLE_OPEN };
            if ui
                .small_button(lock)
                .on_hover_text(if is_group {
                    "Locked groups (and everything inside) can't be selected \
                     on the canvas or edited"
                } else {
                    "Locked objects can't be selected on the canvas or edited"
                })
                .clicked()
            {
                self.toggle_flag(id, true);
            }
            let mut text = egui::RichText::new(label);
            if hidden || self.model.eff_hidden(id) {
                text = text.weak();
            }
            let on = self.sel_contains(id);
            // The label is a drag source (drag-to-reparent) and a drop
            // target; plain clicks keep the selection semantics. NOT
            // egui's `dnd_drag_source` — its overlay interact registers
            // on top of the row and steals the clicks; sensing
            // click+drag on the label itself keeps both behaviors
            // (drags engage only past the movement threshold).
            let resp = ui
                .selectable_label(on, text)
                .interact(egui::Sense::click_and_drag());
            if resp.drag_started() {
                egui::DragAndDrop::set_payload(ui.ctx(), id);
            }
            if resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if let Some(dragged) = resp.dnd_release_payload::<u64>() {
                if *dragged != id {
                    // Onto a group: into it. Onto a leaf: alongside it.
                    let target = if is_group {
                        Some(id)
                    } else {
                        self.model
                            .find(id)
                            .and_then(|i| self.model.objects[i].parent)
                    };
                    self.reparent_row(*dragged, target);
                }
            } else if resp.dnd_hover_payload::<u64>().is_some() {
                ui.painter().rect_stroke(
                    resp.rect.expand(1.0),
                    theme::RADIUS,
                    egui::Stroke::new(1.0, theme::SEL),
                );
            }
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
                        self.selected = display_ids[lo..=hi].to_vec();
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
                // Row actions target the clicked row's set: the whole
                // selection when the row is part of it, else this row.
                if !self.sel_contains(id) {
                    self.select_only(id);
                    self.tree_anchor = Some(id);
                }
                if ui.button("Group (Ctrl+G)").clicked() {
                    self.group_selected();
                    ui.close_menu();
                }
                if is_group && ui.button("Ungroup (Ctrl+Shift+G)").clicked() {
                    self.ungroup_selected();
                    ui.close_menu();
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
                if ui.button(if locked { "Unlock" } else { "Lock" }).clicked() {
                    self.toggle_flag(id, true);
                    ui.close_menu();
                }
                if ui.button(if hidden { "Show" } else { "Hide" }).clicked() {
                    self.toggle_flag(id, false);
                    ui.close_menu();
                }
            });
        });
    }

    /// Apply a tree drag-drop reparent, surfacing the model's refusal
    /// (cycle prevention) in the status line.
    fn reparent_row(&mut self, id: u64, target: Option<u64>) {
        self.finish_gesture();
        match self.model.reparent(id, target) {
            Ok(()) => {
                self.entered_group = None;
                self.status = match target {
                    Some(g) => format!("Moved #{id:02} into group #{g:02}."),
                    None => format!("Moved #{id:02} to the top level."),
                };
            }
            Err(e) => self.status = format!("Can't move: {e}."),
        }
    }

    /// The probe section (plan v4.1, T2-B): persistent point probes as
    /// tree entries with a delete action, plus the place-probe arming
    /// button. Probes are not sketch objects — they live in
    /// `Settings.probes` (U3 fold); this reads the per-frame snapshot
    /// and edits through `Cmd`.
    fn tree_probes(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        ui.label(
            egui::RichText::new("Probes")
                .small()
                .color(theme::INK_3),
        );
        let ps = self.phys_cache;
        let rows: Vec<(u32, String, String)> = self
            .probe_ui
            .rows
            .iter()
            .map(|&(id, pos)| {
                (
                    id,
                    format!("Probe P{} ({:.0}, {:.0})", id, pos[0], pos[1]),
                    format!(
                        "at {} , {} from the top-left corner",
                        fmt_len(ps.len_m(pos[0])),
                        fmt_len(ps.len_m(pos[1]))
                    ),
                )
            })
            .collect();
        let count = rows.len();
        let arming = self.probe_arming;
        ui.indent("tree_probes", |ui| {
            for (id, label, hover) in &rows {
                let resp = ui.selectable_label(false, label).on_hover_text(hover);
                resp.context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        cmds.push(Cmd::RemoveProbe(*id));
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
                self.probe_arming = true;
                // Placing a probe must not also draw a shape.
                self.finish_gesture();
                self.tool = Tool::Select;
                self.status =
                    "Click the canvas to place the probe (Esc cancels).".into();
            }
        });
    }

    /// Toggle one object's `locked` (true) or `hidden` (false) flag,
    /// undoably; engaging either drops the object (and, for a group,
    /// its subtree) from the selection.
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
            for sid in self.model.subtree_ids(id) {
                self.deselect(sid);
            }
        }
    }
}
