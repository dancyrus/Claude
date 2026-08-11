//! Every physical quantity that reaches the screen is formatted here —
//! one formatter per quantity, each owning its own decimal count (phase
//! 4 of the UI overhaul plan). No readout does its own `{:.N}` on a
//! physical value.
//!
//! T2-D: the formatters render in one of two unit systems, selected by
//! the ribbon's units control. SI is the adaptive metric layout below;
//! decimal inch follows ASME drawing practice — lengths in decimal
//! inches with no leading zero before the point (".500 in", never
//! fractions), derived quantities in the inch–pound–second system
//! (in/s, psi, lb/in³, in²/s). Time, angles and every dimensionless
//! readout are identical in both systems. Canonical values everywhere
//! else in the app stay SI — the system is applied at format time only,
//! and input boxes keep their canonical SI value per the panel
//! convention (canonical value in the box, unit in the label).

use std::sync::atomic::{AtomicBool, Ordering};

/// The two unit systems T2-D ships. `DecimalInch` is ASME decimal-inch
/// practice, not feet-and-fractions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum UnitSystem {
    Si,
    DecimalInch,
}

/// Track-era store (same precedent as T2-A/T2-B): `app.rs` is frozen
/// while U4 runs, so the mode lives here instead of `Settings`. Fold it
/// into `Settings` + a `Cmd` + scene persistence at the track merge.
static INCH_MODE: AtomicBool = AtomicBool::new(false);

pub(crate) fn unit_system() -> UnitSystem {
    if INCH_MODE.load(Ordering::Relaxed) {
        UnitSystem::DecimalInch
    } else {
        UnitSystem::Si
    }
}

pub(crate) fn set_unit_system(s: UnitSystem) {
    INCH_MODE.store(s == UnitSystem::DecimalInch, Ordering::Relaxed);
}

const M_PER_IN: f32 = 0.0254;
const PA_PER_PSI: f32 = 6894.757;
/// kg/m³ → lb/in³.
const LB_IN3_PER_KG_M3: f32 = 3.612_729e-5;
/// m²/s → in²/s.
const IN2_PER_M2: f32 = 1.0 / (M_PER_IN * M_PER_IN);

/// ASME decimal-inch number: fixed decimals, and a value below one
/// drops the leading zero (".500", "-.010") the way an inch dimension
/// is written on a drawing.
fn asme_inch(v: f32, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if let Some(rest) = s.strip_prefix("0.") {
        format!(".{rest}")
    } else if let Some(rest) = s.strip_prefix("-0.") {
        format!("-.{rest}")
    } else {
        s
    }
}

/// Length: SI → adaptive mm/cm/m/km; decimal inch → inches only (no
/// feet, per decimal-inch practice), tenths of a thou below 1 in, thou
/// to 100 in, then coarser.
pub(crate) fn fmt_len(m: f32) -> String {
    if unit_system() == UnitSystem::DecimalInch {
        let v = m / M_PER_IN;
        let a = v.abs();
        let d = if a < 1.0 {
            4
        } else if a < 100.0 {
            3
        } else if a < 10_000.0 {
            2
        } else {
            1
        };
        return format!("{} in", asme_inch(v, d));
    }
    let a = m.abs();
    if a < 0.01 {
        format!("{:.1} mm", m * 1e3)
    } else if a < 1.0 {
        format!("{:.1} cm", m * 1e2)
    } else if a < 1000.0 {
        format!("{:.2} m", m)
    } else {
        format!("{:.2} km", m * 1e-3)
    }
}

/// Time in seconds → adaptive µs/ms/s/min (same in both systems).
pub(crate) fn fmt_time(s: f32) -> String {
    let a = s.abs();
    if a < 1e-3 {
        format!("{:.1} µs", s * 1e6)
    } else if a < 1.0 {
        format!("{:.2} ms", s * 1e3)
    } else if a < 120.0 {
        format!("{:.2} s", s)
    } else {
        format!("{:.1} min", s / 60.0)
    }
}

/// Speed: SI → cm/s below 0.1, two decimals to 100 m/s, whole numbers
/// above (a rocket-exhaust readout doesn't need centimetres); decimal
/// inch → in/s, decimals shrinking as the number grows.
pub(crate) fn fmt_speed(v: f32) -> String {
    if unit_system() == UnitSystem::DecimalInch {
        let ips = v / M_PER_IN;
        let a = ips.abs();
        return if a < 1.0 {
            format!("{ips:.3} in/s")
        } else if a < 1000.0 {
            format!("{ips:.2} in/s")
        } else {
            format!("{ips:.0} in/s")
        };
    }
    let a = v.abs();
    if a < 0.1 {
        format!("{:.1} cm/s", v * 100.0)
    } else if a < 100.0 {
        format!("{:.2} m/s", v)
    } else {
        format!("{:.0} m/s", v)
    }
}

/// Gauge pressure: SI → adaptive mPa/Pa/kPa; decimal inch → psi,
/// scientific below a millipsi (gauge deltas here are small).
pub(crate) fn fmt_pressure(p: f32) -> String {
    if unit_system() == UnitSystem::DecimalInch {
        let psi = p / PA_PER_PSI;
        let a = psi.abs();
        return if a < 1e-3 {
            format!("{psi:.2e} psi")
        } else if a < 1.0 {
            format!("{psi:.4} psi")
        } else if a < 1000.0 {
            format!("{psi:.2} psi")
        } else {
            format!("{psi:.0} psi")
        };
    }
    let a = p.abs();
    if a < 0.1 {
        format!("{:.1} mPa", p * 1e3)
    } else if a < 1000.0 {
        format!("{:.2} Pa", p)
    } else {
        format!("{:.2} kPa", p * 1e-3)
    }
}

/// Angle in degrees, one decimal.
pub(crate) fn fmt_angle(deg: f32) -> String {
    format!("{deg:.1}°")
}

/// Mach number, dimensionless, three decimals (the mockup's `1.600`).
pub(crate) fn fmt_mach(m: f32) -> String {
    format!("{m:.3}")
}

/// Courant number, two decimals.
pub(crate) fn fmt_cfl(c: f32) -> String {
    format!("{c:.2}")
}

/// Density: SI → kg/m³, one decimal below 10 (air is 1.2, not "1"),
/// whole numbers above; decimal inch → lb/in³, scientific for gases
/// (air is 4.3e-5 lb/in³ — the ips system's numbers are just like
/// that), three decimals for anything denser.
pub(crate) fn fmt_density(rho: f32) -> String {
    if unit_system() == UnitSystem::DecimalInch {
        let d = rho * LB_IN3_PER_KG_M3;
        return if d.abs() < 1e-3 {
            format!("{d:.2e} lb/in³")
        } else {
            format!("{d:.3} lb/in³")
        };
    }
    if rho.abs() < 10.0 {
        format!("{rho:.1} kg/m³")
    } else {
        format!("{rho:.0} kg/m³")
    }
}

/// Kinematic viscosity, scientific in both systems: m²/s or in²/s.
pub(crate) fn fmt_kvisc(nu: f32) -> String {
    if unit_system() == UnitSystem::DecimalInch {
        format!("{:.2e} in²/s", nu * IN2_PER_M2)
    } else {
        format!("{nu:.2e} m²/s")
    }
}

/// Vorticity / angular frequency in 1/s, one decimal (per second is
/// unit-system neutral).
pub(crate) fn fmt_omega(w: f32) -> String {
    format!("{w:.1} 1/s")
}

/// Dimensionless multiplier ("~9×"), whole numbers.
pub(crate) fn fmt_factor(f: f32) -> String {
    format!("{f:.0}×")
}

/// Simulation rate relative to real time — three decimals below 0.1 so
/// slow (software-rendered) machines don't floor to "0.00×".
pub(crate) fn fmt_sim_rate(r: f32) -> String {
    if r.abs() < 0.0995 {
        format!("{r:.3}× real")
    } else {
        format!("{r:.2}× real")
    }
}

/// Zoom level as a percentage: 100 % = one grid cell per framebuffer
/// pixel (px_per_cell × 100). One decimal below 10 % so extreme
/// zoom-out doesn't floor to "0 %".
pub(crate) fn fmt_zoom(px_per_cell: f32) -> String {
    let pct = px_per_cell * 100.0;
    if pct < 9.95 {
        format!("{pct:.1} %")
    } else {
        format!("{pct:.0} %")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test body so the global mode never races another test; SI is
    /// restored at the end because it is the process-wide default.
    #[test]
    fn inch_mode_formats_and_si_round_trip() {
        assert_eq!(unit_system(), UnitSystem::Si);
        assert_eq!(fmt_len(0.0254), "2.5 cm");
        assert_eq!(fmt_speed(34.0), "34.00 m/s");

        set_unit_system(UnitSystem::DecimalInch);
        // 25.4 mm = exactly 1 inch; 12.7 mm = .500 in, ASME leading
        // zero dropped; negative keeps the sign ahead of the point.
        assert_eq!(fmt_len(0.0254), "1.000 in");
        assert_eq!(fmt_len(0.0127), ".5000 in");
        assert_eq!(fmt_len(-0.0127), "-.5000 in");
        assert_eq!(fmt_len(25.4), "1000.00 in");
        assert_eq!(fmt_speed(0.0254), "1.00 in/s");
        assert_eq!(fmt_pressure(PA_PER_PSI), "1.00 psi");
        assert_eq!(fmt_density(1.2), "4.34e-5 lb/in³");
        assert_eq!(fmt_kvisc(1.0), "1.55e3 in²/s");
        // Unit-system-neutral formatters are untouched by the toggle.
        assert_eq!(fmt_time(1.0), "1.00 s");
        assert_eq!(fmt_omega(3.26), "3.3 1/s");

        set_unit_system(UnitSystem::Si);
        assert_eq!(fmt_len(0.0254), "2.5 cm");
    }
}
