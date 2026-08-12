//! Share-ready PNG export (U5): the annotated variant appends a sheet
//! below the canvas pixels with the legend (and its pinned range), a
//! scale bar, and a run-conditions block — the numbers a raw
//! screenshot loses. Everything physical formats through `ui/units.rs`
//! in the exporter's ACTIVE unit system (a display preference, §Third
//! integration), so the sheet states the system explicitly and every
//! value carries its unit: the PNG must describe itself.
//!
//! Text is set in egui's embedded Hack (the app's monospace face —
//! numeric readouts are monospace by rule), rasterized with `ab_glyph`
//! (already in the tree via epaint; pinned as a direct dependency).
//! Colors come from `ui/theme.rs`; the color bars sample the CPU
//! mirrors of the shader colormaps in app.rs.

use crate::app::{coolwarm_color, inferno_color, ExportKind, FlowPaintApp};
use crate::sim::{ColorMap, GpuSim, RangeMode, RenderMode, SolverMode, UnitSystem};
use ab_glyph::{Font, PxScale, ScaleFont};
use image::RgbaImage;

use super::theme;
use super::units::{
    self, fmt_density, fmt_kvisc, fmt_len, fmt_mach, fmt_omega, fmt_pressure,
    fmt_speed, fmt_time,
};

/// Everything the annotated sheet prints, gathered app-side so the
/// composer is a pure function of it (and testable without a GPU).
pub(in crate::app) struct ExportInfo {
    pub solver: SolverMode,
    pub fluid_name: &'static str,
    pub rho: f32,
    pub nu: f32,
    pub a_inf: f32,
    pub mach: f32,
    pub re: u32,
    pub grid_w: usize,
    pub grid_h: usize,
    pub margin: usize,
    pub dx_m: f32,
    pub sim_time_s: f32,
    pub unit_system: UnitSystem,
    pub mode: RenderMode,
    pub map: ColorMap,
    pub range_mode: RangeMode,
    pub sat_phys: f32,
    /// Bottom of the scale (queue item 4): 0 for Speed and −max for
    /// the diverging modes until a Manual range moves it.
    pub min_phys: f32,
}

impl FlowPaintApp {
    /// The one export path (`Cmd::ExportPng`): render the canvas at
    /// 1 px per cell, optionally compose the annotated sheet, save.
    pub(in crate::app) fn export_png_file(
        &self,
        sim: &GpuSim,
        path: &std::path::Path,
        kind: ExportKind,
    ) -> Result<(), String> {
        let canvas = sim.export_canvas_image()?;
        let img = match kind {
            ExportKind::Canvas => canvas,
            ExportKind::Annotated => {
                let s = &sim.settings;
                let fr = s.ranges[s.render_mode as usize];
                let info = ExportInfo {
                    solver: s.solver,
                    fluid_name: self.fluid_name,
                    rho: self.fluid_rho,
                    nu: self.fluid_nu,
                    a_inf: self.fluid_a,
                    mach: s.mach,
                    re: self.stats_re,
                    grid_w: self.stats_grid.0,
                    grid_h: self.stats_grid.1,
                    margin: self.stats_margin,
                    dx_m: self.phys_cache.dx,
                    sim_time_s: self.sim_time_s as f32,
                    unit_system: s.unit_system,
                    mode: s.render_mode,
                    map: fr.map,
                    range_mode: fr.mode,
                    sat_phys: fr.sat_phys,
                    min_phys: fr.min_phys,
                };
                compose_annotated(canvas, &info)?
            }
        };
        img.save(path).map_err(|e| e.to_string())
    }
}

// --- Sheet text -------------------------------------------------------

/// The run-conditions block, one string per line. Every physical value
/// formats through `ui/units.rs` (so it renders in the exporter's
/// active system WITH its unit), and the last line names the system —
/// two people exporting the same scene may legitimately print
/// different numbers, and the sheet must say which language it speaks.
fn run_conditions_lines(info: &ExportInfo) -> Vec<String> {
    let mut lines = Vec::with_capacity(6);
    lines.push(match info.solver {
        SolverMode::Lbm => format!("Fluid   {} · ρ {}", info.fluid_name, fmt_density(info.rho)),
        SolverMode::Euler => {
            format!("Fluid   {} · ρ {}", info.fluid_name, fmt_density(info.rho))
        }
    });
    lines.push(match info.solver {
        SolverMode::Lbm => format!("Re ≈ {} · ν {}", info.re, fmt_kvisc(info.nu)),
        SolverMode::Euler => {
            format!("Mach M∞ {} · a∞ {}", fmt_mach(info.mach), fmt_speed(info.a_inf))
        }
    });
    lines.push(format!(
        "Grid    {}×{} (+{} margin) · cell Δx {}",
        info.grid_w,
        info.grid_h,
        info.margin,
        fmt_len(info.dx_m)
    ));
    lines.push(format!(
        "Domain  {} × {}",
        fmt_len(info.grid_w as f32 * info.dx_m),
        fmt_len(info.grid_h as f32 * info.dx_m)
    ));
    lines.push(format!("Elapsed t {}", fmt_time(info.sim_time_s)));
    lines.push(match info.unit_system {
        UnitSystem::Si => "Units   SI (metric)".to_string(),
        UnitSystem::DecimalInch => {
            "Units   ASME decimal inch (in · in/s · psi · lbm/ft³)".to_string()
        }
    });
    lines
}

fn solver_title(solver: SolverMode) -> &'static str {
    match solver {
        SolverMode::Lbm => "FlowPaint — LBM (incompressible)",
        SolverMode::Euler => "FlowPaint — Euler (compressible)",
    }
}

/// Legend header and the bar's end labels. Mirrors `ui/legend.rs`: the
/// speed scale runs min (0 until a Manual range raises it) →
/// saturation, vorticity and pressure run min → max (symmetric about
/// zero until a Manual range moves the bottom, queue item 4), and
/// Smoke is a passive tracer with no scale.
fn legend_strings(info: &ExportInfo) -> (String, Option<(String, String)>) {
    let map = match info.map {
        ColorMap::Inferno => "Inferno",
        ColorMap::Coolwarm => "Coolwarm",
    };
    let range = match info.range_mode {
        RangeMode::Auto => "Auto",
        RangeMode::Locked => "Locked",
        RangeMode::Manual => "Manual",
    };
    // The same label shapes as the on-screen legend, so the sheet and
    // the panel never disagree.
    let signed = |v: f32, fmt: fn(f32) -> String| {
        if v < 0.0 {
            format!("-{}", fmt(-v))
        } else {
            format!("+{}", fmt(v))
        }
    };
    match info.mode {
        RenderMode::Dye => (
            "Smoke — passive tracer (arbitrary units)".to_string(),
            None,
        ),
        RenderMode::Speed => (
            format!("Speed |u| — {map} · range {range}"),
            Some((
                if info.min_phys == 0.0 {
                    "0".to_string()
                } else {
                    fmt_speed(info.min_phys)
                },
                format!("≥ {}", fmt_speed(info.sat_phys)),
            )),
        ),
        RenderMode::Vorticity => (
            format!("Vorticity ω — {map} · range {range}"),
            Some((
                signed(info.min_phys, fmt_omega),
                signed(info.sat_phys, fmt_omega),
            )),
        ),
        RenderMode::Pressure => (
            format!("Pressure Δp — {map} · range {range}"),
            Some((
                signed(info.min_phys, fmt_pressure),
                signed(info.sat_phys, fmt_pressure),
            )),
        ),
    }
}

/// Nice 1-2-5 scale-bar length for a 1 px = 1 cell image, stepped in
/// the active display unit (`units::nice_len_m`): physical length and
/// its width in pixels.
fn scale_bar_length(dx_m: f32, canvas_w: u32) -> (f32, u32) {
    let target_px = (canvas_w as f32 / 5.0).min(280.0).max(60.0);
    let len_m = units::nice_len_m(target_px * dx_m);
    (len_m, (len_m / dx_m).round().max(1.0) as u32)
}

// --- Rasterizing ------------------------------------------------------

fn mono_font() -> Result<ab_glyph::FontRef<'static>, String> {
    use std::sync::OnceLock;
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    let bytes = BYTES.get_or_init(|| {
        egui::FontDefinitions::default()
            .font_data
            .get("Hack")
            .map(|fd| fd.font.to_vec())
            .unwrap_or_default()
    });
    ab_glyph::FontRef::try_from_slice(bytes)
        .map_err(|e| format!("embedded font unavailable: {e:?}"))
}

fn put_blend(img: &mut RgbaImage, x: i32, y: i32, color: egui::Color32, alpha: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let a = alpha.clamp(0.0, 1.0);
    let p = img.get_pixel_mut(x as u32, y as u32);
    for c in 0..3 {
        let src = [color.r(), color.g(), color.b()][c] as f32;
        p.0[c] = (src * a + p.0[c] as f32 * (1.0 - a)).round() as u8;
    }
    p.0[3] = 255;
}

fn fill_rect(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, color: egui::Color32) {
    for yy in y..y + h {
        for xx in x..x + w {
            put_blend(img, xx, yy, color, 1.0);
        }
    }
}

/// Draw one line of text at (x, top_y); returns the caret advance.
fn draw_text(
    img: &mut RgbaImage,
    font: &ab_glyph::FontRef<'_>,
    px: f32,
    x: f32,
    top_y: f32,
    color: egui::Color32,
    text: &str,
) -> f32 {
    let scaled = font.as_scaled(PxScale::from(px));
    let baseline = top_y + scaled.ascent();
    let mut caret = x;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let glyph = gid.with_scale_and_position(PxScale::from(px), ab_glyph::point(caret, baseline));
        if let Some(og) = font.outline_glyph(glyph) {
            let b = og.px_bounds();
            og.draw(|gx, gy, cov| {
                put_blend(img, b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, color, cov);
            });
        }
        caret += scaled.h_advance(gid);
    }
    caret - x
}

fn text_width(font: &ab_glyph::FontRef<'_>, px: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(px));
    text.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum()
}

fn colormap_sample(map: ColorMap, t: f32) -> egui::Color32 {
    match map {
        ColorMap::Inferno => inferno_color(t),
        ColorMap::Coolwarm => coolwarm_color(2.0 * t - 1.0),
    }
}

// --- Composition ------------------------------------------------------

const PAD: i32 = 18;
const F_TITLE: f32 = 17.0;
const F_BODY: f32 = 14.0;
const LINE_H: i32 = 21;
const BAR_W: i32 = 256;
const BAR_H: i32 = 14;

/// Append the annotation sheet below the canvas pixels (which stay
/// untouched). Blocks flow left to right and wrap to a second row when
/// the canvas is narrow; the sheet height follows the content.
pub(in crate::app) fn compose_annotated(
    canvas: RgbaImage,
    info: &ExportInfo,
) -> Result<RgbaImage, String> {
    let font = mono_font()?;
    let (cw, ch) = (canvas.width() as i32, canvas.height() as i32);

    let cond = run_conditions_lines(info);
    let title = solver_title(info.solver);
    let (legend_head, legend_labels) = legend_strings(info);
    let (bar_len_m, bar_len_px) = scale_bar_length(info.dx_m, canvas.width());
    let bar_label = fmt_len(bar_len_m);

    // Block metrics.
    let cond_w = cond
        .iter()
        .map(|l| text_width(&font, F_BODY, l))
        .fold(text_width(&font, F_TITLE, title), f32::max)
        .ceil() as i32;
    let cond_h = LINE_H + 6 + cond.len() as i32 * LINE_H;
    let legend_w = (text_width(&font, F_BODY, &legend_head).ceil() as i32).max(BAR_W);
    let legend_h = LINE_H + BAR_H + 4 + LINE_H;
    let scale_w = (bar_len_px as i32).max(text_width(&font, F_BODY, &bar_label).ceil() as i32);
    let scale_h = LINE_H + 14;

    // Side by side when they fit; otherwise the legend and scale bar
    // wrap to a second row under the conditions block.
    let gap = 2 * PAD;
    let one_row = PAD + cond_w + gap + legend_w + gap + scale_w + PAD <= cw;
    let strip_h = if one_row {
        2 * PAD + cond_h.max(legend_h).max(scale_h)
    } else {
        2 * PAD + cond_h + PAD + legend_h.max(scale_h)
    };

    let mut img = RgbaImage::new(canvas.width(), (ch + 2 + strip_h) as u32);
    image::imageops::replace(&mut img, &canvas, 0, 0);
    fill_rect(&mut img, 0, ch, cw, 2, theme::LINE);
    fill_rect(&mut img, 0, ch + 2, cw, strip_h, theme::PANEL);

    let top = ch + 2 + PAD;

    // Conditions block.
    let mut y = top;
    draw_text(&mut img, &font, F_TITLE, PAD as f32, y as f32, theme::INK, title);
    y += LINE_H + 6;
    for line in &cond {
        draw_text(&mut img, &font, F_BODY, PAD as f32, y as f32, theme::INK_2, line);
        y += LINE_H;
    }

    // Legend block.
    let (lx, ly) = if one_row {
        (PAD + cond_w + gap, top)
    } else {
        (PAD, top + cond_h + PAD)
    };
    draw_text(&mut img, &font, F_BODY, lx as f32, ly as f32, theme::INK, &legend_head);
    if let Some((lo, hi)) = &legend_labels {
        let by = ly + LINE_H;
        for i in 0..BAR_W {
            let t = i as f32 / (BAR_W - 1) as f32;
            let c = colormap_sample(info.map, t);
            fill_rect(&mut img, lx + i, by, 1, BAR_H, c);
        }
        // 1 px border so the bar's dark end separates from the panel.
        fill_rect(&mut img, lx - 1, by - 1, BAR_W + 2, 1, theme::LINE_2);
        fill_rect(&mut img, lx - 1, by + BAR_H, BAR_W + 2, 1, theme::LINE_2);
        fill_rect(&mut img, lx - 1, by, 1, BAR_H, theme::LINE_2);
        fill_rect(&mut img, lx + BAR_W, by, 1, BAR_H, theme::LINE_2);
        let label_y = (by + BAR_H + 4) as f32;
        draw_text(&mut img, &font, F_BODY, lx as f32, label_y, theme::INK_2, lo);
        let hw = text_width(&font, F_BODY, hi);
        draw_text(&mut img, &font, F_BODY, (lx + BAR_W) as f32 - hw, label_y, theme::INK_2, hi);
    }

    // Scale bar block.
    let (sx, sy) = if one_row {
        (PAD + cond_w + gap + legend_w + gap, top)
    } else {
        (PAD + legend_w + gap, top + cond_h + PAD)
    };
    let lw = text_width(&font, F_BODY, &bar_label);
    let bar_x = sx + ((scale_w - bar_len_px as i32) / 2).max(0);
    draw_text(
        &mut img,
        &font,
        F_BODY,
        sx as f32 + (scale_w as f32 - lw) * 0.5,
        sy as f32,
        theme::INK,
        &bar_label,
    );
    let bar_y = sy + LINE_H + 6;
    fill_rect(&mut img, bar_x, bar_y, bar_len_px as i32, 2, theme::SCALE_BAR);
    fill_rect(&mut img, bar_x, bar_y - 5, 1, 5, theme::SCALE_BAR);
    fill_rect(&mut img, bar_x + bar_len_px as i32 - 1, bar_y - 5, 1, 5, theme::SCALE_BAR);

    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_info(us: UnitSystem, range_mode: RangeMode) -> ExportInfo {
        ExportInfo {
            solver: SolverMode::Lbm,
            fluid_name: "air",
            rho: 1.2,
            nu: 1.5e-5,
            a_inf: 343.0,
            mach: 1.6,
            re: 921,
            grid_w: 1920,
            grid_h: 960,
            margin: 256,
            dx_m: 1.0 / 1920.0,
            sim_time_s: 0.01736,
            unit_system: us,
            mode: RenderMode::Speed,
            map: ColorMap::Inferno,
            range_mode,
            sat_phys: 0.27,
            min_phys: 0.0,
        }
    }

    /// The sheet is self-describing: every physical value carries its
    /// unit and the block names the system, in both systems.
    #[test]
    fn run_conditions_carry_units_and_name_the_system() {
        let _g = units::UNIT_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        units::set_unit_system(UnitSystem::Si);
        let si = run_conditions_lines(&sample_info(UnitSystem::Si, RangeMode::Auto));
        let all = si.join("\n");
        for unit in ["kg/m³", "m²/s", "mm", "ms"] {
            assert!(all.contains(unit), "missing {unit} in:\n{all}");
        }
        assert!(all.contains("Units   SI (metric)"));

        units::set_unit_system(UnitSystem::DecimalInch);
        let inch =
            run_conditions_lines(&sample_info(UnitSystem::DecimalInch, RangeMode::Auto));
        let all = inch.join("\n");
        for unit in ["lbm/ft³", "in²/s", "in", "ms"] {
            assert!(all.contains(unit), "missing {unit} in:\n{all}");
        }
        assert!(all.contains("ASME decimal inch"));
        units::set_unit_system(UnitSystem::Si);

        // Euler prints Mach and a∞ instead of Re and ν.
        let mut info = sample_info(UnitSystem::Si, RangeMode::Auto);
        info.solver = SolverMode::Euler;
        let all = run_conditions_lines(&info).join("\n");
        assert!(all.contains("Mach M∞ 1.600") && all.contains("a∞ 343"));
    }

    /// A locked range prints its pinned physical value in the legend
    /// header labels — the T2-A round trip the annotated export exists
    /// to preserve.
    #[test]
    fn legend_prints_locked_range_value() {
        let _g = units::UNIT_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        units::set_unit_system(UnitSystem::Si);
        let info = sample_info(UnitSystem::Si, RangeMode::Locked);
        let (head, labels) = legend_strings(&info);
        assert!(head.contains("range Locked"), "{head}");
        let (lo, hi) = labels.expect("speed has a scale");
        assert_eq!(lo, "0");
        assert_eq!(hi, format!("≥ {}", fmt_speed(0.27)));
    }

    /// Composition appends the sheet without touching canvas pixels.
    #[test]
    fn compose_appends_sheet_and_preserves_canvas() {
        let _g = units::UNIT_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        units::set_unit_system(UnitSystem::Si);
        let mut canvas = RgbaImage::new(1200, 300);
        for p in canvas.pixels_mut() {
            *p = image::Rgba([10, 200, 30, 255]);
        }
        let info = sample_info(UnitSystem::Si, RangeMode::Locked);
        let out = compose_annotated(canvas, &info).expect("compose");
        assert_eq!(out.width(), 1200);
        assert!(out.height() > 300, "sheet not appended");
        // Canvas pixels intact.
        assert_eq!(out.get_pixel(0, 0).0, [10, 200, 30, 255]);
        assert_eq!(out.get_pixel(1199, 299).0, [10, 200, 30, 255]);
        // The sheet is not blank: some pixel differs from the panel bg.
        let bg = theme::PANEL;
        let drawn = (0..out.width()).step_by(3).any(|x| {
            (302..out.height()).step_by(3).any(|y| {
                let p = out.get_pixel(x, y).0;
                p[0] != bg.r() || p[1] != bg.g() || p[2] != bg.b()
            })
        });
        assert!(drawn, "annotation strip is blank");
        // Narrow canvas wraps instead of clipping: still composes.
        let narrow = RgbaImage::new(480, 200);
        let out2 = compose_annotated(narrow, &info).expect("compose narrow");
        assert!(out2.height() > out.height() - 300 + 200, "narrow layout did not wrap");
    }
}
