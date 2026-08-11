//! U4 geometry operations: the vector eraser's boolean subtraction and
//! the paint bucket's flood fill + contour tracing. These are paired on
//! purpose — the degenerate-case guards at the top are written once and
//! shared by both (see docs/u4-eraser-design.md).
//!
//! Everything here works on plain point lists in one coordinate space;
//! the app conjugates the eraser stroke into each object's STORED space
//! before calling in (a similarity maps a disc to a disc, so the
//! capsules stay exact — the U3 uniform-scale payoff).

use crate::geometry::CELL_FLUID;

// --- Degenerate-case guards (shared: eraser booleans + bucket trace) --
//
// The 0.5-cell minimum radii in ui/canvas.rs are the precedent: nothing
// thinner than a cell survives an operation as an object of its own.

/// Open fragments shorter than this (cells) are dropped.
pub const MIN_RUN_LEN: f32 = 1.0;
/// Closed fragments with less area than this (cells²) are dropped
/// (a 0.5-cell-radius disc is ~0.79 cells²).
pub const MIN_AREA: f32 = 1.0;
/// Vertices closer than this merge; intersections this close together
/// are one crossing seen twice (a vertex graze).
pub const WELD_EPS: f32 = 1e-3;

// --- Small vector helpers ----------------------------------------------

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
fn len(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// Distance from a point to a segment.
pub fn seg_point_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = sub(b, a);
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 > 1e-12 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    len(sub(p, lerp(a, b, t)))
}

/// Distance between two segments (0 when they cross).
pub fn seg_seg_dist(a0: [f32; 2], a1: [f32; 2], b0: [f32; 2], b1: [f32; 2]) -> f32 {
    if segs_intersect(a0, a1, b0, b1).is_some() {
        return 0.0;
    }
    seg_point_dist(a0, b0, b1)
        .min(seg_point_dist(a1, b0, b1))
        .min(seg_point_dist(b0, a0, a1))
        .min(seg_point_dist(b1, a0, a1))
}

/// Segment intersection: Some((t on a, u on b)) for a transversal
/// crossing within both segments.
pub fn segs_intersect(
    a0: [f32; 2],
    a1: [f32; 2],
    b0: [f32; 2],
    b1: [f32; 2],
) -> Option<(f32, f32)> {
    let d1 = sub(a1, a0);
    let d2 = sub(b1, b0);
    let denom = d1[0] * d2[1] - d1[1] * d2[0];
    if denom.abs() < 1e-12 {
        return None;
    }
    let e = sub(b0, a0);
    let t = (e[0] * d2[1] - e[1] * d2[0]) / denom;
    let u = (e[0] * d1[1] - e[1] * d1[0]) / denom;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some((t, u))
    } else {
        None
    }
}

// --- Capsules ------------------------------------------------------------

/// A swept-disc segment of the eraser stroke. `a == b` is a plain disc.
#[derive(Clone, Copy)]
pub struct Capsule {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub r: f32,
}

impl Capsule {
    /// Signed clearance of a point: distance to the core segment minus
    /// the radius (inflated by `extra_r`). Negative = inside.
    fn clearance(&self, p: [f32; 2], extra_r: f32) -> f32 {
        seg_point_dist(p, self.a, self.b) - (self.r + extra_r)
    }

    /// Polygonize into a convex ring with positive shoelace area.
    fn polygonize(&self) -> Vec<[f32; 2]> {
        let r = self.r.max(0.5); // the canvas 0.5-cell guard, re-asserted
        // Semicircle segment count from a ~0.15-cell chord tolerance.
        let dev = (1.0f32 - (0.15 / r).min(0.9)).acos().max(0.05);
        let n = ((std::f32::consts::PI / (2.0 * dev)).ceil() as usize).clamp(6, 24);
        let d = sub(self.b, self.a);
        let ang0 = if len(d) > 1e-6 { d[1].atan2(d[0]) } else { 0.0 };
        let mut pts = Vec::with_capacity(2 * n + 2);
        for k in 0..=n {
            let t = ang0 - std::f32::consts::FRAC_PI_2
                + std::f32::consts::PI * k as f32 / n as f32;
            pts.push([self.b[0] + r * t.cos(), self.b[1] + r * t.sin()]);
        }
        for k in 0..=n {
            let t = ang0 + std::f32::consts::FRAC_PI_2
                + std::f32::consts::PI * k as f32 / n as f32;
            pts.push([self.a[0] + r * t.cos(), self.a[1] + r * t.sin()]);
        }
        dedupe_ring(&mut pts);
        if signed_area(&pts) < 0.0 {
            pts.reverse();
        }
        pts
    }
}

// --- Polygon helpers -------------------------------------------------------

/// Shoelace area; "CCW" below means positive.
pub fn signed_area(pts: &[[f32; 2]]) -> f32 {
    let n = pts.len();
    let mut a = 0.0;
    for i in 0..n {
        let p = pts[i];
        let q = pts[(i + 1) % n];
        a += p[0] * q[1] - q[0] * p[1];
    }
    a * 0.5
}

/// Even-odd point-in-polygon.
pub fn point_in_polygon(p: [f32; 2], pts: &[[f32; 2]]) -> bool {
    let n = pts.len();
    let mut inside = false;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        if (a[1] <= p[1]) != (b[1] <= p[1]) {
            let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// Point strictly inside a convex CCW (positive-area) ring.
fn point_in_convex(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        if (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) < 0.0 {
            return false;
        }
    }
    true
}

fn dedupe_ring(pts: &mut Vec<[f32; 2]>) {
    pts.dedup_by(|a, b| len(sub(*a, *b)) < WELD_EPS);
    while pts.len() > 1 && len(sub(pts[0], *pts.last().unwrap())) < WELD_EPS {
        pts.pop();
    }
}

fn path_len(pts: &[[f32; 2]]) -> f32 {
    pts.windows(2).map(|w| len(sub(w[1], w[0]))).sum()
}

// --- Centerline clipping (Line, open Poly, unfilled closed Poly) --------

pub enum ClipPath {
    /// The stroke never reached the path.
    Untouched,
    /// The whole path fell inside the stroke.
    Erased,
    /// Surviving open runs, in path order (guards already applied).
    Runs(Vec<Vec<[f32; 2]>>),
}

/// Clip a centerline path against the stroke: keep the parts with
/// positive clearance from every capsule. `extra_r` inflates each
/// capsule by the ink half-thickness so what disappears matches what
/// the user sees, not an invisible centerline. Cut points are found by
/// sampling + bisection — robust against capsule unions with no case
/// analysis.
pub fn clip_path(
    pts: &[[f32; 2]],
    closed: bool,
    caps: &[Capsule],
    extra_r: f32,
) -> ClipPath {
    let n = pts.len();
    if n < 2 || caps.is_empty() {
        return ClipPath::Untouched;
    }
    let clearance = |p: [f32; 2]| {
        caps.iter()
            .map(|c| c.clearance(p, extra_r))
            .fold(f32::MAX, f32::min)
    };
    let segs = if closed { n } else { n - 1 };
    let min_r = caps.iter().map(|c| c.r).fold(f32::MAX, f32::min) + extra_r;
    let step = (min_r * 0.5).max(0.25);

    let mut any_cut = false;
    let mut any_kept = false;
    let mut runs: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut cur: Option<Vec<[f32; 2]>> = None;
    let mut kept_from_start = false; // segment 0 kept from t=0 (ring merge)

    for k in 0..segs {
        let a = pts[k];
        let b = pts[(k + 1) % n];
        let sl = len(sub(b, a)).max(1e-6);
        let samples = ((sl / step).ceil() as usize).clamp(2, 512);
        let f = |t: f32| clearance(lerp(a, b, t));
        let mut kept: Vec<(f32, f32)> = Vec::new();
        let mut prev_v = f(0.0) >= 0.0;
        let mut start = if prev_v { Some(0.0) } else { None };
        for s in 1..=samples {
            let t = s as f32 / samples as f32;
            let v = f(t) >= 0.0;
            if v != prev_v {
                let (mut lo, mut hi) = ((s - 1) as f32 / samples as f32, t);
                for _ in 0..24 {
                    let mid = (lo + hi) * 0.5;
                    if (f(mid) >= 0.0) == prev_v {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                let tc = (lo + hi) * 0.5;
                if v {
                    start = Some(tc);
                } else if let Some(t0) = start.take() {
                    kept.push((t0, tc));
                }
                prev_v = v;
            }
        }
        if let Some(t0) = start {
            kept.push((t0, 1.0));
        }

        if kept.len() == 1 && kept[0].0 <= 0.0 && kept[0].1 >= 1.0 {
            // Whole segment kept.
            any_kept = true;
            if k == 0 {
                kept_from_start = true;
            }
            match &mut cur {
                Some(run) => run.push(b),
                None => cur = Some(vec![a, b]),
            }
            continue;
        }
        any_cut = true;
        if kept.is_empty() {
            if let Some(run) = cur.take() {
                runs.push(run);
            }
            continue;
        }
        for (t0, t1) in kept {
            any_kept = true;
            let p0 = lerp(a, b, t0);
            let p1 = lerp(a, b, t1);
            if t0 <= 1e-4 && cur.is_some() {
                cur.as_mut().unwrap().push(p1);
            } else {
                if let Some(run) = cur.take() {
                    runs.push(run);
                }
                if t0 <= 1e-4 && k == 0 {
                    kept_from_start = true;
                }
                cur = Some(vec![p0, p1]);
            }
            if t1 < 1.0 - 1e-4 {
                if let Some(run) = cur.take() {
                    runs.push(run);
                }
            }
        }
    }
    if let Some(run) = cur.take() {
        if closed && any_cut && kept_from_start && !runs.is_empty() {
            // The run wrapping past the ring seam continues into the
            // first run.
            let mut merged = run;
            merged.extend(runs.remove(0));
            runs.insert(0, merged);
        } else {
            runs.push(run);
        }
    }

    if !any_kept {
        return ClipPath::Erased;
    }
    if !any_cut {
        return ClipPath::Untouched;
    }
    // Guards: weld, then drop runs below the minimum length.
    let mut out = Vec::new();
    for mut run in runs {
        run.dedup_by(|a, b| len(sub(*a, *b)) < WELD_EPS);
        if run.len() >= 2 && path_len(&run) >= MIN_RUN_LEN {
            out.push(run);
        }
    }
    if out.is_empty() {
        return ClipPath::Erased;
    }
    ClipPath::Runs(out)
}

// --- Filled-polygon subtraction --------------------------------------------

pub enum PolySubtract {
    Untouched,
    Erased,
    /// The stroke stayed wholly interior: subtracting would need a hole
    /// `Shape::Poly` cannot represent. The caller refuses with a status
    /// message (see the design doc).
    WouldHole,
    /// Surviving closed pieces (guards applied), largest first.
    Pieces(Vec<Vec<[f32; 2]>>),
}

/// Subtract the stroke footprint from a filled simple polygon.
/// Capsules subtract one at a time (each polygonizes to a CONVEX ring),
/// ordered so the working boundary stays connected to the original one:
/// a stroke that starts interior but crosses the edge subtracts fine;
/// only a stroke whose whole footprint is interior refuses.
pub fn subtract_polygon(poly: &[[f32; 2]], caps: &[Capsule]) -> PolySubtract {
    if poly.len() < 3 || caps.is_empty() {
        return PolySubtract::Untouched;
    }
    let mut subject: Vec<[f32; 2]> = poly.to_vec();
    dedupe_ring(&mut subject);
    if subject.len() < 3 {
        return PolySubtract::Untouched;
    }
    if signed_area(&subject) < 0.0 {
        subject.reverse();
    }

    // Classify each capsule against the ORIGINAL boundary.
    #[derive(Clone, Copy, PartialEq)]
    enum Rel {
        Crossing,
        Inside,
        Outside,
    }
    let sn = subject.len();
    let rel: Vec<Rel> = caps
        .iter()
        .map(|c| {
            let crossing = (0..sn).any(|i| {
                seg_seg_dist(c.a, c.b, subject[i], subject[(i + 1) % sn]) <= c.r
            });
            if crossing {
                Rel::Crossing
            } else if point_in_polygon(c.a, &subject) {
                Rel::Inside
            } else {
                Rel::Outside
            }
        })
        .collect();
    if rel.iter().all(|r| *r == Rel::Outside) {
        return PolySubtract::Untouched;
    }

    // BFS over capsule overlap from the boundary-crossing capsules, so
    // each subtraction step keeps the working boundary connected. An
    // Inside capsule never reached this way is an island — a hole.
    let m = caps.len();
    let adjacent = |i: usize, j: usize| {
        seg_seg_dist(caps[i].a, caps[i].b, caps[j].a, caps[j].b)
            < caps[i].r + caps[j].r
    };
    let mut seen = vec![false; m];
    let mut order: Vec<usize> = Vec::with_capacity(m);
    let mut queue: std::collections::VecDeque<usize> = Default::default();
    for i in 0..m {
        if rel[i] == Rel::Crossing {
            seen[i] = true;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        order.push(i);
        for j in 0..m {
            if !seen[j] && rel[j] != Rel::Outside && adjacent(i, j) {
                seen[j] = true;
                queue.push_back(j);
            }
        }
    }
    if (0..m).any(|i| rel[i] == Rel::Inside && !seen[i]) {
        return PolySubtract::WouldHole;
    }
    if order.is_empty() {
        return PolySubtract::Untouched;
    }

    let mut pieces: Vec<Vec<[f32; 2]>> = vec![subject];
    for &ci in &order {
        // Deterministic sub-0.5% radius jitter, keyed by stroke index:
        // consecutive same-radius capsules on a near-straight drag carve
        // slot walls EXACTLY tangent to the next capsule's boundary, and
        // coincident boundaries corrupt the crossing walk. Distinct radii
        // keep every pair of walls strictly separated; the growth is far
        // below a cell, so the erase looks identical.
        let jitter = 1.0 + 0.004 * ((ci % 5) as f32 + 1.0) / 5.0;
        let clip = Capsule {
            a: caps[ci].a,
            b: caps[ci].b,
            r: caps[ci].r * jitter,
        }
        .polygonize();
        let mut next: Vec<Vec<[f32; 2]>> = Vec::new();
        for piece in pieces {
            match poly_minus_convex(&piece, &clip) {
                ConvexSub::Untouched => next.push(piece),
                ConvexSub::Erased => {}
                // A hole here means the BFS connectivity argument lost
                // to float noise; skipping the capsule for this piece
                // loses a sliver of erase but stays sound.
                ConvexSub::Hole => next.push(piece),
                ConvexSub::Pieces(ps) => next.extend(ps),
            }
        }
        pieces = next;
        if pieces.is_empty() {
            return PolySubtract::Erased;
        }
    }

    // Guards: weld and drop slivers.
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    for mut p in pieces {
        dedupe_ring(&mut p);
        if p.len() >= 3 && signed_area(&p).abs() >= MIN_AREA {
            out.push(p);
        }
    }
    if out.is_empty() {
        return PolySubtract::Erased;
    }
    out.sort_by(|a, b| {
        signed_area(b)
            .abs()
            .partial_cmp(&signed_area(a).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    PolySubtract::Pieces(out)
}

enum ConvexSub {
    Untouched,
    Erased,
    Hole,
    Pieces(Vec<Vec<[f32; 2]>>),
}

/// One crossing of the subject boundary with the clip boundary.
struct Crossing {
    pos: [f32; 2],
    s_edge: usize,
    s_t: f32,
    c_edge: usize,
    c_t: f32,
}

fn param_key(t: f32) -> u32 {
    (t.clamp(0.0, 1.0) * 1e7) as u32
}

/// Simple polygon minus a CONVEX ring (a Greiner–Hormann difference
/// walk): keep the subject arcs outside the clip, join them with the
/// clip arcs inside the subject, traversed backward. Both rings CCW
/// (positive shoelace).
fn poly_minus_convex(subject: &[[f32; 2]], clip: &[[f32; 2]]) -> ConvexSub {
    let sn = subject.len();
    let cn = clip.len();
    if sn < 3 || cn < 3 {
        return ConvexSub::Untouched;
    }

    let mut hits: Vec<Crossing> = Vec::new();
    for i in 0..sn {
        let (a0, a1) = (subject[i], subject[(i + 1) % sn]);
        for j in 0..cn {
            let (b0, b1) = (clip[j], clip[(j + 1) % cn]);
            if let Some((t, u)) = segs_intersect(a0, a1, b0, b1) {
                hits.push(Crossing {
                    pos: lerp(a0, a1, t),
                    s_edge: i,
                    s_t: t,
                    c_edge: j,
                    c_t: u,
                });
            }
        }
    }
    // A vertex graze shows up as two near-identical crossings on the
    // two adjacent edges; weld to one.
    hits.sort_by(|a, b| (a.s_edge, param_key(a.s_t)).cmp(&(b.s_edge, param_key(b.s_t))));
    hits.dedup_by(|a, b| len(sub(a.pos, b.pos)) < WELD_EPS);
    if hits.len() >= 2 {
        let first = hits.first().unwrap().pos;
        let last = hits.last().unwrap().pos;
        if len(sub(first, last)) < WELD_EPS {
            hits.pop();
        }
    }

    if hits.len() < 2 {
        // No transversal crossings: containment decides.
        if point_in_convex(subject[0], clip) {
            return ConvexSub::Erased;
        }
        if point_in_polygon(clip[0], subject) {
            return ConvexSub::Hole;
        }
        return ConvexSub::Untouched;
    }

    // Subject ring: for each crossing, its position along the ring; the
    // subject walk needs "next crossing after (edge, t)".
    let nh = hits.len();
    let mut s_order: Vec<usize> = (0..nh).collect();
    s_order.sort_by(|&a, &b| {
        (hits[a].s_edge, param_key(hits[a].s_t)).cmp(&(hits[b].s_edge, param_key(hits[b].s_t)))
    });
    let mut s_rank = vec![0usize; nh];
    for (k, &h) in s_order.iter().enumerate() {
        s_rank[h] = k;
    }
    let mut c_order: Vec<usize> = (0..nh).collect();
    c_order.sort_by(|&a, &b| {
        (hits[a].c_edge, param_key(hits[a].c_t)).cmp(&(hits[b].c_edge, param_key(hits[b].c_t)))
    });
    let mut c_rank = vec![0usize; nh];
    for (k, &h) in c_order.iter().enumerate() {
        c_rank[h] = k;
    }

    // A crossing is an EXIT when the subject is outside the clip just
    // after it (walking forward).
    let mut is_exit = vec![false; nh];
    for h in 0..nh {
        let next = s_order[(s_rank[h] + 1) % nh];
        // Midpoint along the subject between crossing h and the next
        // crossing — subject arcs between crossings are uniformly
        // inside or outside, so any interior point of the arc works.
        let mid = subject_arc_midpoint(subject, &hits[h], &hits[next]);
        is_exit[h] = !point_in_convex(mid, clip);
    }

    let mut visited = vec![false; nh];
    let mut pieces: Vec<Vec<[f32; 2]>> = Vec::new();
    for start in 0..nh {
        if !is_exit[start] || visited[start] {
            continue;
        }
        let mut piece: Vec<[f32; 2]> = Vec::new();
        let mut h = start;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 2 * (sn + cn + nh) {
                break; // malformed (self-intersecting subject): bail
            }
            visited[h] = true;
            piece.push(hits[h].pos);
            // Subject arc forward from h to the next crossing e,
            // collecting the subject vertices in between.
            let e = s_order[(s_rank[h] + 1) % nh];
            push_subject_arc(&mut piece, subject, &hits[h], &hits[e]);
            visited[e] = true;
            piece.push(hits[e].pos);
            // Clip arc BACKWARD from e to its clip-order predecessor
            // (the difference keeps clip arcs reversed).
            let x = c_order[(c_rank[e] + nh - 1) % nh];
            push_clip_arc_reversed(&mut piece, clip, &hits[e], &hits[x]);
            h = x;
            if h == start {
                break;
            }
        }
        dedupe_ring(&mut piece);
        if piece.len() >= 3 {
            pieces.push(piece);
        }
    }

    if pieces.is_empty() {
        return if point_in_convex(subject[0], clip) {
            ConvexSub::Erased
        } else {
            ConvexSub::Untouched
        };
    }
    ConvexSub::Pieces(pieces)
}

/// Subject vertices strictly between two crossings, walking forward.
fn push_subject_arc(
    piece: &mut Vec<[f32; 2]>,
    subject: &[[f32; 2]],
    from: &Crossing,
    to: &Crossing,
) {
    let sn = subject.len();
    if from.s_edge == to.s_edge && to.s_t > from.s_t {
        return; // same edge, no vertices between
    }
    // Vertices passed: end of from's edge, then each subsequent edge
    // start, up to (and including) the start of to's edge.
    let mut e = (from.s_edge + 1) % sn;
    let mut guard = 0;
    loop {
        piece.push(subject[e]);
        if e == to.s_edge {
            break;
        }
        e = (e + 1) % sn;
        guard += 1;
        if guard > sn + 1 {
            break;
        }
    }
}

/// A point on the subject boundary strictly between two crossings
/// (walking forward) — used only for the inside/outside test.
fn subject_arc_midpoint(
    subject: &[[f32; 2]],
    from: &Crossing,
    to: &Crossing,
) -> [f32; 2] {
    let sn = subject.len();
    if from.s_edge == to.s_edge && to.s_t > from.s_t {
        let a = subject[from.s_edge];
        let b = subject[(from.s_edge + 1) % sn];
        return lerp(a, b, (from.s_t + to.s_t) * 0.5);
    }
    // The arc passes the end vertex of from's edge; step just past the
    // crossing toward it.
    let a = subject[from.s_edge];
    let b = subject[(from.s_edge + 1) % sn];
    lerp(a, b, from.s_t + (1.0 - from.s_t) * 0.5)
}

/// Clip vertices between two crossings, walking BACKWARD along the
/// clip winding, excluding the crossing points themselves.
fn push_clip_arc_reversed(
    piece: &mut Vec<[f32; 2]>,
    clip: &[[f32; 2]],
    from: &Crossing,
    to: &Crossing,
) {
    let cn = clip.len();
    if from.c_edge == to.c_edge && to.c_t < from.c_t {
        return; // same edge, walking back, no vertices between
    }
    // Walking backward from (c_edge, c_t): the first vertex passed is
    // clip[c_edge] (the edge's start), then each previous edge's start,
    // stopping once we're on to's edge.
    let mut e = from.c_edge;
    let mut guard = 0;
    loop {
        piece.push(clip[e]);
        e = (e + cn - 1) % cn;
        if e == to.c_edge {
            break;
        }
        guard += 1;
        if guard > cn + 1 {
            break;
        }
    }
}

// --- Paint bucket: flood fill + contour tracing ---------------------------

pub enum Flood {
    /// The clicked cell is not fluid.
    NotFluid,
    /// The region reaches the domain edge; filling would flood the domain.
    OpenToEdge,
    /// The enclosed region as a mask over the w×h grid.
    Region(Vec<bool>),
}

/// Flood-fill 4-connected fluid from `start` over a w×h cell grid.
pub fn flood_region(cell: &[u32], w: usize, h: usize, start: (usize, usize)) -> Flood {
    let (sx, sy) = start;
    if sx >= w || sy >= h || cell[sy * w + sx] != CELL_FLUID {
        return Flood::NotFluid;
    }
    let mut mask = vec![false; w * h];
    let mut open = false;
    let mut stack = vec![(sx, sy)];
    mask[sy * w + sx] = true;
    while let Some((x, y)) = stack.pop() {
        if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
            open = true;
        }
        let mut push = |nx: usize, ny: usize, stack: &mut Vec<(usize, usize)>| {
            let i = ny * w + nx;
            if !mask[i] && cell[i] == CELL_FLUID {
                mask[i] = true;
                stack.push((nx, ny));
            }
        };
        if x > 0 {
            push(x - 1, y, &mut stack);
        }
        if x + 1 < w {
            push(x + 1, y, &mut stack);
        }
        if y > 0 {
            push(x, y - 1, &mut stack);
        }
        if y + 1 < h {
            push(x, y + 1, &mut stack);
        }
    }
    if open {
        return Flood::OpenToEdge;
    }
    Flood::Region(mask)
}

/// Trace the OUTER contour of a mask as an ordered loop of cell-corner
/// coordinates. Starts at the top-left-most masked cell (whose top-left
/// corner is provably on the outer boundary and is not a saddle) and
/// walks corner to corner keeping the region on the walk's inside;
/// saddle corners resolve by preferring the sharpest available turn, so
/// diagonally-touching cells never connect (matching the 4-connected
/// flood).
pub fn trace_mask(mask: &[bool], w: usize, h: usize) -> Vec<[f32; 2]> {
    let at = |x: i32, y: i32| -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < h
            && mask[y as usize * w + x as usize]
    };
    let Some(start_cell) = (0..w * h).find(|&i| mask[i]) else {
        return Vec::new();
    };
    let (sx, sy) = ((start_cell % w) as i32, (start_cell / w) as i32);
    let start = (sx, sy);
    let mut pos = start;
    let mut dir = (1i32, 0i32); // along the start cell's top edge
    let mut out: Vec<[f32; 2]> = Vec::new();
    let right_of = |d: (i32, i32)| (-d.1, d.0);
    let left_of = |d: (i32, i32)| (d.1, -d.0);
    // A lattice edge from a corner in direction d lies on the boundary
    // when the region is on one specific side (derived from the walk's
    // start convention: region below when heading +x).
    let boundary = |corner: (i32, i32), d: (i32, i32)| -> bool {
        let tl = at(corner.0 - 1, corner.1 - 1);
        let tr = at(corner.0, corner.1 - 1);
        let bl = at(corner.0 - 1, corner.1);
        let br = at(corner.0, corner.1);
        match d {
            (1, 0) => br && !tr,
            (-1, 0) => tl && !bl,
            (0, 1) => bl && !br,
            (0, -1) => tr && !tl,
            _ => false,
        }
    };
    let mut guard = 0usize;
    loop {
        guard += 1;
        if guard > 4 * (w + 2) * (h + 2) {
            break; // malformed mask; bail with what we have
        }
        out.push([pos.0 as f32, pos.1 as f32]);
        pos = (pos.0 + dir.0, pos.1 + dir.1);
        if pos == start {
            break;
        }
        let mut turned = false;
        for cand in [right_of(dir), dir, left_of(dir)] {
            if boundary(pos, cand) {
                dir = cand;
                turned = true;
                break;
            }
        }
        if !turned {
            dir = (-dir.0, -dir.1); // single-cell nub: reverse
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::CELL_WALL;

    fn cap(a: [f32; 2], b: [f32; 2], r: f32) -> Capsule {
        Capsule { a, b, r }
    }

    // --- clip_path -----------------------------------------------------

    #[test]
    fn open_polyline_split_by_disc() {
        let pts = vec![[0.0, 0.0], [40.0, 0.0]];
        match clip_path(&pts, false, &[cap([20.0, 0.0], [20.0, 0.0], 4.0)], 0.0) {
            ClipPath::Runs(runs) => {
                assert_eq!(runs.len(), 2);
                assert!((runs[0].last().unwrap()[0] - 16.0).abs() < 0.1);
                assert!((runs[1][0][0] - 24.0).abs() < 0.1);
            }
            _ => panic!("expected a split"),
        }
    }

    #[test]
    fn polyline_fully_inside_erases() {
        let pts = vec![[9.0, 10.0], [11.0, 10.0]];
        match clip_path(&pts, false, &[cap([10.0, 10.0], [10.0, 10.0], 5.0)], 0.0) {
            ClipPath::Erased => {}
            _ => panic!("expected erased"),
        }
    }

    #[test]
    fn polyline_missed_is_untouched() {
        let pts = vec![[0.0, 0.0], [40.0, 0.0]];
        match clip_path(&pts, false, &[cap([20.0, 30.0], [20.0, 30.0], 4.0)], 0.0) {
            ClipPath::Untouched => {}
            _ => panic!("expected untouched"),
        }
    }

    #[test]
    fn thickness_inflation_reaches_farther() {
        // Disc r=3 centered 4 cells off the centerline: misses a thin
        // stroke, cuts a thick one (half-thickness 2 → reach 5).
        let pts = vec![[0.0, 0.0], [40.0, 0.0]];
        let caps = [cap([20.0, 4.0], [20.0, 4.0], 3.0)];
        assert!(matches!(
            clip_path(&pts, false, &caps, 0.0),
            ClipPath::Untouched
        ));
        assert!(matches!(clip_path(&pts, false, &caps, 2.0), ClipPath::Runs(_)));
    }

    #[test]
    fn closed_ring_cut_once_opens_into_one_run() {
        let pts = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
        match clip_path(&pts, true, &[cap([0.0, 10.0], [0.0, 10.0], 3.0)], 0.0) {
            ClipPath::Runs(runs) => {
                assert_eq!(runs.len(), 1);
                assert!(path_len(&runs[0]) > 60.0);
            }
            _ => panic!("expected one open run"),
        }
    }

    #[test]
    fn closed_ring_cut_twice_gives_two_runs() {
        let pts = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]];
        let caps = [
            cap([0.0, 10.0], [0.0, 10.0], 3.0),
            cap([20.0, 10.0], [20.0, 10.0], 3.0),
        ];
        match clip_path(&pts, true, &caps, 0.0) {
            ClipPath::Runs(runs) => assert_eq!(runs.len(), 2),
            _ => panic!("expected two runs"),
        }
    }

    #[test]
    fn tiny_fragment_dropped_by_guard() {
        // Two discs whose coverage leaves a 0.2-cell shard between them
        // and healthy stubs at the ends: the shard must die, the stubs
        // survive.
        let pts = vec![[0.0, 0.0], [40.0, 0.0]];
        let caps = [
            cap([10.0, 0.0], [10.0, 0.0], 8.0),   // covers [2, 18]
            cap([25.0, 0.0], [25.0, 0.0], 6.8),   // covers [18.2, 31.8]
        ];
        match clip_path(&pts, false, &caps, 0.0) {
            ClipPath::Runs(runs) => {
                assert_eq!(runs.len(), 2, "the 0.2-cell shard must be dropped");
                assert!(path_len(&runs[0]) >= MIN_RUN_LEN);
                assert!(path_len(&runs[1]) >= MIN_RUN_LEN);
            }
            _ => panic!("expected runs"),
        }
    }

    // --- subtract_polygon ------------------------------------------------

    fn square(s: f32) -> Vec<[f32; 2]> {
        vec![[0.0, 0.0], [s, 0.0], [s, s], [0.0, s]]
    }

    #[test]
    fn bite_from_edge_keeps_one_piece() {
        let p = square(20.0);
        let caps = [cap([10.0, 0.0], [10.0, 0.0], 4.0)];
        match subtract_polygon(&p, &caps) {
            PolySubtract::Pieces(ps) => {
                assert_eq!(ps.len(), 1);
                let a = signed_area(&ps[0]).abs();
                // 400 minus roughly half the disc (~25).
                assert!(a < 395.0 && a > 360.0, "area {a}");
            }
            _ => panic!("expected one bitten piece"),
        }
    }

    #[test]
    fn stroke_through_middle_splits_in_two() {
        let p = square(20.0);
        let caps = [cap([10.0, -5.0], [10.0, 25.0], 2.0)];
        match subtract_polygon(&p, &caps) {
            PolySubtract::Pieces(ps) => {
                assert_eq!(ps.len(), 2);
                let total: f32 = ps.iter().map(|p| signed_area(p).abs()).sum();
                // 400 minus a 4-wide band (~80).
                assert!(total < 340.0 && total > 300.0, "total {total}");
            }
            _ => panic!("expected a split"),
        }
    }

    #[test]
    fn interior_stroke_refuses_with_hole() {
        let p = square(20.0);
        let caps = [cap([10.0, 10.0], [10.0, 10.0], 3.0)];
        assert!(matches!(subtract_polygon(&p, &caps), PolySubtract::WouldHole));
    }

    #[test]
    fn interior_start_crossing_edge_subtracts() {
        // Stroke starts dead centre and drags out past the right edge:
        // the union reaches the boundary, so it must NOT refuse.
        let p = square(20.0);
        let mut caps = Vec::new();
        let mut x = 10.0;
        while x < 26.0 {
            caps.push(cap([x, 10.0], [x + 1.5, 10.0], 2.0));
            x += 1.5;
        }
        match subtract_polygon(&p, &caps) {
            PolySubtract::Pieces(ps) => {
                let total: f32 = ps.iter().map(|p| signed_area(p).abs()).sum();
                assert!(total < 370.0, "carved area missing: {total}");
            }
            PolySubtract::WouldHole => panic!("BFS ordering failed: refused"),
            _ => panic!("expected pieces"),
        }
    }

    #[test]
    fn covering_stroke_erases() {
        let p = square(10.0);
        let caps = [cap([5.0, 5.0], [5.0, 5.0], 12.0)];
        assert!(matches!(subtract_polygon(&p, &caps), PolySubtract::Erased));
    }

    #[test]
    fn miss_is_untouched() {
        let p = square(10.0);
        let caps = [cap([30.0, 30.0], [30.0, 30.0], 3.0)];
        assert!(matches!(subtract_polygon(&p, &caps), PolySubtract::Untouched));
    }

    #[test]
    fn graze_leaves_no_sliver_pieces() {
        // A disc that barely nicks a corner: either untouched or one
        // piece near the original area — never micro-fragments.
        let p = square(20.0);
        let caps = [cap([20.2, 20.2], [20.2, 20.2], 0.6)];
        match subtract_polygon(&p, &caps) {
            PolySubtract::Untouched => {}
            PolySubtract::Pieces(ps) => {
                assert_eq!(ps.len(), 1);
                assert!(signed_area(&ps[0]).abs() > 395.0);
            }
            _ => panic!("graze must not erase or refuse"),
        }
    }

    // --- flood + trace ----------------------------------------------------

    #[test]
    fn flood_open_region_refuses() {
        let (w, h) = (8, 8);
        let cell = vec![CELL_FLUID; w * h];
        assert!(matches!(flood_region(&cell, w, h, (4, 4)), Flood::OpenToEdge));
    }

    #[test]
    fn flood_on_wall_is_not_fluid() {
        let (w, h) = (4, 4);
        let mut cell = vec![CELL_FLUID; w * h];
        cell[w + 1] = CELL_WALL;
        assert!(matches!(flood_region(&cell, w, h, (1, 1)), Flood::NotFluid));
    }

    #[test]
    fn flood_enclosed_region_and_trace() {
        // Wall box ring at x,y ∈ {2,7}: the enclosed interior is the
        // 4×4 block 3..=6.
        let (w, h) = (10, 10);
        let mut cell = vec![CELL_FLUID; w * h];
        for i in 2..=7usize {
            for &(x, y) in &[(i, 2), (i, 7), (2, i), (7, i)] {
                cell[y * w + x] = CELL_WALL;
            }
        }
        match flood_region(&cell, w, h, (4, 4)) {
            Flood::Region(mask) => {
                assert_eq!(mask.iter().filter(|&&m| m).count(), 16);
                let contour = trace_mask(&mask, w, h);
                assert!(contour.len() >= 4);
                let a = signed_area(&contour).abs();
                assert!((a - 16.0).abs() < 0.5, "traced area {a}");
                // Every corner lies on the [3,7]×[3,7] boundary.
                for p in &contour {
                    assert!(p[0] >= 3.0 && p[0] <= 7.0 && p[1] >= 3.0 && p[1] <= 7.0);
                }
            }
            _ => panic!("expected enclosed region"),
        }
    }

    #[test]
    fn trace_l_shape_has_six_corners_after_weld() {
        // L-shaped mask: cells (0,0),(0,1),(1,1).
        let (w, h) = (3, 3);
        let mut mask = vec![false; w * h];
        mask[0] = true; // (0,0)
        mask[w] = true; // (0,1)
        mask[w + 1] = true; // (1,1)
        let contour = trace_mask(&mask, w, h);
        // The raw trace visits every lattice corner on the boundary;
        // count the DISTINCT direction changes instead.
        let mut corners = 0;
        let n = contour.len();
        for i in 0..n {
            let a = contour[i];
            let b = contour[(i + 1) % n];
            let c = contour[(i + 2) % n];
            let d1 = [b[0] - a[0], b[1] - a[1]];
            let d2 = [c[0] - b[0], c[1] - b[1]];
            if d1[0] * d2[1] - d1[1] * d2[0] != 0.0 {
                corners += 1;
            }
        }
        assert_eq!(corners, 6);
        assert!((signed_area(&contour).abs() - 3.0).abs() < 1e-3);
    }

    #[test]
    fn trace_ignores_diagonal_pinch() {
        // Two cells touching only diagonally: the 4-connected flood
        // never produces this in one region, but the tracer must not
        // cross the pinch if handed one — it stays on the start cell's
        // component.
        let (w, h) = (2, 2);
        let mut mask = vec![false; w * h];
        mask[0] = true; // (0,0)
        mask[3] = true; // (1,1)
        let contour = trace_mask(&mask, w, h);
        assert!((signed_area(&contour).abs() - 1.0).abs() < 1e-3);
    }
}

