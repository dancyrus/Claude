//! The right-hand legend: flow numbers in physical units and the
//! color-scale bar for the current view.

use crate::app::{coolwarm_color, inferno_color, Cmd, FlowPaintApp, UiSnapshot};
use crate::sim::{ColorMap, FieldRange, RangeMode, RenderMode, SolverMode};
use eframe::egui;

use super::units::{
    self, fmt_density, fmt_kvisc, fmt_len, fmt_mach, fmt_omega,
    fmt_pressure, fmt_sim_rate, fmt_speed, fmt_time,
};

impl FlowPaintApp {
    /// Physical value of one render-buffer unit of a field — the factor
    /// the legend uses to invert the shader's normalization (T2-A). All
    /// unit conversions for the color range live here, app-side; the sim
    /// stores render-unit values only.
    fn range_phys_per_render(&self, mode: RenderMode, snap: &UiSnapshot) -> f32 {
        let ps = self.phys_cache;
        let euler = snap.solver == SolverMode::Euler;
        match mode {
            // LBM: lattice cells/step. Euler: the buffer stores u * dt
            // with u in units of a∞.
            RenderMode::Speed => {
                if euler {
                    self.fluid_a / snap.euler_dt.max(1e-6)
                } else {
                    ps.u_phys(1.0)
                }
            }
            RenderMode::Vorticity => 1.0 / ps.dt,
            // LBM: density deviation, p = cs² · Δρ. Euler: the density
            // buffer stores 1 + 0.1 · (p − p∞), p in units of ρ∞ a∞².
            RenderMode::Pressure => {
                if euler {
                    10.0 * self.fluid_rho * self.fluid_a * self.fluid_a
                } else {
                    ps.pressure_pa(1.0, self.fluid_rho)
                }
            }
            RenderMode::Dye => 1.0,
        }
    }

    /// Saturation point of a mode's AUTO range in render units — where
    /// the shader's normalization clips with the current settings (each
    /// formula inverts the corresponding mapping in render.wgsl).
    fn auto_sat_render(mode: RenderMode, snap: &UiSnapshot) -> f32 {
        let gain = snap.display_gain.max(1e-3);
        let inlet = if snap.solver == SolverMode::Euler {
            snap.mach * snap.euler_dt
        } else {
            snap.flow
        };
        match mode {
            RenderMode::Speed => (inlet * 1.6).max(1e-3) / gain,
            RenderMode::Vorticity => inlet.max(0.02) / (4.0 * gain),
            RenderMode::Pressure => 1.0 / (25.0 * gain),
            RenderMode::Dye => 1.0,
        }
    }

    /// Reconcile the snapshot's color ranges with this frame's physical
    /// scaling: a pinned (Locked/Manual) range holds its physical value
    /// and gets its render-unit twin rewritten, while Auto tracks the
    /// current settings in both — so switching to Locked or Manual starts
    /// from exactly what is on screen, with no separate capture step.
    /// Runs every frame from `update`, before any panel draws; the
    /// synced twins are written back to `Settings` when commands apply.
    pub(in crate::app) fn sync_color_ranges(&self, snap: &mut UiSnapshot) {
        for mode in [RenderMode::Speed, RenderMode::Vorticity, RenderMode::Pressure] {
            let k = self.range_phys_per_render(mode, snap).max(1e-12);
            let auto = Self::auto_sat_render(mode, snap);
            let fr = &mut snap.ranges[mode as usize];
            match fr.mode {
                RangeMode::Locked | RangeMode::Manual => {
                    fr.sat_render = (fr.sat_phys / k).max(1e-9);
                    fr.min_render = fr.min_phys / k;
                }
                RangeMode::Auto => {
                    fr.sat_render = auto;
                    fr.sat_phys = fr.sat_render * k;
                    // The mode's natural bottom (every pre-item-4
                    // scale): 0 for Speed, symmetric for the rest.
                    fr.min_render =
                        if mode == RenderMode::Speed { 0.0 } else { -auto };
                    fr.min_phys = fr.min_render * k;
                }
            }
        }
    }

    /// The right-hand legend: the important flow numbers in physical
    /// units, plus a color-scale bar for the current view.
    pub(in crate::app) fn legend_panel(
        &mut self,
        ctx: &egui::Context,
        snap: UiSnapshot,
        cmds: &mut Vec<Cmd>,
    ) {
        if !self.show_legend {
            return;
        }
        let ps = self.phys_cache;
        let (_vw, vh) = self.stats_grid;
        egui::SidePanel::right("legend").default_width(200.0).show(ctx, |ui| {
            // Scroll rather than clip: at the 900×600 minimum the color
            // bar and range controls land below the flow-numbers grid.
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            ui.label(super::theme::heading("Flow numbers"));
            egui::Grid::new("legend_grid")
                .num_columns(2)
                .striped(true)
                .min_col_width(80.0)
                .show(ui, |ui| {
                    let mut row = |k: &str, v: String| {
                        ui.label(k);
                        ui.monospace(v);
                        ui.end_row();
                    };
                    let euler = snap.solver == SolverMode::Euler;
                    let u_inf = if euler {
                        snap.mach * self.fluid_a
                    } else {
                        ps.u_phys(snap.flow)
                    };
                    row(
                        "Solver",
                        if euler { "Euler (compressible)" } else { "LBM (incompressible)" }
                            .to_string(),
                    );
                    row("Fluid", self.fluid_name.to_string());
                    if euler {
                        row("a∞", fmt_speed(self.fluid_a));
                        row("Mach M∞", fmt_mach(snap.mach));
                    } else {
                        row("ν", fmt_kvisc(self.fluid_nu));
                    }
                    row("ρ", fmt_density(self.fluid_rho));
                    row(
                        "Domain",
                        format!(
                            "{} × {}",
                            fmt_len(self.domain_width_m),
                            fmt_len(ps.len_m(vh as f32))
                        ),
                    );
                    row("Cell Δx", fmt_len(ps.dx));
                    row("Step Δt", fmt_time(ps.dt));
                    row("Inlet U∞", fmt_speed(u_inf));
                    row(
                        "Ref. length",
                        fmt_len(ps.len_m(0.16 * vh as f32)),
                    );
                    if euler {
                        row("Reynolds", "∞ (inviscid)".to_string());
                    } else {
                        row("Reynolds", format!("{}", self.stats_re));
                    }
                    row(
                        "Dyn. press.",
                        fmt_pressure(0.5 * self.fluid_rho * u_inf * u_inf),
                    );
                    row(
                        "Sim rate",
                        fmt_sim_rate(self.stats_steps_per_s * ps.dt),
                    );
                    row("Sim time", fmt_time(self.sim_time_s as f32));
                });
            ui.separator();

            // Color-scale legend for the current view. `sync_color_ranges`
            // (run in `update` before the panels) wrote the saturation
            // point in physical units for every range mode — Auto still
            // inverts the shader's normalization, Locked and Manual read
            // the pinned value (T2-A).
            let fr: FieldRange = snap.ranges[snap.mode as usize];
            match snap.mode {
                RenderMode::Dye => {
                    ui.label("Smoke view: dye brightness");
                    ui.label(
                        egui::RichText::new(
                            "(passive tracer — arbitrary units)",
                        )
                        .small()
                        .weak(),
                    );
                }
                RenderMode::Speed => {
                    ui.label("Speed |u|");
                    Self::colormap_bar(ui, fr.map);
                    ui.horizontal(|ui| {
                        super::theme::mono_small(ui, lo_label(fr.min_phys, fmt_speed));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                super::theme::mono_small(ui, format!("≥ {}", fmt_speed(fr.sat_phys)))
                            },
                        );
                    });
                    self.range_controls(ui, snap.mode, fr, cmds);
                }
                RenderMode::Vorticity => {
                    ui.label("Vorticity ω (curl)");
                    Self::colormap_bar(ui, fr.map);
                    ui.horizontal(|ui| {
                        super::theme::mono_small(ui, signed_label(fr.min_phys, fmt_omega));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                super::theme::mono_small(ui, signed_label(fr.sat_phys, fmt_omega))
                            },
                        );
                    });
                    ui.small("red: clockwise · blue: counter-clockwise");
                    self.range_controls(ui, snap.mode, fr, cmds);
                }
                RenderMode::Pressure => {
                    ui.label("Pressure Δp (gauge)");
                    Self::colormap_bar(ui, fr.map);
                    ui.horizontal(|ui| {
                        super::theme::mono_small(ui, signed_label(fr.min_phys, fmt_pressure));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                super::theme::mono_small(
                                    ui,
                                    signed_label(fr.sat_phys, fmt_pressure),
                                )
                            },
                        );
                    });
                    ui.small("relative to ambient (0 = undisturbed)");
                    self.range_controls(ui, snap.mode, fr, cmds);
                }
            }
            });
        });
    }

    /// The scale controls under the color bar (plan v4.1, T2-A): the
    /// range selector (Auto follows the flow settings, Locked pins the
    /// scale as it is, Manual takes a typed saturation value in physical
    /// units) and the colormap picker — per render mode. They live here,
    /// next to the scale they govern, because the Results ribbon has no
    /// width left at the 900 px minimum.
    fn range_controls(
        &self,
        ui: &mut egui::Ui,
        mode: RenderMode,
        fr: FieldRange,
        cmds: &mut Vec<Cmd>,
    ) {
        // Edits go out as commands like every other setting. Picking
        // Locked needs no capture step: `sync_color_ranges` already left
        // the on-screen value in `sat_phys`, and `Settings` receives the
        // synced twins right before this frame's commands apply.
        let mut set_mode: Option<RangeMode> = None;
        let mut set_phys: Option<f32> = None;
        let mut set_min: Option<f32> = None;
        let mut set_map: Option<ColorMap> = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("range").small().color(super::theme::INK_3));
            let current = match fr.mode {
                RangeMode::Auto => "Auto",
                RangeMode::Locked => "Locked",
                RangeMode::Manual => "Manual",
            };
            egui::ComboBox::from_id_salt("legend_color_range")
                .width(88.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if super::theme::toggle(ui, fr.mode == RangeMode::Auto, "Auto")
                        .on_hover_text(
                            "The scale follows the inlet condition and the \
                             display gain",
                        )
                        .clicked()
                    {
                        set_mode = Some(RangeMode::Auto);
                    }
                    if super::theme::toggle(ui, fr.mode == RangeMode::Locked, "Locked")
                        .on_hover_text(
                            "Pin the scale as it is now, so two screenshots \
                             stay comparable",
                        )
                        .clicked()
                    {
                        set_mode = Some(RangeMode::Locked);
                    }
                    if super::theme::toggle(ui, fr.mode == RangeMode::Manual, "Manual")
                        .on_hover_text("Type the top of the scale")
                        .clicked()
                    {
                        set_mode = Some(RangeMode::Manual);
                    }
                });
        });
        if fr.mode == RangeMode::Manual {
            let mut v = fr.sat_phys;
            // Typed in the active unit system; committed canonical SI
            // (T2-D input adapter).
            let unit = match mode {
                RenderMode::Speed => units::speed_input_unit(),
                RenderMode::Vorticity => units::omega_input_unit(),
                RenderMode::Pressure => units::pressure_input_unit(),
                RenderMode::Dye => units::dimensionless_input_unit(),
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("max").small().color(super::theme::INK_3));
                if ui
                    .add(
                        egui::DragValue::new(&mut v)
                            .range(1e-4..=1e9)
                            .speed((fr.sat_phys.abs() * 0.01).max(0.001))
                            .custom_formatter(move |x, _| unit.fmt(x))
                            .custom_parser(move |s| unit.parse(s))
                            .suffix(unit.suffix),
                    )
                    .on_hover_text("Value where the scale ends (the last color)")
                    .changed()
                {
                    set_phys = Some(v);
                }
            });
            // The bottom of the scale (queue item 4). Its default is
            // the legacy shape — 0 for Speed, −max for the diverging
            // modes — so a scene that never touches it looks as before.
            let mut lo = fr.min_phys;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("min").small().color(super::theme::INK_3));
                if ui
                    .add(
                        egui::DragValue::new(&mut lo)
                            .range(-1e9..=1e9)
                            .speed(((fr.sat_phys - fr.min_phys).abs() * 0.01).max(0.001))
                            .custom_formatter(move |x, _| unit.fmt(x))
                            .custom_parser(move |s| unit.parse(s))
                            .suffix(unit.suffix),
                    )
                    .on_hover_text(match mode {
                        RenderMode::Speed => {
                            "Speed where the scale starts (not below 0); \
                             lower speeds show the first color"
                        }
                        _ => "Value where the scale starts (the first color)",
                    })
                    .changed()
                {
                    set_min = Some(lo);
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("map").small().color(super::theme::INK_3));
            let map_name = |m: ColorMap| match m {
                ColorMap::Inferno => "Inferno",
                ColorMap::Coolwarm => "Coolwarm",
            };
            egui::ComboBox::from_id_salt("legend_colormap")
                .width(88.0)
                .selected_text(map_name(fr.map))
                .show_ui(ui, |ui| {
                    for m in [ColorMap::Inferno, ColorMap::Coolwarm] {
                        if super::theme::toggle(ui, fr.map == m, map_name(m))
                            .on_hover_text(match m {
                                ColorMap::Inferno => {
                                    "Sequential: dark to bright yellow"
                                }
                                ColorMap::Coolwarm => {
                                    "Diverging: blue and red around the middle"
                                }
                            })
                            .clicked()
                        {
                            set_map = Some(m);
                        }
                    }
                });
        });
        if let Some(m) = set_mode {
            cmds.push(Cmd::SetRangeMode(mode, m));
        }
        if let Some(v) = set_phys {
            cmds.push(Cmd::SetRangeMax(mode, v));
        }
        if let Some(v) = set_min {
            cmds.push(Cmd::SetRangeMin(mode, v));
        }
        if let Some(m) = set_map {
            cmds.push(Cmd::SetColorMap(mode, m));
        }
    }

    /// The color-scale bar, drawn with the view's chosen map. `t` runs
    /// 0..1 left to right; the diverging map spans its full ramp across
    /// that, matching the shader's swap convention (render.wgsl flags
    /// bit 1).
    fn colormap_bar(ui: &mut egui::Ui, map: ColorMap) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().min(184.0), 14.0),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        let n = 48;
        for i in 0..n {
            let t0 = i as f32 / n as f32;
            let t1 = (i + 1) as f32 / n as f32;
            let t = (t0 + t1) * 0.5;
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.min.x + rect.width() * t0, rect.min.y),
                    egui::pos2(rect.min.x + rect.width() * t1, rect.max.y),
                ),
                0.0,
                match map {
                    ColorMap::Inferno => inferno_color(t),
                    ColorMap::Coolwarm => coolwarm_color(t * 2.0 - 1.0),
                },
            );
        }
    }
}

/// Bottom-of-scale label for Speed (queue item 4): the bare "0"
/// everyone knows while the bottom is untouched, the formatted value
/// once a Manual range raised it.
fn lo_label(v: f32, fmt: impl Fn(f32) -> String) -> String {
    if v == 0.0 {
        "0".to_string()
    } else {
        fmt(v)
    }
}

/// Signed end-of-scale label for the diverging modes: an explicit sign
/// on a magnitude-formatted value, so the symmetric look stays
/// "-x … +x" and an asymmetric bottom prints its own sign.
fn signed_label(v: f32, fmt: impl Fn(f32) -> String) -> String {
    if v < 0.0 {
        format!("-{}", fmt(-v))
    } else {
        format!("+{}", fmt(v))
    }
}
