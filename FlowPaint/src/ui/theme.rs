//! The theme: every visual constant in the app resolves through this
//! module. Palette, corner radius, type sizes and panel dimensions are
//! ported from the `:root` block of `docs/ui-target.html` (the layout
//! mockup) — change them there conceptually, here concretely.
//!
//! Exceptions, per the overhaul plan: the inferno/coolwarm colormap stop
//! tables stay in `app.rs` (they mirror `shaders/render.wgsl` and must
//! not drift from it), and `def_smoke` stays in `app.rs` (scene content,
//! not chrome).

use eframe::egui;
use egui::{Color32, FontId, Margin, Rounding, Stroke, TextStyle, Vec2};

// --- Palette (docs/ui-target.html :root) ------------------------------

/// `--chrome` — window chrome: menu bar, title strip (placed in phase 3).
#[allow(dead_code)]
pub(crate) const CHROME: Color32 = Color32::from_rgb(34, 38, 44);
/// `--panel` — panel background.
pub(crate) const PANEL: Color32 = Color32::from_rgb(41, 46, 53);
/// `--panel-2` — raised/hovered panel surfaces.
pub(crate) const PANEL_2: Color32 = Color32::from_rgb(47, 53, 61);
/// `--view` — the graphics-window background.
pub(crate) const VIEW_BG: Color32 = Color32::from_rgb(18, 21, 26);
/// `#1c2025` — value-box fill (the mockup's `.dv` field background).
pub(crate) const FIELD_BG: Color32 = Color32::from_rgb(28, 32, 37);
/// `--line` — panel borders and separators.
pub(crate) const LINE: Color32 = Color32::from_rgb(58, 65, 73);
/// `--line-2` — control borders (hover, value boxes).
pub(crate) const LINE_2: Color32 = Color32::from_rgb(70, 78, 88);
/// `--ink` — primary text.
pub(crate) const INK: Color32 = Color32::from_rgb(223, 228, 234);
/// `--ink-2` — secondary text (labels, idle controls).
pub(crate) const INK_2: Color32 = Color32::from_rgb(154, 164, 176);
/// `--ink-3` — tertiary text (captions, units; placed in phase 3).
#[allow(dead_code)]
pub(crate) const INK_3: Color32 = Color32::from_rgb(109, 119, 131);

// --- The two accent roles ---------------------------------------------

/// `--sel` — selection and active state. The only "positive" accent.
pub(crate) const SEL: Color32 = Color32::from_rgb(63, 184, 174);
/// `--sel-bg` — fill behind selected/active controls.
pub(crate) const SEL_BG: Color32 = Color32::from_rgb(30, 63, 62);
/// Text on top of `SEL_BG` (the mockup's `.bt.on` color `#cfeeeb`).
/// egui's `widgets.active` state means *pressed/focused*, not selected —
/// persistent selection gets this color via [`toggle`], which is why the
/// selectable call sites go through that helper.
pub(crate) const SEL_INK: Color32 = Color32::from_rgb(207, 238, 235);
/// `--bad` — destructive actions. The only other accent.
pub(crate) const BAD: Color32 = Color32::from_rgb(207, 111, 98);
/// `--warn` — cautions in readouts (not an interactive accent).
pub(crate) const WARN: Color32 = Color32::from_rgb(209, 162, 74);

// --- Canvas overlay colors (drawn with the painter, not widgets) ------

/// Vertex handle fill (was hardcoded white).
pub(crate) const HANDLE_FILL: Color32 = INK;
/// Vertex handle outline (was hardcoded black).
pub(crate) const HANDLE_OUTLINE: Color32 = VIEW_BG;
/// Faint snap-grid overlay lines: INK at alpha 12. The premultiplied
/// channels are what ecolor's `from_rgba_unmultiplied(223, 228, 234, 12)`
/// produces — the multiply happens in LINEAR light, not gamma space, so
/// the naive `c * 12 / 255` would be ~5x too dark.
pub(crate) const GRID_HINT: Color32 = Color32::from_rgba_premultiplied(52, 54, 55, 12);

// --- Geometry ---------------------------------------------------------

/// `--r` — near-square corner radius on every widget.
pub(crate) const RADIUS: f32 = 3.0;

/// Panel dimensions from the mockup, consumed by the phase 3 layout.
/// Kept here so the layout rebuild reads them instead of re-inventing.
#[allow(dead_code)]
pub(crate) mod dim {
    /// Model-tree panel default width.
    pub(crate) const TREE_WIDTH: f32 = 212.0;
    /// Settings panel default width.
    pub(crate) const SETTINGS_WIDTH: f32 = 258.0;
    /// Ribbon body height (excluding the tab strip).
    pub(crate) const RIBBON_HEIGHT: f32 = 86.0;
    /// Ribbon tab strip height.
    pub(crate) const RIBBON_TABS_HEIGHT: f32 = 27.0;
    /// Menu bar height.
    pub(crate) const MENU_HEIGHT: f32 = 26.0;
    /// Message line height.
    pub(crate) const MSG_HEIGHT: f32 = 22.0;
    /// Status strip height.
    pub(crate) const STATUS_HEIGHT: f32 = 24.0;
}

// --- Type scale -------------------------------------------------------
// The mockup's base size is 12 px with 10–11 px captions; headings are
// modest (panel headers, not display type).

const TEXT_BODY: f32 = 12.0;
const TEXT_SMALL: f32 = 10.0;
const TEXT_HEADING: f32 = 14.0;
const TEXT_MONO: f32 = 12.0;

// --- Application ------------------------------------------------------

/// Build the whole style once and install it, with the icon font.
/// Called once at startup; nothing else in the app may set style fields.
pub(crate) fn apply(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // Phosphor icons ride along in the proportional family; phase 3
    // places them (always beside a text label, never alone).
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(TEXT_HEADING)),
        (TextStyle::Body, FontId::proportional(TEXT_BODY)),
        (TextStyle::Button, FontId::proportional(TEXT_BODY)),
        (TextStyle::Small, FontId::proportional(TEXT_SMALL)),
        (TextStyle::Monospace, FontId::monospace(TEXT_MONO)),
    ]
    .into();
    // Every numeric value box (DragValue, and the value readouts inside
    // Sliders) renders in monospace with tabular figures — instrument
    // digits, not toy digits.
    style.drag_value_text_style = TextStyle::Monospace;

    // One spacing system instead of per-site add_space calls.
    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(6.0, 5.0);
    s.button_padding = Vec2::new(9.0, 3.0);
    s.indent = 12.0;
    s.slider_width = 120.0;
    s.interact_size = Vec2::new(40.0, 18.0);
    s.window_margin = Margin::same(8.0);
    s.menu_margin = Margin::same(6.0);

    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = FIELD_BG;
    v.faint_bg_color = PANEL_2;
    v.code_bg_color = FIELD_BG;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.hyperlink_color = SEL;
    v.warn_fg_color = WARN;
    v.error_fg_color = BAD;
    v.selection.bg_fill = SEL_BG;
    v.selection.stroke = Stroke::new(1.0, SEL);

    let r = Rounding::same(RADIUS);
    v.window_rounding = r;
    v.menu_rounding = r;

    // Widget states, mapped from the mockup's control styling: flat idle
    // controls on the panel color, hairline borders appearing on hover,
    // teal fill while pressed/focused (egui's "active"). Persistent
    // selection is styled by the `toggle` helper below, because egui
    // hardwires selected-SelectableLabel text to `selection.stroke` and
    // drops the border the mockup specifies.
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK_2);
    w.noninteractive.rounding = r;

    w.inactive.bg_fill = PANEL_2;
    w.inactive.weak_bg_fill = PANEL_2;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, INK_2);
    w.inactive.rounding = r;

    w.hovered.bg_fill = PANEL_2;
    w.hovered.weak_bg_fill = PANEL_2;
    w.hovered.bg_stroke = Stroke::new(1.0, LINE_2);
    w.hovered.fg_stroke = Stroke::new(1.5, INK);
    w.hovered.rounding = r;

    w.active.bg_fill = SEL_BG;
    w.active.weak_bg_fill = SEL_BG;
    w.active.bg_stroke = Stroke::new(1.0, SEL);
    w.active.fg_stroke = Stroke::new(1.5, SEL_INK);
    w.active.rounding = r;

    w.open.bg_fill = PANEL_2;
    w.open.weak_bg_fill = PANEL_2;
    w.open.bg_stroke = Stroke::new(1.0, LINE);
    w.open.fg_stroke = Stroke::new(1.0, INK);
    w.open.rounding = r;

    ctx.set_style(style);
}

// --- Widget helpers ---------------------------------------------------

/// A persistent-selection toggle, styled like the mockup's `.bt.on` /
/// `.pill.on`: `SEL_BG` fill, 1 px `SEL` border, `SEL_INK` text. egui's
/// `selectable_label` can't render that combination (it takes its text
/// color from `selection.stroke` and drops the border), so selection
/// call sites use this instead.
pub(crate) fn toggle(ui: &mut egui::Ui, on: bool, label: impl Into<String>) -> egui::Response {
    let text = egui::RichText::new(label.into()).color(if on { SEL_INK } else { INK_2 });
    let mut btn = egui::Button::new(text).rounding(Rounding::same(RADIUS));
    if on {
        btn = btn.fill(SEL_BG).stroke(Stroke::new(1.0, SEL));
    }
    ui.add(btn)
}

/// A panel section heading in primary ink (the default text color is the
/// secondary `INK_2`; the mockup's panel headers use `--ink`).
pub(crate) fn heading(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text.into()).heading().color(INK)
}

/// A derived-value secondary line (the mockup's `.der` row): small
/// monospace in tertiary ink, under the control whose canonical value
/// it re-expresses.
pub(crate) fn derived(ui: &mut egui::Ui, text: String) -> egui::Response {
    ui.label(
        egui::RichText::new(text)
            .monospace()
            .size(10.0)
            .color(INK_3),
    )
}

/// A small monospace readout (numeric captions like the legend's
/// colorbar labels): numbers stay tabular even at caption size.
pub(crate) fn mono_small(ui: &mut egui::Ui, text: String) -> egui::Response {
    ui.label(egui::RichText::new(text).monospace().size(10.0))
}

/// A ribbon button: phosphor icon over a text label (never icon-only),
/// with the mockup's `.bt` styling — flat at rest, hairline on hover,
/// teal selected state, coral tint for destructive actions. Painted
/// manually so the two type sizes stay centered as a unit.
pub(crate) fn ribbon_button(
    ui: &mut egui::Ui,
    on: bool,
    destructive: bool,
    icon: &str,
    label: &str,
) -> egui::Response {
    let label_w = ui.fonts(|f| {
        f.layout_no_wrap(label.to_owned(), FontId::proportional(10.0), Color32::WHITE)
            .size()
            .x
    });
    let size = egui::vec2((label_w + 14.0).max(52.0), 46.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let base_ink = if !ui.is_enabled() {
        INK_3
    } else if destructive {
        BAD
    } else {
        INK_2
    };
    let (fill, stroke, ink) = if on {
        (SEL_BG, Stroke::new(1.0, SEL), SEL_INK)
    } else if resp.hovered() {
        (
            PANEL_2,
            Stroke::new(1.0, if destructive { BAD } else { LINE_2 }),
            if destructive { BAD } else { INK },
        )
    } else {
        (Color32::TRANSPARENT, Stroke::NONE, base_ink)
    };
    let p = ui.painter();
    p.rect(rect, Rounding::same(RADIUS), fill, stroke);
    p.text(
        rect.center_top() + egui::vec2(0.0, 15.0),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::proportional(16.0),
        ink,
    );
    p.text(
        rect.center_bottom() - egui::vec2(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.0),
        ink,
    );
    resp
}
