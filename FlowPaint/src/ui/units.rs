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
//! (in/s, psi, in²/s — density deliberately lbm/ft³, see
//! `LBFT3_PER_KGM3`). Time, angles and every dimensionless readout are
//! identical in both systems. Canonical values everywhere else in the
//! app stay SI — the system is applied at the UI boundary only: the
//! formatters below on the way out, and the `InputUnit` adapters at
//! the bottom of this file on the way in (input boxes accept and
//! display the active unit and commit canonical SI).

use std::sync::atomic::{AtomicBool, Ordering};

/// The two unit systems T2-D ships (`DecimalInch` is ASME decimal-inch
/// practice, not feet-and-fractions). Defined in sim.rs because
/// `Settings.unit_system` is the store of record — the third and LAST
/// instance of the track-era static→Settings fold (after T2-A's ranges
/// and T2-B's probes).
pub(crate) use crate::sim::UnitSystem;

/// Frame-scoped MIRROR of `Settings.unit_system`, nothing more: it
/// exists so the `fmt_*` formatters keep stateless signatures at their
/// ~60 call sites. `FlowPaintApp::update` writes it once per frame
/// (from the snapshot, before any panel draws) and is the only
/// production writer; edits go through `Cmd::SetUnitSystem` like every
/// other setting. Do not use this as a store — that pattern is closed.
static INCH_MODE: AtomicBool = AtomicBool::new(false);

pub(crate) fn unit_system() -> UnitSystem {
    if INCH_MODE.load(Ordering::Relaxed) {
        UnitSystem::DecimalInch
    } else {
        UnitSystem::Si
    }
}

/// Sync the mirror. Called from `update` each frame (and from tests).
pub(crate) fn set_unit_system(s: UnitSystem) {
    INCH_MODE.store(s == UnitSystem::DecimalInch, Ordering::Relaxed);
}

/// Preference-file spelling of a unit system (queue item 7). Stable
/// strings — they live in users' config files, so renaming one is a
/// compatibility break, not a refactor.
pub(crate) fn unit_system_pref_str(s: UnitSystem) -> &'static str {
    match s {
        UnitSystem::Si => "si",
        UnitSystem::DecimalInch => "inch",
    }
}

/// Inverse of `unit_system_pref_str`; unknown spellings (a newer build's
/// value, hand edits) yield None and the default stands.
pub(crate) fn unit_system_from_pref(v: &str) -> Option<UnitSystem> {
    match v {
        "si" => Some(UnitSystem::Si),
        "inch" => Some(UnitSystem::DecimalInch),
        _ => None,
    }
}

/// Tests that flip the process-wide mirror serialize on this (cargo
/// runs test fns in parallel); every such test must hold it and leave
/// the mirror on SI.
#[cfg(test)]
pub(crate) static UNIT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const M_PER_IN: f32 = 0.0254;
const PA_PER_PSI: f32 = 6894.757;
/// kg/m³ → lbm/ft³. Density is the one deliberate mixed unit in inch
/// mode: lb/in³ is arithmetically consistent but nobody carries gas
/// density that way — the number an engineer recognizes for air is
/// ≈ 0.0765 lbm/ft³, and mixed units are normal in ips practice.
const LBFT3_PER_KGM3: f32 = 0.062_428;
/// m²/s → in²/s.
const IN2_PER_M2: f32 = 1.0 / (M_PER_IN * M_PER_IN);

/// Nearest "nice" 1-2-5 length to `target_m`, stepped in the ACTIVE
/// display unit (metres or inches), so a scale bar reads a round
/// number in either system — 10.000 in, not 7.874 in (U5; shared by
/// the canvas scale bar and the export sheet).
pub(crate) fn nice_len_m(target_m: f32) -> f32 {
    let unit = if unit_system() == UnitSystem::DecimalInch { M_PER_IN } else { 1.0 };
    let t = (target_m / unit).max(1e-12);
    let decade = 10f32.powf(t.log10().floor());
    let mut best = f32::INFINITY;
    let mut len = decade;
    for m in [1.0, 2.0, 5.0, 10.0] {
        let err = ((m * decade) / t).ln().abs();
        if err < best {
            best = err;
            len = m * decade;
        }
    }
    len * unit
}

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
/// whole numbers above; decimal inch → lbm/ft³ (see LBFT3_PER_KGM3:
/// the recognizable engineering number, air ≈ 0.0765), four decimals
/// for gases, coarser as the number grows.
pub(crate) fn fmt_density(rho: f32) -> String {
    if unit_system() == UnitSystem::DecimalInch {
        let d = rho * LBFT3_PER_KGM3;
        let a = d.abs();
        return if a < 1.0 {
            format!("{d:.4} lbm/ft³")
        } else if a < 100.0 {
            format!("{d:.2} lbm/ft³")
        } else {
            format!("{d:.0} lbm/ft³")
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

// --- Input adapters --------------------------------------------------
//
// Input boxes accept and display the ACTIVE unit system; only the
// stored value is canonical SI. Call sites wire an `InputUnit` into
// `DragValue::custom_formatter` / `custom_parser`, so typing "24" in
// inch mode commits 0.6096 m while the box keeps reading "24.00 in".
// The canonical-value convention governs storage and call-site
// formatting, not what the user types.

/// One editable quantity's active display unit: display value =
/// canonical / `canon_per_unit`. Copyable so the formatter and parser
/// closures can each capture it.
#[derive(Clone, Copy)]
pub(crate) struct InputUnit {
    pub(crate) suffix: &'static str,
    pub(crate) canon_per_unit: f32,
}

impl InputUnit {
    /// Format a canonical (SI) value for the edit box, in the display
    /// unit. Plain decimal notation with adaptive precision (enough
    /// decimals that formatting does not eat the committed value —
    /// the round trip is pinned by test), scientific only for the
    /// tiny gauge pressures.
    pub(crate) fn fmt(&self, canonical: f64) -> String {
        let x = canonical / self.canon_per_unit as f64;
        let a = x.abs();
        if x == 0.0 {
            "0".into()
        } else if a >= 1000.0 {
            format!("{x:.0}")
        } else if a >= 1.0 {
            format!("{x:.2}")
        } else if a >= 1e-3 {
            format!("{x:.4}")
        } else {
            format!("{x:.2e}")
        }
    }

    /// Parse edit-box text as a display-unit value, back to canonical
    /// SI. Tolerates a typed-in copy of the suffix ("24 in").
    pub(crate) fn parse(&self, text: &str) -> Option<f64> {
        let t = text.trim();
        let t = t.strip_suffix(self.suffix.trim()).unwrap_or(t).trim();
        t.parse::<f64>().ok().map(|v| v * self.canon_per_unit as f64)
    }
}

pub(crate) fn len_input_unit() -> InputUnit {
    match unit_system() {
        UnitSystem::Si => InputUnit { suffix: " m", canon_per_unit: 1.0 },
        UnitSystem::DecimalInch => InputUnit { suffix: " in", canon_per_unit: M_PER_IN },
    }
}

pub(crate) fn speed_input_unit() -> InputUnit {
    match unit_system() {
        UnitSystem::Si => InputUnit { suffix: " m/s", canon_per_unit: 1.0 },
        UnitSystem::DecimalInch => InputUnit { suffix: " in/s", canon_per_unit: M_PER_IN },
    }
}

pub(crate) fn pressure_input_unit() -> InputUnit {
    match unit_system() {
        UnitSystem::Si => InputUnit { suffix: " Pa", canon_per_unit: 1.0 },
        UnitSystem::DecimalInch => InputUnit { suffix: " psi", canon_per_unit: PA_PER_PSI },
    }
}

/// Per-second is unit-system neutral.
pub(crate) fn omega_input_unit() -> InputUnit {
    InputUnit { suffix: " 1/s", canon_per_unit: 1.0 }
}

pub(crate) fn dimensionless_input_unit() -> InputUnit {
    InputUnit { suffix: "", canon_per_unit: 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test body so the global mode never races another test; SI is
    /// restored at the end because it is the process-wide default.
    #[test]
    fn inch_mode_formats_and_si_round_trip() {
        let _g = UNIT_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
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
        // Density is deliberately lbm/ft³ (the recognizable number),
        // not lb/in³ — see LBFT3_PER_KGM3.
        assert_eq!(fmt_density(1.2), "0.0749 lbm/ft³");
        assert_eq!(fmt_density(1.225), "0.0765 lbm/ft³");
        assert_eq!(fmt_density(1000.0), "62.43 lbm/ft³");
        assert_eq!(fmt_kvisc(1.0), "1.55e3 in²/s");
        // Unit-system-neutral formatters are untouched by the toggle.
        assert_eq!(fmt_time(1.0), "1.00 s");
        assert_eq!(fmt_omega(3.26), "3.3 1/s");

        // Input round trip: type 24 in, commit, switch systems, switch
        // back — the committed canonical value must not drift and the
        // box must keep reading exactly "24.00 in".
        let inch = len_input_unit();
        let c1 = inch.parse("24").expect("parse 24 in");
        assert!((c1 - 0.6096).abs() < 1e-6);
        assert_eq!(inch.fmt(c1), "24.00");
        assert_eq!(inch.parse(&inch.fmt(c1)), Some(c1));
        assert_eq!(inch.parse("24 in"), Some(c1)); // typed suffix tolerated
        set_unit_system(UnitSystem::Si);
        let si = len_input_unit();
        let shown_si = si.fmt(c1);
        assert_eq!(shown_si, "0.6096");
        // Even a re-commit of the SI display drifts by less than a
        // micrometre and still reads back as exactly 24.00 in.
        let c2 = si.parse(&shown_si).expect("parse SI display");
        assert!((c2 - c1).abs() < 1e-6);
        set_unit_system(UnitSystem::DecimalInch);
        assert_eq!(len_input_unit().fmt(c1), "24.00");
        assert_eq!(len_input_unit().fmt(c2), "24.00");
        // The other input quantities convert on the same path.
        assert_eq!(pressure_input_unit().parse("1"), Some(PA_PER_PSI as f64));
        assert_eq!(speed_input_unit().fmt(0.0254), "1.00");
        assert_eq!(omega_input_unit().fmt(2.5), "2.50");

        set_unit_system(UnitSystem::Si);
        assert_eq!(fmt_len(0.0254), "2.5 cm");
    }

    /// The preference-file spellings round-trip and stay stable, and
    /// unknown values fall back to None (the default then stands) —
    /// queue item 7.
    #[test]
    fn unit_pref_strings_round_trip_and_reject_unknown() {
        for s in [UnitSystem::Si, UnitSystem::DecimalInch] {
            assert_eq!(unit_system_from_pref(unit_system_pref_str(s)), Some(s));
        }
        assert_eq!(unit_system_pref_str(UnitSystem::Si), "si");
        assert_eq!(unit_system_pref_str(UnitSystem::DecimalInch), "inch");
        assert_eq!(unit_system_from_pref("feet"), None);
        assert_eq!(unit_system_from_pref(""), None);
    }
}
