//! The object inspector (selected-object properties) and the defaults
//! panel for newly drawn objects.

use crate::app::{Cmd, FlowPaintApp, Gesture, UiSnapshot};
use crate::model::{ObjMaterial, Shape};
use crate::sim::{RenderMode, SolverMode};
use eframe::egui;

use super::units::{fmt_factor, fmt_len, fmt_mach, fmt_speed};

use super::theme;

impl FlowPaintApp {
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
                        if self.selected.is_some()
                            && !matches!(self.gesture, Gesture::None)
                        {
                            // Mid-gesture: the object panel would fight
                            // the drag.
                            ui.label(theme::heading("Object"));
                            ui.label(
                                egui::RichText::new("(finish the gesture…)").weak(),
                            );
                        } else if let Some(id) = self.selected {
                            self.object_panel(ui, id, snap, cmds);
                        } else {
                            self.defaults_panel(ui, cmds);
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
            self.selected = None;
            return;
        };
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
        };
        ui.label(super::theme::heading(format!("Object — {kind}")));

        let is_stamp = matches!(obj.shape, Shape::Stamp { .. });
        let can_fill = matches!(obj.shape, Shape::Rect { .. } | Shape::Ellipse { .. });
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
                .add(egui::Slider::new(&mut obj.fan_mult, 0.2..=2.0).text("fan speed ×"))
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
        let (_, mut stage_rot, mut stage_scale) = match self.inspector_stage {
            Some(s) if s.0 == id => s,
            _ => (id, 0.0, 100.0),
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
                .on_hover_text("Scale the object about its center.")
                .changed()
            {
                obj.scale_by(stage_scale / p_old);
                changed = true;
            }
        });
        self.inspector_stage = Some((id, stage_rot, stage_scale));

        ui.horizontal(|ui| {
            if ui.button("Duplicate (Ctrl+D)").clicked() {
                self.duplicate_selected();
            }
            if ui.button("Delete (Del)").clicked() {
                self.selected = None;
                self.model.remove(id);
            }
        });
        // Deleting or duplicating invalidates `i`/`before`; bail out.
        if self.selected != Some(id) {
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
                        .range(0.2..=2.0)
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
                 Euler solver limits them at Mach 8, which almost never binds. \
                 In compressible mode the bell accelerates the jet through the \
                 throat.",
            )
            .small()
            .weak(),
        );
        changed
    }

    /// Defaults applied to newly drawn objects.
    pub(in crate::app) fn defaults_panel(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
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
                egui::Slider::new(&mut self.def_fan_mult, 0.2..=2.0).text("fan speed ×"),
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
