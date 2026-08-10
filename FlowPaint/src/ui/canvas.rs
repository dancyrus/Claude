//! The graphics window: the wgpu paint callback, the pointer gesture
//! state machine, and the selection/snap overlays.

use crate::app::{Cmd, FlowPaintApp, Gesture, Tool, ViewRequest};
use crate::model::{Shape, SketchObject};
use crate::sim::{GpuSim, ViewportMapping};
use eframe::egui;

use super::units::{fmt_angle, fmt_len};

// Free-zoom bounds: view_zoom is a multiplier over the letterbox fit
// scale, additionally capped in absolute framebuffer px per cell; the
// pan clamp keeps at least this much grid on screen in each axis.
const ZOOM_MIN: f32 = 0.125;
const ZOOM_MAX: f32 = 64.0;
const MAX_PX_PER_CELL: f32 = 512.0;
const MIN_VISIBLE_PX: f32 = 64.0;
/// Wheel-zoom rate per scroll point (factor = exp(rate * delta)).
const ZOOM_WHEEL_RATE: f32 = 0.005;

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
                cmds.push(Cmd::SetMapping(mapping));

                self.canvas_interaction(&response, mapping, ppp, pan_mode);

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
                // Union of the whole selection's bounds.
                let mut b: Option<crate::geometry::GridRect> = None;
                for &id in &self.selected {
                    if let Some(i) = self.model.find(id) {
                        let ob = self.model.objects[i].bounds();
                        b = Some(match b {
                            Some(u) => u.union(ob),
                            None => ob,
                        });
                    }
                }
                let Some(b) = b else { return };
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
        let (shift, alt) =
            response.ctx.input(|i| (i.modifiers.shift, i.modifiers.alt));
        let pointer = response.interact_pointer_pos();

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
                        None => self.snap_point(raw),
                    }
                } else {
                    self.snap_point(raw)
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
                    Tool::Select => self.select_press(raw, handle_r, click_slop, shift),
                    Tool::Line => {
                        self.finish_gesture();
                        let a = self.snap_point(raw);
                        let obj = self.new_object(Shape::Line { a, b: a });
                        let id = obj.id;
                        self.model.add(obj);
                        self.gesture = Gesture::DrawShape { id, anchor: a };
                        self.select_only(id);
                    }
                    Tool::Rect | Tool::Ellipse => {
                        self.finish_gesture();
                        let a = self.snap_point(raw);
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
                            let p = self.snap_point(raw);
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
                            self.mutate_live(id, |o| o.translate(d));
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
                            None => self.snap_point(raw),
                        }
                    } else {
                        self.snap_point(raw)
                    };
                    self.mutate_live(id, |o| o.set_handle(idx, p));
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
            )
        {
            self.finish_gesture();
        }
    }

    /// A press with the Select tool. In order: grab a handle of the
    /// single selected object; Shift-click toggles set membership;
    /// plain click picks (and starts moving the whole selection, or
    /// just the hit object after reselecting); empty space starts a
    /// rubber band — additive with Shift, replacing without.
    fn select_press(&mut self, p: [f32; 2], handle_r: f32, click_slop: f32, shift: bool) {
        self.finish_gesture();
        // Handle grab: single selection only (handles aren't drawn for
        // multi-selections), and never on a locked object.
        if let Some(id) = self.single_sel() {
            if let Some(i) = self.model.find(id) {
                if !self.model.objects[i].locked {
                    let handles = self.model.objects[i].handles();
                    let mut best: Option<(usize, f32)> = None;
                    for (idx, h) in handles.iter().enumerate() {
                        let d = Self::dist(p, *h);
                        if d <= handle_r && best.map(|(_, bd)| d < bd).unwrap_or(true)
                        {
                            best = Some((idx, d));
                        }
                    }
                    if let Some((idx, _)) = best {
                        let before = self.model.objects[i].clone();
                        self.gesture = Gesture::HandleDrag { id, idx, before };
                        return;
                    }
                }
            }
        }
        match self.model.hit_test(p, click_slop) {
            Some(id) if shift => {
                // Shift-click adds/removes; no move gesture either way.
                self.select_toggle(id);
            }
            Some(id) => {
                if !self.sel_contains(id) {
                    self.select_only(id);
                }
                // Move every editable member of the selection as a unit.
                let before: Vec<(u64, SketchObject)> = self
                    .editable_selection()
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

    /// Live rubber-band update: selection = base ∪ (band hits), INTERSECT
    /// semantics (see CLAUDE.md), skipping locked and hidden objects.
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
            .filter(|o| !o.locked && !o.hidden && o.intersects_rect(min, max))
            .map(|o| o.id)
            .collect();
        for id in hits {
            self.select_add(id);
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
        let p = if shift { self.angle_snap(prev, raw) } else { self.snap_point(raw) };
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
            let b = if shift { self.angle_snap(anchor, raw) } else { self.snap_point(raw) };
            self.mutate_live(id, |o| {
                if let Shape::Line { b: bb, .. } = &mut o.shape {
                    *bb = b;
                }
            });
            return;
        }
        let mut q = self.snap_point(raw);
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
            Gesture::MoveSel { .. } | Gesture::RubberBand { .. } | Gesture::None => {
                self.selected.clone()
            }
        };
        let accent = super::theme::SEL;
        let stroke = egui::Stroke::new(1.5, accent);
        for &id in &ids {
            let Some(i) = self.model.find(id) else { continue };
            self.draw_outline(painter, &self.model.objects[i], stroke, &to_screen);
        }
        if ids.len() != 1 {
            return;
        }
        let Some(i) = self.model.find(ids[0]) else { return };
        let obj = &self.model.objects[i];

        // Vertex handles (not on locked objects — they aren't editable).
        if !obj.locked {
            for h in obj.handles() {
                let pos = to_screen(h);
                let r = egui::Rect::from_center_size(pos, egui::vec2(7.0, 7.0));
                painter.rect_filled(r, 1.0, super::theme::HANDLE_FILL);
                painter.rect_stroke(
                    r,
                    1.0,
                    egui::Stroke::new(1.0, super::theme::HANDLE_OUTLINE),
                );
            }
        }

        // Dimensions in physical units.
        let ps = self.phys_cache;
        let dims = match &obj.shape {
            Shape::Line { a, b } => {
                let l = Self::dist(*a, *b);
                let ang = -(b[1] - a[1]).atan2(b[0] - a[0]).to_degrees();
                format!("L {}   ∠ {}", fmt_len(ps.len_m(l)), fmt_angle(ang))
            }
            Shape::Poly { pts, closed } => {
                let n = pts.len();
                let segs = if *closed { n } else { n.saturating_sub(1) };
                let mut l = 0.0;
                for k in 0..segs {
                    l += Self::dist(pts[k], pts[(k + 1) % n]);
                }
                format!("{n} pts   L {}", fmt_len(ps.len_m(l)))
            }
            Shape::Rect { half, .. } => format!(
                "{} × {}",
                fmt_len(ps.len_m(half[0] * 2.0)),
                fmt_len(ps.len_m(half[1] * 2.0))
            ),
            Shape::Ellipse { r, .. } => format!(
                "⌀ {} × {}",
                fmt_len(ps.len_m(r[0] * 2.0)),
                fmt_len(ps.len_m(r[1] * 2.0))
            ),
            Shape::Stamp { raster, scale, .. } => format!(
                "{} × {}",
                fmt_len(ps.len_m((raster.rect.2 - raster.rect.0) as f32 * scale)),
                fmt_len(ps.len_m((raster.rect.3 - raster.rect.1) as f32 * scale))
            ),
        };
        let b = obj.bounds();
        let pos = to_screen([b.x0 as f32, b.y0 as f32]) - egui::vec2(0.0, 4.0);
        painter.text(
            pos,
            egui::Align2::LEFT_BOTTOM,
            dims,
            egui::TextStyle::Monospace.resolve(ui.style()),
            accent,
        );
    }

    /// One object's selection outline (shared by the single- and
    /// multi-selection overlay paths).
    fn draw_outline(
        &self,
        painter: &egui::Painter,
        obj: &SketchObject,
        stroke: egui::Stroke,
        to_screen: &impl Fn([f32; 2]) -> egui::Pos2,
    ) {
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
