//! The graphics window: the wgpu paint callback, the pointer gesture
//! state machine, and the selection/snap overlays.

use crate::app::{Cmd, FlowPaintApp, Gesture, Tool};
use crate::model::Shape;
use crate::sim::{GpuSim, ViewportMapping};
use eframe::egui;

use super::units::{fmt_angle, fmt_len};

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
                let mapping =
                    ViewportMapping::fit([x0, y0], [x1 - x0, y1 - y0], gw, gh);
                cmds.push(Cmd::SetMapping(mapping));

                self.canvas_interaction(&response, mapping, ppp);

                // The simulation paints itself via the wgpu callback.
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    FlowPaintCallback,
                ));

                self.canvas_overlays(ui, mapping, ppp);
            });
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
    ) {
        let to_cell = |pos: egui::Pos2| -> [f32; 2] {
            mapping.px_to_cell([pos.x * ppp, pos.y * ppp])
        };
        self.hover_cell = response.hover_pos().map(to_cell);

        let px_per_cell = mapping.px_per_cell.max(1e-3);
        // Pick thresholds in screen space so they feel the same at every
        // zoom, with a floor in cells for very zoomed-in grids.
        let handle_r = (8.0 * ppp / px_per_cell).max(2.0);
        let click_slop = (4.0 * ppp / px_per_cell).max(2.0);
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
                        self.selected = None;
                    }
                }
                _ => {}
            }
        }

        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer {
                let raw = to_cell(pos);
                match self.tool {
                    Tool::Select => self.select_press(raw, handle_r, click_slop),
                    Tool::Line => {
                        self.finish_gesture();
                        let a = self.snap_point(raw);
                        let obj = self.new_object(Shape::Line { a, b: a });
                        let id = obj.id;
                        self.model.add(obj);
                        self.gesture = Gesture::DrawShape { id, anchor: a };
                        self.selected = Some(id);
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
                        self.selected = Some(id);
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
                            self.selected = Some(id);
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
                        self.selected = Some(id);
                    }
                }
            }
        }

        // --- Drag updates ---------------------------------------------

        if response.dragged_by(egui::PointerButton::Primary) {
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
                } else if let Gesture::MoveObj { id, last, .. } = &self.gesture {
                    let (id, last) = (*id, *last);
                    let eff = if self.snap_enabled { self.snap_point(raw) } else { raw };
                    let d = [eff[0] - last[0], eff[1] - last[1]];
                    if d != [0.0; 2] {
                        self.mutate_live(id, |o| o.translate(d));
                        if let Gesture::MoveObj { last, .. } = &mut self.gesture {
                            *last = eff;
                        }
                    }
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
                Gesture::MoveObj { .. }
                    | Gesture::HandleDrag { .. }
                    | Gesture::DrawShape { .. }
                    | Gesture::DrawPencil { .. }
            )
        {
            self.finish_gesture();
        }
    }

    /// A press with the Select tool: grab a handle of the selected object,
    /// else pick (and start moving) the topmost object under the cursor,
    /// else clear the selection.
    fn select_press(&mut self, p: [f32; 2], handle_r: f32, click_slop: f32) {
        self.finish_gesture();
        if let Some(id) = self.selected {
            if let Some(i) = self.model.find(id) {
                let handles = self.model.objects[i].handles();
                let mut best: Option<(usize, f32)> = None;
                for (idx, h) in handles.iter().enumerate() {
                    let d = Self::dist(p, *h);
                    if d <= handle_r && best.map(|(_, bd)| d < bd).unwrap_or(true) {
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
        if let Some(id) = self.model.hit_test(p, click_slop) {
            self.selected = Some(id);
            if let Some(i) = self.model.find(id) {
                let before = self.model.objects[i].clone();
                let start = if self.snap_enabled { self.snap_point(p) } else { p };
                self.gesture = Gesture::MoveObj { id, before, last: start };
            }
        } else {
            self.selected = None;
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
            self.selected = Some(id);
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
                let mut x = 0.0f32;
                while x <= vw as f32 + 0.1 {
                    painter.line_segment(
                        [to_screen([x, 0.0]), to_screen([x, vh as f32])],
                        stroke,
                    );
                    x += s;
                }
                let mut y = 0.0f32;
                while y <= vh as f32 + 0.1 {
                    painter.line_segment(
                        [to_screen([0.0, y]), to_screen([vw as f32, y])],
                        stroke,
                    );
                    y += s;
                }
            }
        }

        // The active object: the one being drawn/edited, else the selection.
        let active = match &self.gesture {
            Gesture::DrawShape { id, .. }
            | Gesture::DrawPoly { id }
            | Gesture::DrawPencil { id }
            | Gesture::MoveObj { id, .. }
            | Gesture::HandleDrag { id, .. } => Some(*id),
            Gesture::None => self.selected,
        };
        let Some(id) = active else { return };
        let Some(i) = self.model.find(id) else { return };
        let obj = &self.model.objects[i];

        let accent = super::theme::SEL;
        let stroke = egui::Stroke::new(1.5, accent);

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

        // Vertex handles.
        for h in obj.handles() {
            let pos = to_screen(h);
            let r = egui::Rect::from_center_size(pos, egui::vec2(7.0, 7.0));
            painter.rect_filled(r, 1.0, super::theme::HANDLE_FILL);
            painter.rect_stroke(r, 1.0, egui::Stroke::new(1.0, super::theme::HANDLE_OUTLINE));
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
