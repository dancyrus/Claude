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

/// A similarity transform: translate + rotate + UNIFORM scale — the
/// only transform family that composes through nested groups without
/// producing shear the shape set cannot represent (a rotated Rect has
/// no shear parameter). Maps a group's local space into its parent's
/// space. Transform COMPOSITION ORDER (see CLAUDE.md): an object's own
/// stored geometry/transform applies first, then each ancestor outward
/// — world = T_root ∘ … ∘ T_parent (stored).
#[derive(Clone, Copy, PartialEq)]
pub struct Sim2 {
    pub t: [f32; 2],
    pub rot: f32,
    pub s: f32,
}

impl Sim2 {
    pub const IDENTITY: Sim2 = Sim2 { t: [0.0, 0.0], rot: 0.0, s: 1.0 };

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        let v = self.apply_vec(p);
        [self.t[0] + v[0], self.t[1] + v[1]]
    }

    /// Rotate+scale only — for direction/delta vectors.
    pub fn apply_vec(&self, v: [f32; 2]) -> [f32; 2] {
        let (sn, cs) = self.rot.sin_cos();
        [
            (v[0] * cs - v[1] * sn) * self.s,
            (v[0] * sn + v[1] * cs) * self.s,
        ]
    }

    /// `self ∘ other`: apply `other` first, then `self`.
    pub fn compose(&self, other: Sim2) -> Sim2 {
        Sim2 {
            t: self.apply(other.t),
            rot: self.rot + other.rot,
            s: self.s * other.s,
        }
    }

    pub fn inverse(&self) -> Sim2 {
        let s = 1.0 / self.s.max(1e-9);
        let (sn, cs) = (-self.rot).sin_cos();
        Sim2 {
            t: [
                -(self.t[0] * cs - self.t[1] * sn) * s,
                -(self.t[0] * sn + self.t[1] * cs) * s,
            ],
            rot: -self.rot,
            s,
        }
    }
}

/// An affine reflection across a line — deliberately NOT a `Sim2`
/// (det = −1; the similarity family cannot express it), which is why
/// mirroring bakes into stored geometry per shape instead of composing
/// through group transforms.
#[derive(Clone, Copy)]
pub struct Reflect2 {
    /// A point on the line.
    o: [f32; 2],
    /// Unit direction of the line.
    d: [f32; 2],
}

impl Reflect2 {
    /// The reflection across the line through `a` and `b`; `None` when
    /// they coincide (no line to reflect across).
    pub fn across(a: [f32; 2], b: [f32; 2]) -> Option<Reflect2> {
        let v = [b[0] - a[0], b[1] - a[1]];
        let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
        if l < 1e-6 {
            return None;
        }
        Some(Reflect2 { o: a, d: [v[0] / l, v[1] / l] })
    }

    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        let r = [p[0] - self.o[0], p[1] - self.o[1]];
        let t = r[0] * self.d[0] + r[1] * self.d[1];
        [
            2.0 * (self.o[0] + t * self.d[0]) - p[0],
            2.0 * (self.o[1] + t * self.d[1]) - p[1],
        ]
    }

    /// The line's angle: a stored shape angle `x` reflects to
    /// `2·angle − x`.
    pub fn angle(&self) -> f32 {
        self.d[1].atan2(self.d[0])
    }
}

/// Shape geometry, in visible-canvas cells (an object's coordinates are
/// expressed in its PARENT's space — world when `parent` is None).
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
    /// `scale` is deliberately a single f32: non-uniform scaling of a
    /// raster stamp is out of scope (the inspector's tooltip says so).
    Stamp { raster: GeoRegion, c: [f32; 2], scale: f32, angle: f32 },
    /// A group node (U3): no geometry of its own, just the similarity
    /// transform mapping its local space into its parent's space.
    /// Children reference it via `SketchObject::parent`. NOTE: appended
    /// last so bincode variant indices of older scene files hold.
    Group { t: [f32; 2], rot: f32, scale: f32 },
    /// Circular arc (queue item 8): centre, radius, start angle and a
    /// SIGNED sweep, radians (positive = counter-clockwise in cell
    /// space). Similarity transforms keep circles circular — the same
    /// fact that pinned group scaling to uniform at U3. Appended after
    /// `Group` so every older file's variant indices hold; files that
    /// may contain one save as scene v11.
    Arc { c: [f32; 2], r: f32, start: f32, sweep: f32 },
    /// Catmull-Rom spline through `pts` (queue item 8): the stored
    /// points stay the live, draggable truth; curve samples are derived
    /// on demand and never persisted. Appended after `Arc`.
    Spline { pts: Vec<[f32; 2]>, closed: bool },
}

impl Shape {
    /// A group's transform as a `Sim2` (identity for non-groups).
    pub fn group_sim(&self) -> Sim2 {
        match self {
            Shape::Group { t, rot, scale } => Sim2 { t: *t, rot: *rot, s: *scale },
            _ => Sim2::IDENTITY,
        }
    }
}

/// Sample an arc into a chain of points, adaptively: roughly one point
/// per 1.5 cells of arc length, clamped so a tiny arc still draws round
/// (8) and a huge one stays bounded (256). Shared by every consumer —
/// rasterizer, hit tests, eraser conversion — so they all see the same
/// curve.
pub fn sample_arc(c: [f32; 2], r: f32, start: f32, sweep: f32) -> Vec<[f32; 2]> {
    let len = (r.abs() * sweep.abs()).max(1e-3);
    let n = ((len / 1.5).ceil() as usize).clamp(8, 256);
    (0..=n)
        .map(|i| {
            let a = start + sweep * (i as f32 / n as f32);
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect()
}

/// Circle through three points → (centre, radius, start, sweep), the
/// sweep signed so the arc runs a → mid → b. `None` when the points
/// are (near-)collinear or the fit degenerates — callers keep the old
/// geometry in that case.
pub fn arc_from_three(
    a: [f32; 2],
    mid: [f32; 2],
    b: [f32; 2],
) -> Option<([f32; 2], f32, f32, f32)> {
    let d = 2.0
        * (a[0] * (mid[1] - b[1]) + mid[0] * (b[1] - a[1]) + b[0] * (a[1] - mid[1]));
    if d.abs() < 1e-6 {
        return None;
    }
    let sq = |p: [f32; 2]| p[0] * p[0] + p[1] * p[1];
    let ux = (sq(a) * (mid[1] - b[1]) + sq(mid) * (b[1] - a[1]) + sq(b) * (a[1] - mid[1])) / d;
    let uy = (sq(a) * (b[0] - mid[0]) + sq(mid) * (a[0] - b[0]) + sq(b) * (mid[0] - a[0])) / d;
    let r = ((a[0] - ux).powi(2) + (a[1] - uy).powi(2)).sqrt();
    if !r.is_finite() || !(0.25..=1e6).contains(&r) {
        return None;
    }
    let ang = |p: [f32; 2]| (p[1] - uy).atan2(p[0] - ux);
    let tau = std::f32::consts::TAU;
    // CCW distance from `from` to `to` in [0, tau).
    let ccw = |from: f32, to: f32| {
        let mut dd = to - from;
        while dd < 0.0 {
            dd += tau;
        }
        dd % tau
    };
    let (sa, ma, ba) = (ang(a), ang(mid), ang(b));
    let sweep_ccw = ccw(sa, ba);
    let sweep = if ccw(sa, ma) <= sweep_ccw { sweep_ccw } else { sweep_ccw - tau };
    if sweep.abs() < 1e-3 {
        return None;
    }
    Some(([ux, uy], r, sa, sweep))
}

/// Sample a Catmull-Rom spline through `pts` (8 segments per span,
/// centripetal-free uniform parameterization — visibly smooth at cell
/// scale without the degenerate-knot bookkeeping). Endpoints are
/// clamped (open) or wrapped (closed); fewer than 3 points fall back
/// to the polyline itself.
pub fn sample_spline(pts: &[[f32; 2]], closed: bool) -> Vec<[f32; 2]> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let at = |i: isize| -> [f32; 2] {
        if closed {
            pts[i.rem_euclid(n as isize) as usize]
        } else {
            pts[i.clamp(0, n as isize - 1) as usize]
        }
    };
    let spans = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(spans * 8 + 1);
    for s in 0..spans {
        let (p0, p1, p2, p3) = (
            at(s as isize - 1),
            at(s as isize),
            at(s as isize + 1),
            at(s as isize + 2),
        );
        let steps = 8;
        let last_span = s + 1 == spans;
        let top = if last_span && !closed { steps } else { steps - 1 };
        for k in 0..=top {
            let t = k as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c2: f32, d: f32| {
                0.5 * ((2.0 * b)
                    + (-a + c2) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c2 - d) * t2
                    + (-a + 3.0 * b - 3.0 * c2 + d) * t3)
            };
            out.push([
                f(p0[0], p1[0], p2[0], p3[0]),
                f(p0[1], p1[1], p2[1], p3[1]),
            ]);
        }
    }
    out
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
    /// Not click-selectable and not editable (tree-managed; persists).
    /// Effective state includes ancestors — see `SketchModel::eff_locked`.
    pub locked: bool,
    /// Not rasterized and not click-selectable (tree-managed; persists).
    /// Effective state includes ancestors — see `SketchModel::eff_hidden`.
    pub hidden: bool,
    /// Enclosing group id (U3, scene v8+). None = a root object whose
    /// coordinates are world coordinates. NOTE: appended last so bincode
    /// stays positional-compatible via the SketchObjectV7 mirror.
    pub parent: Option<u64>,
}

impl SketchObject {
    /// Axis-aligned bounds in visible cells, including thickness.
    /// Stored (parent-space) coordinates; for the world footprint of an
    /// object inside groups use `bounds_under(parent_abs)`.
    pub fn bounds(&self) -> GridRect {
        self.bounds_under(Sim2::IDENTITY)
    }

    /// Bounds under an outer similarity `m` (the ancestor composition),
    /// computed on the transformed parameters so rotated shapes keep a
    /// tight AABB — no raster clone for stamps.
    pub fn bounds_under(&self, m: Sim2) -> GridRect {
        let pad = (self.thickness * m.s * 0.5 + 2.0).ceil();
        let (min, max) = match &self.shape {
            Shape::Line { a, b } => {
                let a = m.apply(*a);
                let b = m.apply(*b);
                (
                    [a[0].min(b[0]), a[1].min(b[1])],
                    [a[0].max(b[0]), a[1].max(b[1])],
                )
            }
            Shape::Poly { pts, .. } => {
                // A zero-point poly (corrupt file) must yield an empty
                // rect, not a fold over f32::MAX that overflows the casts.
                if pts.is_empty() {
                    return GridRect { x0: 0, y0: 0, x1: 0, y1: 0 };
                }
                let mut min = [f32::MAX, f32::MAX];
                let mut max = [f32::MIN, f32::MIN];
                for p in pts {
                    let p = m.apply(*p);
                    min[0] = min[0].min(p[0]);
                    min[1] = min[1].min(p[1]);
                    max[0] = max[0].max(p[0]);
                    max[1] = max[1].max(p[1]);
                }
                (min, max)
            }
            Shape::Rect { c, half, angle } | Shape::Ellipse { c, r: half, angle } => {
                let c = m.apply(*c);
                let (s, co) = (angle + m.rot).sin_cos();
                let half = [half[0] * m.s, half[1] * m.s];
                let ex = (half[0] * co).abs() + (half[1] * s).abs();
                let ey = (half[0] * s).abs() + (half[1] * co).abs();
                ([c[0] - ex, c[1] - ey], [c[0] + ex, c[1] + ey])
            }
            Shape::Stamp { raster, c, scale, angle } => {
                let c = m.apply(*c);
                let (w, h) = raster_dims(raster);
                let hx = w as f32 * 0.5 * scale * m.s;
                let hy = h as f32 * 0.5 * scale * m.s;
                let (s, co) = (angle + m.rot).sin_cos();
                let ex = (hx * co).abs() + (hy * s).abs();
                let ey = (hx * s).abs() + (hy * co).abs();
                ([c[0] - ex, c[1] - ey], [c[0] + ex, c[1] + ey])
            }
            // A group has no geometry; its world footprint is the
            // subtree's (SketchModel::world_bounds).
            Shape::Group { .. } => return GridRect { x0: 0, y0: 0, x1: 0, y1: 0 },
            Shape::Arc { c, r, start, sweep } => {
                let mut min = [f32::MAX, f32::MAX];
                let mut max = [f32::MIN, f32::MIN];
                for p in sample_arc(m.apply(*c), *r * m.s, *start + m.rot, *sweep) {
                    min = [min[0].min(p[0]), min[1].min(p[1])];
                    max = [max[0].max(p[0]), max[1].max(p[1])];
                }
                (min, max)
            }
            Shape::Spline { pts, closed } => {
                if pts.is_empty() {
                    return GridRect { x0: 0, y0: 0, x1: 0, y1: 0 };
                }
                let mut min = [f32::MAX, f32::MAX];
                let mut max = [f32::MIN, f32::MIN];
                for p in sample_spline(pts, *closed) {
                    let p = m.apply(p);
                    min = [min[0].min(p[0]), min[1].min(p[1])];
                    max = [max[0].max(p[0]), max[1].max(p[1])];
                }
                (min, max)
            }
        };
        GridRect {
            x0: (min[0] - pad) as i32,
            y0: (min[1] - pad) as i32,
            x1: (max[0] + pad) as i32 + 1,
            y1: (max[1] + pad) as i32 + 1,
        }
    }

    /// Object centre in STORED (parent-space) coordinates (translation
    /// origin, default rotation pivot).
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
            Shape::Arc { c, .. } => *c,
            Shape::Group { t, .. } => *t,
            Shape::Spline { pts, .. } => {
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
            Shape::Rect { c, .. }
            | Shape::Ellipse { c, .. }
            | Shape::Stamp { c, .. }
            | Shape::Arc { c, .. } => {
                c[0] += d[0];
                c[1] += d[1];
            }
            Shape::Group { t, .. } => {
                t[0] += d[0];
                t[1] += d[1];
            }
            Shape::Spline { pts, .. } => {
                for p in pts {
                    p[0] += d[0];
                    p[1] += d[1];
                }
            }
        }
    }

    /// Rotate by `da` radians about the centre (baked for point shapes).
    pub fn rotate_by(&mut self, da: f32) {
        self.rotate_about(self.center(), da);
    }

    /// Rotate by `da` radians about an arbitrary pivot (stored/parent
    /// space). Groups fold it into their transform — O(1), the subtree
    /// follows through composition.
    pub fn rotate_about(&mut self, pivot: [f32; 2], da: f32) {
        let (s, co) = da.sin_cos();
        let rot = |p: &mut [f32; 2]| {
            let dx = p[0] - pivot[0];
            let dy = p[1] - pivot[1];
            p[0] = pivot[0] + dx * co - dy * s;
            p[1] = pivot[1] + dx * s + dy * co;
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
            Shape::Rect { c, angle, .. }
            | Shape::Ellipse { c, angle, .. }
            | Shape::Stamp { c, angle, .. } => {
                rot(c);
                *angle += da;
            }
            Shape::Group { t, rot: g_rot, .. } => {
                rot(t);
                *g_rot += da;
            }
            Shape::Arc { c, start, .. } => {
                rot(c);
                *start += da;
            }
            Shape::Spline { pts, .. } => {
                for p in pts {
                    rot(p);
                }
            }
        }
        self.fan_angle += da;
    }

    /// Baked plume color of a stamp: the dye RGB of its first
    /// dye-emitting fan cell (a generated engine's chamber inlet).
    /// `None` for non-stamps and for stamps without such cells.
    pub fn stamp_plume_rgb(&self) -> Option<[f32; 3]> {
        let Shape::Stamp { raster, .. } = &self.shape else {
            return None;
        };
        raster
            .fan
            .iter()
            .zip(&raster.dye_src)
            .find(|(f, d)| (f[0] != 0.0 || f[1] != 0.0) && d[3] > 0.0)
            .map(|(_, d)| [d[0], d[1], d[2]])
    }

    /// Scale about the centre (uniform).
    pub fn scale_by(&mut self, f: f32) {
        self.scale_about(self.center(), f);
    }

    /// Uniform scale about an arbitrary pivot (stored/parent space).
    /// Like `scale_by`, outline thickness is deliberately NOT scaled for
    /// leaf shapes; a group's transform scale does thin its subtree's
    /// strokes, because composition applies to everything inside.
    pub fn scale_about(&mut self, pivot: [f32; 2], f: f32) {
        let f = f.clamp(0.05, 50.0);
        let sc = |p: &mut [f32; 2]| {
            p[0] = pivot[0] + (p[0] - pivot[0]) * f;
            p[1] = pivot[1] + (p[1] - pivot[1]) * f;
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
            Shape::Rect { c, half, .. } | Shape::Ellipse { c, r: half, .. } => {
                sc(c);
                half[0] *= f;
                half[1] *= f;
            }
            Shape::Stamp { c, scale, .. } => {
                sc(c);
                *scale *= f;
            }
            Shape::Group { t, scale, .. } => {
                sc(t);
                *scale = (*scale * f).clamp(1e-3, 1e3);
            }
            Shape::Arc { c, r, .. } => {
                sc(c);
                *r *= f;
            }
            Shape::Spline { pts, .. } => {
                for p in pts {
                    sc(p);
                }
            }
        }
    }

    /// Bake an outer similarity into the stored geometry (flattening an
    /// object out of its group chain, or re-expressing it into another
    /// parent space). Unlike the interactive scale ops this DOES scale
    /// thickness — it converts between coordinate spaces, so everything
    /// world-visible must map.
    pub fn apply_sim(&mut self, m: Sim2) {
        if m.is_identity() {
            return;
        }
        match &mut self.shape {
            Shape::Line { a, b } => {
                *a = m.apply(*a);
                *b = m.apply(*b);
            }
            Shape::Poly { pts, .. } => {
                for p in pts {
                    *p = m.apply(*p);
                }
            }
            Shape::Rect { c, half, angle } | Shape::Ellipse { c, r: half, angle } => {
                *c = m.apply(*c);
                *angle += m.rot;
                half[0] *= m.s;
                half[1] *= m.s;
            }
            Shape::Stamp { c, scale, angle, .. } => {
                *c = m.apply(*c);
                *angle += m.rot;
                *scale *= m.s;
            }
            Shape::Group { t, rot, scale } => {
                *t = m.apply(*t);
                *rot += m.rot;
                *scale *= m.s;
            }
            Shape::Arc { c, r, start, .. } => {
                *c = m.apply(*c);
                *r *= m.s;
                *start += m.rot;
            }
            Shape::Spline { pts, .. } => {
                for p in pts {
                    *p = m.apply(*p);
                }
            }
        }
        self.thickness *= m.s;
        self.fan_angle += m.rot;
    }

    /// Re-express the stored geometry from one parent space into
    /// another so the world-space result is unchanged (reparenting,
    /// grouping, ungrouping).
    pub fn reexpress(&mut self, old_abs: Sim2, new_abs: Sim2) {
        self.apply_sim(new_abs.inverse().compose(old_abs));
    }

    /// Reflect the stored geometry across a line in THIS object's
    /// stored (parent) space. A reflection is not a similarity, so it
    /// bakes per shape: point shapes reflect their points, parametric
    /// shapes conjugate their angle (`x → 2θ − x`, θ the line angle —
    /// Rect/Ellipse are symmetric in their local y, so that alone is
    /// exact), a stamp additionally flips its raster rows, and a Group
    /// node conjugates its transform to `M ∘ G ∘ M`. Because
    /// `M ∘ G₁ ∘ G₂ ∘ leaf = (M G₁ M)(M G₂ M)(M leaf)` (M² = id), the
    /// SAME reflection applies at every level of a subtree: mirroring
    /// a group is "conjugate every node, reflect every leaf" with one
    /// line, and composition reproduces the mirrored world geometry.
    pub fn reflect(&mut self, m: Reflect2) {
        let two_theta = 2.0 * m.angle();
        match &mut self.shape {
            Shape::Line { a, b } => {
                *a = m.apply(*a);
                *b = m.apply(*b);
            }
            Shape::Poly { pts, .. } => {
                for p in pts {
                    *p = m.apply(*p);
                }
            }
            Shape::Rect { c, angle, .. } | Shape::Ellipse { c, angle, .. } => {
                *c = m.apply(*c);
                *angle = two_theta - *angle;
            }
            Shape::Stamp { raster, c, angle, .. } => {
                *c = m.apply(*c);
                *angle = two_theta - *angle;
                flip_raster_rows(raster);
            }
            Shape::Group { t, rot, scale } => {
                let g = Sim2 { t: *t, rot: *rot, s: *scale };
                *t = m.apply(g.apply(m.apply([0.0, 0.0])));
                *rot = -*rot;
            }
            Shape::Arc { c, start, sweep, .. } => {
                // A point at angle x maps to 2θ − x, so the start
                // conjugates and the sweep flips direction.
                *c = m.apply(*c);
                *start = two_theta - *start;
                *sweep = -*sweep;
            }
            Shape::Spline { pts, .. } => {
                for p in pts {
                    *p = m.apply(*p);
                }
            }
        }
        // A reflection is an isometry: thickness holds; the fan aim
        // conjugates like any other stored angle.
        self.fan_angle = two_theta - self.fan_angle;
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
            // A group's rotation and scale are ratios; only its
            // translation is a length. Child coordinates rescale in
            // their own objects, so the composed world result scales
            // uniformly as intended.
            Shape::Group { t, .. } => sc(t),
            Shape::Arc { c, r, .. } => {
                sc(c);
                *r *= f;
            }
            Shape::Spline { pts, .. } => {
                for p in pts {
                    sc(p);
                }
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
                // A filled polygon hits anywhere inside (U4), not just
                // near its outline.
                if self.filled && *closed && n >= 3 && crate::geomops::point_in_polygon(p, pts)
                {
                    return true;
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
            // Groups are picked through their members (the model's
            // hit_test maps a member hit to its outermost group).
            Shape::Group { .. } => false,
            Shape::Arc { c, r, start, sweep } => {
                let pts = sample_arc(*c, *r, *start, *sweep);
                (0..pts.len() - 1).any(|i| seg_dist(p, pts[i], pts[i + 1]) <= t)
            }
            Shape::Spline { pts, closed } => {
                if pts.is_empty() {
                    return false;
                }
                if pts.len() == 1 {
                    return dist(p, pts[0]) <= t;
                }
                let s = sample_spline(pts, *closed);
                if self.filled
                    && *closed
                    && s.len() >= 3
                    && crate::geomops::point_in_polygon(p, &s)
                {
                    return true;
                }
                let n = s.len();
                let segs = if *closed { n } else { n - 1 };
                (0..segs).any(|i| seg_dist(p, s[i], s[(i + 1) % n]) <= t)
            }
        }
    }

    /// Hit test with the query point given in WORLD coordinates and the
    /// ancestor composition `m`: the point maps into stored space, the
    /// slop divides by the composed scale (thickness compares happen in
    /// stored units).
    pub fn hit_under(&self, p: [f32; 2], slop: f32, m: Sim2) -> bool {
        if m.is_identity() {
            return self.hit(p, slop);
        }
        self.hit(m.inverse().apply(p), slop / m.s.max(1e-9))
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
            // Stamps and groups: move/rotate/scale via panel or gizmo.
            Shape::Stamp { .. } | Shape::Group { .. } => Vec::new(),
            // Arc: start, bulge (mid-arc), end — dragging any re-fits
            // the circle through the three (set_handle).
            Shape::Arc { c, r, start, sweep } => {
                let at = |a: f32| [c[0] + r * a.cos(), c[1] + r * a.sin()];
                vec![at(*start), at(*start + sweep * 0.5), at(*start + *sweep)]
            }
            Shape::Spline { pts, .. } => pts.clone(),
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
            Shape::Stamp { .. } | Shape::Group { .. } => {}
            Shape::Arc { c, r, start, sweep } => {
                let at = |a: f32| [c[0] + *r * a.cos(), c[1] + *r * a.sin()];
                let (mut a, mut mid, mut b) =
                    (at(*start), at(*start + *sweep * 0.5), at(*start + *sweep));
                match idx {
                    0 => a = p,
                    1 => mid = p,
                    _ => b = p,
                }
                if let Some((nc, nr, ns, nw)) = arc_from_three(a, mid, b) {
                    *c = nc;
                    *r = nr;
                    *start = ns;
                    *sweep = nw;
                }
            }
            Shape::Spline { pts, .. } => {
                if let Some(v) = pts.get_mut(idx) {
                    *v = p;
                }
            }
        }
    }

    /// Rubber-band test, INTERSECT semantics (see CLAUDE.md): true when
    /// the object's outline crosses the axis-aligned band `[min, max]`
    /// (visible cells) or either wholly contains the other. Thin open
    /// geometry dominates FlowPaint scenes, so touching the band selects.
    pub fn intersects_rect(&self, min: [f32; 2], max: [f32; 2]) -> bool {
        if matches!(self.shape, Shape::Group { .. }) {
            // Groups band-select through their members (app-level walk).
            return false;
        }
        let seg_hits = |a: [f32; 2], b: [f32; 2]| seg_intersects_aabb(a, b, min, max);
        let outline_hits = match &self.shape {
            Shape::Line { a, b } => seg_hits(*a, *b),
            Shape::Poly { pts, closed } => {
                let n = pts.len();
                if n == 1 {
                    return point_in_aabb(pts[0], min, max);
                }
                let segs = if *closed { n } else { n.saturating_sub(1) };
                (0..segs).any(|i| seg_hits(pts[i], pts[(i + 1) % n]))
            }
            Shape::Rect { .. } | Shape::Stamp { .. } => {
                let cs = self.corners();
                (0..4).any(|i| seg_hits(cs[i], cs[(i + 1) % 4]))
            }
            Shape::Ellipse { c, r, angle } => {
                let (s, co) = angle.sin_cos();
                let n = 24usize;
                let pt = |i: usize| -> [f32; 2] {
                    let t = i as f32 / n as f32 * std::f32::consts::TAU;
                    let lx = r[0] * t.cos();
                    let ly = r[1] * t.sin();
                    [c[0] + lx * co - ly * s, c[1] + lx * s + ly * co]
                };
                (0..n).any(|i| seg_hits(pt(i), pt((i + 1) % n)))
            }
            Shape::Group { .. } => false, // early-returned above
            Shape::Arc { c, r, start, sweep } => {
                let pts = sample_arc(*c, *r, *start, *sweep);
                (0..pts.len() - 1).any(|i| seg_hits(pts[i], pts[i + 1]))
            }
            Shape::Spline { pts, closed } => {
                if pts.is_empty() {
                    return false;
                }
                if pts.len() == 1 {
                    return point_in_aabb(pts[0], min, max);
                }
                let s = sample_spline(pts, *closed);
                let n = s.len();
                let segs = if *closed { n } else { n - 1 };
                (0..segs).any(|i| seg_hits(s[i], s[(i + 1) % n]))
            }
        };
        // Containment fallbacks: band inside a filled shape, or shape
        // centre inside a band larger than the outline sampling caught.
        outline_hits
            || point_in_aabb(self.center(), min, max)
            || self.hit([(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5], 0.0)
    }

    /// The four oriented-box corners of a Rect or Stamp (its outline for
    /// band tests); degenerate for other shapes.
    fn corners(&self) -> [[f32; 2]; 4] {
        let (c, half, angle) = match &self.shape {
            Shape::Rect { c, half, angle } => (*c, *half, *angle),
            Shape::Stamp { raster, c, scale, angle } => {
                let (w, h) = raster_dims(raster);
                (*c, [w as f32 * 0.5 * scale, h as f32 * 0.5 * scale], *angle)
            }
            _ => return [self.center(); 4],
        };
        let (s, co) = angle.sin_cos();
        [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(kx, ky)| {
            let lx = kx * half[0];
            let ly = ky * half[1];
            [c[0] + lx * co - ly * s, c[1] + lx * s + ly * co]
        })
    }
}

fn point_in_aabb(p: [f32; 2], min: [f32; 2], max: [f32; 2]) -> bool {
    p[0] >= min[0] && p[0] <= max[0] && p[1] >= min[1] && p[1] <= max[1]
}

/// Segment-vs-AABB via the slab method (endpoints inside count as hits).
fn seg_intersects_aabb(a: [f32; 2], b: [f32; 2], min: [f32; 2], max: [f32; 2]) -> bool {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for k in 0..2 {
        let d = b[k] - a[k];
        if d.abs() < 1e-9 {
            if a[k] < min[k] || a[k] > max[k] {
                return false;
            }
        } else {
            let (mut lo, mut hi) = ((min[k] - a[k]) / d, (max[k] - a[k]) / d);
            if lo > hi {
                std::mem::swap(&mut lo, &mut hi);
            }
            t0 = t0.max(lo);
            t1 = t1.min(hi);
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// Flip a stamp raster vertically (row y ↔ row h−1−y) and negate the
/// stored fan vectors' y components — the local-space FlipY that pairs
/// with the angle conjugation in `SketchObject::reflect` so a mirrored
/// stamp samples exactly its reflected cells (cell y covers [y, y+1),
/// so the discrete row swap IS the reflection about the raster centre).
fn flip_raster_rows(r: &mut GeoRegion) {
    let (w, h) = raster_dims(r);
    for y in 0..h / 2 {
        let (a, b) = (y * w, (h - 1 - y) * w);
        for x in 0..w {
            r.cell.swap(a + x, b + x);
            r.fan.swap(a + x, b + x);
            r.dye_src.swap(a + x, b + x);
        }
    }
    for f in &mut r.fan {
        f[1] = -f[1];
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

// One point-to-segment distance in the crate (U4): geomops owns it.
fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    crate::geomops::seg_point_dist(p, a, b)
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
    /// Insertion at a specific z-slot (eraser split fragments sit right
    /// above the object they came from; the paint bucket's fill goes to
    /// the bottom). `Add` always re-appends at the top on redo, so it
    /// cannot express these.
    Insert(usize, SketchObject),
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
    /// Z-order permutation: (id order before, id order after).
    Reorder(Vec<u64>, Vec<u64>),
    /// One user action across a whole selection: undo/redo as a unit
    /// (members applied in reverse for undo). `coalesce` follows the
    /// Modify convention — consecutive panel edits to the same id set
    /// merge into the open group.
    Group { ops: Vec<ModelOp>, coalesce: bool },
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

    // --- Nested groups (U3) -------------------------------------------
    // An object's stored coordinates live in its parent group's space.
    // TRANSFORM COMPOSITION ORDER (fixed once, see CLAUDE.md): the
    // child's own transform applies first, then each ancestor outward:
    // world = T_root ∘ … ∘ T_parent (stored). Every walk below is
    // hop-capped so a corrupt file's parent cycle cannot hang the app
    // (loads also break cycles outright — see load_scene).

    const MAX_DEPTH: usize = 256;

    /// The composed ancestor transform of `id`: stored (parent-space)
    /// coordinates → world coordinates. Identity for root objects.
    pub fn parent_abs(&self, id: u64) -> Sim2 {
        let Some(i) = self.find(id) else { return Sim2::IDENTITY };
        self.abs_of(self.objects[i].parent)
    }

    /// The composed transform of a group chain starting at `parent`
    /// (that group's local space → world).
    pub fn abs_of(&self, parent: Option<u64>) -> Sim2 {
        let mut acc = Sim2::IDENTITY;
        let mut cur = parent;
        for _ in 0..Self::MAX_DEPTH {
            let Some(pid) = cur else { return acc };
            let Some(pi) = self.find(pid) else { return acc };
            // Walking inward→outward while left-composing yields
            // T_outer ∘ … ∘ T_inner — child first, ancestors outward.
            acc = self.objects[pi].shape.group_sim().compose(acc);
            cur = self.objects[pi].parent;
        }
        acc
    }

    /// True when `id` sits somewhere below `ancestor` in the tree.
    pub fn is_descendant(&self, id: u64, ancestor: u64) -> bool {
        let Some(mut i) = self.find(id) else { return false };
        for _ in 0..Self::MAX_DEPTH {
            match self.objects[i].parent {
                Some(p) if p == ancestor => return true,
                Some(p) => match self.find(p) {
                    Some(pi) => i = pi,
                    None => return false,
                },
                None => return false,
            }
        }
        false
    }

    /// Nesting depth: 0 for roots, 1 for direct group members, …
    pub fn depth(&self, id: u64) -> usize {
        let Some(mut i) = self.find(id) else { return 0 };
        let mut d = 0;
        for _ in 0..Self::MAX_DEPTH {
            match self.objects[i].parent.and_then(|p| self.find(p)) {
                Some(pi) => {
                    d += 1;
                    i = pi;
                }
                None => return d,
            }
        }
        d
    }

    /// Direct children of a group, in model (z) order.
    pub fn children_of(&self, id: u64) -> Vec<u64> {
        self.objects
            .iter()
            .filter(|o| o.parent == Some(id))
            .map(|o| o.id)
            .collect()
    }

    /// `id` plus every descendant, in model (z) order.
    pub fn subtree_ids(&self, id: u64) -> Vec<u64> {
        self.objects
            .iter()
            .filter(|o| o.id == id || self.is_descendant(o.id, id))
            .map(|o| o.id)
            .collect()
    }

    /// Effective hidden state: the object's own flag or any ancestor's.
    pub fn eff_hidden(&self, id: u64) -> bool {
        self.eff_flag(id, |o| o.hidden)
    }

    /// Effective locked state: the object's own flag or any ancestor's.
    pub fn eff_locked(&self, id: u64) -> bool {
        self.eff_flag(id, |o| o.locked)
    }

    fn eff_flag(&self, id: u64, f: impl Fn(&SketchObject) -> bool) -> bool {
        let Some(mut i) = self.find(id) else { return false };
        for _ in 0..Self::MAX_DEPTH {
            if f(&self.objects[i]) {
                return true;
            }
            match self.objects[i].parent.and_then(|p| self.find(p)) {
                Some(pi) => i = pi,
                None => return false,
            }
        }
        false
    }

    /// World-space AABB of an object — a leaf's transformed bounds, a
    /// group's subtree union. None for an empty group.
    pub fn world_bounds(&self, id: u64) -> Option<GridRect> {
        let i = self.find(id)?;
        if matches!(self.objects[i].shape, Shape::Group { .. }) {
            let mut acc: Option<GridRect> = None;
            for sid in self.subtree_ids(id) {
                let si = self.find(sid)?;
                if matches!(self.objects[si].shape, Shape::Group { .. }) {
                    continue;
                }
                let b = self.objects[si].bounds_under(self.parent_abs(sid));
                acc = Some(match acc {
                    Some(u) => u.union(b),
                    None => b,
                });
            }
            acc
        } else {
            Some(self.objects[i].bounds_under(self.parent_abs(id)))
        }
    }

    /// Object centre in WORLD coordinates. (Exercised by the U3
    /// composition tests; panels compute it from their staged copy.)
    #[allow(dead_code)]
    pub fn world_center(&self, id: u64) -> Option<[f32; 2]> {
        let i = self.find(id)?;
        Some(self.parent_abs(id).apply(self.objects[i].center()))
    }

    /// Damage-mark an object's WORLD footprint (stored bounds are
    /// parent-space, so ancestor transforms apply first). Falls back to
    /// a full-grid mark when the world footprint can't be resolved.
    pub fn mark_world_dirty(&mut self, id: u64) {
        let Some(i) = self.find(id) else { return };
        let b = if matches!(self.objects[i].shape, Shape::Group { .. }) {
            self.world_bounds(id)
        } else {
            Some(self.objects[i].bounds_under(self.parent_abs(id)))
        };
        match b {
            Some(b) => self.mark_dirty(b),
            None => {} // empty group: nothing rasterized, nothing dirty
        }
    }

    /// World-footprint damage for an object VALUE that may no longer be
    /// (or not yet be) in the list (undo/redo replay): resolve its
    /// ancestor chain against the current list; a broken chain marks
    /// the whole grid — correctness over precision during replays.
    fn mark_value_world_dirty(&mut self, o: &SketchObject) {
        if matches!(o.shape, Shape::Group { .. }) {
            // Its subtree members mark themselves in the same replay
            // batch; a group node alone has no raster footprint.
            return;
        }
        let mut acc = Sim2::IDENTITY;
        let mut cur = o.parent;
        for _ in 0..Self::MAX_DEPTH {
            let Some(pid) = cur else {
                let b = o.bounds_under(acc);
                self.mark_dirty(b);
                return;
            };
            match self.find(pid) {
                Some(pi) => {
                    acc = self.objects[pi].shape.group_sim().compose(acc);
                    cur = self.objects[pi].parent;
                }
                None => break, // chain broken mid-replay: be conservative
            }
        }
        self.mark_all_dirty();
    }

    /// Translate by a WORLD delta, converted into the object's stored
    /// space. No undo record (gesture-live semantics); marks damage.
    pub fn translate_world(&mut self, id: u64, d: [f32; 2]) {
        let dl = self.parent_abs(id).inverse().apply_vec(d);
        self.mark_world_dirty(id);
        if let Some(i) = self.find(id) {
            self.objects[i].translate(dl);
        }
        self.mark_world_dirty(id);
    }

    /// Rotate by `da` about a WORLD pivot. A similarity conjugates a
    /// rotation to a rotation of the same angle, so the stored-space
    /// rotation is also `da`, about the mapped pivot. Marks damage; no
    /// undo record.
    pub fn rotate_world(&mut self, id: u64, pivot_world: [f32; 2], da: f32) {
        let pivot = self.parent_abs(id).inverse().apply(pivot_world);
        self.mark_world_dirty(id);
        if let Some(i) = self.find(id) {
            self.objects[i].rotate_about(pivot, da);
        }
        self.mark_world_dirty(id);
    }

    /// Uniform scale by `f` about a WORLD pivot (same conjugation
    /// argument as `rotate_world`). Marks damage; no undo record.
    pub fn scale_world(&mut self, id: u64, pivot_world: [f32; 2], f: f32) {
        let pivot = self.parent_abs(id).inverse().apply(pivot_world);
        self.mark_world_dirty(id);
        if let Some(i) = self.find(id) {
            self.objects[i].scale_about(pivot, f);
        }
        self.mark_world_dirty(id);
    }

    /// Topmost click-selectable object hit at `p` (world cells). Locked
    /// and hidden subtrees are skipped — they are tree-managed. Returns
    /// the LEAF id; the app maps it to a group scope for selection.
    pub fn hit_test(&self, p: [f32; 2], slop: f32) -> Option<u64> {
        self.objects
            .iter()
            .rev()
            .find(|o| {
                !matches!(o.shape, Shape::Group { .. })
                    && !self.eff_locked(o.id)
                    && !self.eff_hidden(o.id)
                    && o.hit_under(p, slop, self.parent_abs(o.id))
            })
            .map(|o| o.id)
    }

    pub fn add(&mut self, obj: SketchObject) {
        self.mark_value_world_dirty(&obj);
        self.undo.push(ModelOp::Add(obj.clone()));
        self.redo.clear();
        self.objects.push(obj);
    }

    /// Add a whole set as ONE undo entry (paste, multi-duplicate).
    pub fn add_many(&mut self, objs: Vec<SketchObject>) {
        if objs.is_empty() {
            return;
        }
        let mut ops = Vec::with_capacity(objs.len());
        for obj in objs {
            self.mark_value_world_dirty(&obj);
            ops.push(ModelOp::Add(obj.clone()));
            self.objects.push(obj);
        }
        self.undo.push(ModelOp::Group { ops, coalesce: false });
        self.redo.clear();
    }

    /// Remove a whole set as ONE undo entry. Member Remove indices are
    /// captured sequentially, so the group's reverse-order undo restores
    /// each object at a valid slot.
    pub fn remove_many(&mut self, ids: &[u64]) {
        let mut ops = Vec::new();
        for &id in ids {
            if let Some(i) = self.find(id) {
                // Mark while the ancestor chain is still intact, then
                // remove (a subtree delete removes parents too).
                self.mark_world_dirty(id);
                let obj = self.objects.remove(i);
                ops.push(ModelOp::Remove(i, obj));
            }
        }
        if !ops.is_empty() {
            self.undo.push(ModelOp::Group { ops, coalesce: false });
            self.redo.clear();
        }
    }

    /// Insert at a z-slot, undoably (the paint bucket's fill goes to the
    /// BOTTOM so interior island walls keep winning their overlaps —
    /// `model.objects` order is z-order).
    pub fn insert_at(&mut self, index: usize, obj: SketchObject) {
        let i = index.min(self.objects.len());
        self.mark_value_world_dirty(&obj);
        self.undo.push(ModelOp::Insert(i, obj.clone()));
        self.redo.clear();
        self.objects.insert(i, obj);
    }

    /// Apply one eraser stroke's outcome as ONE undo entry. Each change
    /// replaces an object with 0..n fragments: none = fully erased, one
    /// = trimmed in place, several = a split — the first fragment keeps
    /// the original id and z-slot, the rest insert contiguously above it
    /// so overlap resolution doesn't shift. Ops are captured against the
    /// live list in order; the group's reverse-order undo unwinds them
    /// at valid indices (the remove_many convention).
    pub fn apply_erase(&mut self, changes: Vec<(u64, Vec<SketchObject>)>) {
        let mut ops = Vec::new();
        for (id, repl) in changes {
            let Some(i) = self.find(id) else { continue };
            self.mark_world_dirty(id);
            if repl.is_empty() {
                let obj = self.objects.remove(i);
                ops.push(ModelOp::Remove(i, obj));
                continue;
            }
            let before = self.objects[i].clone();
            self.objects[i] = repl[0].clone();
            ops.push(ModelOp::Modify {
                i,
                before,
                after: repl[0].clone(),
                coalesce: false,
            });
            self.mark_world_dirty(id);
            for (k, frag) in repl.iter().enumerate().skip(1) {
                let at = i + k;
                self.objects.insert(at, frag.clone());
                ops.push(ModelOp::Insert(at, frag.clone()));
                self.mark_world_dirty(frag.id);
            }
        }
        if !ops.is_empty() {
            self.undo.push(ModelOp::Group { ops, coalesce: false });
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
            self.mark_value_world_dirty(&before);
            self.mark_world_dirty(id);
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
        self.mark_value_world_dirty(&before);
        self.mark_world_dirty(id);
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

    /// Record one finished edit across a set — ONE undo entry (gesture
    /// end for a multi-object move). `pairs` are (id, before) captured at
    /// gesture start; unchanged members are dropped.
    pub fn record_modify_many(&mut self, pairs: &[(u64, SketchObject)]) {
        let ops = self.build_modify_ops(pairs, false);
        if !ops.is_empty() {
            self.undo.push(ModelOp::Group { ops, coalesce: false });
            self.redo.clear();
        }
    }

    /// Like `record_modify_many`, but consecutive PANEL edits to the same
    /// id set merge into one undo step (the record_modify_coalesced
    /// convention, lifted to sets — one undo reverts the whole slider
    /// session across the whole selection).
    pub fn record_modify_many_coalesced(&mut self, pairs: &[(u64, SketchObject)]) {
        let ops = self.build_modify_ops(pairs, true);
        if ops.is_empty() {
            return;
        }
        self.redo.clear();
        let ids: Vec<u64> = ops
            .iter()
            .map(|op| match op {
                ModelOp::Modify { after, .. } => after.id,
                _ => unreachable!("build_modify_ops emits Modify only"),
            })
            .collect();
        if let Some(ModelOp::Group { ops: top, coalesce: true }) = self.undo.last_mut() {
            let top_ids: Vec<u64> = top
                .iter()
                .filter_map(|op| match op {
                    ModelOp::Modify { after, coalesce: true, .. } => Some(after.id),
                    _ => None,
                })
                .collect();
            if top_ids == ids {
                for (slot, op) in top.iter_mut().zip(ops) {
                    if let (
                        ModelOp::Modify { after: dst, .. },
                        ModelOp::Modify { after: src, .. },
                    ) = (slot, op)
                    {
                        *dst = src;
                    }
                }
                return;
            }
        }
        self.undo.push(ModelOp::Group { ops, coalesce: true });
    }

    fn build_modify_ops(
        &mut self,
        pairs: &[(u64, SketchObject)],
        coalesce: bool,
    ) -> Vec<ModelOp> {
        let mut ops = Vec::new();
        for (id, before) in pairs {
            if let Some(i) = self.find(*id) {
                if self.objects[i] == *before {
                    continue;
                }
                let after = self.objects[i].clone();
                self.mark_value_world_dirty(before);
                self.mark_world_dirty(*id);
                ops.push(ModelOp::Modify {
                    i,
                    before: before.clone(),
                    after,
                    coalesce,
                });
            }
        }
        ops
    }

    /// Reorder the object list to the given id order (z-order: later =
    /// painted later = wins overlaps), undoably. Ids must be a
    /// permutation of the current set; anything else is a no-op.
    pub fn reorder(&mut self, new_ids: Vec<u64>) {
        let old_ids: Vec<u64> = self.objects.iter().map(|o| o.id).collect();
        if new_ids == old_ids || new_ids.len() != old_ids.len() {
            return;
        }
        if !self.apply_order(&new_ids) {
            return;
        }
        self.undo.push(ModelOp::Reorder(old_ids, new_ids));
        self.redo.clear();
    }

    /// Permute `objects` into the given id order, damage-marking every
    /// object whose index changed (overlap winners can flip).
    fn apply_order(&mut self, ids: &[u64]) -> bool {
        // A true permutation only: every id resolves, and no id twice
        // (a duplicate would clone one object and drop another).
        let mut seen = Vec::with_capacity(ids.len());
        let mut new = Vec::with_capacity(self.objects.len());
        for &id in ids {
            match self.find(id) {
                Some(i) if !seen.contains(&i) => {
                    seen.push(i);
                    new.push(self.objects[i].clone());
                }
                _ => return false, // not a permutation; leave untouched
            }
        }
        if new.len() != self.objects.len() {
            return false;
        }
        let moved: Vec<u64> = new
            .iter()
            .enumerate()
            .filter(|(i, o)| self.objects[*i].id != o.id)
            .map(|(_, o)| o.id)
            .collect();
        self.objects = new;
        for id in moved {
            self.mark_world_dirty(id);
        }
        true
    }

    // --- Group / ungroup / reparent (U3): one undo entry each ---------

    /// Group the given objects under a fresh Group node — ONE undo
    /// entry; returns the new group's id. Callers pass OUTERMOST ids
    /// only (no id may be a descendant of another). The group's parent
    /// is the members' common parent when they agree, else the root;
    /// members changing parent space are re-expressed so nothing moves
    /// on screen.
    pub fn group_objects(&mut self, ids: &[u64]) -> Option<u64> {
        let ids: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|&id| self.find(id).is_some())
            .collect();
        if ids.is_empty() {
            return None;
        }
        let first_parent = self.objects[self.find(ids[0])?].parent;
        let common = if ids
            .iter()
            .all(|&id| self.objects[self.find(id).unwrap()].parent == first_parent)
        {
            first_parent
        } else {
            None
        };
        let gid = self.fresh_id();
        let group = SketchObject {
            id: gid,
            // Identity transform: the group's space IS its parent's, so
            // members already under `common` keep their coordinates.
            shape: Shape::Group { t: [0.0, 0.0], rot: 0.0, scale: 1.0 },
            material: ObjMaterial::Wall,
            thickness: 1.0,
            filled: false,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: common,
        };
        let mut ops = Vec::with_capacity(ids.len() + 1);
        self.objects.push(group.clone());
        ops.push(ModelOp::Add(group));
        for &id in &ids {
            let i = self.find(id).unwrap();
            let before = self.objects[i].clone();
            let old_abs = self.parent_abs(id);
            self.objects[i].parent = Some(gid);
            let new_abs = self.parent_abs(id);
            self.objects[i].reexpress(old_abs, new_abs);
            let after = self.objects[i].clone();
            // World geometry is unchanged by construction: no damage.
            ops.push(ModelOp::Modify { i, before, after, coalesce: false });
        }
        self.undo.push(ModelOp::Group { ops, coalesce: false });
        self.redo.clear();
        Some(gid)
    }

    /// Dissolve a group: its direct children move to its parent (world
    /// geometry unchanged), the node itself is removed — ONE undo entry.
    pub fn ungroup(&mut self, gid: u64) -> bool {
        let Some(gi) = self.find(gid) else { return false };
        if !matches!(self.objects[gi].shape, Shape::Group { .. }) {
            return false;
        }
        let gparent = self.objects[gi].parent;
        let mut ops = Vec::new();
        for id in self.children_of(gid) {
            let i = self.find(id).unwrap();
            let before = self.objects[i].clone();
            let old_abs = self.parent_abs(id);
            self.objects[i].parent = gparent;
            let new_abs = self.parent_abs(id);
            self.objects[i].reexpress(old_abs, new_abs);
            let after = self.objects[i].clone();
            ops.push(ModelOp::Modify { i, before, after, coalesce: false });
        }
        // Remove the node last: the member Modify indices above were
        // captured with it present, and the group's reverse-order undo
        // reinserts it first, so they stay valid both ways.
        let gi = self.find(gid).unwrap();
        let node = self.objects.remove(gi);
        ops.push(ModelOp::Remove(gi, node));
        self.undo.push(ModelOp::Group { ops, coalesce: false });
        self.redo.clear();
        true
    }

    /// Move `id` under `new_parent` (None = root), re-expressed so its
    /// world geometry holds — one undo entry. CYCLE PREVENTION: a group
    /// can never become its own descendant (or its own parent); such
    /// reparents are refused.
    pub fn reparent(&mut self, id: u64, new_parent: Option<u64>) -> Result<(), &'static str> {
        let Some(i) = self.find(id) else {
            return Err("no such object");
        };
        if let Some(np) = new_parent {
            if np == id {
                return Err("a group can't be moved into itself");
            }
            let Some(pi) = self.find(np) else {
                return Err("no such group");
            };
            if !matches!(self.objects[pi].shape, Shape::Group { .. }) {
                return Err("target is not a group");
            }
            if self.is_descendant(np, id) {
                return Err("a group can't be moved into its own subtree");
            }
        }
        if self.objects[i].parent == new_parent {
            return Ok(()); // no-op, no undo entry
        }
        let before = self.objects[i].clone();
        let old_abs = self.parent_abs(id);
        self.objects[i].parent = new_parent;
        let new_abs = self.parent_abs(id);
        self.objects[i].reexpress(old_abs, new_abs);
        let after = self.objects[i].clone();
        self.undo.push(ModelOp::Modify { i, before, after, coalesce: false });
        self.redo.clear();
        Ok(())
    }

    // --- Mirror / linear array (deferred out of U4, landed after) ----
    // Both produce INDEPENDENT deep copies — fresh ids, parent links
    // rewired within each copy, nothing shared with the originals —
    // and commit as ONE undo entry via add_many's Group op.

    /// Deep-copy the subtree at `root`: fresh ids, parent links
    /// remapped WITHIN the copy (the copied root keeps the original
    /// root's parent, staying its sibling), `f` applied to every copy
    /// (`true` = the root). Returns (copies in model order, new root
    /// id). Nothing is added to the model here.
    fn copy_subtree_with(
        &mut self,
        root: u64,
        mut f: impl FnMut(&mut SketchObject, bool),
    ) -> Option<(Vec<SketchObject>, u64)> {
        self.find(root)?;
        let ids = self.subtree_ids(root);
        // Fresh ids minted first, then parents remapped — children can
        // precede their group node in z-order (the paste convention).
        let mut id_map = std::collections::HashMap::new();
        for &id in &ids {
            id_map.insert(id, self.fresh_id());
        }
        let mut copies = Vec::with_capacity(ids.len());
        for &id in &ids {
            let i = self.find(id).unwrap();
            let mut c = self.objects[i].clone();
            c.id = id_map[&id];
            if let Some(np) = c.parent.and_then(|p| id_map.get(&p).copied()) {
                c.parent = Some(np);
            }
            f(&mut c, id == root);
            copies.push(c);
        }
        Some((copies, id_map[&root]))
    }

    /// Mirror the subtrees rooted at `roots` (OUTERMOST ids — the
    /// transform_targets convention) across the WORLD line a–b: one
    /// independent deep copy per root, all added as ONE undo entry.
    /// Each copy stays a sibling of its original; the world line
    /// conjugates into that root's parent space (a similarity maps the
    /// line, and the reflection about the mapped line IS the
    /// conjugated reflection), and the same parent-space reflection
    /// then applies to every node of the subtree (see
    /// `SketchObject::reflect`). Returns the new root ids.
    pub fn mirror_subtrees(&mut self, roots: &[u64], a: [f32; 2], b: [f32; 2]) -> Vec<u64> {
        if Reflect2::across(a, b).is_none() {
            return Vec::new();
        }
        let mut all = Vec::new();
        let mut new_roots = Vec::new();
        for &root in roots {
            if self.find(root).is_none() {
                continue;
            }
            let inv = self.parent_abs(root).inverse();
            let Some(m) = Reflect2::across(inv.apply(a), inv.apply(b)) else {
                continue;
            };
            if let Some((copies, nr)) = self.copy_subtree_with(root, |c, _| c.reflect(m))
            {
                new_roots.push(nr);
                all.extend(copies);
            }
        }
        self.add_many(all);
        new_roots
    }

    /// Linear array: `copies` additional deep copies of each subtree
    /// in `roots` (OUTERMOST ids), the k-th translated by k·`step`
    /// (WORLD cells), all added as ONE undo entry. Only each copy's
    /// root translates — the step converts into its kept parent's
    /// space and the subtree follows through composition. Returns the
    /// new root ids.
    pub fn array_subtrees(
        &mut self,
        roots: &[u64],
        step: [f32; 2],
        copies: usize,
    ) -> Vec<u64> {
        let mut all = Vec::new();
        let mut new_roots = Vec::new();
        for k in 1..=copies {
            let d = [step[0] * k as f32, step[1] * k as f32];
            for &root in roots {
                if self.find(root).is_none() {
                    continue;
                }
                let dl = self.parent_abs(root).inverse().apply_vec(d);
                if let Some((copies_k, nr)) = self.copy_subtree_with(root, |c, is_root| {
                    if is_root {
                        c.translate(dl);
                    }
                }) {
                    new_roots.push(nr);
                    all.extend(copies_k);
                }
            }
        }
        self.add_many(all);
        new_roots
    }

    /// Remove an object added by the in-flight gesture along with its
    /// undo record, as if it was never drawn (Esc / degenerate shapes).
    pub fn cancel_last_add(&mut self, id: u64) {
        if let Some(i) = self.find(id) {
            self.mark_world_dirty(id);
            self.objects.remove(i);
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
            self.apply_undo(&op);
            self.redo.push(op);
        }
    }

    pub fn redo(&mut self) {
        if let Some(op) = self.redo.pop() {
            self.apply_redo(&op);
            self.undo.push(op);
        }
    }

    fn apply_undo(&mut self, op: &ModelOp) {
        match op {
            ModelOp::Add(o) => {
                if let Some(i) = self.find(o.id) {
                    self.mark_world_dirty(o.id);
                    self.objects.remove(i);
                }
            }
            ModelOp::Remove(i, o) => {
                self.objects.insert((*i).min(self.objects.len()), o.clone());
                self.mark_world_dirty(o.id);
            }
            ModelOp::Insert(i, o) => {
                self.mark_world_dirty(o.id);
                // The captured index is valid inside a group's reverse
                // replay; fall back to the id if a stray op drifted.
                if self.objects.get(*i).map(|x| x.id) == Some(o.id) {
                    self.objects.remove(*i);
                } else if let Some(j) = self.find(o.id) {
                    self.objects.remove(j);
                }
            }
            ModelOp::Modify { i, before, .. } => {
                // Mark both states' WORLD footprints (a group's covers
                // its subtree), with the list state that holds each.
                let id = before.id;
                self.mark_world_dirty(id);
                if let Some(slot) = self.objects.get_mut(*i) {
                    *slot = before.clone();
                }
                self.mark_world_dirty(id);
            }
            ModelOp::Replace(old, _new) => {
                self.objects = old.clone();
                self.mark_all_dirty();
            }
            ModelOp::Reorder(old_ids, _new_ids) => {
                self.apply_order(old_ids);
            }
            // Members undo in reverse so sequential Remove indices (and
            // any future dependent members) unwind correctly.
            ModelOp::Group { ops, .. } => {
                for member in ops.iter().rev() {
                    self.apply_undo(member);
                }
            }
        }
    }

    fn apply_redo(&mut self, op: &ModelOp) {
        match op {
            ModelOp::Add(o) => {
                self.objects.push(o.clone());
                self.mark_world_dirty(o.id);
            }
            ModelOp::Remove(i, o) => {
                self.mark_world_dirty(o.id);
                if *i < self.objects.len() {
                    self.objects.remove(*i);
                } else {
                    self.objects.pop();
                }
            }
            ModelOp::Insert(i, o) => {
                self.objects.insert((*i).min(self.objects.len()), o.clone());
                self.mark_world_dirty(o.id);
            }
            ModelOp::Modify { i, after, .. } => {
                let id = after.id;
                self.mark_world_dirty(id);
                if let Some(slot) = self.objects.get_mut(*i) {
                    *slot = after.clone();
                }
                self.mark_world_dirty(id);
            }
            ModelOp::Replace(_old, new) => {
                self.objects = new.clone();
                self.mark_all_dirty();
            }
            ModelOp::Reorder(_old_ids, new_ids) => {
                self.apply_order(new_ids);
            }
            ModelOp::Group { ops, .. } => {
                for member in ops {
                    self.apply_redo(member);
                }
            }
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
            // Group nodes have no raster footprint; hiding one hides
            // its whole subtree (eff_hidden walks the ancestor chain).
            if matches!(o.shape, Shape::Group { .. }) || self.eff_hidden(o.id) {
                continue;
            }
            let abs = self.parent_abs(o.id);
            if o.bounds_under(abs).intersect(region_for_test).is_empty() {
                continue;
            }
            if abs.is_identity() {
                rasterize_object(geo, o, clip, m);
            } else {
                // Bake the ancestor transforms into a world-space clone
                // so the shape rasterizers stay world-space-only. (For
                // stamps this clones the raster; the damage region
                // bounds how often it happens.)
                let mut flat = o.clone();
                flat.apply_sim(abs);
                rasterize_object(geo, &flat, clip, m);
            }
        }
        geo.touch(clip);
    }
}

// --- Object rasterizers (all clipped to `clip`, full-grid coords) ----

/// Shared open/closed chain rasterizer: single point → dot, filled
/// closed ring → even-odd scanline (thickness ignored, like filled
/// Rect/Ellipse — U4), otherwise capsule per segment. Polys pass their
/// stored vertices; arcs and splines pass their curve samples, so a
/// chained Fan blows along the curve exactly as drawn.
fn rasterize_chain(
    geo: &mut Geometry,
    obj: &SketchObject,
    pts: &[[f32; 2]],
    closed: bool,
    clip: GridRect,
    m: i32,
    t_r: f32,
) {
    let mf = m as f32;
    let n = pts.len();
    if n == 0 {
        return;
    }
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
    if obj.filled && closed && n >= 3 {
        // Filled ring: even-odd scanline over the cell centres.
        let rect = obj.bounds();
        let rect = GridRect {
            x0: rect.x0 + m,
            y0: rect.y0 + m,
            x1: rect.x1 + m,
            y1: rect.y1 + m,
        }
        .intersect(clip);
        let w = geo.w;
        let mut write = cell_writer(geo, obj);
        let mut xs: Vec<f32> = Vec::with_capacity(8);
        for y in rect.y0..rect.y1 {
            let yc = y as f32 + 0.5 - mf;
            xs.clear();
            for i in 0..n {
                let a = pts[i];
                let b = pts[(i + 1) % n];
                if (a[1] <= yc) != (b[1] <= yc) {
                    xs.push(a[0] + (yc - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
            xs.sort_by(|p, q| p.partial_cmp(q).unwrap());
            for pair in xs.chunks_exact(2) {
                // Cells whose centre lies inside the span.
                let x0 = ((pair[0] + mf - 0.5).ceil() as i32).max(rect.x0);
                let x1 = ((pair[1] + mf - 0.5).floor() as i32).min(rect.x1 - 1);
                for x in x0..=x1 {
                    write((y as usize) * w + x as usize, None);
                }
            }
        }
        return;
    }
    let segs = if closed { n } else { n.saturating_sub(1) };
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
        Shape::Group { .. } => {} // no geometry (filtered by the caller)
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
            rasterize_chain(geo, obj, pts, *closed, clip, m, t_r);
        }
        Shape::Arc { c, r, start, sweep } => {
            let pts = sample_arc(*c, *r, *start, *sweep);
            rasterize_chain(geo, obj, &pts, false, clip, m, t_r);
        }
        Shape::Spline { pts, closed } => {
            let sampled = sample_spline(pts, *closed);
            rasterize_chain(geo, obj, &sampled, *closed, clip, m, t_r);
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
                    // placement. `obj.fan_angle` is deliberately NOT
                    // applied: stamp fan vectors stay locked to the
                    // stamp's geometric `angle`. Rotating the chamber
                    // flow independently of the bell would aim thrust
                    // into the converging wall — a nozzle is aimed by
                    // rotating the whole object.
                    let f = raster.fan[si];
                    let m = obj.fan_mult;
                    geo.fan[gi] = [
                        (f[0] * co - f[1] * s) * m,
                        (f[0] * s + f[1] * co) * m,
                        (f[2] + obj.fan_gust).clamp(0.0, 1.0),
                        f[3],
                    ];
                    // Fan-cell dye takes the object's smoke color (baked
                    // alpha kept), so a generated engine's plume recolors
                    // like a hand-placed fan's.
                    let d = raster.dye_src[si];
                    geo.dye_src[gi] = if (f[0] != 0.0 || f[1] != 0.0) && d[3] > 0.0 {
                        [obj.smoke_rgb[0], obj.smoke_rgb[1], obj.smoke_rgb[2], d[3]]
                    } else {
                        d
                    };
                }
            }
        }
    }
}

// --- Tests -----------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(model: &mut SketchModel, c: [f32; 2]) -> u64 {
        let id = model.fresh_id();
        model.add(SketchObject {
            id,
            shape: Shape::Rect { c, half: [10.0, 10.0], angle: 0.0 },
            material: ObjMaterial::Wall,
            thickness: 4.0,
            filled: true,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        });
        id
    }

    fn ids(m: &SketchModel) -> Vec<u64> {
        m.objects.iter().map(|o| o.id).collect()
    }

    #[test]
    fn insert_at_undoes_and_redoes_at_its_slot() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        let b = obj(&mut m, [50.0, 0.0]);
        let id = m.fresh_id();
        let mut o = m.objects[0].clone();
        o.id = id;
        m.insert_at(0, o);
        assert_eq!(ids(&m), vec![id, a, b]);
        m.undo();
        assert_eq!(ids(&m), vec![a, b]);
        m.redo();
        assert_eq!(ids(&m), vec![id, a, b]);
    }

    #[test]
    fn apply_erase_split_is_one_undo_entry_with_contiguous_fragments() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        let b = obj(&mut m, [50.0, 0.0]);
        let c = obj(&mut m, [100.0, 0.0]);
        let order0: Vec<u64> = ids(&m);
        let before_b = m.objects[m.find(b).unwrap()].clone();
        // Split b into two fragments and delete c, as one stroke.
        let mut f1 = before_b.clone();
        f1.shape = Shape::Line { a: [40.0, 0.0], b: [45.0, 0.0] };
        let f2_id = m.fresh_id();
        let mut f2 = before_b.clone();
        f2.id = f2_id;
        f2.shape = Shape::Line { a: [55.0, 0.0], b: [60.0, 0.0] };
        m.apply_erase(vec![(b, vec![f1, f2]), (c, Vec::new())]);
        // Fragments sit contiguously at b's slot; c is gone.
        assert_eq!(ids(&m), vec![a, b, f2_id]);
        assert!(matches!(
            m.objects[m.find(b).unwrap()].shape,
            Shape::Line { .. }
        ));
        // ONE undo restores everything, including c's slot and b's shape.
        m.undo();
        assert_eq!(ids(&m), order0);
        assert!(m.objects[m.find(b).unwrap()] == before_b);
        m.redo();
        assert_eq!(ids(&m), vec![a, b, f2_id]);
    }

    #[test]
    fn filled_closed_poly_rasterizes_its_interior() {
        let mut m = SketchModel::default();
        let id = m.fresh_id();
        m.add(SketchObject {
            id,
            // A 10×10 axis-aligned diamond (so scanlines cross edges).
            shape: Shape::Poly {
                pts: vec![[10.0, 2.0], [18.0, 10.0], [10.0, 18.0], [2.0, 10.0]],
                closed: true,
            },
            material: ObjMaterial::Wall,
            thickness: 2.0,
            filled: true,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        });
        let mut geo = Geometry::new(24, 24);
        m.rasterize_region(&mut geo, GridRect::full(24, 24), 0, false);
        // The centre is solid, the diamond's area is roughly half the
        // bounding box, and cells outside the bounds stay fluid.
        assert_eq!(geo.cell[10 * 24 + 10], CELL_WALL);
        let walls = geo.cell.iter().filter(|&&c| c == CELL_WALL).count();
        // Ideal area = d²/2 = 128; the cell-centre test quantizes.
        assert!((100..160).contains(&walls), "diamond fill {walls} cells");
        assert_eq!(geo.cell[0], CELL_FLUID);
        // An unfilled copy must NOT fill the interior (outline only).
        let i = m.find(id).unwrap();
        m.objects[i].filled = false;
        m.mark_all_dirty();
        let mut geo2 = Geometry::new(24, 24);
        m.rasterize_region(&mut geo2, GridRect::full(24, 24), 0, false);
        assert_eq!(geo2.cell[10 * 24 + 10], CELL_FLUID);
    }

    #[test]
    fn filled_closed_poly_hit_tests_its_interior() {
        let mut m = SketchModel::default();
        let id = m.fresh_id();
        let mut o = SketchObject {
            id,
            shape: Shape::Poly {
                pts: vec![[10.0, 2.0], [18.0, 10.0], [10.0, 18.0], [2.0, 10.0]],
                closed: true,
            },
            material: ObjMaterial::Wall,
            thickness: 2.0,
            filled: true,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        };
        assert!(o.hit([10.0, 10.0], 0.0), "interior must hit when filled");
        o.filled = false;
        assert!(!o.hit([10.0, 10.0], 0.0), "interior must miss when unfilled");
        assert!(o.hit([10.0, 2.5], 1.0), "outline still hits");
        let _ = m.add(o);
    }

    #[test]
    fn remove_many_is_one_undo_entry_and_restores_order() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        let b = obj(&mut m, [50.0, 0.0]);
        let c = obj(&mut m, [100.0, 0.0]);
        let order0 = ids(&m);
        m.remove_many(&[a, c]);
        assert_eq!(ids(&m), vec![b]);
        m.undo(); // ONE undo restores both, at their original slots
        assert_eq!(ids(&m), order0);
        m.redo();
        assert_eq!(ids(&m), vec![b]);
    }

    #[test]
    fn add_many_is_one_undo_entry() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        let mut o1 = m.objects[0].clone();
        o1.id = m.fresh_id();
        let mut o2 = m.objects[0].clone();
        o2.id = m.fresh_id();
        let (i1, i2) = (o1.id, o2.id);
        m.add_many(vec![o1, o2]);
        assert_eq!(ids(&m), vec![a, i1, i2]);
        m.undo();
        assert_eq!(ids(&m), vec![a]);
        m.redo();
        assert_eq!(ids(&m), vec![a, i1, i2]);
    }

    #[test]
    fn modify_many_records_one_entry_and_coalesces_by_id_set() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        let b = obj(&mut m, [50.0, 0.0]);
        // A "panel edit" across the set, twice (slider frames): the
        // second merges into the first, one undo reverts to the start.
        for step in [1.0f32, 2.0] {
            let pairs: Vec<(u64, SketchObject)> = [a, b]
                .iter()
                .map(|&id| {
                    let i = m.find(id).unwrap();
                    let before = m.objects[i].clone();
                    m.objects[i].thickness = 4.0 + step;
                    (id, before)
                })
                .collect();
            m.record_modify_many_coalesced(&pairs);
        }
        assert_eq!(m.objects[0].thickness, 6.0);
        m.undo();
        assert_eq!(m.objects[0].thickness, 4.0);
        assert_eq!(m.objects[1].thickness, 4.0);
        assert!(!m.can_undo() || {
            m.undo();
            m.objects[0].thickness == 4.0
        });
    }

    #[test]
    fn reorder_round_trips_and_rejects_non_permutations() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        let b = obj(&mut m, [50.0, 0.0]);
        let c = obj(&mut m, [100.0, 0.0]);
        m.reorder(vec![c, a, b]);
        assert_eq!(ids(&m), vec![c, a, b]);
        m.undo();
        assert_eq!(ids(&m), vec![a, b, c]);
        m.redo();
        assert_eq!(ids(&m), vec![c, a, b]);
        // Not a permutation: ignored.
        m.reorder(vec![a, a, b]);
        assert_eq!(ids(&m), vec![c, a, b]);
    }

    #[test]
    fn hit_test_skips_locked_and_hidden() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        assert_eq!(m.hit_test([0.0, 0.0], 1.0), Some(a));
        m.objects[0].locked = true;
        assert_eq!(m.hit_test([0.0, 0.0], 1.0), None);
        m.objects[0].locked = false;
        m.objects[0].hidden = true;
        assert_eq!(m.hit_test([0.0, 0.0], 1.0), None);
    }

    // --- U3: nested groups and transform composition ------------------

    /// Two nested groups, both rotated: the composed transform is the
    /// child's stored geometry first, then each ancestor OUTWARD (see
    /// CLAUDE.md). Getting the order wrong flips the sign of the
    /// off-pivot displacement, which this asserts against.
    #[test]
    fn composition_is_child_then_ancestors_outward() {
        let mut m = SketchModel::default();
        let leaf = obj(&mut m, [10.0, 0.0]);
        let inner = m.group_objects(&[leaf]).unwrap();
        let outer = m.group_objects(&[inner]).unwrap();
        // Inner rotates +90° about the world origin; outer translates
        // by (100, 0). Child-then-outward: (10,0) → rot → (0,10) →
        // translate → (100,10). The wrong order (ancestors first)
        // would rotate the translation too and land at (-10, 100).
        let ii = m.find(inner).unwrap();
        m.objects[ii].rotate_about([0.0, 0.0], std::f32::consts::FRAC_PI_2);
        let oi = m.find(outer).unwrap();
        m.objects[oi].translate([100.0, 0.0]);
        let c = m.world_center(leaf).unwrap();
        assert!((c[0] - 100.0).abs() < 1e-3, "x = {}", c[0]);
        assert!((c[1] - 10.0).abs() < 1e-3, "y = {}", c[1]);
    }

    /// Grouping and ungrouping never move world geometry, even when the
    /// enclosing chain carries rotation and scale.
    #[test]
    fn group_ungroup_round_trip_holds_world_geometry() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [40.0, 20.0]);
        let g = m.group_objects(&[a]).unwrap();
        let gi = m.find(g).unwrap();
        m.objects[gi].rotate_about([0.0, 0.0], 0.7);
        m.objects[gi].scale_about([5.0, 5.0], 2.0);
        let before = m.world_center(a).unwrap();
        // Group the (already transformed) group again, then dissolve
        // both: the leaf must not move.
        let outer = m.group_objects(&[g]).unwrap();
        let mid = m.world_center(a).unwrap();
        assert!((before[0] - mid[0]).abs() < 1e-2 && (before[1] - mid[1]).abs() < 1e-2);
        assert!(m.ungroup(outer));
        assert!(m.ungroup(g));
        let after = m.world_center(a).unwrap();
        assert!(
            (before[0] - after[0]).abs() < 1e-2 && (before[1] - after[1]).abs() < 1e-2,
            "world centre moved: {before:?} → {after:?}"
        );
        // Fully dissolved: the leaf is a root object again.
        let ai = m.find(a).unwrap();
        assert_eq!(m.objects[ai].parent, None);
    }

    /// CYCLE PREVENTION on reparent: a group can never become its own
    /// descendant (or its own parent) — such reparents are refused and
    /// leave the model untouched.
    #[test]
    fn reparent_refuses_cycles() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        let b = obj(&mut m, [50.0, 0.0]);
        let inner = m.group_objects(&[a]).unwrap();
        let outer = m.group_objects(&[inner]).unwrap();
        // Self-parenting.
        assert!(m.reparent(outer, Some(outer)).is_err());
        // Direct child.
        assert!(m.reparent(outer, Some(inner)).is_err());
        // Deeper descendant: make one more level below inner.
        let deepest = m.group_objects(&[b]).unwrap();
        m.reparent(deepest, Some(inner)).unwrap();
        assert!(m.reparent(outer, Some(deepest)).is_err());
        // Structure is intact after the refusals.
        assert_eq!(m.objects[m.find(inner).unwrap()].parent, Some(outer));
        assert_eq!(m.objects[m.find(outer).unwrap()].parent, None);
        // Reparenting to a NON-group is refused too.
        assert!(m.reparent(inner, Some(a)).is_err());
        // A legal reparent round-trips through undo.
        m.reparent(deepest, None).unwrap();
        assert_eq!(m.objects[m.find(deepest).unwrap()].parent, None);
        m.undo();
        assert_eq!(m.objects[m.find(deepest).unwrap()].parent, Some(inner));
    }

    /// Grouping is one undo entry; undoing it restores parents and
    /// removes the node.
    #[test]
    fn group_is_one_undo_entry() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        let b = obj(&mut m, [50.0, 0.0]);
        let g = m.group_objects(&[a, b]).unwrap();
        assert_eq!(m.objects[m.find(a).unwrap()].parent, Some(g));
        m.undo();
        assert!(m.find(g).is_none());
        assert_eq!(m.objects[m.find(a).unwrap()].parent, None);
        m.redo();
        assert_eq!(m.objects[m.find(b).unwrap()].parent, Some(g));
    }

    /// Locking or hiding a group takes effect for the whole subtree
    /// (hit tests skip it), while the members' own flags stay clear.
    #[test]
    fn group_flags_apply_to_subtree() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [0.0, 0.0]);
        let g = m.group_objects(&[a]).unwrap();
        assert_eq!(m.hit_test([0.0, 0.0], 1.0), Some(a));
        let gi = m.find(g).unwrap();
        m.objects[gi].locked = true;
        assert!(m.eff_locked(a));
        assert_eq!(m.hit_test([0.0, 0.0], 1.0), None);
        m.objects[gi].locked = false;
        m.objects[gi].hidden = true;
        assert!(m.eff_hidden(a));
        assert_eq!(m.hit_test([0.0, 0.0], 1.0), None);
    }

    /// Hit tests resolve through a transformed chain: after rotating a
    /// group, the member is hit at its NEW world position only.
    #[test]
    fn hit_test_through_transformed_group() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [40.0, 0.0]);
        let g = m.group_objects(&[a]).unwrap();
        let gi = m.find(g).unwrap();
        m.objects[gi].rotate_about([0.0, 0.0], std::f32::consts::FRAC_PI_2);
        assert_eq!(m.hit_test([0.0, 40.0], 1.0), Some(a));
        assert_eq!(m.hit_test([40.0, 0.0], 1.0), None);
    }

    #[test]
    fn rubber_band_intersect_semantics() {
        let line = SketchObject {
            id: 1,
            shape: Shape::Line { a: [0.0, 0.0], b: [100.0, 100.0] },
            material: ObjMaterial::Wall,
            thickness: 2.0,
            filled: false,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        };
        // Band crossing the segment: selected (INTERSECT, not contain).
        assert!(line.intersects_rect([40.0, 40.0], [60.0, 60.0]));
        // Band inside the line's bounding box but off the geometry
        // (the empty corner of the diagonal): NOT selected.
        assert!(!line.intersects_rect([70.0, 10.0], [90.0, 30.0]));
        // Band containing the whole object: selected.
        assert!(line.intersects_rect([-10.0, -10.0], [110.0, 110.0]));
    }

    // --- Mirror / linear array ----------------------------------------

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-2 && (a[1] - b[1]).abs() < 1e-2
    }

    /// Mirroring across a picked line reflects the world geometry,
    /// keeps parametric shapes parametric, and is ONE undo entry.
    #[test]
    fn mirror_reflects_across_picked_line() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [10.0, 4.0]);
        let new = m.mirror_subtrees(&[a], [0.0, -5.0], [0.0, 5.0]); // x = 0
        assert_eq!(new.len(), 1);
        let c = m.world_center(new[0]).unwrap();
        assert!(close(c, [-10.0, 4.0]), "mirrored centre {c:?}");
        let ni = m.find(new[0]).unwrap();
        assert!(matches!(m.objects[ni].shape, Shape::Rect { .. }));
        // The original did not move.
        assert!(close(m.world_center(a).unwrap(), [10.0, 4.0]));
        m.undo(); // ONE entry removes the copy
        assert!(m.find(new[0]).is_none());
        assert!(m.find(a).is_some());
        m.redo();
        assert!(m.find(new[0]).is_some());
    }

    /// A degenerate line (a == b) mirrors nothing.
    #[test]
    fn mirror_refuses_degenerate_line() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [10.0, 4.0]);
        let new = m.mirror_subtrees(&[a], [3.0, 3.0], [3.0, 3.0]);
        assert!(new.is_empty());
        assert_eq!(m.objects.len(), 1);
        let _ = a;
    }

    /// Mirroring a group copies the WHOLE subtree with parent links
    /// rewired to the new copies — and the copy is not entangled with
    /// its source: transforming or editing the original afterwards
    /// leaves the mirrored copy's world geometry untouched (and vice
    /// versa).
    #[test]
    fn mirrored_group_is_deep_and_disentangled() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [10.0, 2.0]);
        let b = obj(&mut m, [20.0, 12.0]);
        let g = m.group_objects(&[a, b]).unwrap();
        let new = m.mirror_subtrees(&[g], [0.0, 0.0], [0.0, 1.0]); // x = 0
        assert_eq!(new.len(), 1);
        let ng = new[0];
        assert_ne!(ng, g);
        // Two members, fresh ids, parents rewired to the NEW group.
        let members = m.children_of(ng);
        assert_eq!(members.len(), 2);
        for &c in &members {
            assert!(c != a && c != b, "copy shares an id with its source");
            assert_eq!(m.objects[m.find(c).unwrap()].parent, Some(ng));
        }
        // Originals still point at the original group.
        assert_eq!(m.objects[m.find(a).unwrap()].parent, Some(g));
        // World geometry reflected: the copies' centres are the
        // originals' with x negated.
        let mut want: Vec<[f32; 2]> = [a, b]
            .iter()
            .map(|&id| {
                let c = m.world_center(id).unwrap();
                [-c[0], c[1]]
            })
            .collect();
        for &c in &members {
            let wc = m.world_center(c).unwrap();
            let hit = want.iter().position(|&w| close(w, wc));
            assert!(hit.is_some(), "unexpected copy centre {wc:?}");
            want.remove(hit.unwrap());
        }
        // DISENTANGLEMENT: move/rotate/scale the ORIGINAL group; the
        // copy's members hold still.
        let frozen: Vec<[f32; 2]> =
            members.iter().map(|&c| m.world_center(c).unwrap()).collect();
        m.translate_world(g, [37.0, -12.0]);
        m.rotate_world(g, [0.0, 0.0], 0.6);
        m.scale_world(g, [5.0, 5.0], 1.4);
        for (i, &c) in members.iter().enumerate() {
            assert!(
                close(m.world_center(c).unwrap(), frozen[i]),
                "copy moved with its source"
            );
        }
        // And the reverse: moving the copy leaves the original alone.
        let oa = m.world_center(a).unwrap();
        m.translate_world(ng, [5.0, 5.0]);
        assert!(close(m.world_center(a).unwrap(), oa), "source moved with its copy");
    }

    /// Mirroring a root nested under transformed ancestors conjugates
    /// the world line into the root's parent space: the copy's leaf
    /// lands exactly at the reflection of the original leaf's world
    /// position, and the copy stays a sibling of its original.
    #[test]
    fn mirror_conjugates_through_transformed_ancestors() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [10.0, 5.0]);
        let inner = m.group_objects(&[a]).unwrap();
        let outer = m.group_objects(&[inner]).unwrap();
        let oi = m.find(outer).unwrap();
        m.objects[oi].rotate_about([3.0, 7.0], 0.8);
        m.objects[oi].scale_about([1.0, 2.0], 1.7);
        m.objects[oi].translate([4.0, -6.0]);
        let (la, lb) = ([-3.0, 4.0], [9.0, -2.0]);
        let wa = m.world_center(a).unwrap();
        let new = m.mirror_subtrees(&[inner], la, lb);
        assert_eq!(new.len(), 1);
        // Sibling: the copy sits under OUTER, next to inner.
        assert_eq!(m.objects[m.find(new[0]).unwrap()].parent, Some(outer));
        let leaf = m.children_of(new[0])[0];
        let expect = Reflect2::across(la, lb).unwrap().apply(wa);
        let got = m.world_center(leaf).unwrap();
        assert!(close(got, expect), "got {got:?}, want {expect:?}");
    }

    /// The linear array steps each copy by k·step in WORLD space (even
    /// through a rotated parent), rewires parents, and is ONE undo
    /// entry for all copies together.
    #[test]
    fn array_steps_in_world_space_one_undo_entry() {
        let mut m = SketchModel::default();
        let a = obj(&mut m, [10.0, 5.0]);
        let inner = m.group_objects(&[a]).unwrap();
        let outer = m.group_objects(&[inner]).unwrap();
        let oi = m.find(outer).unwrap();
        m.objects[oi].rotate_about([0.0, 0.0], std::f32::consts::FRAC_PI_2);
        let w0 = m.world_center(a).unwrap();
        let n0 = m.objects.len();
        let new = m.array_subtrees(&[inner], [30.0, 10.0], 2);
        assert_eq!(new.len(), 2);
        for (k, &nr) in new.iter().enumerate() {
            let leaf = m.children_of(nr)[0];
            let wc = m.world_center(leaf).unwrap();
            let f = (k + 1) as f32;
            assert!(
                close(wc, [w0[0] + 30.0 * f, w0[1] + 10.0 * f]),
                "copy {k} at {wc:?}"
            );
            // Independence: fresh subtree, parent rewired.
            assert_eq!(m.objects[m.find(leaf).unwrap()].parent, Some(nr));
            assert_ne!(leaf, a);
        }
        m.undo(); // ONE entry removes every copy
        assert_eq!(m.objects.len(), n0);
        m.redo();
        assert_eq!(m.objects.len(), n0 + 4); // 2 copies × (group + leaf)
    }

    /// A mirrored stamp rasterizes exactly the row-flipped cells of
    /// its source (the raster flip + angle conjugation pairing).
    #[test]
    fn mirrored_stamp_rasterizes_flipped() {
        use crate::geometry::Geometry;
        // A 4×4 raster with one asymmetric wall cell and a fan cell.
        // (Even height + integer centre keep grid-cell centres off the
        // raster row boundaries, so the discrete flip is exact.)
        let (w, h) = (4usize, 4usize);
        let mut cell = vec![CELL_FLUID; w * h];
        let mut fan = vec![[0.0f32; 4]; w * h];
        let dye = vec![[0.0f32; 4]; w * h];
        cell[0] = CELL_WALL; // top-left
        cell[1 * w + 2] = CELL_INLET;
        fan[1 * w + 2] = [0.5, 0.25, 0.0, 0.0];
        let raster = GeoRegion {
            rect: (0, 0, w as i32, h as i32),
            cell,
            fan,
            dye_src: dye,
        };
        let mut m = SketchModel::default();
        let id = m.fresh_id();
        m.add(SketchObject {
            id,
            shape: Shape::Stamp { raster, c: [8.0, 8.0], scale: 1.0, angle: 0.0 },
            material: ObjMaterial::Wall,
            thickness: 1.0,
            filled: false,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        });
        let mut geo_src = Geometry::new(16, 16);
        m.rasterize_region(&mut geo_src, GridRect::full(16, 16), 0, false);
        // Mirror across the horizontal line y = 8 (the stamp's centre
        // row): the result must be the vertical flip of the original.
        let new = m.mirror_subtrees(&[id], [0.0, 8.0], [1.0, 8.0]);
        assert_eq!(new.len(), 1);
        // Rasterize the copy alone.
        let keep = new[0];
        let src_i = m.find(id).unwrap();
        m.objects[src_i].hidden = true;
        let mut geo_cpy = Geometry::new(16, 16);
        m.rasterize_region(&mut geo_cpy, GridRect::full(16, 16), 0, false);
        for y in 0..16usize {
            for x in 0..16usize {
                assert_eq!(
                    geo_cpy.cell[y * 16 + x],
                    geo_src.cell[(15 - y) * 16 + x],
                    "cell ({x},{y}) is not the flip of its source"
                );
            }
        }
        // The fan vector flips its y component and keeps x.
        let fi = (0..256).find(|&i| geo_cpy.cell[i] == CELL_INLET).unwrap();
        assert!((geo_cpy.fan[fi][0] - 0.5).abs() < 1e-4);
        assert!((geo_cpy.fan[fi][1] + 0.25).abs() < 1e-4);
        let _ = keep;
    }

    /// Arc geometry invariants (queue item 8): the 3-point fit passes
    /// through all three points with the sweep running through the
    /// middle one, and a double reflection restores the original.
    #[test]
    fn arc_fit_and_reflect_roundtrip() {
        let (a, mid, b) = ([10.0f32, 0.0], [0.0, 10.0], [-10.0, 0.0]);
        let (c, r, start, sweep) = arc_from_three(a, mid, b).expect("fit");
        assert!((c[0]).abs() < 1e-3 && (c[1]).abs() < 1e-3);
        assert!((r - 10.0).abs() < 1e-3);
        let at = |t: f32| [c[0] + r * (start + sweep * t).cos(), c[1] + r * (start + sweep * t).sin()];
        let d = |p: [f32; 2], q: [f32; 2]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
        assert!(d(at(0.0), a) < 1e-2);
        assert!(d(at(0.5), mid) < 1e-2);
        assert!(d(at(1.0), b) < 1e-2);
        // Collinear points refuse instead of producing a huge circle.
        assert!(arc_from_three([0.0, 0.0], [5.0, 0.0], [10.0, 0.0]).is_none());

        // Reflecting an Arc object twice across the same line restores
        // its endpoints (reflection is an involution).
        let mut o = SketchObject {
            id: 1,
            shape: Shape::Arc { c: [30.0, 40.0], r: 12.0, start: 0.7, sweep: 1.9 },
            material: ObjMaterial::Wall,
            thickness: 2.0,
            filled: false,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        };
        let ends = |o: &SketchObject| o.handles();
        let before = ends(&o);
        let m = Reflect2::across([0.0, 0.0], [1.0, 2.0]).expect("line");
        o.reflect(m);
        o.reflect(m);
        let after = ends(&o);
        for (p, q) in before.iter().zip(after.iter()) {
            assert!(d(*p, *q) < 1e-2, "{p:?} vs {q:?}");
        }
    }

    /// Spline sampling passes through every stored point (Catmull-Rom
    /// interpolates, it does not approximate) and the closed form wraps.
    #[test]
    fn spline_samples_interpolate_control_points() {
        let pts = vec![[0.0f32, 0.0], [20.0, 10.0], [40.0, -10.0], [60.0, 5.0]];
        let sam = sample_spline(&pts, false);
        let d = |p: [f32; 2], q: [f32; 2]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
        for cp in &pts {
            assert!(
                sam.iter().any(|sp| d(*sp, *cp) < 1e-3),
                "control point {cp:?} not on the sampled curve"
            );
        }
        assert!(d(sam[0], pts[0]) < 1e-6);
        assert!(d(*sam.last().unwrap(), pts[3]) < 1e-3);
        let closed = sample_spline(&pts, true);
        assert_eq!(closed.len(), pts.len() * 8);
    }

    /// Arcs and splines rasterize into the solver grid (queue item 8):
    /// an arc lays wall cells along its curve — not along its chord —
    /// and a filled closed spline fills its interior.
    #[test]
    fn arc_and_spline_rasterize_to_walls() {
        let mut m = SketchModel::default();
        let id = m.fresh_id();
        m.add(SketchObject {
            id,
            // Upper semicircle, radius 12 around (24, 24).
            shape: Shape::Arc { c: [24.0, 24.0], r: 12.0, start: 0.0, sweep: std::f32::consts::PI },
            material: ObjMaterial::Wall,
            thickness: 2.0,
            filled: false,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        });
        let mut geo = Geometry::new(48, 48);
        m.rasterize_region(&mut geo, GridRect::full(48, 48), 0, false);
        // On the curve (apex of the semicircle):
        assert_eq!(geo.cell[(24 + 12) * 48 + 24], CELL_WALL, "apex");
        // On the chord midpoint — must be FLUID (an arc is not a line):
        assert_eq!(geo.cell[24 * 48 + 24], CELL_FLUID, "chord midpoint");
        // Ends:
        assert_eq!(geo.cell[24 * 48 + 36], CELL_WALL, "start end");
        assert_eq!(geo.cell[24 * 48 + 12], CELL_WALL, "far end");

        // A filled closed spline fills its interior.
        let mut m2 = SketchModel::default();
        let id2 = m2.fresh_id();
        m2.add(SketchObject {
            id: id2,
            shape: Shape::Spline {
                pts: vec![[24.0, 8.0], [40.0, 24.0], [24.0, 40.0], [8.0, 24.0]],
                closed: true,
            },
            material: ObjMaterial::Wall,
            thickness: 2.0,
            filled: true,
            fan_mult: 1.0,
            fan_gust: 0.0,
            fan_phase: 0.0,
            fan_angle: 0.0,
            smoke_rgb: [0.0; 3],
            locked: false,
            hidden: false,
            parent: None,
        });
        let mut geo2 = Geometry::new(48, 48);
        m2.rasterize_region(&mut geo2, GridRect::full(48, 48), 0, false);
        assert_eq!(geo2.cell[24 * 48 + 24], CELL_WALL, "spline interior");
        assert_eq!(geo2.cell[2 * 48 + 2], CELL_FLUID, "outside stays fluid");
        // The curve bulges OUTSIDE the control quad (Catmull-Rom
        // overshoot): the fill follows the curve, so a point just
        // outside the quad's edge midpoint is still wall.
        let walls = geo2.cell.iter().filter(|&&c| c == CELL_WALL).count();
        assert!(walls > 400, "filled spline area {walls}");
    }
}
