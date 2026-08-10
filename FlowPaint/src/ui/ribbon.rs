//! The ribbon: a tab strip over a fixed-height body of grouped controls.
//! Phase 3a builds the frame — tabs, group boxes, vertical rules and
//! captions; phase 3b moves the controls in.

use crate::app::{FlowPaintApp, RibbonTab};
use eframe::egui;

use super::theme;

/// Group captions per tab (the mockup's group layout, adjusted to the
/// v3 plan's group assignments). 3b replaces the placeholders with the
/// real controls.
fn groups(tab: RibbonTab) -> &'static [&'static str] {
    match tab {
        RibbonTab::Home => &["Run", "Scene", "History"],
        RibbonTab::Geometry => &["Sketch tools", "Material", "Sketch aids"],
        RibbonTab::Physics => &["Solver", "Inlet condition", "Integration", "Domain"],
        RibbonTab::Study => &["Generators", "Scene presets"],
        RibbonTab::Results => &["Field", "Display", "Mapping"],
    }
}

impl FlowPaintApp {
    pub(in crate::app) fn ribbon(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("ribbon_tabs")
            .exact_height(theme::dim::RIBBON_TABS_HEIGHT)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in RibbonTab::ALL {
                        if theme::toggle(ui, self.ribbon_tab == tab, label).clicked() {
                            self.ribbon_tab = tab;
                        }
                    }
                });
            });

        egui::TopBottomPanel::top("ribbon_body")
            .exact_height(theme::dim::RIBBON_HEIGHT)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for caption in groups(self.ribbon_tab) {
                        self.ribbon_group(ui, caption);
                        ui.separator(); // vertical rule between groups
                    }
                });
            });
    }

    /// One ribbon group: an items area over a small centered caption.
    /// The items area is an empty placeholder until 3b.
    fn ribbon_group(&mut self, ui: &mut egui::Ui, caption: &str) {
        ui.vertical(|ui| {
            ui.set_width(96.0);
            // 3b draws the group's controls in this space.
            ui.allocate_space(egui::vec2(96.0, 54.0));
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(caption)
                        .small()
                        .color(theme::INK_3),
                );
            });
        });
    }
}
