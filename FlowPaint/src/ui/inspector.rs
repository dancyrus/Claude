//! The object inspector (selected-object properties) and the defaults
//! panel for newly drawn objects.

use crate::app::{Cmd, FlowPaintApp, Gesture};
use crate::model::{ObjMaterial, Shape};
use crate::sim::RenderMode;
use eframe::egui;

use super::units::{fmt_len};

use super::theme;

impl FlowPaintApp {
    /// The settings panel: the property block for whatever the tree has
    /// selected. The three-way branch (mid-gesture placeholder, object
    /// inspector, defaults) moved here unchanged from the old control
    /// column — the mid-gesture guard exists because the inspector
    /// fights an active drag.
    pub(in crate::app) fn settings_panel(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
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
                            self.object_panel(ui, id, cmds);
                        } else {
                            self.defaults_panel(ui, cmds);
                        }
                    });
            });
    }

    /// Properties of the selected object: every knob edits the live model
    /// (undoably, with per-widget coalescing).
    pub(in crate::app) fn object_panel(&mut self, ui: &mut egui::Ui, id: u64, cmds: &mut Vec<Cmd>) {
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

        // Fan physics: for drawn fans, and for generated parts that carry
        // fan cells (a rocket nozzle's chamber inlet).
        let stamp_has_fans = match &obj.shape {
            Shape::Stamp { raster, .. } => {
                raster.cell.iter().any(|&c| c == crate::geometry::CELL_INLET)
            }
            _ => false,
        };
        if obj.material == ObjMaterial::Fan || stamp_has_fans {
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
            // Chained shapes blow along their segments; solid shapes and
            // stamps have a free direction (stamps rotate with the part).
            if obj.material == ObjMaterial::Fan
                && (matches!(obj.shape, Shape::Rect { .. } | Shape::Ellipse { .. })
                    && obj.filled)
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
            let mut c = egui::Color32::from_rgb(
                (obj.smoke_rgb[0] * 255.0) as u8,
                (obj.smoke_rgb[1] * 255.0) as u8,
                (obj.smoke_rgb[2] * 255.0) as u8,
            );
            ui.horizontal(|ui| {
                ui.label("Smoke color:");
                if ui.color_edit_button_srgba(&mut c).changed() {
                    obj.smoke_rgb = [
                        c.r() as f32 / 255.0,
                        c.g() as f32 / 255.0,
                        c.b() as f32 / 255.0,
                    ];
                    changed = true;
                }
            });
        }

        ui.horizontal(|ui| {
            ui.label("Rotate");
            for (label, da) in [("-15°", -15.0f32), ("+15°", 15.0), ("+90°", 90.0)] {
                if ui.small_button(label).clicked() {
                    obj.rotate_by(da.to_radians());
                    changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Scale");
            for (label, f) in [("×0.8", 0.8f32), ("×1.25", 1.25)] {
                if ui.small_button(label).clicked() {
                    obj.scale_by(f);
                    changed = true;
                }
            }
        });

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
