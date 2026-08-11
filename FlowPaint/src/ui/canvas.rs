//! The graphics window: the wgpu paint callback, the pointer gesture
//! state machine, the selection/snap overlays, and (U3) the transform
//! gizmo — corner handles scale, the rotate handle above the box
//! rotates, the pivot marker drags. All gizmo edits go through the
//! model's world-space ops, so they compose correctly through nested
//! groups (transform order: child first, then each ancestor outward —
//! see CLAUDE.md).

use crate::app::{Cmd, FlowPaintApp, Gesture, OsnapHit, OsnapKind, Tool, ViewRequest};
use crate::model::{Shape, Sim2, SketchObject};
use crate::sim::{GpuSim, ViewportMapping};
use eframe::egui;

use super::units::{fmt_angle, fmt_factor, fmt_len};

/// Object-snap radius in screen POINTS — constant across zoom (U4).
const SNAP_RADIUS_PT: f32 = 10.0;

// Free-zoom bounds: view_zoom is a multiplier over the letterbox fit
// scale, additionally capped in absolute framebuffer px per cell; the
// pan clamp keeps at least this much grid on screen in each axis.
const ZOOM_MIN: f32 = 0.125;
const ZOOM_MAX: f32 = 64.0;
const MAX_PX_PER_CELL: f32 = 512.0;
const MIN_VISIBLE_PX: f32 = 64.0;
/// Wheel-zoom rate per scroll point (factor = exp(rate * delta)).
const ZOOM_WHEEL_RATE: f32 = 0.005;

// Gizmo geometry, in screen points (constant size at every zoom).
const GIZMO_PAD_PT: f32 = 10.0;
const GIZMO_ROT_OFF_PT: f32 = 20.0;
const GIZMO_HANDLE_PT: f32 = 8.0;

/// The gizmo's interactive points, in world cells for one frame.
pub(in crate::app) struct GizmoLayout {
    pub box_min: [f32; 2],
    pub box_max: [f32; 2],
    pub corners: [[f32; 2]; 4],
    pub rotate: [f32; 2],
    pub pivot: [f32; 2],
}

impl FlowPaintApp {
    pub(in crate::app) fn canvas(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                // Sense::drag (not click_and_drag): drags then start on the
                // press frame at the press position, so clicks land exactly
                // where the user pressed instead of ~6 pt into the gesture.
                let response = ui.allocate_rect(rect, egui::Sense::drag());

                let ppp = ctx.pixels_per_point();
                let (gw, gh) = self.stats_grid;
                if gw == 0 || gh == 0 {
                    return;
                }
                // Round to whole physical pixels the same way egui rounds
                // the render-pass viewport, so the particle overlay's NDC
                // math matches the viewport the GPU actually uses.
                let x0 = (rect.min.x * ppp).round();
                let y0 = (rect.min.y * ppp).round();
                let x1 = (rect.max.x * ppp).round();
                let y1 = (rect.max.y * ppp).round();
                let fit =
                    ViewportMapping::fit([x0, y0], [x1 - x0, y1 - y0], gw, gh);

                // Zoom and pan are a view transform on px_per_cell and
                // lb_origin only — grid, margin and physics untouched.
                self.apply_view_request(&fit);
                if self.view_fit {
                    self.view_zoom = 1.0;
                    self.view_center = [gw as f32 * 0.5, gh as f32 * 0.5];
                }
                let pan_mode = self.view_nav(ctx, &response, &fit, ppp);
                let mapping = self.view_mapping(&fit, gw, gh);
                self.view_px_per_cell = mapping.px_per_cell;
                // The pushed mapping is also what status.rs draws probe
                // markers with — the canvas owns the view transform.
                self.canvas_mapping = Some(mapping);
                cmds.push(Cmd::SetMapping(mapping));

                self.canvas_interaction(&response, mapping, ppp, pan_mode, cmds);

                // The simulation paints itself via the wgpu callback.
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    FlowPaintCallback,
                ));

                self.canvas_overlays(ui, mapping, ppp);
                self.scale_bar(ui, rect, mapping, ppp);
            });
    }

    /// Clamp the zoom multiplier: the [ZOOM_MIN, ZOOM_MAX] range, and an
    /// absolute cap of MAX_PX_PER_CELL framebuffer px per cell.
    fn clamp_zoom(&self, fit_scale: f32, z: f32) -> f32 {
        let max_abs = MAX_PX_PER_CELL / fit_scale.max(1e-6);
        z.clamp(ZOOM_MIN, ZOOM_MAX.min(max_abs).max(ZOOM_MIN))
    }

    /// Consume a pending Fit / OneToOne / Selection request (ribbon
    /// buttons and Ctrl+0/1/2 set it; only the canvas knows the viewport).
    fn apply_view_request(&mut self, fit: &ViewportMapping) {
        let Some(req) = self.view_request.take() else { return };
        match req {
            ViewRequest::Fit => self.view_fit = true,
            ViewRequest::OneToOne => {
                // Exactly one framebuffer px per cell (up to the clamps).
                self.view_zoom =
                    self.clamp_zoom(fit.px_per_cell, 1.0 / fit.px_per_cell.max(1e-6));
                self.view_fit = false;
            }
            ViewRequest::Selection => {
                // Union of the whole selection's WORLD bounds (groups
                // and grouped members resolve their ancestor chain).
                let Some(b) = self.selection_world_bounds() else { return };
                let bw = (b.x1 - b.x0).max(1) as f32;
                let bh = (b.y1 - b.y0).max(1) as f32;
                // ~10% padding on each side of the object's bounds.
                let s = (fit.vp_size[0] / (bw * 1.2))
                    .min(fit.vp_size[1] / (bh * 1.2));
                self.view_zoom =
                    self.clamp_zoom(fit.px_per_cell, s / fit.px_per_cell.max(1e-6));
                self.view_center = [
                    (b.x0 + b.x1) as f32 * 0.5,
                    (b.y0 + b.y1) as f32 * 0.5,
                ];
                self.view_fit = false;
            }
        }
    }

    /// Wheel/pinch zoom at the cursor plus middle- or space-drag pan.
    /// Returns true while Space claims the primary button for panning,
    /// so canvas_interaction must not draw or select with it.
    fn view_nav(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        fit: &ViewportMapping,
        ppp: f32,
    ) -> bool {
        let vc = [
            fit.vp_origin[0] + fit.vp_size[0] * 0.5,
            fit.vp_origin[1] + fit.vp_size[1] * 0.5,
        ];

        // Zoom, only with the pointer over the canvas. Wheel comes via
        // smooth_scroll_delta, pinch (and Ctrl+wheel, which egui folds
        // into it) via zoom_delta — the two sources are disjoint, so
        // multiplying them never double-counts an event.
        if let Some(pos) = response.hover_pos() {
            let (scroll, pinch) =
                ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll * ZOOM_WHEEL_RATE).exp() * pinch;
            if (factor - 1.0).abs() > 1e-4 {
                let z0 = self.view_zoom;
                let z1 = self.clamp_zoom(fit.px_per_cell, z0 * factor);
                if z1 != z0 {
                    let s0 = fit.px_per_cell * z0;
                    let s1 = fit.px_per_cell * z1;
                    let p = [pos.x * ppp, pos.y * ppp];
                    // Keep the cell under the cursor under the cursor:
                    // c = center + (p - vc)/s is invariant across s0→s1.
                    self.view_center[0] += (p[0] - vc[0]) * (1.0 / s0 - 1.0 / s1);
                    self.view_center[1] += (p[1] - vc[1]) * (1.0 / s0 - 1.0 / s1);
                    self.view_zoom = z1;
                    self.view_fit = false;
                }
            }
        }

        // Pan: middle-drag, or primary-drag while Space is held (the
        // trackpad alternative). While Space is down it owns the primary
        // button on the canvas outright.
        let space_down = ctx.input(|i| i.key_down(egui::Key::Space));
        let mid_pan = response.dragged_by(egui::PointerButton::Middle);
        let space_pan =
            space_down && response.dragged_by(egui::PointerButton::Primary);
        if mid_pan || space_pan {
            let d = response.drag_delta();
            let s = (fit.px_per_cell * self.view_zoom).max(1e-6);
            if d != egui::Vec2::ZERO {
                self.view_center[0] -= d.x * ppp / s;
                self.view_center[1] -= d.y * ppp / s;
                self.view_fit = false;
                if space_pan {
                    self.space_pan_suppress = true;
                }
            }
        }
        space_down
    }

    /// Build the frame's mapping from the fit scale and the view state,
    /// clamping the pan so at least MIN_VISIBLE_PX of grid stays on
    /// screen in each axis (the domain can never leave the screen).
    fn view_mapping(
        &mut self,
        fit: &ViewportMapping,
        gw: usize,
        gh: usize,
    ) -> ViewportMapping {
        self.view_zoom = self.clamp_zoom(fit.px_per_cell, self.view_zoom);
        let s = fit.px_per_cell * self.view_zoom;
        let vc = [
            fit.vp_origin[0] + fit.vp_size[0] * 0.5,
            fit.vp_origin[1] + fit.vp_size[1] * 0.5,
        ];
        let grid = [gw as f32, gh as f32];
        for a in 0..2 {
            let vp_lo = fit.vp_origin[a];
            let vp_hi = fit.vp_origin[a] + fit.vp_size[a];
            // lb <= vp_hi - MIN and lb + grid*s >= vp_lo + MIN, with
            // lb = vc - center*s, solved for center.
            let lo = (vc[a] - vp_hi + MIN_VISIBLE_PX) / s;
            let hi = (vc[a] - vp_lo - MIN_VISIBLE_PX) / s + grid[a];
            self.view_center[a] = if lo <= hi {
                self.view_center[a].clamp(lo, hi)
            } else {
                // Viewport narrower than 2*MIN_VISIBLE_PX: pin to centre.
                (lo + hi) * 0.5
            };
        }
        ViewportMapping {
            vp_origin: fit.vp_origin,
            vp_size: fit.vp_size,
            lb_origin: [
                vc[0] - self.view_center[0] * s,
                vc[1] - self.view_center[1] * s,
            ],
            px_per_cell: s,
        }
    }

    /// Persistent scale bar, bottom-left: a round physical length (1-2-5
    /// progression) whose on-screen width lands near 100 pt, with end
    /// ticks and a centred label. Updates live with the zoom.
    fn scale_bar(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        mapping: ViewportMapping,
        ppp: f32,
    ) {
        const TARGET_PT: f32 = 100.0; // nearest 1-2-5 lands in ~63–158 pt
        const MARGIN_PT: f32 = 14.0;
        const TICK_PT: f32 = 5.0;
        let ps = self.phys_cache;
        let pt_per_m = mapping.px_per_cell / (ppp * ps.dx.max(1e-12));
        if !pt_per_m.is_finite() || pt_per_m <= 0.0 {
            return;
        }
        // Nearest 1-2-5 length in log space to the target width.
        let target_m = TARGET_PT / pt_per_m;
        let decade = 10f32.powf(target_m.log10().floor());
        let mut len_m = decade;
        let mut best = f32::INFINITY;
        for m in [1.0, 2.0, 5.0, 10.0] {
            let err = ((m * decade) / target_m).ln().abs();
            if err < best {
                best = err;
                len_m = m * decade;
            }
        }
        let w_pt = len_m * pt_per_m;
        let a = rect.left_bottom() + egui::vec2(MARGIN_PT, -MARGIN_PT);
        let b = a + egui::vec2(w_pt, 0.0);
        let stroke = egui::Stroke::new(1.5, super::theme::SCALE_BAR);
        let painter = ui.painter();
        painter.line_segment([a, b], stroke);
        painter.line_segment([a + egui::vec2(0.0, -TICK_PT), a], stroke);
        painter.line_segment([b + egui::vec2(0.0, -TICK_PT), b], stroke);
        painter.text(
            egui::pos2((a.x + b.x) * 0.5, a.y - TICK_PT - 2.0),
            egui::Align2::CENTER_BOTTOM,
            fmt_len(ps.len_m(len_m / ps.dx.max(1e-12))),
            egui::TextStyle::Monospace.resolve(ui.style()),
            super::theme::SCALE_BAR,
        );
    }

    pub(in crate::app) fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    }

    /// Ramer–Douglas–Peucker simplification for pencil strokes, so
    /// freehand curves become clean, light polylines with draggable
    /// vertices.
    pub(in crate::app) fn simplify_stroke(pts: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
        if pts.len() <= 2 {
            return pts.to_vec();
        }
        fn rdp(pts: &[[f32; 2]], eps: f32, out: &mut Vec<[f32; 2]>) {
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            let mut worst = 0.0f32;
            let mut wi = 0usize;
            let ab = [b[0] - a[0], b[1] - a[1]];
            let l2 = ab[0] * ab[0] + ab[1] * ab[1];
            for (k, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
                let d = if l2 < 1e-9 {
                    ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt()
                } else {
                    ((p[0] - a[0]) * ab[1] - (p[1] - a[1]) * ab[0]).abs() / l2.sqrt()
                };
                if d > worst {
                    worst = d;
                    wi = k;
                }
            }
            if worst > eps && wi > 0 {
                rdp(&pts[..=wi], eps, out);
                out.pop(); // the split point would land twice
                rdp(&pts[wi..], eps, out);
            } else {
                out.push(a);
                out.push(b);
            }
        }
        let mut out = Vec::new();
        rdp(pts, eps, &mut out);
        out
    }

    fn canvas_interaction(
        &mut self,
        response: &egui::Response,
        mapping: ViewportMapping,
        ppp: f32,
        pan_mode: bool,
        cmds: &mut Vec<Cmd>,
    ) {
        let to_cell = |pos: egui::Pos2| -> [f32; 2] {
            mapping.px_to_cell([pos.x * ppp, pos.y * ppp])
        };
        self.hover_cell = response.hover_pos().map(to_cell);

        let px_per_cell = mapping.px_per_cell.max(1e-3);
        // Pick thresholds purely in screen space (converted to cells for
        // the model's hit tests): a constant pick radius in screen pt at
        // every zoom. A floor in cells would balloon into a huge grab
        // radius at high zoom (2 cells at 64 px/cell is 128 px).
        let handle_r = 8.0 * ppp / px_per_cell;
        let click_slop = 4.0 * ppp / px_per_cell;
        let pt_per_cell = px_per_cell / ppp;
        let (shift, alt, ctrl) = response
            .ctx
            .input(|i| (i.modifiers.shift, i.modifiers.alt, i.modifiers.command));
        let pointer = response.interact_pointer_pos();

        // Object snaps (U4): resolve THE frame's candidate once, from
        // the active pointer position; every snapped coordinate this
        // frame consumes it via snap_active. Ctrl suspends; the object
        // being drawn is excluded so it can't snap to itself.
        self.osnap_hit = None;
        let osnap_tool = matches!(
            self.tool,
            Tool::Line | Tool::Rect | Tool::Ellipse | Tool::Polyline | Tool::Measure
        ) || matches!(self.gesture, Gesture::HandleDrag { .. });
        // Shift hands the position to angle-snap (or the square/circle
        // constraint), which overrides any object snap — computing one
        // anyway would draw a marker at a point the vertex won't go to.
        if self.osnap_enabled && osnap_tool && !ctrl && !shift && !pan_mode {
            if let Some(pos) = pointer.or(response.hover_pos()) {
                let cursor = to_cell(pos);
                let radius = SNAP_RADIUS_PT * ppp / px_per_cell;
                let exclude = match &self.gesture {
                    Gesture::DrawShape { id, .. }
                    | Gesture::DrawPoly { id }
                    | Gesture::HandleDrag { id, .. } => Some(*id),
                    _ => None,
                };
                let anchor = match &self.gesture {
                    Gesture::DrawShape { anchor, .. } => Some(*anchor),
                    Gesture::Measure { a, .. } => Some(*a),
                    Gesture::DrawPoly { id } => {
                        self.model.find(*id).and_then(|i| {
                            match &self.model.objects[i].shape {
                                Shape::Poly { pts, .. } if pts.len() >= 2 => {
                                    Some(pts[pts.len() - 2])
                                }
                                _ => None,
                            }
                        })
                    }
                    _ => None,
                };
                self.osnap_hit = self.compute_osnap(cursor, radius, exclude, anchor);
            }
        }

        // Armed probe placement (from the tree) claims the next click
        // over the field outright — it must not also select or draw.
        if self.probe_arming && !pan_mode {
            if response.drag_started_by(egui::PointerButton::Primary) {
                if let Some(pos) = pointer {
                    let c = to_cell(pos);
                    let (vw, vh) = self.stats_grid;
                    if c[0] >= 0.0 && c[1] >= 0.0 && c[0] < vw as f32 && c[1] < vh as f32
                    {
                        cmds.push(Cmd::AddProbe(c));
                        self.probe_arming = false;
                    }
                    // Over the letterbox: stay armed.
                }
            }
            return;
        }

        // Double-click enters a group: subsequent clicks select one
        // level below it ("selecting a group selects its subtree;
        // entering it allows selecting a child individually").
        if !pan_mode
            && self.tool == Tool::Select
            && response
                .ctx
                .input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary))
        {
            if let Some(pos) = response.hover_pos() {
                let p = to_cell(pos);
                if let Some(leaf) = self.model.hit_test(p, click_slop) {
                    let target = self.pick_target(leaf);
                    let is_group = self
                        .model
                        .find(target)
                        .map(|i| matches!(self.model.objects[i].shape, Shape::Group { .. }))
                        .unwrap_or(false);
                    if is_group && target != leaf {
                        // The second press started a move gesture on the
                        // group — cancel it, enter, select the child.
                        self.gesture = Gesture::None;
                        self.entered_group = Some(target);
                        let child = self.child_toward(target, leaf);
                        self.select_only(child);
                        self.status =
                            "Entered the group — clicks now select its members \
                             (Esc leaves)."
                                .into();
                    }
                }
            }
        }

        // Live polyline rubber vertex follows the cursor between clicks.
        if let Gesture::DrawPoly { id } = &self.gesture {
            let id = *id;
            if let Some(pos) = response.hover_pos() {
                let raw = to_cell(pos);
                let prev = self.model.find(id).and_then(|i| {
                    match &self.model.objects[i].shape {
                        Shape::Poly { pts, .. } if pts.len() >= 2 => {
                            Some(pts[pts.len() - 2])
                        }
                        _ => None,
                    }
                });
                let p = if shift {
                    match prev {
                        Some(a) => self.angle_snap(a, raw),
                        None => self.snap_active(raw),
                    }
                } else {
                    self.snap_active(raw)
                };
                self.mutate_live(id, |o| {
                    if let Shape::Poly { pts, .. } = &mut o.shape {
                        if let Some(last) = pts.last_mut() {
                            *last = p;
                        }
                    }
                });
            }
        }

        // --- Presses --------------------------------------------------

        if response.drag_started_by(egui::PointerButton::Secondary) {
            match self.tool {
                // Right-click finishes the polyline, CAD-style.
                Tool::Polyline => self.finish_gesture(),
                Tool::Select => {
                    if matches!(self.gesture, Gesture::None) {
                        self.deselect_all();
                    }
                }
                _ => {}
            }
        }

        // While Space is held the primary button pans (view_nav); it must
        // not also draw or select.
        if !pan_mode && response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer {
                let raw = to_cell(pos);
                match self.tool {
                    Tool::Select => {
                        self.select_press(raw, handle_r, click_slop, shift, pt_per_cell)
                    }
                    Tool::Line => {
                        self.finish_gesture();
                        let a = self.snap_active(raw);
                        let obj = self.new_object(Shape::Line { a, b: a });
                        let id = obj.id;
                        self.model.add(obj);
                        self.gesture = Gesture::DrawShape { id, anchor: a };
                        self.select_only(id);
                    }
                    Tool::Rect | Tool::Ellipse => {
                        self.finish_gesture();
                        let a = self.snap_active(raw);
                        let shape = if self.tool == Tool::Rect {
                            Shape::Rect { c: a, half: [0.5, 0.5], angle: 0.0 }
                        } else {
                            Shape::Ellipse { c: a, r: [0.5, 0.5], angle: 0.0 }
                        };
                        let obj = self.new_object(shape);
                        let id = obj.id;
                        self.model.add(obj);
                        self.gesture = Gesture::DrawShape { id, anchor: a };
                        self.select_only(id);
                    }
                    Tool::Polyline => {
                        if let Gesture::DrawPoly { id } = &self.gesture {
                            let id = *id;
                            self.poly_click(id, raw, shift, handle_r);
                        } else {
                            self.finish_gesture();
                            let p = self.snap_active(raw);
                            let obj = self.new_object(Shape::Poly {
                                pts: vec![p, p],
                                closed: false,
                            });
                            let id = obj.id;
                            self.model.add(obj);
                            self.gesture = Gesture::DrawPoly { id };
                            self.select_only(id);
                            self.status =
                                "Polyline: click to add vertices, Enter/right-click to \
                                 finish, click the first vertex to close."
                                    .into();
                        }
                    }
                    Tool::Pencil => {
                        self.finish_gesture();
                        let obj = self
                            .new_object(Shape::Poly { pts: vec![raw], closed: false });
                        let id = obj.id;
                        self.model.add(obj);
                        self.gesture = Gesture::DrawPencil { id };
                        self.select_only(id);
                    }
                    Tool::Bucket => {
                        self.finish_gesture();
                        self.bucket_fill(raw);
                    }
                    Tool::Eraser => {
                        self.finish_gesture();
                        // Raw points, no snapping: an eraser follows the
                        // hand. The edit commits on release.
                        self.gesture = Gesture::Erase { pts: vec![raw] };
                    }
                    Tool::Measure => {
                        self.finish_gesture();
                        let a = self.snap_active(raw);
                        self.gesture = Gesture::Measure { a, b: a };
                    }
                }
            }
        }

        // --- Drag updates ---------------------------------------------

        if !pan_mode && response.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer {
                let raw = to_cell(pos);
                if let Gesture::DrawShape { id, anchor } = &self.gesture {
                    let (id, anchor) = (*id, *anchor);
                    self.update_draw_shape(id, anchor, raw, shift, alt);
                } else if let Gesture::DrawPencil { id } = &self.gesture {
                    let id = *id;
                    self.mutate_live(id, |o| {
                        if let Shape::Poly { pts, .. } = &mut o.shape {
                            let far = pts
                                .last()
                                .map(|l| Self::dist(*l, raw) >= 2.0)
                                .unwrap_or(true);
                            if far {
                                pts.push(raw);
                            }
                        }
                    });
                } else if let Gesture::MoveSel { before, last } = &self.gesture {
                    let ids: Vec<u64> = before.iter().map(|(id, _)| *id).collect();
                    let last = *last;
                    let eff = if self.snap_enabled { self.snap_point(raw) } else { raw };
                    let d = [eff[0] - last[0], eff[1] - last[1]];
                    if d != [0.0; 2] {
                        for id in ids {
                            // World-space delta through the ancestor
                            // chain — a member of a rotated group moves
                            // with the cursor, not along its local axes.
                            self.model.translate_world(id, d);
                        }
                        if let Gesture::MoveSel { last, .. } = &mut self.gesture {
                            *last = eff;
                        }
                    }
                } else if matches!(self.gesture, Gesture::RubberBand { .. }) {
                    self.rubber_band_update(raw);
                } else if let Gesture::HandleDrag { id, idx, .. } = &self.gesture {
                    let (id, idx) = (*id, *idx);
                    let p = if shift {
                        // Shift angle-snaps a line endpoint about the other.
                        let other = self.model.find(id).and_then(|i| {
                            match &self.model.objects[i].shape {
                                Shape::Line { a, b } => {
                                    Some(if idx == 0 { *b } else { *a })
                                }
                                _ => None,
                            }
                        });
                        match other {
                            Some(o) => self.angle_snap(o, raw),
                            None => self.snap_active(raw),
                        }
                    } else {
                        self.snap_active(raw)
                    };
                    // Handles are grabbed in world space; the stored
                    // shape lives in its parent group's space.
                    let p_local = self.model.parent_abs(id).inverse().apply(p);
                    self.mutate_live(id, |o| o.set_handle(idx, p_local));
                } else if let Gesture::GizmoRotate { pivot, start_ang, .. } = &self.gesture
                {
                    let (pivot, start_ang) = (*pivot, *start_ang);
                    let ang = (raw[1] - pivot[1]).atan2(raw[0] - pivot[0]);
                    let mut total = ang - start_ang;
                    if shift {
                        // The angle-snap increment constrains gizmo
                        // rotation, like the draw tools.
                        let step =
                            self.snap_angle_deg.clamp(1.0, 90.0).to_radians();
                        total = (total / step).round() * step;
                    }
                    self.gizmo_apply_rotate(total);
                } else if let Gesture::GizmoScale { pivot, start_dist, .. } = &self.gesture
                {
                    let (pivot, start_dist) = (*pivot, *start_dist);
                    let d = Self::dist(raw, pivot).max(1e-3);
                    let total = (d / start_dist.max(1e-3)).clamp(0.05, 50.0);
                    self.gizmo_apply_scale(total);
                } else if matches!(self.gesture, Gesture::GizmoPivot) {
                    let p = self.snap_point(raw);
                    self.gizmo_pivot = Some(p);
                    self.gizmo_pivot_sel = self.selected.clone();
                } else if let Gesture::Erase { pts } = &mut self.gesture {
                    // Decimate to ~half the radius so capsule counts
                    // stay bounded on slow scribbles.
                    let min_step = (self.eraser_radius * 0.5).max(0.75);
                    let far = pts
                        .last()
                        .map(|l| Self::dist(*l, raw) >= min_step)
                        .unwrap_or(true);
                    if far {
                        pts.push(raw);
                    }
                } else if let Gesture::Measure { a, .. } = &self.gesture {
                    let a = *a;
                    let b = if shift {
                        self.angle_snap(a, raw)
                    } else {
                        self.snap_active(raw)
                    };
                    if let Gesture::Measure { b: bb, .. } = &mut self.gesture {
                        *bb = b;
                    }
                }
            }
        }

        // --- Releases -------------------------------------------------

        // Drag-driven gestures end when the primary button is up — not
        // only on drag_stopped_by(Primary), which egui can skip when its
        // drag latch dies mid-gesture (e.g. releasing the right button
        // during a left-drag). finish_gesture handles degenerate cancels
        // and pencil simplification. A polyline persists across clicks.
        let primary_down = response.ctx.input(|i| i.pointer.primary_down());
        if !primary_down
            && matches!(
                &self.gesture,
                Gesture::MoveSel { .. }
                    | Gesture::HandleDrag { .. }
                    | Gesture::DrawShape { .. }
                    | Gesture::DrawPencil { .. }
                    | Gesture::RubberBand { .. }
                    | Gesture::GizmoRotate { .. }
                    | Gesture::GizmoScale { .. }
                    | Gesture::GizmoPivot
                    | Gesture::Erase { .. }
                    | Gesture::Measure { .. }
            )
        {
            self.finish_gesture();
        }
    }

    /// The frame's snapped coordinate: the object-snap candidate when
    /// one is in range (it wins over the grid — U4), else the grid snap.
    fn snap_active(&self, raw: [f32; 2]) -> [f32; 2] {
        match &self.osnap_hit {
            Some(h) => h.pos,
            None => self.snap_point(raw),
        }
    }

    /// Find the object-snap candidate near `cursor` (world cells).
    /// Priority: kind first (endpoint > intersection > midpoint >
    /// center > perpendicular), then distance. Hidden objects are
    /// skipped; LOCKED objects still snap (they're reference geometry).
    fn compute_osnap(
        &self,
        cursor: [f32; 2],
        radius: f32,
        exclude: Option<u64>,
        anchor: Option<[f32; 2]>,
    ) -> Option<OsnapHit> {
        let mut best: Option<(OsnapKind, f32, [f32; 2])> = None;
        let mut consider = |kind: OsnapKind, p: [f32; 2]| {
            let d = Self::dist(p, cursor);
            if d > radius {
                return;
            }
            let better = match &best {
                Some((bk, bd, _)) => (kind, d) < (*bk, *bd),
                None => true,
            };
            if better {
                best = Some((kind, d, p));
            }
        };
        // Segment pool for intersection / perpendicular candidates
        // ((owner id, a, b) in world space), gathered from objects near
        // the cursor only.
        let mut pool: Vec<(u64, [f32; 2], [f32; 2])> = Vec::new();
        const POOL_CAP: usize = 64;

        for o in &self.model.objects {
            if Some(o.id) == exclude
                || matches!(o.shape, Shape::Group { .. })
                || self.model.eff_hidden(o.id)
            {
                continue;
            }
            let abs = self.model.parent_abs(o.id);
            let b = o.bounds_under(abs);
            if cursor[0] < b.x0 as f32 - radius
                || cursor[0] > b.x1 as f32 + radius
                || cursor[1] < b.y0 as f32 - radius
                || cursor[1] > b.y1 as f32 + radius
            {
                continue;
            }
            let ap = |p: [f32; 2]| abs.apply(p);
            match &o.shape {
                Shape::Line { a, b } => {
                    let (aw, bw) = (ap(*a), ap(*b));
                    consider(OsnapKind::Endpoint, aw);
                    consider(OsnapKind::Endpoint, bw);
                    consider(
                        OsnapKind::Midpoint,
                        [(aw[0] + bw[0]) * 0.5, (aw[1] + bw[1]) * 0.5],
                    );
                    if pool.len() < POOL_CAP {
                        pool.push((o.id, aw, bw));
                    }
                }
                Shape::Poly { pts, closed } => {
                    let n = pts.len();
                    let segs = if *closed { n } else { n.saturating_sub(1) };
                    for p in pts {
                        consider(OsnapKind::Endpoint, ap(*p));
                    }
                    for k in 0..segs {
                        let aw = ap(pts[k]);
                        let bw = ap(pts[(k + 1) % n]);
                        consider(
                            OsnapKind::Midpoint,
                            [(aw[0] + bw[0]) * 0.5, (aw[1] + bw[1]) * 0.5],
                        );
                        if pool.len() < POOL_CAP {
                            pool.push((o.id, aw, bw));
                        }
                    }
                    if *closed {
                        consider(OsnapKind::Center, ap(o.center()));
                    }
                }
                Shape::Rect { c, half, angle } => {
                    consider(OsnapKind::Center, ap(*c));
                    let (s, co) = angle.sin_cos();
                    let corner = |kx: f32, ky: f32| -> [f32; 2] {
                        let lx = kx * half[0];
                        let ly = ky * half[1];
                        ap([c[0] + lx * co - ly * s, c[1] + lx * s + ly * co])
                    };
                    let cs = [
                        corner(-1.0, -1.0),
                        corner(1.0, -1.0),
                        corner(1.0, 1.0),
                        corner(-1.0, 1.0),
                    ];
                    for k in 0..4 {
                        consider(OsnapKind::Endpoint, cs[k]);
                        let m = [
                            (cs[k][0] + cs[(k + 1) % 4][0]) * 0.5,
                            (cs[k][1] + cs[(k + 1) % 4][1]) * 0.5,
                        ];
                        consider(OsnapKind::Midpoint, m);
                        if pool.len() < POOL_CAP {
                            pool.push((o.id, cs[k], cs[(k + 1) % 4]));
                        }
                    }
                }
                Shape::Ellipse { c, r, angle } => {
                    consider(OsnapKind::Center, ap(*c));
                    // Quadrant points, under the ellipse's rotation.
                    let (s, co) = angle.sin_cos();
                    for (kx, ky) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
                    {
                        let lx = kx * r[0];
                        let ly = ky * r[1];
                        consider(
                            OsnapKind::Midpoint,
                            ap([c[0] + lx * co - ly * s, c[1] + lx * s + ly * co]),
                        );
                    }
                }
                Shape::Stamp { raster, c, scale, angle } => {
                    consider(OsnapKind::Center, ap(*c));
                    let hx = (raster.rect.2 - raster.rect.0).max(0) as f32 * 0.5 * scale;
                    let hy = (raster.rect.3 - raster.rect.1).max(0) as f32 * 0.5 * scale;
                    let (s, co) = angle.sin_cos();
                    for (kx, ky) in
                        [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    {
                        let lx = kx * hx;
                        let ly = ky * hy;
                        consider(
                            OsnapKind::Endpoint,
                            ap([c[0] + lx * co - ly * s, c[1] + lx * s + ly * co]),
                        );
                    }
                }
                Shape::Group { .. } => {}
            }
        }

        // Intersections between pooled segments of DIFFERENT objects
        // (same-object neighbors already share snapped endpoints).
        for i in 0..pool.len() {
            for j in (i + 1)..pool.len() {
                if pool[i].0 == pool[j].0 {
                    continue;
                }
                if let Some((t, _)) = crate::geomops::segs_intersect(
                    pool[i].1, pool[i].2, pool[j].1, pool[j].2,
                ) {
                    let a = pool[i].1;
                    let b = pool[i].2;
                    consider(
                        OsnapKind::Intersection,
                        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                    );
                }
            }
        }

        // Perpendicular feet from the gesture's anchor.
        if let Some(a) = anchor {
            for (_, s0, s1) in &pool {
                let d = [s1[0] - s0[0], s1[1] - s0[1]];
                let l2 = d[0] * d[0] + d[1] * d[1];
                if l2 < 1e-9 {
                    continue;
                }
                let t = ((a[0] - s0[0]) * d[0] + (a[1] - s0[1]) * d[1]) / l2;
                if !(0.02..=0.98).contains(&t) {
                    continue;
                }
                consider(
                    OsnapKind::Perpendicular,
                    [s0[0] + d[0] * t, s0[1] + d[1] * t],
                );
            }
        }

        best.map(|(kind, _, pos)| OsnapHit { pos, kind })
    }

    /// Restore-and-reapply for the gizmo rotate drag: the model returns
    /// to the gesture-start state, then the accumulated angle applies
    /// about the pivot in one step (no per-frame error compounding).
    fn gizmo_apply_rotate(&mut self, total: f32) {
        let (before, pivot) = match &self.gesture {
            Gesture::GizmoRotate { before, pivot, .. } => (before.clone(), *pivot),
            _ => return,
        };
        self.restore_before(&before);
        for (id, _) in &before {
            self.model.rotate_world(*id, pivot, total);
        }
        if let Gesture::GizmoRotate { total: t, .. } = &mut self.gesture {
            *t = total;
        }
    }

    /// Restore-and-reapply for the gizmo scale drag (uniform — see the
    /// inspector tooltip: similarity transforms are the only family
    /// that survives nested rotated groups without shear).
    fn gizmo_apply_scale(&mut self, total: f32) {
        let (before, pivot) = match &self.gesture {
            Gesture::GizmoScale { before, pivot, .. } => (before.clone(), *pivot),
            _ => return,
        };
        self.restore_before(&before);
        for (id, _) in &before {
            self.model.scale_world(*id, pivot, total);
        }
        if let Gesture::GizmoScale { total: t, .. } = &mut self.gesture {
            *t = total;
        }
    }

    /// The ancestor chain of `leaf`, leaf first, root last.
    fn ancestor_chain(&self, leaf: u64) -> Vec<u64> {
        let mut chain = vec![leaf];
        let mut cur = leaf;
        for _ in 0..64 {
            match self.model.find(cur).and_then(|i| self.model.objects[i].parent) {
                Some(p) => {
                    chain.push(p);
                    cur = p;
                }
                None => break,
            }
        }
        chain
    }

    /// Map a hit LEAF to what a click selects: something already
    /// selected on its chain (so a second click drags the group, not a
    /// member), else one level below the entered group, else the
    /// outermost group.
    fn pick_target(&self, leaf: u64) -> u64 {
        let chain = self.ancestor_chain(leaf);
        if let Some(&sel) = chain.iter().find(|&&c| self.sel_contains(c)) {
            return sel;
        }
        if let Some(e) = self.entered_group {
            if let Some(pos) = chain.iter().position(|&c| c == e) {
                if pos > 0 {
                    return chain[pos - 1];
                }
            }
        }
        *chain.last().unwrap_or(&leaf)
    }

    /// The direct child of `group` on the way down to `leaf`.
    fn child_toward(&self, group: u64, leaf: u64) -> u64 {
        let chain = self.ancestor_chain(leaf);
        chain
            .iter()
            .position(|&c| c == group)
            .and_then(|pos| pos.checked_sub(1))
            .map(|i| chain[i])
            .unwrap_or(leaf)
    }

    /// Band-select scope for a leaf: below the entered group when
    /// applicable, else the outermost ancestor.
    fn band_scope(&self, leaf: u64) -> u64 {
        let chain = self.ancestor_chain(leaf);
        if let Some(e) = self.entered_group {
            if let Some(pos) = chain.iter().position(|&c| c == e) {
                if pos > 0 {
                    return chain[pos - 1];
                }
            }
        }
        *chain.last().unwrap_or(&leaf)
    }

    /// A press with the Select tool. In order: grab a gizmo handle
    /// (pivot, rotate, corner scale) or a vertex handle of the single
    /// selected object — whichever is nearest; Shift-click toggles set
    /// membership; plain click picks (groups pick as a whole; a second
    /// click inside the selection moves it); empty space starts a
    /// rubber band — additive with Shift, replacing without.
    fn select_press(
        &mut self,
        p: [f32; 2],
        handle_r: f32,
        click_slop: f32,
        shift: bool,
        pt_per_cell: f32,
    ) {
        self.finish_gesture();

        // Candidate handles: the single object's vertex handles (world
        // space) and the selection gizmo's — nearest within reach wins.
        enum Grab {
            Vertex(usize),
            Corner,
            Rotate,
            Pivot,
        }
        let mut best: Option<(Grab, f32)> = None;
        let mut consider = |g: Grab, at: [f32; 2], p: [f32; 2], r: f32| {
            let d = Self::dist(p, at);
            if d <= r {
                if best.as_ref().map(|(_, bd)| d < *bd).unwrap_or(true) {
                    best = Some((g, d));
                }
            }
        };
        if let Some(id) = self.single_sel() {
            if let Some(i) = self.model.find(id) {
                if !self.model.eff_locked(id) {
                    let abs = self.model.parent_abs(id);
                    for (idx, h) in self.model.objects[i].handles().iter().enumerate() {
                        consider(Grab::Vertex(idx), abs.apply(*h), p, handle_r);
                    }
                }
            }
        }
        if let Some(giz) = self.gizmo_layout(pt_per_cell) {
            consider(Grab::Pivot, giz.pivot, p, handle_r);
            consider(Grab::Rotate, giz.rotate, p, handle_r);
            for c in giz.corners {
                consider(Grab::Corner, c, p, handle_r);
            }
        }
        match best.map(|(g, _)| g) {
            Some(Grab::Vertex(idx)) => {
                let id = self.single_sel().unwrap();
                if let Some(i) = self.model.find(id) {
                    let before = self.model.objects[i].clone();
                    self.gesture = Gesture::HandleDrag { id, idx, before };
                }
                return;
            }
            Some(Grab::Pivot) => {
                self.gesture = Gesture::GizmoPivot;
                return;
            }
            Some(grab @ (Grab::Rotate | Grab::Corner)) => {
                let Some(pivot) = self.transform_pivot() else { return };
                let before: Vec<(u64, SketchObject)> = self
                    .transform_targets()
                    .iter()
                    .filter_map(|&sid| {
                        self.model
                            .find(sid)
                            .map(|i| (sid, self.model.objects[i].clone()))
                    })
                    .collect();
                if before.is_empty() {
                    return;
                }
                self.gesture = match grab {
                    Grab::Rotate => Gesture::GizmoRotate {
                        before,
                        pivot,
                        start_ang: (p[1] - pivot[1]).atan2(p[0] - pivot[0]),
                        total: 0.0,
                    },
                    _ => Gesture::GizmoScale {
                        before,
                        pivot,
                        start_dist: Self::dist(p, pivot).max(1e-3),
                        total: 1.0,
                    },
                };
                return;
            }
            None => {}
        }

        match self.model.hit_test(p, click_slop) {
            Some(leaf) if shift => {
                // Shift-click adds/removes the picked scope; no move
                // gesture either way.
                let id = self.pick_target(leaf);
                self.select_toggle(id);
            }
            Some(leaf) => {
                let id = self.pick_target(leaf);
                if !self.sel_contains(id) {
                    self.select_only(id);
                }
                // Move the selection's outermost editable members as a
                // unit (a selected group moves; its members follow).
                let before: Vec<(u64, SketchObject)> = self
                    .transform_targets()
                    .iter()
                    .filter_map(|&sid| {
                        self.model.find(sid).map(|i| (sid, self.model.objects[i].clone()))
                    })
                    .collect();
                if !before.is_empty() {
                    let start =
                        if self.snap_enabled { self.snap_point(p) } else { p };
                    self.gesture = Gesture::MoveSel { before, last: start };
                }
            }
            None => {
                self.entered_group = None;
                let base = if shift {
                    self.selected.clone()
                } else {
                    self.deselect_all();
                    Vec::new()
                };
                self.gesture = Gesture::RubberBand {
                    anchor: p,
                    corner: p,
                    base,
                };
            }
        }
    }

    /// Live rubber-band update: selection = base ∪ (band hits mapped to
    /// their group scope), INTERSECT semantics (see CLAUDE.md),
    /// skipping locked and hidden subtrees.
    fn rubber_band_update(&mut self, corner: [f32; 2]) {
        let Gesture::RubberBand { anchor, corner: c, base, .. } = &mut self.gesture
        else {
            return;
        };
        *c = corner;
        let (anchor, base) = (*anchor, base.clone());
        let min = [anchor[0].min(corner[0]), anchor[1].min(corner[1])];
        let max = [anchor[0].max(corner[0]), anchor[1].max(corner[1])];
        self.selected = base;
        let hits: Vec<u64> = self
            .model
            .objects
            .iter()
            .filter(|o| {
                !matches!(o.shape, Shape::Group { .. })
                    && !self.model.eff_locked(o.id)
                    && !self.model.eff_hidden(o.id)
                    && self.band_hits(o, min, max)
            })
            .map(|o| o.id)
            .collect();
        for leaf in hits {
            let id = self.band_scope(leaf);
            self.select_add(id);
        }
    }

    /// Band test through the ancestor chain: identity chains test the
    /// stored geometry directly; transformed ones test a flattened
    /// clone (band drags are frame-rate work, the clone is accepted).
    fn band_hits(&self, o: &SketchObject, min: [f32; 2], max: [f32; 2]) -> bool {
        let abs = self.model.parent_abs(o.id);
        if abs.is_identity() {
            o.intersects_rect(min, max)
        } else {
            let mut flat = o.clone();
            flat.apply_sim(abs);
            flat.intersects_rect(min, max)
        }
    }

    /// A click while building a polyline: fix the rubber vertex (or close
    /// the polygon when clicking the first vertex).
    fn poly_click(&mut self, id: u64, raw: [f32; 2], shift: bool, handle_r: f32) {
        let Some(i) = self.model.find(id) else {
            self.gesture = Gesture::None;
            return;
        };
        let (first, prev, len) = match &self.model.objects[i].shape {
            Shape::Poly { pts, .. } => (
                pts.first().copied().unwrap_or(raw),
                if pts.len() >= 2 { pts[pts.len() - 2] } else { raw },
                pts.len(),
            ),
            _ => {
                self.gesture = Gesture::None;
                return;
            }
        };
        let p = if shift { self.angle_snap(prev, raw) } else { self.snap_active(raw) };
        if len >= 4 && Self::dist(p, first) <= handle_r {
            // Close the polygon: drop the rubber vertex and mark closed.
            self.gesture = Gesture::None;
            self.mutate_live(id, |o| {
                if let Shape::Poly { pts, closed } = &mut o.shape {
                    pts.pop();
                    *closed = true;
                }
            });
            self.model.finalize_last_add(id);
            self.select_only(id);
            self.status = "Closed the polygon.".into();
            return;
        }
        // Fix the rubber vertex at p and start the next one.
        self.mutate_live(id, |o| {
            if let Shape::Poly { pts, .. } = &mut o.shape {
                if let Some(last) = pts.last_mut() {
                    *last = p;
                }
                pts.push(p);
            }
        });
    }

    /// Rubber-band update for line/rect/ellipse drawing, with the CAD
    /// constraints: Shift angle-snaps lines and squares circles; Alt grows
    /// rects/ellipses from their centre.
    fn update_draw_shape(
        &mut self,
        id: u64,
        anchor: [f32; 2],
        raw: [f32; 2],
        shift: bool,
        alt: bool,
    ) {
        let is_line = self
            .model
            .find(id)
            .map(|i| matches!(self.model.objects[i].shape, Shape::Line { .. }))
            .unwrap_or(false);
        if is_line {
            let b = if shift { self.angle_snap(anchor, raw) } else { self.snap_active(raw) };
            self.mutate_live(id, |o| {
                if let Shape::Line { b: bb, .. } = &mut o.shape {
                    *bb = b;
                }
            });
            return;
        }
        let mut q = self.snap_active(raw);
        if shift {
            let dx = q[0] - anchor[0];
            let dy = q[1] - anchor[1];
            let m = dx.abs().max(dy.abs());
            q = [anchor[0] + m * dx.signum(), anchor[1] + m * dy.signum()];
        }
        let (c, half) = if alt {
            (
                anchor,
                [
                    (q[0] - anchor[0]).abs().max(0.5),
                    (q[1] - anchor[1]).abs().max(0.5),
                ],
            )
        } else {
            (
                [(anchor[0] + q[0]) * 0.5, (anchor[1] + q[1]) * 0.5],
                [
                    ((q[0] - anchor[0]).abs() * 0.5).max(0.5),
                    ((q[1] - anchor[1]).abs() * 0.5).max(0.5),
                ],
            )
        };
        self.mutate_live(id, |o| match &mut o.shape {
            Shape::Rect { c: cc, half: hh, .. } | Shape::Ellipse { c: cc, r: hh, .. } => {
                *cc = c;
                *hh = half;
            }
            _ => {}
        });
    }

    /// Selection outline, vertex handles, dimensions and the snap grid.
    fn canvas_overlays(&mut self, ui: &egui::Ui, mapping: ViewportMapping, ppp: f32) {
        let to_screen = |c: [f32; 2]| -> egui::Pos2 {
            egui::pos2(
                (mapping.lb_origin[0] + c[0] * mapping.px_per_cell) / ppp,
                (mapping.lb_origin[1] + c[1] * mapping.px_per_cell) / ppp,
            )
        };
        let painter = ui.painter();

        // Domain extent (U4): with the full grid rendered (margin
        // included), shade the margin ring, outline the usable
        // interior, and label the margin in cells and physical units.
        // View-only — every readout stays in visible-cell coordinates.
        if self.extent_on {
            let (vw, vh) = self.stats_grid;
            let m = self.stats_margin as f32;
            let interior =
                egui::Rect::from_two_pos(to_screen([0.0, 0.0]), to_screen([vw as f32, vh as f32]));
            if m > 0.0 {
                let outer = egui::Rect::from_two_pos(
                    to_screen([-m, -m]),
                    to_screen([vw as f32 + m, vh as f32 + m]),
                );
                let fill = super::theme::extent_margin_fill();
                // Four bands around the interior (no overlap).
                painter.rect_filled(
                    egui::Rect::from_min_max(outer.min, egui::pos2(outer.max.x, interior.min.y)),
                    0.0,
                    fill,
                );
                painter.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(outer.min.x, interior.max.y), outer.max),
                    0.0,
                    fill,
                );
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(outer.min.x, interior.min.y),
                        egui::pos2(interior.min.x, interior.max.y),
                    ),
                    0.0,
                    fill,
                );
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(interior.max.x, interior.min.y),
                        egui::pos2(outer.max.x, interior.max.y),
                    ),
                    0.0,
                    fill,
                );
            }
            painter.rect_stroke(
                interior,
                0.0,
                egui::Stroke::new(1.0, super::theme::EXTENT_OUTLINE),
            );
            let ps = self.phys_cache;
            let label = if self.stats_margin > 0 {
                format!(
                    "usable interior — sponge margin {} cells = {} each side",
                    self.stats_margin,
                    fmt_len(ps.len_m(m)),
                )
            } else {
                "usable interior — no simulated margin".to_string()
            };
            painter.text(
                interior.min + egui::vec2(6.0, 4.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::TextStyle::Monospace.resolve(ui.style()),
                super::theme::EXTENT_OUTLINE,
            );
        }

        // Faint snap grid while a draw tool is armed.
        if self.snap_enabled && self.tool != Tool::Select {
            let s = self.snap_spacing.max(1.0);
            let step_pt = s * mapping.px_per_cell / ppp;
            if step_pt >= 8.0 {
                let (vw, vh) = self.stats_grid;
                let stroke = egui::Stroke::new(
                    1.0,
                    super::theme::GRID_HINT,
                );
                // Clip the loops to the visible cell range: at high zoom
                // the grid extends far off-screen, and painting thousands
                // of clipped-away segments is pure waste.
                let clip = ui.clip_rect();
                let cell_lo = mapping.px_to_cell([
                    clip.min.x * ppp,
                    clip.min.y * ppp,
                ]);
                let cell_hi = mapping.px_to_cell([
                    clip.max.x * ppp,
                    clip.max.y * ppp,
                ]);
                let x_lo = cell_lo[0].max(0.0);
                let x_hi = cell_hi[0].min(vw as f32);
                let y_lo = cell_lo[1].max(0.0);
                let y_hi = cell_hi[1].min(vh as f32);
                let mut x = (x_lo / s).ceil() * s;
                while x <= x_hi + 0.1 {
                    painter.line_segment(
                        [to_screen([x, y_lo]), to_screen([x, y_hi])],
                        stroke,
                    );
                    x += s;
                }
                let mut y = (y_lo / s).ceil() * s;
                while y <= y_hi + 0.1 {
                    painter.line_segment(
                        [to_screen([x_lo, y]), to_screen([x_hi, y])],
                        stroke,
                    );
                    y += s;
                }
            }
        }

        // The rubber band, while one is being dragged.
        if let Gesture::RubberBand { anchor, corner, .. } = &self.gesture {
            let r = egui::Rect::from_two_pos(to_screen(*anchor), to_screen(*corner));
            painter.rect(
                r,
                0.0,
                super::theme::rubber_fill(),
                egui::Stroke::new(1.0, super::theme::SEL),
            );
        }

        // The active objects: the one being drawn/edited, else the whole
        // selection. Handles and dimensions draw only for a single one.
        let ids: Vec<u64> = match &self.gesture {
            Gesture::DrawShape { id, .. }
            | Gesture::DrawPoly { id }
            | Gesture::DrawPencil { id }
            | Gesture::HandleDrag { id, .. } => vec![*id],
            _ => self.selected.clone(),
        };
        let accent = super::theme::SEL;
        let stroke = egui::Stroke::new(1.5, accent);
        for &id in &ids {
            let Some(i) = self.model.find(id) else { continue };
            if matches!(self.model.objects[i].shape, Shape::Group { .. }) {
                // A group outlines every leaf of its subtree.
                for sid in self.model.subtree_ids(id) {
                    let Some(si) = self.model.find(sid) else { continue };
                    let so = &self.model.objects[si];
                    if !matches!(so.shape, Shape::Group { .. }) {
                        let abs = self.model.parent_abs(sid);
                        self.draw_outline(painter, so, abs, stroke, &to_screen);
                    }
                }
            } else {
                let abs = self.model.parent_abs(id);
                self.draw_outline(painter, &self.model.objects[i], abs, stroke, &to_screen);
            }
        }

        // The transform gizmo (U3): padded selection box with corner
        // scale handles, the rotate handle above, the pivot marker.
        let pt_per_cell = mapping.px_per_cell / ppp;
        self.draw_gizmo(ui, painter, pt_per_cell, &to_screen);

        // Eraser (U4): translucent preview of the swept stroke while it
        // collects, and the radius cursor whenever the tool is armed.
        if self.tool == Tool::Eraser {
            let r_pt = self.eraser_radius.max(0.5) * pt_per_cell;
            if let Gesture::Erase { pts } = &self.gesture {
                let fill = super::theme::eraser_fill();
                let path: Vec<egui::Pos2> = pts.iter().map(|p| to_screen(*p)).collect();
                for p in &path {
                    painter.circle_filled(*p, r_pt, fill);
                }
                if path.len() >= 2 {
                    painter.add(egui::Shape::line(
                        path,
                        egui::Stroke::new(2.0 * r_pt, fill),
                    ));
                }
            }
            if let Some(c) = self.hover_cell {
                painter.circle_stroke(
                    to_screen(c),
                    r_pt,
                    egui::Stroke::new(1.0, super::theme::BAD),
                );
            }
        }

        // Measure (U4): the picked span with its live readout.
        if let Gesture::Measure { a, b } = &self.gesture {
            let (pa, pb) = (to_screen(*a), to_screen(*b));
            let stroke = egui::Stroke::new(1.5, super::theme::SNAP_MARK);
            painter.line_segment([pa, pb], stroke);
            painter.circle_stroke(pa, 3.0, stroke);
            painter.circle_stroke(pb, 3.0, stroke);
            let ps = self.phys_cache;
            let l = Self::dist(*a, *b);
            if l > 0.1 {
                let ang = -(b[1] - a[1]).atan2(b[0] - a[0]).to_degrees();
                painter.text(
                    egui::pos2((pa.x + pb.x) * 0.5, (pa.y + pb.y) * 0.5 - 10.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("L {}   ∠ {}", fmt_len(ps.len_m(l)), fmt_angle(ang)),
                    egui::TextStyle::Monospace.resolve(ui.style()),
                    super::theme::SNAP_MARK,
                );
            }
        }

        // Object-snap indicator (U4): a kind-shaped marker at the
        // candidate, with its name alongside.
        if let Some(hit) = &self.osnap_hit {
            let p = to_screen(hit.pos);
            let stroke = egui::Stroke::new(1.5, super::theme::SNAP_MARK);
            match hit.kind {
                OsnapKind::Endpoint => {
                    painter.rect_stroke(
                        egui::Rect::from_center_size(p, egui::vec2(9.0, 9.0)),
                        0.0,
                        stroke,
                    );
                }
                OsnapKind::Midpoint => {
                    // Triangle.
                    let pts = vec![
                        p + egui::vec2(0.0, -5.5),
                        p + egui::vec2(5.0, 4.0),
                        p + egui::vec2(-5.0, 4.0),
                    ];
                    painter.add(egui::Shape::closed_line(pts, stroke));
                }
                OsnapKind::Center => {
                    painter.circle_stroke(p, 5.0, stroke);
                    painter.circle_filled(p, 1.5, super::theme::SNAP_MARK);
                }
                OsnapKind::Intersection => {
                    painter.line_segment(
                        [p + egui::vec2(-4.5, -4.5), p + egui::vec2(4.5, 4.5)],
                        stroke,
                    );
                    painter.line_segment(
                        [p + egui::vec2(-4.5, 4.5), p + egui::vec2(4.5, -4.5)],
                        stroke,
                    );
                }
                OsnapKind::Perpendicular => {
                    // Right-angle mark.
                    painter.line_segment(
                        [p + egui::vec2(-5.0, 5.0), p + egui::vec2(-5.0, -3.0)],
                        stroke,
                    );
                    painter.line_segment(
                        [p + egui::vec2(-5.0, 5.0), p + egui::vec2(3.0, 5.0)],
                        stroke,
                    );
                    painter.line_segment(
                        [p + egui::vec2(-5.0, 0.0), p + egui::vec2(0.0, 0.0)],
                        stroke,
                    );
                    painter.line_segment(
                        [p + egui::vec2(0.0, 0.0), p + egui::vec2(0.0, 5.0)],
                        stroke,
                    );
                }
            }
            painter.text(
                p + egui::vec2(8.0, -8.0),
                egui::Align2::LEFT_BOTTOM,
                hit.kind.label(),
                egui::TextStyle::Small.resolve(ui.style()),
                super::theme::SNAP_MARK,
            );
        }

        if ids.len() != 1 {
            return;
        }
        let Some(i) = self.model.find(ids[0]) else { return };
        let obj = &self.model.objects[i];
        let abs = self.model.parent_abs(ids[0]);

        // Vertex handles (not on locked objects — they aren't editable;
        // shown at their WORLD positions).
        if !self.model.eff_locked(ids[0]) {
            for h in obj.handles() {
                let pos = to_screen(abs.apply(h));
                let r = egui::Rect::from_center_size(pos, egui::vec2(7.0, 7.0));
                painter.rect_filled(r, 1.0, super::theme::HANDLE_FILL);
                painter.rect_stroke(
                    r,
                    1.0,
                    egui::Stroke::new(1.0, super::theme::HANDLE_OUTLINE),
                );
            }
        }

        // Dimensions in physical units (world lengths — the ancestor
        // scale applies).
        let ps = self.phys_cache;
        let s = abs.s;
        let dims = match &obj.shape {
            Shape::Line { a, b } => {
                let aw = abs.apply(*a);
                let bw = abs.apply(*b);
                let l = Self::dist(aw, bw);
                let ang = -(bw[1] - aw[1]).atan2(bw[0] - aw[0]).to_degrees();
                format!("L {}   ∠ {}", fmt_len(ps.len_m(l)), fmt_angle(ang))
            }
            Shape::Poly { pts, closed } => {
                let n = pts.len();
                let segs = if *closed { n } else { n.saturating_sub(1) };
                let mut l = 0.0;
                for k in 0..segs {
                    l += Self::dist(pts[k], pts[(k + 1) % n]);
                }
                format!("{n} pts   L {}", fmt_len(ps.len_m(l * s)))
            }
            Shape::Rect { half, .. } => format!(
                "{} × {}",
                fmt_len(ps.len_m(half[0] * 2.0 * s)),
                fmt_len(ps.len_m(half[1] * 2.0 * s))
            ),
            Shape::Ellipse { r, .. } => format!(
                "⌀ {} × {}",
                fmt_len(ps.len_m(r[0] * 2.0 * s)),
                fmt_len(ps.len_m(r[1] * 2.0 * s))
            ),
            Shape::Stamp { raster, scale, .. } => format!(
                "{} × {}",
                fmt_len(ps.len_m((raster.rect.2 - raster.rect.0) as f32 * scale * s)),
                fmt_len(ps.len_m((raster.rect.3 - raster.rect.1) as f32 * scale * s))
            ),
            Shape::Group { .. } => String::new(),
        };
        if dims.is_empty() {
            return;
        }
        let b = obj.bounds_under(abs);
        let pos = to_screen([b.x0 as f32, b.y0 as f32]) - egui::vec2(0.0, 4.0);
        painter.text(
            pos,
            egui::Align2::LEFT_BOTTOM,
            dims,
            egui::TextStyle::Monospace.resolve(ui.style()),
            accent,
        );
    }

    /// The gizmo's world-cell geometry for this frame: the padded
    /// selection box, corner scale handles, the rotate handle above the
    /// top edge, and the transform pivot. None when the Select tool is
    /// inactive, the selection is empty or uneditable, or a non-gizmo
    /// gesture is running.
    fn gizmo_layout(&mut self, pt_per_cell: f32) -> Option<GizmoLayout> {
        if self.tool != Tool::Select {
            return None;
        }
        if !matches!(
            self.gesture,
            Gesture::None
                | Gesture::GizmoRotate { .. }
                | Gesture::GizmoScale { .. }
                | Gesture::GizmoPivot
        ) {
            return None;
        }
        if self.transform_targets().is_empty() {
            return None;
        }
        let b = self.selection_world_bounds()?;
        let k = pt_per_cell.max(1e-6);
        let pad = GIZMO_PAD_PT / k;
        let box_min = [b.x0 as f32 - pad, b.y0 as f32 - pad];
        let box_max = [b.x1 as f32 + pad, b.y1 as f32 + pad];
        let corners = [
            [box_min[0], box_min[1]],
            [box_max[0], box_min[1]],
            [box_max[0], box_max[1]],
            [box_min[0], box_max[1]],
        ];
        let rotate = [
            (box_min[0] + box_max[0]) * 0.5,
            box_min[1] - GIZMO_ROT_OFF_PT / k,
        ];
        // Mid-gesture the pivot is the gesture's (the live bounds move
        // under a rotation; the pivot must not).
        let pivot = match &self.gesture {
            Gesture::GizmoRotate { pivot, .. } | Gesture::GizmoScale { pivot, .. } => {
                *pivot
            }
            _ => self.transform_pivot()?,
        };
        Some(GizmoLayout { box_min, box_max, corners, rotate, pivot })
    }

    /// Draw the gizmo and, mid-drag, the live numeric readout.
    fn draw_gizmo(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        pt_per_cell: f32,
        to_screen: &impl Fn([f32; 2]) -> egui::Pos2,
    ) {
        let Some(giz) = self.gizmo_layout(pt_per_cell) else { return };
        let accent = super::theme::SEL;
        let thin = egui::Stroke::new(1.0, accent);
        let box_rect =
            egui::Rect::from_two_pos(to_screen(giz.box_min), to_screen(giz.box_max));
        painter.rect_stroke(box_rect, 0.0, thin);
        // Corner scale handles.
        for c in giz.corners {
            let r = egui::Rect::from_center_size(
                to_screen(c),
                egui::vec2(GIZMO_HANDLE_PT, GIZMO_HANDLE_PT),
            );
            painter.rect_filled(r, 1.0, super::theme::HANDLE_FILL);
            painter.rect_stroke(r, 1.0, egui::Stroke::new(1.0, accent));
        }
        // Rotate handle: a lollipop above the top edge.
        let rot_pos = to_screen(giz.rotate);
        painter.line_segment(
            [egui::pos2(box_rect.center().x, box_rect.min.y), rot_pos],
            thin,
        );
        painter.circle_filled(rot_pos, GIZMO_HANDLE_PT * 0.5, super::theme::HANDLE_FILL);
        painter.circle_stroke(rot_pos, GIZMO_HANDLE_PT * 0.5, egui::Stroke::new(1.0, accent));
        // Pivot: crosshair circle.
        let piv = to_screen(giz.pivot);
        painter.circle_stroke(piv, 5.0, thin);
        painter.line_segment([piv - egui::vec2(8.0, 0.0), piv + egui::vec2(8.0, 0.0)], thin);
        painter.line_segment([piv - egui::vec2(0.0, 8.0), piv + egui::vec2(0.0, 8.0)], thin);

        // Live readout while a gizmo drag runs (through ui/units.rs).
        let readout = match &self.gesture {
            Gesture::GizmoRotate { total, .. } => {
                Some(fmt_angle(-total.to_degrees()))
            }
            Gesture::GizmoScale { total, .. } => Some(fmt_factor(*total)),
            _ => None,
        };
        if let Some(text) = readout {
            painter.text(
                rot_pos - egui::vec2(0.0, 12.0),
                egui::Align2::CENTER_BOTTOM,
                text,
                egui::TextStyle::Monospace.resolve(ui.style()),
                accent,
            );
        }
    }

    /// One object's selection outline under its ancestor transform
    /// (shared by the single- and multi-selection overlay paths). All
    /// outline points are generated in stored space and mapped through
    /// `abs` — a similarity keeps rects rects and ellipses ellipses, so
    /// no shape needs a raster-heavy flatten here.
    fn draw_outline(
        &self,
        painter: &egui::Painter,
        obj: &SketchObject,
        abs: Sim2,
        stroke: egui::Stroke,
        screen: &impl Fn([f32; 2]) -> egui::Pos2,
    ) {
        let to_screen = |p: [f32; 2]| screen(abs.apply(p));
        match &obj.shape {
            Shape::Line { a, b } => {
                painter.line_segment([to_screen(*a), to_screen(*b)], stroke);
            }
            Shape::Poly { pts, closed } => {
                let path: Vec<egui::Pos2> = pts.iter().map(|p| to_screen(*p)).collect();
                if *closed {
                    painter.add(egui::Shape::closed_line(path, stroke));
                } else {
                    painter.add(egui::Shape::line(path, stroke));
                }
            }
            Shape::Rect { .. } => {
                let path: Vec<egui::Pos2> =
                    obj.handles().iter().map(|p| to_screen(*p)).collect();
                painter.add(egui::Shape::closed_line(path, stroke));
            }
            Shape::Ellipse { c, r, angle } => {
                let (s, co) = angle.sin_cos();
                let path: Vec<egui::Pos2> = (0..48)
                    .map(|k| {
                        let t = k as f32 / 48.0 * std::f32::consts::TAU;
                        let lx = r[0] * t.cos();
                        let ly = r[1] * t.sin();
                        to_screen([
                            c[0] + lx * co - ly * s,
                            c[1] + lx * s + ly * co,
                        ])
                    })
                    .collect();
                painter.add(egui::Shape::closed_line(path, stroke));
            }
            Shape::Stamp { raster, c, scale, angle } => {
                let w = (raster.rect.2 - raster.rect.0).max(0) as f32 * 0.5 * scale;
                let h = (raster.rect.3 - raster.rect.1).max(0) as f32 * 0.5 * scale;
                let (s, co) = angle.sin_cos();
                let path: Vec<egui::Pos2> = [
                    (-w, -h),
                    (w, -h),
                    (w, h),
                    (-w, h),
                ]
                .into_iter()
                .map(|(lx, ly)| {
                    to_screen([c[0] + lx * co - ly * s, c[1] + lx * s + ly * co])
                })
                .collect();
                painter.add(egui::Shape::closed_line(path, stroke));
            }
            Shape::Group { .. } => {} // outlined via its subtree leaves
        }
    }
}
struct FlowPaintCallback;

impl egui_wgpu::CallbackTrait for FlowPaintCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(sim) = resources.get_mut::<GpuSim>() {
            sim.encode_compute(encoder);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(sim) = resources.get::<GpuSim>() {
            sim.draw(render_pass);
        }
    }
}
