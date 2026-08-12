//! The object inspector (selected-object properties) and the defaults
//! panel for newly drawn objects.

use crate::app::{Cmd, FlowPaintApp, Gesture, Tool, UiSnapshot};
use crate::model::{ObjMaterial, Shape};
use crate::sim::{RenderMode, SolverMode};
use eframe::egui;

use super::units::{fmt_factor, fmt_len, fmt_mach, fmt_speed};

use egui_phosphor::regular as ph;

use super::theme;

impl FlowPaintApp {
    /// Mirror & linear-array rows, shared by the single/multi/group
    /// panels — selection ops live in the inspector next to
    /// Duplicate/Delete (there is no ribbon room at the 900 px
    /// minimum). Both add INDEPENDENT copies, one undo entry each.
    fn mirror_array_rows(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Mirror").small().color(theme::INK_3));
            if ui
                .button(ph::FLIP_HORIZONTAL)
                .on_hover_text(
                    "Mirror the selection across the domain's vertical \
                     centerline. The mirrored copies are independent \
                     objects, not instances — one undo step.",
                )
                .clicked()
            {
                self.mirror_selected_axis(true);
            }
            if ui
                .button(ph::FLIP_VERTICAL)
                .on_hover_text(
                    "Mirror the selection across the domain's horizontal \
                     centerline. The mirrored copies are independent \
                     objects, not instances — one undo step.",
                )
                .clicked()
            {
                self.mirror_selected_axis(false);
            }
            let on = self.tool == Tool::Mirror;
            if theme::toggle(ui, on, format!("{} Pick line…", ph::VECTOR_TWO))
                .on_hover_text(
                    "Mirror the selection across a picked line: drag the \
                     line's two points on the canvas (they snap to \
                     endpoints, midpoints and centers like any pick). The \
                     mirrored copies are independent objects, not \
                     instances — one undo step.",
                )
                .clicked()
                && !on
            {
                self.finish_gesture();
                self.tool = Tool::Mirror;
                self.status =
                    "Mirror: drag the line to mirror across (snaps apply; \
                     Shift angle-snaps, Esc cancels)."
                        .into();
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Array").small().color(theme::INK_3));
            if ui
                .button(format!("{} Apply", ph::SQUARES_FOUR))
                .on_hover_text(
                    "Linear array: add count−1 copies of the selection, \
                     each stepped by the spacing. The copies are \
                     independent objects, not instances — one undo step.",
                )
                .clicked()
            {
                self.array_selected();
            }
            ui.add(egui::DragValue::new(&mut self.array_count).range(2..=64))
                .on_hover_text("Total count, the original included");
            ui.add(
                egui::DragValue::new(&mut self.array_step[0])
                    .speed(0.5)
                    .suffix(" X"),
            )
            .on_hover_text("Step between neighbours, in cells");
            ui.add(
                egui::DragValue::new(&mut self.array_step[1])
                    .speed(0.5)
                    .suffix(" Y"),
            )
            .on_hover_text("Step between neighbours, in cells");
        });
        let ps = self.phys_cache;
        theme::derived(
            ui,
            format!(
                "step = {} , {}",
                fmt_len(ps.len_m(self.array_step[0])),
                fmt_len(ps.len_m(self.array_step[1]))
            ),
        );
    }
    /// The settings panel: the property block for whatever the tree has
    /// selected. The three-way branch (mid-gesture placeholder, object
    /// inspector, defaults) moved here unchanged from the old control
    /// column — the mid-gesture guard exists because the inspector
    /// fights an active drag.
    pub(in crate::app) fn settings_panel(
        &mut self,
        ctx: &egui::Context,
        snap: UiSnapshot,
        cmds: &mut Vec<Cmd>,
    ) {
        egui::SidePanel::left("settings")
            .resizable(true)
            .default_width(theme::dim::SETTINGS_WIDTH)
            .show(ctx, |ui| {
                ui.label(theme::heading("Settings"));
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let is_group = self
                            .single_sel()
                            .and_then(|id| self.model.find(id))
                            .map(|i| {
                                matches!(
                                    self.model.objects[i].shape,
                                    crate::model::Shape::Group { .. }
                                )
                            })
                            .unwrap_or(false);
                        if !self.selected.is_empty()
                            && !matches!(self.gesture, Gesture::None)
                        {
                            // Mid-gesture: the object panel would fight
                            // the drag.
                            ui.label(theme::heading("Object"));
                            ui.label(
                                egui::RichText::new("(finish the gesture…)").weak(),
                            );
                        } else if self.selected.len() > 1 {
                            self.multi_panel(ui, cmds);
                        } else if is_group {
                            self.group_panel(ui, snap);
                        } else if let Some(id) = self.single_sel() {
                            self.object_panel(ui, id, snap, cmds);
                        } else {
                            self.defaults_panel(ui, snap, cmds);
                        }
                    });
            });
    }

    /// Properties of the selected object: every knob edits the live model
    /// (undoably, with per-widget coalescing).
    pub(in crate::app) fn object_panel(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        snap: UiSnapshot,
        cmds: &mut Vec<Cmd>,
    ) {
        let Some(i) = self.model.find(id) else {
            self.deselect_all();
            return;
        };
        if self.model.eff_locked(id) {
            ui.label(super::theme::heading("Object — locked"));
            ui.label(
                egui::RichText::new(
                    "Locked objects can't be edited or moved (a locked \
                     enclosing group locks its members too). Unlock from \
                     the model tree (or below).",
                )
                .small()
                .weak(),
            );
            if self.model.objects[i].locked && ui.button("Unlock").clicked() {
                let before = self.model.objects[i].clone();
                self.model.objects[i].locked = false;
                self.model.record_modify(id, before);
            }
            return;
        }
        let before = self.model.objects[i].clone();
        let mut obj = before.clone();
        let mut changed = false;

        let kind = match &obj.shape {
            Shape::Line { .. } => "Line",
            Shape::Poly { closed: true, .. } => "Polygon",
            Shape::Poly { .. } => "Polyline",
            Shape::Rect { .. } => "Rectangle",
            Shape::Ellipse { .. } => "Ellipse",
            Shape::Stamp { .. } => "Generated part",
            Shape::Group { .. } => "Group", // routed to group_panel
        };
        ui.label(super::theme::heading(format!("Object — {kind}")));

        let is_stamp = matches!(obj.shape, Shape::Stamp { .. });
        // U4: closed polylines fill too (the plan's "probably most of
        // what a fill tool needs to be").
        let can_fill = matches!(
            obj.shape,
            Shape::Rect { .. } | Shape::Ellipse { .. } | Shape::Poly { closed: true, .. }
        );
        let ps = self.phys_cache;

        if !is_stamp {
            let mats: [(ObjMaterial, &str); 4] = [
                (ObjMaterial::Wall, "Solid, no-slip"),
                (ObjMaterial::Fan, "Blows along the shape"),
                (ObjMaterial::Smoke, "Passive dye emitter"),
                (ObjMaterial::Drain, "Lets flow leave"),
            ];
            ui.horizontal_wrapped(|ui| {
                for (m, tip) in mats {
                    let resp = super::theme::toggle(ui, obj.material == m, m.label())
                        .on_hover_text(tip);
                    if resp.clicked() && obj.material != m {
                        obj.material = m;
                        changed = true;
                        if m == ObjMaterial::Smoke {
                            cmds.push(Cmd::SetRenderMode(RenderMode::Dye));
                        }
                    }
                }
            });

            if can_fill && ui.checkbox(&mut obj.filled, "Filled").changed() {
                changed = true;
            }
            if !(can_fill && obj.filled) {
                ui.horizontal(|ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut obj.thickness)
                                .range(1.0..=24.0)
                                .speed(0.1)
                                .suffix(" cells"),
                        )
                        .changed();
                    ui.label("thickness");
                });
                super::theme::derived(
                    ui,
                    format!("= {}", fmt_len(ps.len_m(obj.thickness))),
                );
            }
        }

        // Fan physics. A generated part that carries fan cells (a rocket
        // nozzle's chamber inlet) is an ENGINE in the user's mental model
        // and gets its own group; a hand-placed Fan object keeps the
        // generic fan block below, unchanged. Stamps get no blow-direction
        // control, by design rather than omission: stamp fan vectors are
        // locked to the stamp's geometric angle (see the rasterizer's
        // stamp arm in model.rs — rotating the chamber flow independently
        // of the bell would aim thrust into the converging wall). Aiming
        // is done with the object Rotate controls.
        let stamp_fan_mag = match &obj.shape {
            Shape::Stamp { raster, .. } => raster
                .fan
                .iter()
                .map(|f| (f[0] * f[0] + f[1] * f[1]).sqrt())
                .fold(0.0f32, f32::max),
            _ => 0.0,
        };
        if stamp_fan_mag > 0.0 {
            changed |= self.engine_group(ui, snap, &mut obj, stamp_fan_mag);
        } else if obj.material == ObjMaterial::Fan {
            changed |= ui
                .add(
                    egui::Slider::new(
                        &mut obj.fan_mult,
                        crate::sim::fan_mult_range(snap.solver),
                    )
                    .text("fan speed ×"),
                )
                .on_hover_text("Multiplier on the global flow speed")
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut obj.fan_gust, 0.0..=1.0).text("gustiness"))
                .on_hover_text(
                    "Time-varying wander in the fan's direction and strength — \
                     0 is steady, 1 is a blustery day",
                )
                .changed();
            // Chained shapes blow along their segments; solid shapes have
            // a free direction.
            if matches!(obj.shape, Shape::Rect { .. } | Shape::Ellipse { .. })
                && obj.filled
            {
                let mut deg = obj.fan_angle.to_degrees();
                if ui
                    .add(
                        egui::Slider::new(&mut deg, -180.0..=180.0)
                            .text("blow direction °"),
                    )
                    .changed()
                {
                    obj.fan_angle = deg.to_radians();
                    changed = true;
                }
            }
        }
        if obj.material == ObjMaterial::Fan || obj.material == ObjMaterial::Smoke {
            // RGB picker, not srgba: the alpha/blend controls would write
            // premultiplied channels back into smoke_rgb (alpha is fixed
            // by the rasterizer per material).
            ui.horizontal(|ui| {
                ui.label("Smoke color:");
                let mut rgb = obj.smoke_rgb;
                if ui.color_edit_button_rgb(&mut rgb).changed() {
                    obj.smoke_rgb = rgb;
                    changed = true;
                }
            });
        }

        // Staged-delta transform fields, reset per selection: a selection
        // has no single intrinsic angle or size, so an absolute angle
        // field cannot extend to multi-selections rotating about a common
        // pivot (U3). Only the change since the last frame is applied.
        let key = vec![id];
        let (mut stage_rot, mut stage_scale) = match &self.inspector_stage {
            Some((k, r, s)) if *k == key => (*r, *s),
            _ => (0.0, 100.0),
        };
        ui.horizontal(|ui| {
            ui.label("Rotate");
            let r_old = stage_rot;
            if ui
                .add(
                    egui::DragValue::new(&mut stage_rot)
                        .range(-3600.0..=3600.0)
                        .speed(1.0)
                        .suffix("°"),
                )
                .on_hover_text("Rotate the object about its center.")
                .changed()
            {
                obj.rotate_by((stage_rot - r_old).to_radians());
                changed = true;
            }
            if ui.small_button("+90°").clicked() {
                obj.rotate_by(90.0f32.to_radians());
                stage_rot += 90.0;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Scale");
            let p_old = stage_scale;
            if ui
                .add(
                    // The lower bound keeps the applied ratio away from 0.
                    egui::DragValue::new(&mut stage_scale)
                        .range(5.0..=2000.0)
                        .speed(1.0)
                        .suffix(" %"),
                )
                .on_hover_text(if is_stamp {
                    // The plan's explicit tooltip: honesty over a
                    // silently dropped axis.
                    "Scale the stamp about its center. Scaling is uniform: \
                     non-uniform scaling of a raster stamp is out of scope."
                } else {
                    "Scale the object about its center (uniform; reshape \
                     with the canvas handles for per-axis edits)."
                })
                .changed()
            {
                obj.scale_by(stage_scale / p_old);
                changed = true;
            }
        });
        self.inspector_stage = Some((key, stage_rot, stage_scale));
        // Center X/Y: the object's WORLD centre in cells (the canonical
        // grid coordinate; metres on the derived line).
        let abs = self.model.parent_abs(id);
        let c0 = abs.apply(obj.center());
        let mut cw = c0;
        ui.horizontal(|ui| {
            ui.label("Center");
            let cx = ui
                .add(egui::DragValue::new(&mut cw[0]).speed(0.5).suffix(" X"))
                .changed();
            let cy = ui
                .add(egui::DragValue::new(&mut cw[1]).speed(0.5).suffix(" Y"))
                .changed();
            if cx || cy {
                let d = abs.inverse().apply_vec([cw[0] - c0[0], cw[1] - c0[1]]);
                obj.translate(d);
                changed = true;
            }
        });
        super::theme::derived(
            ui,
            format!("= {} , {}", fmt_len(ps.len_m(cw[0])), fmt_len(ps.len_m(cw[1]))),
        );

        ui.horizontal(|ui| {
            if ui.button("Duplicate (Ctrl+D)").clicked() {
                self.duplicate_selected();
            }
            if ui.button("Delete (Del)").clicked() {
                self.delete_selected();
            }
        });
        self.mirror_array_rows(ui);
        // Deleting, duplicating, mirroring or arraying invalidates
        // `i`/`before`; bail out.
        if self.single_sel() != Some(id) {
            return;
        }

        if changed {
            if let Some(i) = self.model.find(id) {
                self.model.objects[i] = obj;
                self.model.record_modify_coalesced(id, before);
            }
        } else {
            ui.label(
                egui::RichText::new(
                    "Drag the object to move it; drag its handles to reshape. \
                     Arrows nudge, Esc deselects.",
                )
                .small()
                .weak(),
            );
        }
    }

    /// The multi-selection panel: shared properties across the set, with
    /// a mixed-value indicator where members disagree — a property is
    /// never silently overwritten by the first object's value; only an
    /// actual edit applies to the whole (editable) selection, as one
    /// coalesced undo entry. Rotate/scale about a common pivot is U3.
    fn multi_panel(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let ids = self.selected.clone();
        let objs: Vec<usize> = ids.iter().filter_map(|&id| self.model.find(id)).collect();
        ui.label(super::theme::heading(format!("Objects — {} selected", objs.len())));
        let locked_n = objs
            .iter()
            .filter(|&&i| self.model.eff_locked(self.model.objects[i].id))
            .count();
        if locked_n > 0 {
            super::theme::derived(
                ui,
                format!("{locked_n} locked (edits skip them)"),
            );
        }

        // Material: highlighted only when the whole selection agrees.
        let mats: [(ObjMaterial, &str); 4] = [
            (ObjMaterial::Wall, "Solid, no-slip"),
            (ObjMaterial::Fan, "Blows along the shape"),
            (ObjMaterial::Smoke, "Passive dye emitter"),
            (ObjMaterial::Drain, "Lets flow leave"),
        ];
        let non_stamp: Vec<usize> = objs
            .iter()
            .copied()
            .filter(|&i| !matches!(self.model.objects[i].shape, Shape::Stamp { .. }))
            .collect();
        if !non_stamp.is_empty() {
            let first = self.model.objects[non_stamp[0]].material;
            let uniform_mat = non_stamp
                .iter()
                .all(|&i| self.model.objects[i].material == first);
            let mut set_mat = None;
            ui.horizontal_wrapped(|ui| {
                for (m, tip) in mats {
                    let on = uniform_mat && first == m;
                    if super::theme::toggle(ui, on, m.label())
                        .on_hover_text(tip)
                        .clicked()
                    {
                        set_mat = Some(m);
                    }
                }
            });
            if !uniform_mat {
                super::theme::derived(ui, "material: (mixed)".into());
            }
            if let Some(m) = set_mat {
                // Stamps carry their own cell types; leave them alone.
                self.edit_selection(|o| {
                    if !matches!(o.shape, Shape::Stamp { .. }) {
                        o.material = m;
                    }
                });
                if m == ObjMaterial::Smoke {
                    cmds.push(Cmd::SetRenderMode(RenderMode::Dye));
                }
            }
        }

        // Thickness: mixed indicator, edits apply to the whole set.
        let thick: Vec<f32> = objs
            .iter()
            .map(|&i| self.model.objects[i].thickness)
            .collect();
        if let Some(&t0) = thick.first() {
            let uniform_t = thick.iter().all(|&t| (t - t0).abs() < 1e-3);
            // Seed the widget from the PRIMARY object; nothing is applied
            // until the user actually edits the value.
            let mut t = self
                .primary_sel()
                .and_then(|id| self.model.find(id))
                .map(|i| self.model.objects[i].thickness)
                .unwrap_or(t0);
            ui.horizontal(|ui| {
                let changed = ui
                    .add(
                        egui::DragValue::new(&mut t)
                            .range(1.0..=24.0)
                            .speed(0.1)
                            .suffix(" cells"),
                    )
                    .changed();
                ui.label(if uniform_t { "thickness" } else { "thickness (mixed)" });
                if changed {
                    self.edit_selection(|o| o.thickness = t);
                }
            });
            if uniform_t {
                let ps = self.phys_cache;
                super::theme::derived(ui, format!("= {}", fmt_len(ps.len_m(t0))));
            }
        }

        // Rotate/scale/centre about the common pivot (U3).
        self.selection_transform_rows(ui);

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Group (Ctrl+G)").clicked() {
                self.group_selected();
            }
            if ui.button("Duplicate (Ctrl+D)").clicked() {
                self.duplicate_selected();
            }
            if ui.button("Delete (Del)").clicked() {
                self.delete_selected();
            }
        });
        self.mirror_array_rows(ui);
        // Z-order for the set (also in the tree's context menu).
        ui.horizontal(|ui| {
            if ui.button("Front").on_hover_text("Ctrl+Shift+]").clicked() {
                self.zorder_selected(2);
            }
            if ui.button("Raise").on_hover_text("Ctrl+]").clicked() {
                self.zorder_selected(1);
            }
            if ui.button("Lower").on_hover_text("Ctrl+[").clicked() {
                self.zorder_selected(-1);
            }
            if ui.button("Back").on_hover_text("Ctrl+Shift+[").clicked() {
                self.zorder_selected(-2);
            }
        });
        ui.label(
            egui::RichText::new(
                "Drag any selected object to move the set; arrows nudge \
                 (Shift = coarse). On the canvas: corner handles scale, \
                 the top handle rotates, drag the crosshair to move the \
                 pivot.",
            )
            .small()
            .weak(),
        );
    }

    /// Rotate / scale / centre for the whole selection about the common
    /// pivot (the gizmo's crosshair; selection-bounds centre until it
    /// is dragged). Staged deltas — a selection has no single intrinsic
    /// angle — applied to the OUTERMOST editable members as one
    /// coalesced undo entry. Scaling is uniform by design: a similarity
    /// is the only transform that survives nested rotated groups
    /// without shear, and raster stamps only carry a single scale.
    fn selection_transform_rows(&mut self, ui: &mut egui::Ui) {
        let Some(pivot) = self.transform_pivot() else { return };
        let key = self.selected.clone();
        let (mut stage_rot, mut stage_scale) = match &self.inspector_stage {
            Some((k, r, s)) if *k == key => (*r, *s),
            _ => (0.0, 100.0),
        };
        ui.horizontal(|ui| {
            ui.label("Rotate");
            let r_old = stage_rot;
            let mut da = 0.0f32;
            if ui
                .add(
                    egui::DragValue::new(&mut stage_rot)
                        .range(-3600.0..=3600.0)
                        .speed(1.0)
                        .suffix("°"),
                )
                .on_hover_text("Rotate the selection about the pivot.")
                .changed()
            {
                da = (stage_rot - r_old).to_radians();
            }
            if ui.small_button("+90°").clicked() {
                da = 90.0f32.to_radians();
                stage_rot += 90.0;
            }
            if da != 0.0 {
                self.transform_selection_world(|m, id| m.rotate_world(id, pivot, da));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Scale");
            let p_old = stage_scale;
            if ui
                .add(
                    egui::DragValue::new(&mut stage_scale)
                        .range(5.0..=2000.0)
                        .speed(1.0)
                        .suffix(" %"),
                )
                .on_hover_text(
                    "Scale the selection about the pivot. Scaling is uniform: \
                     non-uniform scaling of a raster stamp is out of scope.",
                )
                .changed()
            {
                let f = stage_scale / p_old;
                self.transform_selection_world(|m, id| m.scale_world(id, pivot, f));
            }
        });
        self.inspector_stage = Some((key, stage_rot, stage_scale));
        // Centre of the selection's world bounds, in cells.
        if let Some(b) = self.selection_world_bounds() {
            let c0 = [(b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5];
            let mut cw = c0;
            ui.horizontal(|ui| {
                ui.label("Center");
                let cx = ui
                    .add(egui::DragValue::new(&mut cw[0]).speed(0.5).suffix(" X"))
                    .changed();
                let cy = ui
                    .add(egui::DragValue::new(&mut cw[1]).speed(0.5).suffix(" Y"))
                    .changed();
                if cx || cy {
                    let d = [cw[0] - c0[0], cw[1] - c0[1]];
                    self.transform_selection_world(|m, id| m.translate_world(id, d));
                }
            });
            let ps = self.phys_cache;
            super::theme::derived(
                ui,
                format!(
                    "= {} , {}",
                    fmt_len(ps.len_m(cw[0])),
                    fmt_len(ps.len_m(cw[1]))
                ),
            );
        }
    }

    /// The panel for a single selected GROUP: subtree summary, the
    /// common-pivot transforms, the Engine block when the group is a
    /// generated engine (a Fan child driving a stamp child — the parent
    /// link replaces the old raster inspection), and ungroup.
    fn group_panel(&mut self, ui: &mut egui::Ui, snap: UiSnapshot) {
        let Some(gid) = self.single_sel() else { return };
        let n = self.model.subtree_ids(gid).len() - 1;
        ui.label(super::theme::heading(format!("Group — {n} objects")));
        if self.model.eff_locked(gid) {
            ui.label(
                egui::RichText::new(
                    "Locked groups can't be edited or moved. Unlock from \
                     the model tree.",
                )
                .small()
                .weak(),
            );
            return;
        }

        // Engine block: a Fan-material child powering a stamp child.
        let fan_child = self.model.children_of(gid).into_iter().find(|&c| {
            self.model
                .find(c)
                .map(|i| {
                    self.model.objects[i].material == ObjMaterial::Fan
                        && !matches!(self.model.objects[i].shape, Shape::Group { .. })
                })
                .unwrap_or(false)
        });
        let has_stamp_child = self.model.children_of(gid).into_iter().any(|c| {
            self.model
                .find(c)
                .map(|i| matches!(self.model.objects[i].shape, Shape::Stamp { .. }))
                .unwrap_or(false)
        });
        if let (Some(fan_id), true) = (fan_child, has_stamp_child) {
            if let Some(fi) = self.model.find(fan_id) {
                let before = self.model.objects[fi].clone();
                let mut fan = before.clone();
                let drive = self.engine_group(ui, snap, &mut fan, 1.0);
                if drive {
                    self.model.objects[fi] = fan;
                    self.model.record_modify_coalesced(fan_id, before);
                }
            }
        }

        self.selection_transform_rows(ui);

        ui.horizontal(|ui| {
            if ui.button("Ungroup (Ctrl+Shift+G)").clicked() {
                self.ungroup_selected();
            }
            if ui.button("Duplicate (Ctrl+D)").clicked() {
                self.duplicate_selected();
            }
            if ui.button("Delete (Del)").clicked() {
                self.delete_selected();
            }
        });
        self.mirror_array_rows(ui);
        ui.label(
            egui::RichText::new(
                "Double-click a member on the canvas to enter the group \
                 and edit it alone (Esc leaves). Deleting the group \
                 deletes everything inside — one undo restores it.",
            )
            .small()
            .weak(),
        );
    }

    /// The Engine group for a generated part with fan cells: chamber
    /// drive, gustiness, and a readout naming which speed limit binds.
    /// There is no single editable cap — six clamps in three layers, and
    /// the binding ones are shader constants — so the panel reads out
    /// the truth instead of pretending a field exists.
    fn engine_group(
        &self,
        ui: &mut egui::Ui,
        snap: UiSnapshot,
        obj: &mut crate::model::SketchObject,
        stamp_fan_mag: f32,
    ) -> bool {
        let mut changed = false;
        ui.label(super::theme::heading("Engine")).on_hover_text(
            "To aim the nozzle, use the Rotate controls below. The jet \
             direction is locked to the part's geometry.",
        );
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut obj.fan_mult)
                        .range(crate::sim::fan_mult_range(snap.solver))
                        .speed(0.01)
                        .suffix(" ×"),
                )
                .on_hover_text(
                    "Set the chamber drive as a multiple of the inlet speed.",
                )
                .changed();
            ui.label("chamber drive");
        });
        changed |= ui
            .add(egui::Slider::new(&mut obj.fan_gust, 0.0..=1.0).text("gustiness"))
            .on_hover_text("Add slow variation to the jet. Zero gives a steady jet.")
            .changed();
        // The rasterizer recolors the stamp's fan-cell dye with the
        // object's smoke color, so the picker works for engines too.
        // RGB picker, not srgba: the alpha/blend controls would write
        // premultiplied channels back into smoke_rgb (dye alpha is baked).
        ui.horizontal(|ui| {
            ui.label("Plume color:");
            let mut rgb = obj.smoke_rgb;
            if ui
                .color_edit_button_rgb(&mut rgb)
                .on_hover_text("Set the color of the engine plume.")
                .changed()
            {
                obj.smoke_rgb = rgb;
                changed = true;
            }
        });

        // Which layer binds, and by how much the request exceeds it.
        let ps = self.phys_cache;
        let drive = stamp_fan_mag * obj.fan_mult;
        match snap.solver {
            SolverMode::Lbm => {
                let req = snap.flow * drive; // lattice speed at the inlet cells
                super::theme::derived(
                    ui,
                    format!("chamber inlet = {}", fmt_speed(ps.u_phys(req.min(0.3)))),
                );
                if req > 0.3 {
                    super::theme::derived(
                        ui,
                        format!(
                            "limit binds: LBM 0.3 lattice ({} requested)",
                            fmt_factor(req / 0.3)
                        ),
                    );
                } else {
                    super::theme::derived(ui, "no speed limit binds".into());
                }
            }
            SolverMode::Euler => {
                let req_m = snap.mach * drive;
                super::theme::derived(
                    ui,
                    format!(
                        "chamber inlet = M {} = {}",
                        fmt_mach(req_m.min(8.0)),
                        fmt_speed(req_m.min(8.0) * self.fluid_a)
                    ),
                );
                if req_m > 8.0 {
                    super::theme::derived(
                        ui,
                        format!(
                            "limit binds: Euler M 8 ({} requested)",
                            fmt_factor(req_m / 8.0)
                        ),
                    );
                } else {
                    super::theme::derived(ui, "no speed limit binds (Euler: M 8)".into());
                }
            }
        }
        ui.label(
            egui::RichText::new(
                "The LBM solver limits inlet cells to 0.3 lattice speed. The \
                 Euler solver limits them at Mach 8; a strong drive at a high \
                 inlet Mach can reach that limit. In compressible mode the \
                 bell accelerates the jet through the throat.",
            )
            .small()
            .weak(),
        );
        changed
    }

    /// Defaults applied to newly drawn objects.
    pub(in crate::app) fn defaults_panel(
        &mut self,
        ui: &mut egui::Ui,
        snap: UiSnapshot,
        cmds: &mut Vec<Cmd>,
    ) {
        ui.label(super::theme::heading("New objects"));
        let mats: [(ObjMaterial, &str); 4] = [
            (ObjMaterial::Wall, "Solid, no-slip"),
            (ObjMaterial::Fan, "Blows along the shape"),
            (ObjMaterial::Smoke, "Passive dye emitter"),
            (ObjMaterial::Drain, "Lets flow leave"),
        ];
        ui.horizontal_wrapped(|ui| {
            for (m, tip) in mats {
                let resp = super::theme::toggle(ui, self.def_material == m, m.label())
                    .on_hover_text(tip);
                if resp.clicked() {
                    self.def_material = m;
                    // Smoke is only visible in the Smoke view; switch so
                    // the first stroke gives immediate feedback.
                    if m == ObjMaterial::Smoke {
                        cmds.push(Cmd::SetRenderMode(RenderMode::Dye));
                    }
                }
            }
        });
        let ps = self.phys_cache;
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.def_thickness)
                    .range(1.0..=24.0)
                    .speed(0.1)
                    .suffix(" cells"),
            )
            .on_hover_text("Lines, polylines and shape outlines draw at this thickness");
            ui.label("thickness");
        });
        super::theme::derived(ui, format!("= {}", fmt_len(ps.len_m(self.def_thickness))));
        ui.checkbox(&mut self.def_filled, "Filled rect / ellipse")
            .on_hover_text("Off = SolidWorks-style outlines at the set thickness");
        if self.def_material == ObjMaterial::Fan {
            ui.add(
                egui::Slider::new(
                    &mut self.def_fan_mult,
                    crate::sim::fan_mult_range(snap.solver),
                )
                .text("fan speed ×"),
            );
            ui.add(
                egui::Slider::new(&mut self.def_fan_gust, 0.0..=1.0).text("gustiness"),
            );
        }
        if self.def_material == ObjMaterial::Fan || self.def_material == ObjMaterial::Smoke
        {
            ui.horizontal(|ui| {
                ui.label("Smoke color:");
                ui.color_edit_button_srgba(&mut self.def_smoke);
            });
        }
    }
}
