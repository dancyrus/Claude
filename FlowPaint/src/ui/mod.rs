//! The FlowPaint UI, split by panel (phase 1 of the UI overhaul plan).
//! This module only declares the panel files and orchestrates the
//! per-frame draw order; all state stays in `app.rs`.

mod canvas;
mod generators;
mod inspector;
mod legend;
mod menu;
mod ribbon;
mod status;
pub(super) mod theme;
mod tree;
pub(super) mod units;
mod windows;

use super::{Cmd, FlowPaintApp, UiSnapshot};
use eframe::egui;

/// Draw every panel for this frame, in layout order: chrome first (menu,
/// ribbon, full-width status strip), then the left tree and settings
/// columns, the right legend, and the canvas in what remains.
pub(super) fn draw(
    app: &mut FlowPaintApp,
    ctx: &egui::Context,
    snapshot: UiSnapshot,
    cmds: &mut Vec<Cmd>,
) {
    app.menu_bar(ctx, snapshot, cmds);
    app.ribbon(ctx, snapshot, cmds);
    app.status_bar(ctx, cmds);
    app.tree_panel(ctx, cmds);
    app.settings_panel(ctx, snapshot, cmds);
    app.legend_panel(ctx, snapshot, cmds);
    app.canvas(ctx, cmds);
    app.windows(ctx, snapshot, cmds);
}
