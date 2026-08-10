//! The persistent sketch model: everything drawn is a vector object that
//! stays selectable and editable forever. The solver grid is a projection
//! of this model — objects are rasterized into the geometry layers
//! continuously (damage-region based), never destructively "committed".
//!
//! Coordinates are in VISIBLE-canvas cells; rasterization adds the
//! off-screen margin. On resolution changes the model is scaled, so
//! re-rasterization stays crisp (no raster resampling).

use crate::geometry::{
    GeoRegion, Geometry, GridRect, CELL_FLUID, CELL_INLET, CELL_OUTLET, CELL_WALL,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjMaterial {
    Wall,
    Fan,
    Smoke,
    Drain,
}

impl ObjMaterial {
    pub fn label(&self) -> &'static str {
        match self {
            ObjMaterial::Wall => "Wall",
            ObjMaterial::Fan => "Fan",
            ObjMaterial::Smoke => "Smoke",
            ObjMaterial::Drain => "Drain",
        }
    }
}

/// Shape geometry, in visible-canvas cells.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    /// Straight segment.
    Line { a: [f32; 2], b: [f32; 2] },
    /// Connected vertices (sketch polyline or simplified freehand stroke).
    Poly { pts: Vec<[f32; 2]>, closed: bool },
    /// Rotatable rectangle: centre, half-extents, angle (radians).
    Rect { c: [f32; 2], half: [f32; 2], angle: f32 },
    /// Rotatable ellipse: centre, radii, angle (radians).
    Ellipse { c: [f32; 2], r: [f32; 2], angle: f32 },
    /// Raster payload (generator output), with its own transform.
    Stamp { raster: GeoRegion, c: [f32; 2], scale: f32, angle: f32 },
}

/// One sketch object: shape + physical properties.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct SketchObject {
    pub id: u64,
    pub shape: Shape,
    pub material: ObjMaterial,
    /// Outline thickness in cells (ignored when `filled`).
    pub thickness: f32,
    /// Rect/Ellipse only: solid instead of outline.
    pub filled: bool,
    /// Fan physics (used when material == Fan).
    pub fan_mult: f32,
    pub fan_gust: f32,
    pub fan_phase: f32,
    /// Fan direction for filled shapes / stamps, radians (chained shapes
    /// blow along their segments instead).
    pub fan_angle: f32,
    /// Smoke color (used when material == Smoke, and for fan smoke).
    pub smoke_rgb: [f32; 3],
}

impl SketchObject {
    /// Axis-aligned bounds in visible cells, including thickness.
    pub fn bounds(&self) -> GridRect {
        let pad = (self.thickness * 0.5 + 2.0).ceil();
        let (min, max) = match &self.shape {
            Shape::Line { a, b } => (
                [a[0].min(b[0]), a[1].min(b[1])],
                [a[0].max(b[0]), a[1].max(b[1])],
            ),
            Shape::Poly { pts, .. } => {
                // A zero-point poly (corrupt file) must yield an empty
                // rect, not a fold over f32::MAX that overflows the casts.
                if pts.is_empty() {
                    return GridRect { x0: 0, y0: 0, x1: 0, y1: 0 };
                }
                let mut min = [f32::MAX, f32::MAX];
                let mut max = [f32::MIN, f32::MIN];
                for p in pts {
                    min[0] = min[0].min(p[0]);
                    min[1] = min[1].min(p[1]);
                    max[0] = max[0].max(p[0]);
                    max[1] = max[1].max(p[1]);
                }
                (min, max)
            }
            Shape::Rect { c, half, angle } | Shape::Ellipse { c, r: half, angle } => {
                let (s, co) = angle.sin_cos();
                let ex = (half[0] * co).abs() + (half[1] * s).abs();
                let ey = (half[0] * s).abs() + (half[1] * co).abs();
                ([c[0] - ex, c[1] - ey], [c[0] + ex, c[1] + ey])
            }
            Shape::Stamp { raster, c, scale, angle } => {
                let (w, h) = raster_dims(raster);
                let hx = w as f32 * 0.5 * scale;
                let hy = h as f32 * 0.5 * scale;
                let (s, co) = angle.sin_cos();
                let ex = (hx * co).abs() + (hy * s).abs();
                let ey = (hx * s).abs() + (hy * co).abs();
                ([c[0] - ex, c[1] - ey], [c[0] + ex, c[1] + ey])
            }
        };
        GridRect {
            x0: (min[0] - pad) as i32,
            y0: (min[1] - pad) as i32,
            x1: (max[0] + pad) as i32 + 1,
            y1: (max[1] + pad) as i32 + 1,
        }
    }

    /// Object centre (translation origin, rotation pivot).
    pub fn center(&self) -> [f32; 2] {
        match &self.shape {
            Shape::Line { a, b } => [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5],
            Shape::Poly { pts, .. } => {
                if pts.is_empty() {
                    return [0.0, 0.0];
                }
                let mut c = [0.0f32, 0.0];
                for p in pts {
                    c[0] += p[0];
                    c[1] += p[1];
                }
                [c[0] / pts.len() as f32, c[1] / pts.len() as f32]
            }
            Shape::Rect { c, .. } | Shape::Ellipse { c, .. } | Shape::Stamp { c, .. } => *c,
        }
    }

    pub fn translate(&mut self, d: [f32; 2]) {
        match &mut self.shape {
            Shape::Line { a, b } => {
                a[0] += d[0];
                a[1] += d[1];
                b[0] += d[0];
                b[1] += d[1];
            }
            Shape::Poly { pts, .. } => {
                for p in pts {
                    p[0] += d[0];
                    p[1] += d[1];
                }
            }
            Shape::Rect { c, .. } | Shape::Ellipse { c, .. } | Shape::Stamp { c, .. } => {
                c[0] += d[0];
                c[1] += d[1];
            }
        }
    }

    /// Rotate by `da` radians about the centre (baked for point shapes).
    pub fn rotate_by(&mut self, da: f32) {
        let ctr = self.center();
        let (s, co) = da.sin_cos();
        let rot = |p: &mut [f32; 2]| {
            let dx = p[0] - ctr[0];
            let dy = p[1] - ctr[1];
            p[0] = ctr[0] + dx * co - dy * s;
            p[1] = ctr[1] + dx * s + dy * co;
        };
        match &mut self.shape {
            Shape::Line { a, b } => {
                rot(a);
                rot(b);
            }
            Shape::Poly { pts, .. } => {
                for p in pts {
                    rot(p);
                }
            }
            Shape::Rect { angle, .. }
            | Shape::Ellipse { angle, .. }
            | Shape::Stamp { angle, .. } => *angle += da,
        }
        self.fan_angle += da;
    }

    /// Scale about the centre.
    pub fn scale_by(&mut self, f: f32) {
        let f = f.clamp(0.05, 50.0);
        let ctr = self.center();
        let sc = |p: &mut [f32; 2]| {
            p[0] = ctr[0] + (p[0] - ctr[0]) * f;
            p[1] = ctr[1] + (p[1] - ctr[1]) * f;
        };
        match &mut self.shape {
            Shape::Line { a, b } => {
                sc(a);
                sc(b);
            }
            Shape::Poly { pts, .. } => {
                for p in pts {
                    sc(p);
                }
            }
            Shape::Rect { half, .. } | Shape::Ellipse { r: half, .. } => {
                half[0] *= f;
                half[1] *= f;
            }
            Shape::Stamp { scale, .. } => *scale *= f,
        }
    }

    /// Uniformly rescale everything (resolution switches).
    pub fn rescale_all(&mut self, f: f32) {
        let sc = |p: &mut [f32; 2]| {
            p[0] *= f;
            p[1] *= f;
        };
        match &mut self.shape {
            Shape::Line { a, b } => {
                sc(a);
                sc(b);
            }
            Shape::Poly { pts, .. } => {
                for p in pts {
                    sc(p);
                }
            }
            Shape::Rect { c, half, .. } => {
                sc(c);
                half[0] *= f;
                half[1] *= f;
            }
            Shape::Ellipse { c, r, .. } => {
                sc(c);
                r[0] *= f;
                r[1] *= f;
            }
            Shape::Stamp { c, scale, .. } => {
                sc(c);
                *scale *= f;
            }
        }
        self.thickness *= f;
    }

    /// Hit test in visible cells with a screen-space slop (cells).
    pub fn hit(&self, p: [f32; 2], slop: f32) -> bool {
        let t = self.thickness * 0.5 + slop;
        match &self.shape {
            Shape::Line { a, b } => seg_dist(p, *a, *b) <= t,
            Shape::Poly { pts, closed } => {
                let n = pts.len();
                if n == 1 {
                    return dist(p, pts[0]) <= t;
                }
                let segs = if *closed { n } else { n.saturating_sub(1) };
                (0..segs).any(|i| seg_dist(p, pts[i], pts[(i + 1) % n]) <= t)
            }
            Shape::Rect { c, half, angle } => {
                let l = to_local(p, *c, *angle);
                if self.filled {
                    l[0].abs() <= half[0] + slop && l[1].abs() <= half[1] + slop
                } else {
                    let dx = l[0].abs() - half[0];
                    let dy = l[1].abs() - half[1];
                    // Near any edge of the outline.
                    (dx.abs() <= t && l[1].abs() <= half[1] + t)
                        || (dy.abs() <= t && l[0].abs() <= half[0] + t)
                }
            }
            Shape::Ellipse { c, r, angle } => {
                let l = to_local(p, *c, *angle);
                let rx = r[0].max(0.5);
                let ry = r[1].max(0.5);
                let q = (l[0] / rx).powi(2) + (l[1] / ry).powi(2);
                if self.filled {
                    q <= 1.0 + slop / rx.min(ry)
                } else {
                    // Approximate ring distance via normalized offset.
                    let d = (q.sqrt() - 1.0).abs() * rx.min(ry);
                    d <= t
                }
            }
            Shape::Stamp { raster, c, scale, angle } => {
                let l = to_local(p, *c, *angle);
                let (w, h) = raster_dims(raster);
                let sx = l[0] / scale + w as f32 * 0.5;
                let sy = l[1] / scale + h as f32 * 0.5;
                if sx < 0.0 || sy < 0.0 || sx >= w as f32 || sy >= h as f32 {
                    return false;
                }
                let i = (sy as usize) * w + sx as usize;
                raster.cell[i] != CELL_FLUID || raster.dye_src[i][3] > 0.0
            }
        }
    }

    /// Editable handles (visible cells).
    pub fn handles(&self) -> Vec<[f32; 2]> {
        match &self.shape {
            Shape::Line { a, b } => vec![*a, *b],
            Shape::Poly { pts, .. } => pts.clone(),
            Shape::Rect { c, half, angle } | Shape::Ellipse { c, r: half, angle } => {
                let (s, co) = angle.sin_cos();
                [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    .iter()
                    .map(|(kx, ky)| {
                        let lx = kx * half[0];
                        let ly = ky * half[1];
                        [c[0] + lx * co - ly * s, c[1] + lx * s + ly * co]
                    })
                    .collect()
            }
            Shape::Stamp { .. } => Vec::new(), // move/rotate/scale via panel
        }
    }

    /// Move handle `idx` to `p`.
    pub fn set_handle(&mut self, idx: usize, p: [f32; 2]) {
        match &mut self.shape {
            Shape::Line { a, b } => {
                if idx == 0 {
                    *a = p;
                } else {
                    *b = p;
                }
            }
            Shape::Poly { pts, .. } => {
                if let Some(v) = pts.get_mut(idx) {
                    *v = p;
                }
            }
            Shape::Rect { c, half, angle } | Shape::Ellipse { c, r: half, angle } => {
                // Keep the opposite corner fixed; recompute centre/half in
                // the local (rotated) frame.
                let (s, co) = angle.sin_cos();
                let signs =
                    [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
                let (kx, ky) = signs[idx.min(3)];
                let (ox, oy) = (-kx, -ky);
                let opp = [
                    c[0] + (ox * half[0]) * co - (oy * half[1]) * s,
                    c[1] + (ox * half[0]) * s + (oy * half[1]) * co,
                ];
                // Local coords of dragged point relative to the fixed
                // opposite corner.
                let dx = p[0] - opp[0];
                let dy = p[1] - opp[1];
                let lx = dx * co + dy * s;
                let ly = -dx * s + dy * co;
                let new_half = [(lx * 0.5).abs().max(0.5), (ly * 0.5).abs().max(0.5)];
                let lc = [lx * 0.5, ly * 0.5];
                *c = [
                    opp[0] + lc[0] * co - lc[1] * s,
                    opp[1] + lc[0] * s + lc[1] * co,
                ];
                *half = new_half;
            }
            Shape::Stamp { .. } => {}
        }
    }
}

fn raster_dims(r: &GeoRegion) -> (usize, usize) {
    (
        (r.rect.2 - r.rect.0).max(0) as usize,
        (r.rect.3 - r.rect.1).max(0) as usize,
    )
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 > 1e-6 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist(p, [a[0] + t * ab[0], a[1] + t * ab[1]])
}

fn to_local(p: [f32; 2], c: [f32; 2], angle: f32) -> [f32; 2] {
    let (s, co) = angle.sin_cos();
    let dx = p[0] - c[0];
    let dy = p[1] - c[1];
    [dx * co + dy * s, -dx * s + dy * co]
}

// --- Undo ------------------------------------------------------------

pub enum ModelOp {
    Add(SketchObject),
    Remove(usize, SketchObject),
    Modify {
        i: usize,
        before: SketchObject,
        after: SketchObject,
        /// Panel-widget edits merge into an open coalescable op; gesture
        /// records never coalesce, so a drag and the slider tweaks around
        /// it stay separate undo steps.
        coalesce: bool,
    },
    Replace(Vec<SketchObject>, Vec<SketchObject>), // whole-list ops (clear, presets)
}

// --- The model -------------------------------------------------------

#[derive(Default)]
pub struct SketchModel {
    pub objects: Vec<SketchObject>,
    next_id: u64,
    undo: Vec<ModelOp>,
    redo: Vec<ModelOp>,
    /// Region (visible cells) needing re-rasterization; None = clean.
    dirty: Option<GridRect>,
}

impl SketchModel {
    pub fn fresh_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn mark_dirty(&mut self, r: GridRect) {
        if r.is_empty() {
            return;
        }
        self.dirty = Some(match self.dirty {
            Some(d) => d.union(r),
            None => r,
        });
    }

    pub fn mark_all_dirty(&mut self) {
        self.dirty = Some(GridRect {
            x0: i32::MIN / 4,
            y0: i32::MIN / 4,
            x1: i32::MAX / 4,
            y1: i32::MAX / 4,
        });
    }

    pub fn take_dirty(&mut self) -> Option<GridRect> {
        self.dirty.take()
    }

    pub fn find(&self, id: u64) -> Option<usize> {
        self.objects.iter().position(|o| o.id == id)
    }

    /// Topmost object hit at `p`.
    pub fn hit_test(&self, p: [f32; 2], slop: f32) -> Option<u64> {
        self.objects
            .iter()
            .rev()
            .find(|o| o.hit(p, slop))
            .map(|o| o.id)
    }

    pub fn add(&mut self, obj: SketchObject) {
        self.mark_dirty(obj.bounds());
        self.undo.push(ModelOp::Add(obj.clone()));
        self.redo.clear();
        self.objects.push(obj);
    }

    pub fn remove(&mut self, id: u64) {
        if let Some(i) = self.find(id) {
            let obj = self.objects.remove(i);
            self.mark_dirty(obj.bounds());
            self.undo.push(ModelOp::Remove(i, obj));
            self.redo.clear();
        }
    }

    /// Record a finished modification (before captured at gesture start).
    /// A no-op edit (click-select without moving) records nothing, so it
    /// neither pollutes the undo stack nor wipes the redo stack.
    pub fn record_modify(&mut self, id: u64, before: SketchObject) {
        if let Some(i) = self.find(id) {
            if self.objects[i] == before {
                return;
            }
            let after = self.objects[i].clone();
            self.mark_dirty(before.bounds().union(after.bounds()));
            self.undo.push(ModelOp::Modify { i, before, after, coalesce: false });
            self.redo.clear();
        }
    }

    /// Like `record_modify`, but consecutive PANEL edits to the same
    /// object merge into one undo step (sliders emit an edit per frame).
    /// The merged op keeps the original `before`, so one undo reverts the
    /// whole slider session; gesture records (coalesce: false) are never
    /// merged into.
    pub fn record_modify_coalesced(&mut self, id: u64, before: SketchObject) {
        let Some(i) = self.find(id) else { return };
        if self.objects[i] == before {
            return;
        }
        let after = self.objects[i].clone();
        self.mark_dirty(before.bounds().union(after.bounds()));
        self.redo.clear();
        if let Some(ModelOp::Modify { i: j, after: top_after, coalesce: true, .. }) =
            self.undo.last_mut()
        {
            if *j == i && top_after.id == id {
                *top_after = after;
                return;
            }
        }
        self.undo.push(ModelOp::Modify { i, before, after, coalesce: true });
    }

    /// Remove an object added by the in-flight gesture along with its
    /// undo record, as if it was never drawn (Esc / degenerate shapes).
    pub fn cancel_last_add(&mut self, id: u64) {
        if let Some(i) = self.find(id) {
            let obj = self.objects.remove(i);
            self.mark_dirty(obj.bounds());
        }
        if matches!(self.undo.last(), Some(ModelOp::Add(o)) if o.id == id) {
            self.undo.pop();
        }
    }

    /// Sync the pending Add record with the object's final state, so redo
    /// replays what the user actually drew (the object was mutated live
    /// after `add`).
    pub fn finalize_last_add(&mut self, id: u64) {
        if let Some(i) = self.find(id) {
            let after = self.objects[i].clone();
            if let Some(ModelOp::Add(o)) = self.undo.last_mut() {
                if o.id == id {
                    *o = after;
                }
            }
        }
    }

    /// Replace the whole list (clear / presets / scene loads), undoably.
    pub fn replace_all(&mut self, new: Vec<SketchObject>) {
        // Scene loads carry ids minted by ANOTHER session's counter;
        // advance ours past them or fresh_id() would mint duplicates
        // (and every id-keyed op would then resolve to the wrong object).
        self.next_id = self
            .next_id
            .max(new.iter().map(|o| o.id).max().unwrap_or(0));
        let old = std::mem::replace(&mut self.objects, new);
        self.undo
            .push(ModelOp::Replace(old, self.objects.clone()));
        self.redo.clear();
        self.mark_all_dirty();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(op) = self.undo.pop() {
            match &op {
                ModelOp::Add(o) => {
                    if let Some(i) = self.find(o.id) {
                        self.mark_dirty(self.objects[i].bounds());
                        self.objects.remove(i);
                    }
                }
                ModelOp::Remove(i, o) => {
                    self.mark_dirty(o.bounds());
                    self.objects.insert((*i).min(self.objects.len()), o.clone());
                }
                ModelOp::Modify { i, before, after, .. } => {
                    if let Some(slot) = self.objects.get_mut(*i) {
                        *slot = before.clone();
                    }
                    self.mark_dirty(before.bounds().union(after.bounds()));
                }
                ModelOp::Replace(old, _new) => {
                    self.objects = old.clone();
                    self.mark_all_dirty();
                }
            }
            self.redo.push(op);
        }
    }

    pub fn redo(&mut self) {
        if let Some(op) = self.redo.pop() {
            match &op {
                ModelOp::Add(o) => {
                    self.mark_dirty(o.bounds());
                    self.objects.push(o.clone());
                }
                ModelOp::Remove(i, o) => {
                    self.mark_dirty(o.bounds());
                    if *i < self.objects.len() {
                        self.objects.remove(*i);
                    } else {
                        self.objects.pop();
                    }
                }
                ModelOp::Modify { i, before, after, .. } => {
                    if let Some(slot) = self.objects.get_mut(*i) {
                        *slot = after.clone();
                    }
                    self.mark_dirty(before.bounds().union(after.bounds()));
                }
                ModelOp::Replace(_old, new) => {
                    self.objects = new.clone();
                    self.mark_all_dirty();
                }
            }
            self.undo.push(op);
        }
    }

    pub fn rescale_all(&mut self, f: f32) {
        for o in &mut self.objects {
            o.rescale_all(f);
        }
        self.undo.clear();
        self.redo.clear();
        self.mark_all_dirty();
    }

    // --- Rasterization ------------------------------------------------

    /// Re-rasterize `region` (visible cells) into the geometry layers:
    /// clear it, repaint the tunnel bands inside it, then repaint every
    /// intersecting object in list order, clipped to the region.
    pub fn rasterize_region(
        &self,
        geo: &mut Geometry,
        region_vis: GridRect,
        margin: usize,
        wind_tunnel: bool,
    ) {
        let m = margin as i32;
        // Clip in FULL-GRID coordinates: the region (visible coords)
        // offset by the margin, clamped to the grid. The region may
        // extend beyond the visible window — mark_all_dirty covers
        // everything, and objects dragged partly off-canvas keep their
        // off-screen parts physically present in the margin. Covering
        // the margin here is also what paints (and clears) the tunnel
        // bands, which live at the true grid edges.
        let clip = GridRect {
            x0: region_vis.x0.saturating_add(m),
            y0: region_vis.y0.saturating_add(m),
            x1: region_vis.x1.saturating_add(m),
            y1: region_vis.y1.saturating_add(m),
        }
        .clampped(geo.w, geo.h);
        if clip.is_empty() {
            return;
        }

        // 1. Clear.
        for y in clip.y0..clip.y1 {
            for x in clip.x0..clip.x1 {
                let i = (y as usize) * geo.w + x as usize;
                geo.cell[i] = CELL_FLUID;
                geo.fan[i] = [0.0; 4];
                geo.dye_src[i] = [0.0; 4];
            }
        }
        // 2. Tunnel bands (full-grid edge columns).
        if wind_tunnel {
            let gw = geo.w as i32;
            for y in clip.y0..clip.y1 {
                let seed = (y as usize % 12) < 2;
                for x in clip.x0..clip.x1 {
                    let i = (y as usize) * geo.w + x as usize;
                    if x < 2 {
                        geo.cell[i] = CELL_INLET;
                        geo.fan[i] = [1.0, 0.0, 0.0, 0.0];
                        geo.dye_src[i] =
                            if seed { [0.92, 0.94, 1.0, 0.9] } else { [0.0; 4] };
                    } else if x >= gw - 2 {
                        geo.cell[i] = CELL_OUTLET;
                    }
                }
            }
        }
        // 3. Objects in order (bounds are in visible coords, so compare
        // against the clip translated back).
        let region_for_test = GridRect {
            x0: clip.x0 - m,
            y0: clip.y0 - m,
            x1: clip.x1 - m,
            y1: clip.y1 - m,
        };
        for o in &self.objects {
            if o.bounds().intersect(region_for_test).is_empty() {
                continue;
            }
            rasterize_object(geo, o, clip, m);
        }
        geo.touch(clip);
    }
}

// --- Object rasterizers (all clipped to `clip`, full-grid coords) ----

fn cell_writer<'a>(
    geo: &'a mut Geometry,
    obj: &'a SketchObject,
) -> impl FnMut(usize, Option<[f32; 2]>) + 'a {
    // seg_dir: fan direction override for chained shapes.
    let gw = geo.w;
    move |i: usize, seg_dir: Option<[f32; 2]>| {
        let _ = gw;
        match obj.material {
            ObjMaterial::Wall => {
                geo.cell[i] = CELL_WALL;
                geo.fan[i] = [0.0; 4];
                geo.dye_src[i] = [0.0; 4];
            }
            ObjMaterial::Fan => {
                let d = seg_dir
                    .unwrap_or_else(|| [obj.fan_angle.cos(), obj.fan_angle.sin()]);
                geo.cell[i] = CELL_INLET;
                geo.fan[i] = [
                    d[0] * obj.fan_mult,
                    d[1] * obj.fan_mult,
                    obj.fan_gust,
                    obj.fan_phase,
                ];
                geo.dye_src[i] =
                    [obj.smoke_rgb[0], obj.smoke_rgb[1], obj.smoke_rgb[2], 0.8];
            }
            ObjMaterial::Drain => {
                geo.cell[i] = CELL_OUTLET;
                geo.fan[i] = [0.0; 4];
                geo.dye_src[i] = [0.0; 4];
            }
            ObjMaterial::Smoke => {
                if geo.cell[i] == CELL_FLUID {
                    geo.dye_src[i] =
                        [obj.smoke_rgb[0], obj.smoke_rgb[1], obj.smoke_rgb[2], 1.0];
                }
            }
        }
    }
}

fn rasterize_capsule(
    geo: &mut Geometry,
    obj: &SketchObject,
    a_full: [f32; 2],
    b_full: [f32; 2],
    r: f32,
    clip: GridRect,
) {
    let rect = GridRect {
        x0: (a_full[0].min(b_full[0]) - r).floor() as i32,
        y0: (a_full[1].min(b_full[1]) - r).floor() as i32,
        x1: (a_full[0].max(b_full[0]) + r).ceil() as i32 + 1,
        y1: (a_full[1].max(b_full[1]) + r).ceil() as i32 + 1,
    }
    .intersect(clip);
    if rect.is_empty() {
        return;
    }
    let ab = [b_full[0] - a_full[0], b_full[1] - a_full[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let dir = if l2 > 1e-6 {
        let l = l2.sqrt();
        Some([ab[0] / l, ab[1] / l])
    } else {
        None
    };
    let w = geo.w;
    let mut write = cell_writer(geo, obj);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let t = if l2 > 1e-6 {
                (((p[0] - a_full[0]) * ab[0] + (p[1] - a_full[1]) * ab[1]) / l2)
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            let dx = p[0] - (a_full[0] + t * ab[0]);
            let dy = p[1] - (a_full[1] + t * ab[1]);
            if dx * dx + dy * dy <= r * r {
                write((y as usize) * w + x as usize, dir);
            }
        }
    }
}

fn rasterize_object(geo: &mut Geometry, obj: &SketchObject, clip: GridRect, m: i32) {
    let mf = m as f32;
    let t_r = (obj.thickness * 0.5).max(0.6);
    match &obj.shape {
        Shape::Line { a, b } => {
            rasterize_capsule(
                geo,
                obj,
                [a[0] + mf, a[1] + mf],
                [b[0] + mf, b[1] + mf],
                t_r,
                clip,
            );
        }
        Shape::Poly { pts, closed } => {
            let n = pts.len();
            if n == 1 {
                rasterize_capsule(
                    geo,
                    obj,
                    [pts[0][0] + mf, pts[0][1] + mf],
                    [pts[0][0] + mf, pts[0][1] + mf],
                    t_r,
                    clip,
                );
                return;
            }
            let segs = if *closed { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                let a = pts[i];
                let b = pts[(i + 1) % n];
                rasterize_capsule(
                    geo,
                    obj,
                    [a[0] + mf, a[1] + mf],
                    [b[0] + mf, b[1] + mf],
                    t_r,
                    clip,
                );
            }
        }
        Shape::Rect { c, half, angle } => {
            if obj.filled {
                let rect = obj.bounds();
                let rect = GridRect {
                    x0: rect.x0 + m,
                    y0: rect.y0 + m,
                    x1: rect.x1 + m,
                    y1: rect.y1 + m,
                }
                .intersect(clip);
                let w = geo.w;
                let (c, half, angle) = (*c, *half, *angle);
                let mut write = cell_writer(geo, obj);
                for y in rect.y0..rect.y1 {
                    for x in rect.x0..rect.x1 {
                        let l = to_local(
                            [x as f32 + 0.5 - mf, y as f32 + 0.5 - mf],
                            c,
                            angle,
                        );
                        if l[0].abs() <= half[0] && l[1].abs() <= half[1] {
                            write((y as usize) * w + x as usize, None);
                        }
                    }
                }
            } else {
                // Outline: 4 rotated edges.
                let (s, co) = angle.sin_cos();
                let corner = |kx: f32, ky: f32| -> [f32; 2] {
                    let lx = kx * half[0];
                    let ly = ky * half[1];
                    [c[0] + lx * co - ly * s + mf, c[1] + lx * s + ly * co + mf]
                };
                let cs = [
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                ];
                for i in 0..4 {
                    rasterize_capsule(geo, obj, cs[i], cs[(i + 1) % 4], t_r, clip);
                }
            }
        }
        Shape::Ellipse { c, r, angle } => {
            if obj.filled {
                let rect = obj.bounds();
                let rect = GridRect {
                    x0: rect.x0 + m,
                    y0: rect.y0 + m,
                    x1: rect.x1 + m,
                    y1: rect.y1 + m,
                }
                .intersect(clip);
                let w = geo.w;
                let (c, rr, angle) = (*c, *r, *angle);
                let mut write = cell_writer(geo, obj);
                for y in rect.y0..rect.y1 {
                    for x in rect.x0..rect.x1 {
                        let l = to_local(
                            [x as f32 + 0.5 - mf, y as f32 + 0.5 - mf],
                            c,
                            angle,
                        );
                        let q = (l[0] / rr[0].max(0.5)).powi(2)
                            + (l[1] / rr[1].max(0.5)).powi(2);
                        if q <= 1.0 {
                            write((y as usize) * w + x as usize, None);
                        }
                    }
                }
            } else {
                // Outline: capsule ring.
                let n = 72usize;
                let (s, co) = angle.sin_cos();
                let pt = |i: usize| -> [f32; 2] {
                    let t = i as f32 / n as f32 * std::f32::consts::TAU;
                    let lx = r[0] * t.cos();
                    let ly = r[1] * t.sin();
                    [c[0] + lx * co - ly * s + mf, c[1] + lx * s + ly * co + mf]
                };
                for i in 0..n {
                    rasterize_capsule(geo, obj, pt(i), pt((i + 1) % n), t_r, clip);
                }
            }
        }
        Shape::Stamp { raster, c, scale, angle } => {
            let rect = obj.bounds();
            let rect = GridRect {
                x0: rect.x0 + m,
                y0: rect.y0 + m,
                x1: rect.x1 + m,
                y1: rect.y1 + m,
            }
            .intersect(clip);
            if rect.is_empty() {
                return;
            }
            let (w, h) = raster_dims(raster);
            let (s, co) = angle.sin_cos();
            let gw = geo.w;
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    let dx = x as f32 + 0.5 - mf - c[0];
                    let dy = y as f32 + 0.5 - mf - c[1];
                    let lx = (dx * co + dy * s) / scale + w as f32 * 0.5;
                    let ly = (-dx * s + dy * co) / scale + h as f32 * 0.5;
                    if lx < 0.0 || ly < 0.0 || lx >= w as f32 || ly >= h as f32 {
                        continue;
                    }
                    let si = (ly as usize) * w + lx as usize;
                    let sc = raster.cell[si];
                    if sc == CELL_FLUID && raster.dye_src[si][3] <= 0.0 {
                        continue;
                    }
                    let gi = (y as usize) * gw + x as usize;
                    geo.cell[gi] = sc;
                    // Rotate stored fan vectors with the stamp; the
                    // object-level fan knobs act as a multiplier and extra
                    // gustiness so generated parts stay tunable after
                    // placement.
                    let f = raster.fan[si];
                    let m = obj.fan_mult;
                    geo.fan[gi] = [
                        (f[0] * co - f[1] * s) * m,
                        (f[0] * s + f[1] * co) * m,
                        (f[2] + obj.fan_gust).clamp(0.0, 1.0),
                        f[3],
                    ];
                    geo.dye_src[gi] = raster.dye_src[si];
                }
            }
        }
    }
}
