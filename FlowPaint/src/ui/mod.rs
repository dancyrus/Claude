//! The FlowPaint UI, split by panel (phase 1 of the UI overhaul plan).
//! This module only declares the panel files and orchestrates the
//! per-frame draw order; all state stays in `app.rs`.

mod canvas;
mod generators;
mod inspector;
mod legend;
mod menu;
mod panels;
mod status;
mod windows;

use super::{Cmd, FlowPaintApp, UiSnapshot};
use eframe::egui;

/// Draw every panel for this frame, in layout order.
pub(super) fn draw(
    app: &mut FlowPaintApp,
    ctx: &egui::Context,
    snapshot: UiSnapshot,
    cmds: &mut Vec<Cmd>,
) {
    app.menu_bar(ctx, snapshot, cmds);
    app.side_panel(ctx, snapshot, cmds);
    app.legend_panel(ctx, snapshot);
    app.status_bar(ctx);
    app.canvas(ctx, cmds);
    app.windows(ctx, snapshot);
}
