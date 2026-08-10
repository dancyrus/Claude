//! The top menu bar: rare operations only (file IO, undo/redo, grid
//! resolution, domain margin, help).

use crate::app::{Cmd, FlowPaintApp, UiSnapshot};
use crate::sim::{MARGIN_CHOICES, RESOLUTIONS};
use eframe::egui;

impl FlowPaintApp {
    pub(in crate::app) fn menu_bar(&mut self, ctx: &egui::Context, snap: UiSnapshot, cmds: &mut Vec<Cmd>) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New (clear everything)").clicked() {
                        self.finish_gesture();
                        self.deselect_all();
                        self.model.replace_all(Vec::new());
                        cmds.push(Cmd::ResetFlow);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Open scene…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("FlowPaint scene", &["flow"])
                            .pick_file()
                        {
                            self.load_scene(&p, cmds);
                        }
                        ui.close_menu();
                    }
                    if ui.button("Save scene…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("FlowPaint scene", &["flow"])
                            .set_file_name("scene.flow")
                            .save_file()
                        {
                            self.save_scene(&p, snap);
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Export view as PNG…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("PNG image", &["png"])
                            .set_file_name("flowpaint.png")
                            .save_file()
                        {
                            cmds.push(Cmd::ExportPng(p));
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Undo        Ctrl+Z").clicked() {
                        self.finish_gesture();
                        self.model.undo();
                        self.deselect_all();
                        ui.close_menu();
                    }
                    if ui.button("Redo        Ctrl+Y").clicked() {
                        self.finish_gesture();
                        self.model.redo();
                        self.deselect_all();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Copy        Ctrl+C").clicked() {
                        self.copy_selected(&ui.ctx().clone());
                        ui.close_menu();
                    }
                    if ui.button("Paste       Ctrl+V").clicked() {
                        self.paste_clipboard(false);
                        ui.close_menu();
                    }
                    if ui.button("Paste in place  Ctrl+Shift+V").clicked() {
                        self.paste_clipboard(true);
                        ui.close_menu();
                    }
                    if ui.button("Select all  Ctrl+A").clicked() {
                        self.finish_gesture();
                        self.selected = self
                            .model
                            .objects
                            .iter()
                            .filter(|o| !o.locked && !o.hidden)
                            .map(|o| o.id)
                            .collect();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Reset flow (keep sketch)").clicked() {
                        cmds.push(Cmd::ResetFlow);
                        ui.close_menu();
                    }
                });
                ui.menu_button("Simulation", |ui| {
                    ui.menu_button("Grid resolution", |ui| {
                        for (i, (label, _, _)) in RESOLUTIONS.iter().enumerate() {
                            if ui.radio(i == self.res_index, *label).clicked() {
                                self.res_index = i;
                                self.finish_gesture();
                                cmds.push(Cmd::SetResolution(i));
                                ui.close_menu();
                            }
                        }
                    });
                    ui.menu_button("Domain margin", |ui| {
                        ui.label("Extra simulated area around the canvas;");
                        ui.label("edges also get an absorbing sponge layer.");
                        ui.separator();
                        // The size choice survives unchecking, so
                        // rechecking restores it.
                        if ui
                            .checkbox(&mut self.margin_on, "Simulated margin")
                            .changed()
                        {
                            let frac = if self.margin_on {
                                MARGIN_CHOICES[self.margin_index].1
                            } else {
                                0.0
                            };
                            cmds.push(Cmd::SetMarginFrac(frac));
                        }
                        ui.add_enabled_ui(self.margin_on, |ui| {
                            for (i, (label, frac)) in MARGIN_CHOICES.iter().enumerate() {
                                if ui.radio(i == self.margin_index, *label).clicked() {
                                    self.margin_index = i;
                                    cmds.push(Cmd::SetMarginFrac(*frac));
                                    ui.close_menu();
                                }
                            }
                        });
                    });
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("Keyboard shortcuts").clicked() {
                        self.show_shortcuts = true;
                        ui.close_menu();
                    }
                    if ui.button("About FlowPaint V2").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }
}
