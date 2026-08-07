//! The canonical, CPU-side scene: cell types, fan directions and dye
//! sources. All painting mutates these arrays; the GPU copies are updated
//! from the dirty region each frame. Undo/redo snapshots live here too.

use serde::{Deserialize, Serialize};

pub const CELL_FLUID: u32 = 0;
pub const CELL_WALL: u32 = 1;
pub const CELL_INLET: u32 = 2;
pub const CELL_OUTLET: u32 = 3;

/// What the current tool paints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Material {
    Wall,
    Fan,
    Smoke,
    Drain,
    Erase,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GridRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32, // exclusive
    pub y1: i32, // exclusive
}

impl GridRect {
    pub fn empty() -> Self {
        Self { x0: i32::MAX, y0: i32::MAX, x1: i32::MIN, y1: i32::MIN }
    }
    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
    pub fn union(&self, o: GridRect) -> GridRect {
        // A clamped rect can be "empty" with non-sentinel coordinates
        // (e.g. a stamp entirely off one edge); unioning those raw
        // coordinates would inflate the result, so normalize first.
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return *self;
        }
        GridRect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
    pub fn clampped(&self, w: usize, h: usize) -> GridRect {
        GridRect {
            x0: self.x0.max(0),
            y0: self.y0.max(0),
            x1: self.x1.min(w as i32),
            y1: self.y1.min(h as i32),
        }
    }
    pub fn full(w: usize, h: usize) -> GridRect {
        GridRect { x0: 0, y0: 0, x1: w as i32, y1: h as i32 }
    }
}

/// A dense rectangular snapshot of the three geometry layers.
#[derive(Clone, Serialize, Deserialize)]
pub struct GeoRegion {
    pub rect: (i32, i32, i32, i32),
    pub cell: Vec<u32>,
    pub fan: Vec<[f32; 2]>,
    pub dye_src: Vec<[f32; 4]>,
}

impl GeoRegion {
    pub fn byte_size(&self) -> usize {
        self.cell.len() * 4 + self.fan.len() * 8 + self.dye_src.len() * 16 + 64
    }
}

pub struct UndoEntry {
    pub before: GeoRegion,
    pub after: GeoRegion,
}

#[derive(Default)]
pub struct UndoStack {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
}

const UNDO_BYTE_BUDGET: usize = 512 << 20;

impl UndoStack {
    pub fn push(&mut self, entry: UndoEntry) {
        self.redo.clear();
        self.undo.push(entry);
        let mut total: usize = self
            .undo
            .iter()
            .map(|e| e.before.byte_size() + e.after.byte_size())
            .sum();
        while total > UNDO_BYTE_BUDGET && self.undo.len() > 1 {
            let e = self.undo.remove(0);
            total -= e.before.byte_size() + e.after.byte_size();
        }
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// Pops the top undo entry; the caller applies `before` and must then
    /// push the entry to redo via the returned value.
    pub fn pop_undo(&mut self) -> Option<UndoEntry> {
        self.undo.pop()
    }
    pub fn pop_redo(&mut self) -> Option<UndoEntry> {
        self.redo.pop()
    }
    pub fn push_redo(&mut self, e: UndoEntry) {
        self.redo.push(e);
    }
    pub fn push_undo_back(&mut self, e: UndoEntry) {
        self.undo.push(e);
    }
}

/// Extra parameters a stamp needs: fan direction and smoke color.
#[derive(Clone, Copy)]
pub struct BrushContext {
    pub fan_dir: [f32; 2],
    pub dye_rgb: [f32; 3],
}

pub struct Geometry {
    pub w: usize,
    pub h: usize,
    pub cell: Vec<u32>,
    pub fan: Vec<[f32; 2]>,
    pub dye_src: Vec<[f32; 4]>,
    /// Region that needs re-uploading to the GPU (None = clean).
    pub dirty: Option<GridRect>,
}

impl Geometry {
    pub fn new(w: usize, h: usize) -> Self {
        let n = w * h;
        Self {
            w,
            h,
            cell: vec![CELL_FLUID; n],
            fan: vec![[0.0; 2]; n],
            dye_src: vec![[0.0; 4]; n],
            dirty: Some(GridRect::full(w, h)),
        }
    }

    pub fn n(&self) -> usize {
        self.w * self.h
    }

    /// Public dirty-marking for callers that write the layer vectors
    /// directly (e.g. the selection tool).
    pub fn touch(&mut self, r: GridRect) {
        self.mark_dirty(r);
    }

    fn mark_dirty(&mut self, r: GridRect) {
        let r = r.clampped(self.w, self.h);
        if r.is_empty() {
            return;
        }
        self.dirty = Some(match self.dirty {
            Some(d) => d.union(r),
            None => r,
        });
    }

    fn set_cell(&mut self, i: usize, material: Material, ctx: &BrushContext) {
        match material {
            Material::Wall => {
                self.cell[i] = CELL_WALL;
                self.fan[i] = [0.0; 2];
                self.dye_src[i] = [0.0; 4];
            }
            Material::Fan => {
                self.cell[i] = CELL_INLET;
                self.fan[i] = ctx.fan_dir;
                self.dye_src[i] =
                    [ctx.dye_rgb[0], ctx.dye_rgb[1], ctx.dye_rgb[2], 0.8];
            }
            Material::Drain => {
                self.cell[i] = CELL_OUTLET;
                self.fan[i] = [0.0; 2];
                self.dye_src[i] = [0.0; 4];
            }
            Material::Smoke => {
                if self.cell[i] == CELL_FLUID {
                    self.dye_src[i] =
                        [ctx.dye_rgb[0], ctx.dye_rgb[1], ctx.dye_rgb[2], 1.0];
                }
            }
            Material::Erase => {
                self.cell[i] = CELL_FLUID;
                self.fan[i] = [0.0; 2];
                self.dye_src[i] = [0.0; 4];
            }
        }
    }

    /// Bounding rect of a capsule stamp, before clamping.
    pub fn capsule_bounds(a: [f32; 2], b: [f32; 2], r: f32) -> GridRect {
        GridRect {
            x0: (a[0].min(b[0]) - r).floor() as i32,
            y0: (a[1].min(b[1]) - r).floor() as i32,
            x1: (a[0].max(b[0]) + r).ceil() as i32 + 1,
            y1: (a[1].max(b[1]) + r).ceil() as i32 + 1,
        }
    }

    /// Stamp a filled capsule from a to b with the given radius (all in
    /// grid cells). Returns the affected rect.
    pub fn stamp_capsule(
        &mut self,
        a: [f32; 2],
        b: [f32; 2],
        r: f32,
        material: Material,
        ctx: &BrushContext,
    ) -> GridRect {
        let rect = Self::capsule_bounds(a, b, r).clampped(self.w, self.h);
        if rect.is_empty() {
            return rect;
        }

        let ab = [b[0] - a[0], b[1] - a[1]];
        let ab_len2 = ab[0] * ab[0] + ab[1] * ab[1];
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let p = [x as f32 + 0.5, y as f32 + 0.5];
                let ap = [p[0] - a[0], p[1] - a[1]];
                let t = if ab_len2 > 1e-6 {
                    ((ap[0] * ab[0] + ap[1] * ab[1]) / ab_len2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let dx = ap[0] - t * ab[0];
                let dy = ap[1] - t * ab[1];
                if dx * dx + dy * dy <= r * r {
                    self.set_cell((y as usize) * self.w + x as usize, material, ctx);
                }
            }
        }
        self.mark_dirty(rect);
        rect
    }

    /// Stamp a filled axis-aligned rectangle between two corners.
    pub fn stamp_rect(
        &mut self,
        a: [f32; 2],
        b: [f32; 2],
        material: Material,
        ctx: &BrushContext,
    ) -> GridRect {
        let rect = GridRect {
            x0: a[0].min(b[0]).floor() as i32,
            y0: a[1].min(b[1]).floor() as i32,
            x1: a[0].max(b[0]).ceil() as i32,
            y1: a[1].max(b[1]).ceil() as i32,
        }
        .clampped(self.w, self.h);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                self.set_cell((y as usize) * self.w + x as usize, material, ctx);
            }
        }
        self.mark_dirty(rect);
        rect
    }

    /// Stamp a filled ellipse inscribed in the rectangle spanned by a, b.
    pub fn stamp_ellipse(
        &mut self,
        a: [f32; 2],
        b: [f32; 2],
        material: Material,
        ctx: &BrushContext,
    ) -> GridRect {
        let rect = GridRect {
            x0: a[0].min(b[0]).floor() as i32,
            y0: a[1].min(b[1]).floor() as i32,
            x1: a[0].max(b[0]).ceil() as i32,
            y1: a[1].max(b[1]).ceil() as i32,
        }
        .clampped(self.w, self.h);
        let cx = (a[0] + b[0]) * 0.5;
        let cy = (a[1] + b[1]) * 0.5;
        let rx = ((a[0] - b[0]).abs() * 0.5).max(0.5);
        let ry = ((a[1] - b[1]).abs() * 0.5).max(0.5);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let nx = (x as f32 + 0.5 - cx) / rx;
                let ny = (y as f32 + 0.5 - cy) / ry;
                if nx * nx + ny * ny <= 1.0 {
                    self.set_cell((y as usize) * self.w + x as usize, material, ctx);
                }
            }
        }
        self.mark_dirty(rect);
        rect
    }

    /// Extract a dense snapshot of the given rect.
    pub fn extract(&self, rect: GridRect) -> GeoRegion {
        let rect = rect.clampped(self.w, self.h);
        let rw = (rect.x1 - rect.x0).max(0) as usize;
        let rh = (rect.y1 - rect.y0).max(0) as usize;
        let mut cell = Vec::with_capacity(rw * rh);
        let mut fan = Vec::with_capacity(rw * rh);
        let mut dye_src = Vec::with_capacity(rw * rh);
        for y in rect.y0..rect.y1 {
            let row = (y as usize) * self.w;
            for x in rect.x0..rect.x1 {
                let i = row + x as usize;
                cell.push(self.cell[i]);
                fan.push(self.fan[i]);
                dye_src.push(self.dye_src[i]);
            }
        }
        GeoRegion { rect: (rect.x0, rect.y0, rect.x1, rect.y1), cell, fan, dye_src }
    }

    /// Write a dense snapshot back (rect must match the region's).
    pub fn restore(&mut self, region: &GeoRegion) {
        let (x0, y0, x1, y1) = region.rect;
        let rect = GridRect { x0, y0, x1, y1 }.clampped(self.w, self.h);
        let rw = (x1 - x0).max(0) as usize;
        let mut k = 0;
        for y in rect.y0..rect.y1 {
            let row = (y as usize) * self.w;
            let src_row = ((y - y0) as usize) * rw;
            for x in rect.x0..rect.x1 {
                let i = row + x as usize;
                let s = src_row + (x - x0) as usize;
                self.cell[i] = region.cell[s];
                self.fan[i] = region.fan[s];
                self.dye_src[i] = region.dye_src[s];
                k += 1;
            }
        }
        let _ = k;
        self.mark_dirty(rect);
    }

    /// The wind tunnel runs left to right: two inlet columns on the left,
    /// two outlet columns on the right, and dye streaklines seeded at the
    /// inlet every few rows.
    pub fn apply_wind_tunnel(&mut self, enable: bool) {
        let streak = [0.92, 0.94, 1.0, 0.9];
        for y in 0..self.h {
            let seed = (y % 12) < 2;
            for r in 0..2usize {
                // Inlet on the left edge blowing right.
                let i = y * self.w + r;
                if enable {
                    self.cell[i] = CELL_INLET;
                    self.fan[i] = [1.0, 0.0];
                    self.dye_src[i] = if seed { streak } else { [0.0; 4] };
                } else {
                    self.cell[i] = CELL_FLUID;
                    self.fan[i] = [0.0; 2];
                    self.dye_src[i] = [0.0; 4];
                }
                // Outlet on the right edge.
                let o = y * self.w + (self.w - 1 - r);
                if enable {
                    self.cell[o] = CELL_OUTLET;
                } else {
                    self.cell[o] = CELL_FLUID;
                }
                self.fan[o] = [0.0; 2];
                self.dye_src[o] = [0.0; 4];
            }
        }
        self.mark_dirty(GridRect::full(self.w, self.h));
    }

    /// Remove all painted geometry, fans, outlets and dye sources.
    pub fn clear(&mut self) {
        for i in 0..self.n() {
            self.cell[i] = CELL_FLUID;
            self.fan[i] = [0.0; 2];
            self.dye_src[i] = [0.0; 4];
        }
        self.mark_dirty(GridRect::full(self.w, self.h));
    }

    /// Nearest-neighbour resample of another geometry into this one
    /// (used when the grid resolution changes).
    pub fn resample_from(&mut self, old: &Geometry) {
        if old.w == 0 || old.h == 0 {
            return;
        }
        for y in 0..self.h {
            let oy = (y * old.h / self.h).min(old.h - 1);
            for x in 0..self.w {
                let ox = (x * old.w / self.w).min(old.w - 1);
                let oi = oy * old.w + ox;
                let ni = y * self.w + x;
                self.cell[ni] = old.cell[oi];
                self.fan[ni] = old.fan[oi];
                self.dye_src[ni] = old.dye_src[oi];
            }
        }
        self.mark_dirty(GridRect::full(self.w, self.h));
    }
}

// --- Presets ---------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    Cylinder,
    Airfoil,
    Venturi,
    Step,
    Pinball,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::Cylinder,
        Preset::Airfoil,
        Preset::Venturi,
        Preset::Step,
        Preset::Pinball,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Preset::Cylinder => "Cylinder (vortex street)",
            Preset::Airfoil => "Airfoil (NACA 0012)",
            Preset::Venturi => "Venturi nozzle",
            Preset::Step => "Backward-facing step",
            Preset::Pinball => "Pinball cylinders",
        }
    }
}

impl Geometry {
    /// Stamp a preset's walls. Assumes the flow axis is horizontal
    /// (left to right); only overwrites fluid cells, so tunnel edges
    /// survive. Shape proportions are relative to the VISIBLE window
    /// `vis`, but stamping iterates the full grid so channel-style
    /// presets (venturi, step) extend their walls through the margin
    /// instead of letting flow sneak around them.
    pub fn stamp_preset(&mut self, preset: Preset, vis: GridRect) {
        let w = (vis.x1 - vis.x0) as f32;
        let h = (vis.y1 - vis.y0) as f32;

        let circle = |cx: f32, cy: f32, r: f32, x: f32, y: f32| -> bool {
            let dx = x - cx * w;
            let dy = y - cy * h;
            dx * dx + dy * dy <= r * r
        };

        let is_wall: Box<dyn Fn(f32, f32) -> bool> = match preset {
            Preset::Cylinder => {
                let r = 0.08 * h;
                Box::new(move |x, y| circle(0.30, 0.5, r, x, y))
            }
            Preset::Airfoil => {
                // NACA 0012 at ~10 degrees angle of attack.
                let chord = 0.5 * w;
                let alpha = 10.0f32.to_radians();
                let lx = 0.22 * w;
                let ly = 0.48 * h;
                Box::new(move |x, y| {
                    let dx = x - lx;
                    let dy = y - ly;
                    let xc = dx * alpha.cos() + dy * alpha.sin();
                    let yc = -dx * alpha.sin() + dy * alpha.cos();
                    if xc < 0.0 || xc > chord {
                        return false;
                    }
                    let xn = xc / chord;
                    let yt = 0.6
                        * chord
                        * (0.2969 * xn.sqrt() - 0.1260 * xn - 0.3516 * xn * xn
                            + 0.2843 * xn * xn * xn
                            - 0.1015 * xn * xn * xn * xn);
                    yc.abs() <= yt
                })
            }
            Preset::Venturi => Box::new(move |x, y| {
                let s = (x / w - 0.45) / 0.16;
                let gap = 1.0 - 0.62 * (-s * s).exp();
                let half = 0.5 * gap * h;
                let mid = 0.5 * h;
                (y - mid).abs() > half
            }),
            Preset::Step => Box::new(move |x, y| x < 0.32 * w && y < 0.5 * h),
            Preset::Pinball => {
                let r = 0.055 * h;
                let centers: [(f32, f32); 5] = [
                    (0.28, 0.30),
                    (0.28, 0.70),
                    (0.48, 0.50),
                    (0.68, 0.30),
                    (0.68, 0.70),
                ];
                Box::new(move |x, y| {
                    centers.iter().any(|&(cx, cy)| circle(cx, cy, r, x, y))
                })
            }
        };

        for y in 0..self.h {
            for x in 0..self.w {
                let sx = x as f32 + 0.5 - vis.x0 as f32;
                let sy = y as f32 + 0.5 - vis.y0 as f32;
                if is_wall(sx, sy) {
                    let i = y * self.w + x;
                    if self.cell[i] == CELL_FLUID {
                        self.cell[i] = CELL_WALL;
                        self.dye_src[i] = [0.0; 4];
                    }
                }
            }
        }
        self.mark_dirty(GridRect::full(self.w, self.h));
    }
}

// --- Scene files -----------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct SceneFile {
    pub version: u32,
    pub w: u32,
    pub h: u32,
    pub cell: Vec<u32>,
    pub fan: Vec<[f32; 2]>,
    pub dye_src: Vec<[f32; 4]>,
    pub wind_tunnel: bool,
    pub flow_speed: f32,
    pub viscosity: f32,
}

pub const SCENE_VERSION: u32 = 1;
