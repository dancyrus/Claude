//! Every physical quantity that reaches the screen is formatted here —
//! one formatter per quantity, each owning its own decimal count (phase
//! 4 of the UI overhaul plan). No readout does its own `{:.N}` on a
//! physical value.

/// Length in metres → adaptive mm/cm/m/km.
pub(crate) fn fmt_len(m: f32) -> String {
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

/// Time in seconds → adaptive µs/ms/s/min.
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

/// Speed in m/s → cm/s below 0.1, two decimals to 100 m/s, whole
/// numbers above (a rocket-exhaust readout doesn't need centimetres).
pub(crate) fn fmt_speed(v: f32) -> String {
    let a = v.abs();
    if a < 0.1 {
        format!("{:.1} cm/s", v * 100.0)
    } else if a < 100.0 {
        format!("{:.2} m/s", v)
    } else {
        format!("{:.0} m/s", v)
    }
}

/// Gauge pressure in Pa → adaptive mPa/Pa/kPa.
pub(crate) fn fmt_pressure(p: f32) -> String {
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

/// Density in kg/m³ — one decimal below 10 (air is 1.2, not "1"),
/// whole numbers above.
pub(crate) fn fmt_density(rho: f32) -> String {
    if rho.abs() < 10.0 {
        format!("{rho:.1} kg/m³")
    } else {
        format!("{rho:.0} kg/m³")
    }
}

/// Kinematic viscosity in m²/s, scientific.
pub(crate) fn fmt_kvisc(nu: f32) -> String {
    format!("{nu:.2e} m²/s")
}

/// Vorticity / angular frequency in 1/s, one decimal.
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
