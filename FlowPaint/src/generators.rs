//! Parametric geometry generators: NACA 4-digit airfoils and de Laval
//! (converging-diverging) rocket nozzles. Each generator rasterizes into a
//! `GeoRegion` stamp (rect based at 0,0) which the app inserts as a
//! floating selection, so the user can position/rotate it before
//! committing.

use crate::geometry::{GeoRegion, CELL_FLUID, CELL_INLET, CELL_WALL};

// --- Airfoils --------------------------------------------------------

#[derive(Clone, Copy)]
pub struct AirfoilParams {
    /// Maximum camber, percent of chord (NACA first digit).
    pub camber: f32,
    /// Position of maximum camber, percent of chord (NACA second digit x10).
    pub camber_pos: f32,
    /// Maximum thickness, percent of chord (NACA last two digits).
    pub thickness: f32,
    /// Angle of attack, degrees (positive = nose up relative to the flow).
    pub aoa_deg: f32,
    /// Chord length in cells.
    pub chord_cells: f32,
}

impl Default for AirfoilParams {
    fn default() -> Self {
        Self {
            camber: 2.0,
            camber_pos: 40.0,
            thickness: 12.0,
            aoa_deg: 5.0,
            chord_cells: 300.0,
        }
    }
}

/// (name, camber %, camber pos %, thickness %, typical AoA)
pub const AIRFOIL_PRESETS: [(&str, f32, f32, f32, f32); 7] = [
    ("NACA 0012 — the symmetric classic", 0.0, 0.0, 12.0, 5.0),
    ("NACA 2412 — Cessna 172", 2.0, 40.0, 12.0, 5.0),
    ("NACA 4412 — classic high lift", 4.0, 40.0, 12.0, 6.0),
    ("NACA 0015 — aerobatic", 0.0, 0.0, 15.0, 8.0),
    ("NACA 6412 — high camber", 6.0, 40.0, 12.0, 4.0),
    ("Clark Y (approx.) — Spirit of St. Louis", 3.6, 42.0, 11.7, 4.0),
    ("NACA 0006 — thin and slippery", 0.0, 0.0, 6.0, 3.0),
];

/// NACA 4-digit mean camber line and its slope at chordwise position
/// x in [0, 1].
fn naca_camber(m: f32, p: f32, x: f32) -> (f32, f32) {
    if m <= 0.0 || p <= 0.0 {
        return (0.0, 0.0);
    }
    if x < p {
        (m / (p * p) * (2.0 * p * x - x * x), 2.0 * m / (p * p) * (p - x))
    } else {
        let q = 1.0 - p;
        (
            m / (q * q) * ((1.0 - 2.0 * p) + 2.0 * p * x - x * x),
            2.0 * m / (q * q) * (p - x),
        )
    }
}

/// Half-thickness distribution at chordwise position x in [0, 1]
/// (closed trailing edge variant).
fn naca_thickness(t: f32, x: f32) -> f32 {
    5.0 * t
        * (0.2969 * x.sqrt() - 0.1260 * x - 0.3516 * x * x + 0.2843 * x * x * x
            - 0.1036 * x * x * x * x)
}

/// Rasterize an airfoil into a stamp. The airfoil is rotated by -AoA
/// (nose up for flow moving in +x) about its quarter-chord point.
pub fn generate_airfoil(p: &AirfoilParams) -> GeoRegion {
    let m = p.camber / 100.0;
    let cp = (p.camber_pos / 100.0).clamp(0.05, 0.95);
    let t = p.thickness / 100.0;
    let chord = p.chord_cells.max(16.0);
    let alpha = p.aoa_deg.to_radians();

    // Stamp bounds: chord plus generous room for camber, thickness and
    // rotation.
    let half_h = chord * (t + m + alpha.abs().sin() + 0.05);
    let w = (chord * 1.1).ceil() as usize + 4;
    let h = (2.0 * half_h).ceil() as usize + 4;
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
    // Rotation pivot: quarter chord.
    let pivot = 0.25;

    let mut cell = vec![CELL_FLUID; w * h];
    let fan = vec![[0.0f32; 2]; w * h];
    let dye_src = vec![[0.0f32; 4]; w * h];

    let (sin_a, cos_a) = alpha.sin_cos();
    for y in 0..h {
        for x in 0..w {
            // Cell centre relative to the quarter-chord pivot, which sits
            // a quarter chord left of the stamp centre so the chord ends
            // up centred at cx.
            let dx = x as f32 + 0.5 - (cx - (0.5 - pivot) * chord);
            let dy = y as f32 + 0.5 - cy;
            // Sample through R(-alpha): positive AoA pitches the nose up
            // (trailing edge down) for flow moving in +x on a y-down grid,
            // matching the Selection tool's rotation convention.
            let xr = dx * cos_a + dy * sin_a + pivot * chord;
            let yr = -dx * sin_a + dy * cos_a;
            let xn = xr / chord;
            if !(0.0..=1.0).contains(&xn) {
                continue;
            }
            let (yc, _slope) = naca_camber(m, cp, xn);
            let yt = naca_thickness(t, xn);
            // y-down grid: camber bends the airfoil "up" on screen, i.e.
            // toward negative y.
            let y_rel = -yr / chord;
            if (y_rel - yc).abs() <= yt {
                cell[y * w + x] = CELL_WALL;
            }
        }
    }

    GeoRegion { rect: (0, 0, w as i32, h as i32), cell, fan, dye_src }
}

// --- Rocket nozzles --------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NozzleContour {
    Conical,
    Bell,
}

#[derive(Clone, Copy)]
pub struct NozzleParams {
    /// Throat full width in cells.
    pub throat_cells: f32,
    /// Chamber width / throat width.
    pub chamber_ratio: f32,
    /// Exit width / throat width. For a real engine's area ratio
    /// (axisymmetric), the planar 2D analogue is sqrt(area ratio).
    pub exit_ratio: f32,
    /// Converging section length / throat width.
    pub conv_ratio: f32,
    /// Diverging section length / throat width.
    pub div_ratio: f32,
    /// Wall thickness in cells.
    pub wall_cells: f32,
    pub contour: NozzleContour,
    /// Stamp inlet (fan) cells across the chamber entrance.
    pub chamber_fan: bool,
}

impl Default for NozzleParams {
    fn default() -> Self {
        Self {
            throat_cells: 40.0,
            chamber_ratio: 2.4,
            exit_ratio: 4.0,
            conv_ratio: 2.0,
            div_ratio: 6.0,
            wall_cells: 6.0,
            contour: NozzleContour::Bell,
            chamber_fan: true,
        }
    }
}

/// (name, area ratio eps, contour). The generator uses sqrt(eps) as the
/// 2D exit/throat width ratio.
pub const NOZZLE_PRESETS: [(&str, f32, NozzleContour); 6] = [
    ("V-2 / A-4 — where it began (ε≈3.4)", 3.4, NozzleContour::Conical),
    ("F-1 — Saturn V first stage (ε≈16)", 16.0, NozzleContour::Bell),
    ("Merlin 1D — Falcon 9 (ε≈16)", 16.0, NozzleContour::Bell),
    ("RS-25 / SSME — Space Shuttle (ε≈69)", 69.0, NozzleContour::Bell),
    ("Raptor, sea level — Starship (ε≈34)", 34.0, NozzleContour::Bell),
    ("RL10-B2 — vacuum bell (ε≈280)", 280.0, NozzleContour::Bell),
];

/// Inner half-width of the nozzle at axial position x (cells, x=0 at the
/// chamber entrance).
fn nozzle_half_width(p: &NozzleParams, conv_len: f32, div_len: f32, x: f32) -> f32 {
    let rt = p.throat_cells * 0.5; // throat half-width
    let rc = rt * p.chamber_ratio;
    let re = rt * p.exit_ratio;
    if x <= 0.0 {
        return rc;
    }
    if x < conv_len {
        // Smooth cosine contraction chamber -> throat.
        let s = x / conv_len;
        return rt + (rc - rt) * 0.5 * (1.0 + (std::f32::consts::PI * s).cos());
    }
    let xd = x - conv_len;
    if xd >= div_len {
        return re;
    }
    let s = xd / div_len;
    match p.contour {
        // Straight cone throat -> exit.
        NozzleContour::Conical => rt + (re - rt) * s,
        // Parabolic bell: steep initial expansion that relaxes toward the
        // exit (quadratic Bezier: throat, control point at 30% length with
        // 70% of the radius gain, exit).
        NozzleContour::Bell => {
            let ctrl_x = 0.30;
            let ctrl_r = rt + (re - rt) * 0.70;
            let omt = 1.0 - s;
            // Bezier in (x, r); x(s) is monotonic enough for a contour.
            let _ = ctrl_x;
            omt * omt * rt + 2.0 * omt * s * ctrl_r + s * s * re
        }
    }
}

/// Rasterize a nozzle into a stamp, axis along +x (flow left to right:
/// chamber on the left, bell exit on the right).
pub fn generate_nozzle(p: &NozzleParams) -> GeoRegion {
    let rt = p.throat_cells * 0.5;
    let conv_len = p.conv_ratio * p.throat_cells;
    let div_len = p.div_ratio * p.throat_cells;
    let rc = rt * p.chamber_ratio;
    let re = rt * p.exit_ratio;
    let rmax = rc.max(re);
    let chamber_len = (rc * 1.2).max(p.throat_cells);

    let total_len = chamber_len + conv_len + div_len;
    let back_wall = p.wall_cells.ceil();
    let w = (total_len + back_wall).ceil() as usize + 4;
    let h = (2.0 * (rmax + p.wall_cells)).ceil() as usize + 4;
    let cy = h as f32 * 0.5;

    let mut cell = vec![CELL_FLUID; w * h];
    let mut fan = vec![[0.0f32; 2]; w * h];
    let mut dye_src = vec![[0.0f32; 4]; w * h];

    // Inner half-width at any axial position (clamped into the nozzle).
    let half_at = |ax: f32| -> f32 {
        let ax = ax.clamp(0.0, total_len);
        if ax < chamber_len {
            rc
        } else {
            nozzle_half_width(p, conv_len, div_len, ax - chamber_len)
        }
    };

    for y in 0..h {
        for x in 0..w {
            // Axial position: the chamber's back wall spans [-wall, 0).
            let ax = x as f32 + 0.5 - 2.0 - back_wall;
            let ay = (y as f32 + 0.5 - cy).abs();
            let i = y * w + x;

            if ax < -back_wall || ax > total_len {
                continue;
            }
            // Back wall of the chamber, as thick as the side walls (flow
            // enters through the fan strip).
            if ax < 0.0 {
                if ay <= rc + p.wall_cells {
                    cell[i] = CELL_WALL;
                }
                continue;
            }
            // Watertight wall band: use the neighbouring columns' contour
            // too, so steep bell sections can't step past the wall
            // thickness and leave diagonal holes.
            let h0 = half_at(ax - 1.0);
            let h1 = half_at(ax);
            let h2 = half_at(ax + 1.0);
            let hmin = h0.min(h1).min(h2);
            let hmax = h0.max(h1).max(h2);
            if ay > hmin && ay <= hmax + p.wall_cells {
                cell[i] = CELL_WALL;
            } else if ay <= h1 && p.chamber_fan && ax < 3.0 {
                // Fan strip across the chamber entrance, blowing +x, with
                // a little smoke so the plume is visible immediately.
                cell[i] = CELL_INLET;
                fan[i] = [1.0, 0.0];
                dye_src[i] = [0.95, 0.85, 0.55, 0.85];
            }
        }
    }

    GeoRegion { rect: (0, 0, w as i32, h as i32), cell, fan, dye_src }
}
