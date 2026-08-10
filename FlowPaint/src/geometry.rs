//! The CPU-side solver-grid layers: cell types, fan physics and dye
//! sources. The sketch model (model.rs) projects into these arrays; the
//! GPU copies are updated from the dirty region each frame. This module
//! holds only the data structures — all painting logic lives in the
//! model's rasterizer.

use serde::{Deserialize, Serialize};

pub const CELL_FLUID: u32 = 0;
pub const CELL_WALL: u32 = 1;
pub const CELL_INLET: u32 = 2;
pub const CELL_OUTLET: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GridRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32, // exclusive
    pub y1: i32, // exclusive
}

impl GridRect {
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
    pub fn intersect(&self, o: GridRect) -> GridRect {
        GridRect {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
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

/// A dense rectangular snapshot of the three geometry layers — the
/// payload of generator stamps.
///
/// Fan cells pack their physics into 4 components:
/// `[dir.x * speed_mult, dir.y * speed_mult, gustiness, gust_phase]` —
/// the vector's magnitude is the per-fan speed multiplier on the global
/// flow speed, gustiness in 0..1 adds time-varying direction/strength
/// wander, and the phase decorrelates different fans' gusts.
#[derive(Clone, Serialize, Deserialize)]
pub struct GeoRegion {
    pub rect: (i32, i32, i32, i32),
    pub cell: Vec<u32>,
    pub fan: Vec<[f32; 4]>,
    pub dye_src: Vec<[f32; 4]>,
}

pub struct Geometry {
    pub w: usize,
    pub h: usize,
    pub cell: Vec<u32>,
    pub fan: Vec<[f32; 4]>,
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
            fan: vec![[0.0; 4]; n],
            dye_src: vec![[0.0; 4]; n],
            dirty: Some(GridRect::full(w, h)),
        }
    }

    /// Dirty-marking for callers that write the layer vectors directly
    /// (the model's rasterizer).
    pub fn touch(&mut self, r: GridRect) {
        let r = r.clampped(self.w, self.h);
        if r.is_empty() {
            return;
        }
        self.dirty = Some(match self.dirty {
            Some(d) => d.union(r),
            None => r,
        });
    }
}
