//! Design tokens (colors, radii, spacing, typography) and their application to egui's
//! `Style`/`Visuals`.
//!
//! Call [`apply`] once at startup and whenever the theme setting or accent changes (it is
//! cheap but not free, so do not call it every frame). Components read the current tokens
//! with [`palette`].

use eframe::egui::{
    self, epaint::Shadow, style::ScrollStyle, Color32, Context, CornerRadius, CursorIcon,
    FontFamily, FontId, Margin, RichText, Stroke, TextStyle, Theme, ThemePreference, Ui, Vec2,
};

// ---------------------------------------------------------------------------------------
// Scales
// ---------------------------------------------------------------------------------------

/// Corner radii.
pub mod radius {
    pub const SM: u8 = 4;
    pub const MD: u8 = 8;
    pub const LG: u8 = 12;
    pub const XL: u8 = 16;
}

/// Spacing scale (px).
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

/// Typography scale (points).
pub mod size {
    pub const CAPTION: f32 = 11.0;
    pub const BODY: f32 = 13.5;
    pub const TITLE: f32 = 16.0;
    pub const HEADING: f32 = 20.0;
    pub const DISPLAY: f32 = 28.0;
    pub const LYRIC: f32 = 20.0;
    pub const LYRIC_ACTIVE: f32 = 28.0;
}

/// Fixed layout metrics shared by the shell.
pub mod metrics {
    pub const TRANSPORT_HEIGHT: f32 = 76.0;
    pub const SIDEBAR_EXPANDED: f32 = 216.0;
    pub const SIDEBAR_COLLAPSED: f32 = 64.0;
    pub const RIGHT_PANEL_WIDTH: f32 = 340.0;
    pub const ROW_HEIGHT: f32 = 34.0;
}

/// Name of the bold font family registered by [`super::fonts::install`].
pub const BOLD_FAMILY: &str = "bold";

/// Named text styles registered by [`apply`].
pub mod styles {
    pub const CAPTION: &str = "caption";
    pub const BODY_STRONG: &str = "body_strong";
    pub const TITLE: &str = "title";
    pub const DISPLAY: &str = "display";
    pub const LYRIC: &str = "lyric";
    pub const LYRIC_ACTIVE: &str = "lyric_active";
}

// ---------------------------------------------------------------------------------------
// Settings types
// ---------------------------------------------------------------------------------------

/// Theme setting (mirrors `config::Theme`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ThemeMode {
    #[default]
    System,
    Dark,
    Light,
}

/// Accent color presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Accent {
    #[default]
    Teal,
    Blue,
    Violet,
    Rose,
    Amber,
    Green,
}

impl Accent {
    pub const ALL: [Accent; 6] = [
        Accent::Teal,
        Accent::Blue,
        Accent::Violet,
        Accent::Rose,
        Accent::Amber,
        Accent::Green,
    ];

    /// Accent color tuned for the given background brightness.
    pub fn color(self, dark: bool) -> Color32 {
        let hex = match (self, dark) {
            (Accent::Teal, true) => 0x2EC4B6,
            (Accent::Teal, false) => 0x0E9384,
            (Accent::Blue, true) => 0x5B9BFF,
            (Accent::Blue, false) => 0x2563EB,
            (Accent::Violet, true) => 0x9B8CFF,
            (Accent::Violet, false) => 0x6D5AE6,
            (Accent::Rose, true) => 0xFF6F91,
            (Accent::Rose, false) => 0xE0456A,
            (Accent::Amber, true) => 0xF5B041,
            (Accent::Amber, false) => 0xB86E00,
            (Accent::Green, true) => 0x4ACF86,
            (Accent::Green, false) => 0x1E9E5A,
        };
        hex_color(hex)
    }

    /// Stable key for persisting the choice.
    pub fn key(self) -> &'static str {
        match self {
            Accent::Teal => "teal",
            Accent::Blue => "blue",
            Accent::Violet => "violet",
            Accent::Rose => "rose",
            Accent::Amber => "amber",
            Accent::Green => "green",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Accent::ALL.into_iter().find(|a| a.key() == key)
    }
}

// ---------------------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------------------

/// Resolved color tokens for one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// Window background (sidebar, transport bar).
    pub bg: Color32,
    /// Main content surface / panels.
    pub surface: Color32,
    /// Cards, popups, menus.
    pub elevated: Color32,
    /// Hover fill for rows and icon buttons.
    pub hover: Color32,
    /// Pressed fill.
    pub pressed: Color32,
    /// Selected row fill.
    pub selected: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    /// Text input background.
    pub input_bg: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Accent at low alpha, for selections and active nav items.
    pub accent_soft: Color32,
    /// Text/icons drawn on an `accent` fill.
    pub accent_text: Color32,
    /// Accent used as foreground (text/icon) on `bg`/`surface`.
    pub accent_fg: Color32,
    pub danger: Color32,
    pub warning: Color32,
    pub success: Color32,
    pub shadow: Color32,
}

impl Palette {
    pub fn new(dark: bool, accent: Accent) -> Self {
        let a = accent.color(dark);
        if dark {
            Palette {
                dark,
                bg: hex_color(0x0D0F13),
                surface: hex_color(0x15181E),
                elevated: hex_color(0x1E222A),
                hover: hex_color(0x242933),
                pressed: hex_color(0x2C323D),
                selected: blend(hex_color(0x15181E), a, 0.16),
                border: hex_color(0x272C36),
                border_strong: hex_color(0x363C48),
                input_bg: hex_color(0x0F1116),
                text: hex_color(0xE9ECF1),
                text_muted: hex_color(0xA2A9B6),
                text_faint: hex_color(0x6E7685),
                accent: a,
                accent_hover: lighten(a, 0.12),
                accent_soft: Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), 40),
                accent_text: on_color(a),
                accent_fg: lighten(a, 0.08),
                danger: hex_color(0xF0605D),
                warning: hex_color(0xF5B041),
                success: hex_color(0x4ACF86),
                shadow: Color32::from_black_alpha(110),
            }
        } else {
            Palette {
                dark,
                bg: hex_color(0xEEF0F3),
                surface: hex_color(0xFBFBFC),
                elevated: hex_color(0xFFFFFF),
                hover: hex_color(0xE9ECF0),
                pressed: hex_color(0xDDE1E7),
                selected: blend(hex_color(0xFBFBFC), a, 0.12),
                border: hex_color(0xE1E4E9),
                border_strong: hex_color(0xC9CED6),
                input_bg: hex_color(0xF1F3F6),
                text: hex_color(0x15181D),
                text_muted: hex_color(0x5A6170),
                text_faint: hex_color(0x8A92A0),
                accent: a,
                accent_hover: darken(a, 0.10),
                accent_soft: Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), 34),
                accent_text: on_color(a),
                accent_fg: a,
                danger: hex_color(0xD93B3B),
                warning: hex_color(0xB86E00),
                success: hex_color(0x1E9E5A),
                shadow: Color32::from_black_alpha(36),
            }
        }
    }
}

/// Theme state kept in the egui context so components can look tokens up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeState {
    pub mode: ThemeMode,
    pub accent: Accent,
    pub dark: Palette,
    pub light: Palette,
}

fn state_id() -> egui::Id {
    egui::Id::new("lrc_ui_theme_state")
}

/// Current tokens for the theme egui is rendering with.
pub fn palette(ui: &Ui) -> Palette {
    palette_for(ui.ctx(), ui.visuals().dark_mode)
}

/// Tokens for an explicit dark/light choice.
pub fn palette_for(ctx: &Context, dark: bool) -> Palette {
    let state: Option<ThemeState> = ctx.data(|d| d.get_temp(state_id()));
    match state {
        Some(s) if dark => s.dark,
        Some(s) => s.light,
        None => Palette::new(dark, Accent::default()),
    }
}

/// The last applied theme state, if any.
pub fn current_state(ctx: &Context) -> Option<ThemeState> {
    ctx.data(|d| d.get_temp(state_id()))
}

// ---------------------------------------------------------------------------------------
// Fonts / text helpers
// ---------------------------------------------------------------------------------------

/// Family used for strong text. Falls back to proportional when no bold face was installed.
pub fn bold_family(ctx: &Context) -> FontFamily {
    if super::fonts::bold_available(ctx) {
        FontFamily::Name(BOLD_FAMILY.into())
    } else {
        FontFamily::Proportional
    }
}

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn bold_font(ctx: &Context, size: f32) -> FontId {
    FontId::new(size, bold_family(ctx))
}

pub fn style_name(name: &'static str) -> TextStyle {
    TextStyle::Name(name.into())
}

/// Caption text (11pt, muted).
///
/// The text helpers use explicit [`FontId`]s (not the named text styles) so they never panic,
/// even on a frame where [`apply`] has not taken effect yet.
pub fn caption(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text.into())
        .font(FontId::proportional(size::CAPTION))
        .color(palette(ui).text_muted)
}

pub fn muted(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(palette(ui).text_muted)
}

/// Bold body text.
pub fn strong(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).font(bold_font(ui.ctx(), size::BODY))
}

/// 16pt bold.
pub fn title(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).font(bold_font(ui.ctx(), size::TITLE))
}

/// 20pt bold.
pub fn heading(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).font(bold_font(ui.ctx(), size::HEADING))
}

/// 28pt bold.
pub fn display(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).font(bold_font(ui.ctx(), size::DISPLAY))
}

// ---------------------------------------------------------------------------------------
// apply
// ---------------------------------------------------------------------------------------

/// Configure egui's styles for both themes and select the theme preference.
pub fn apply(ctx: &Context, mode: ThemeMode, accent: Accent) {
    let state = ThemeState {
        mode,
        accent,
        dark: Palette::new(true, accent),
        light: Palette::new(false, accent),
    };
    ctx.data_mut(|d| d.insert_temp(state_id(), state));

    let bold = bold_family(ctx);
    let bold_on = bold != FontFamily::Proportional;
    ctx.data_mut(|d| d.insert_temp(applied_id(), (mode, accent, bold_on)));
    ctx.set_style_of(Theme::Dark, build_style(&state.dark, &bold));
    ctx.set_style_of(Theme::Light, build_style(&state.light, &bold));
    ctx.set_theme(match mode {
        ThemeMode::System => ThemePreference::System,
        ThemeMode::Dark => ThemePreference::Dark,
        ThemeMode::Light => ThemePreference::Light,
    });
}

/// Cheap per-frame variant of [`apply`]: re-applies only when the mode, the accent or the
/// availability of the bold font family changed (fonts installed with
/// [`super::fonts::install`] become active one frame later). Call at the top of `update`.
pub fn sync(ctx: &Context, mode: ThemeMode, accent: Accent) {
    let bold = super::fonts::bold_available(ctx);
    let applied: Option<(ThemeMode, Accent, bool)> = ctx.data(|d| d.get_temp(applied_id()));
    if applied != Some((mode, accent, bold)) {
        apply(ctx, mode, accent);
    }
}

fn applied_id() -> egui::Id {
    egui::Id::new("lrc_ui_theme_applied")
}

/// Build a complete `Style` from a palette.
pub fn build_style(p: &Palette, bold: &FontFamily) -> egui::Style {
    let mut style = egui::Style::default();

    // --- text styles ---
    let mut ts = std::collections::BTreeMap::new();
    ts.insert(TextStyle::Small, FontId::proportional(size::CAPTION));
    ts.insert(TextStyle::Body, FontId::proportional(size::BODY));
    ts.insert(TextStyle::Button, FontId::proportional(size::BODY));
    ts.insert(TextStyle::Monospace, FontId::monospace(12.5));
    ts.insert(TextStyle::Heading, FontId::new(size::HEADING, bold.clone()));
    ts.insert(style_name(styles::CAPTION), FontId::proportional(size::CAPTION));
    ts.insert(style_name(styles::BODY_STRONG), FontId::new(size::BODY, bold.clone()));
    ts.insert(style_name(styles::TITLE), FontId::new(size::TITLE, bold.clone()));
    ts.insert(style_name(styles::DISPLAY), FontId::new(size::DISPLAY, bold.clone()));
    ts.insert(style_name(styles::LYRIC), FontId::new(size::LYRIC, bold.clone()));
    ts.insert(style_name(styles::LYRIC_ACTIVE), FontId::new(size::LYRIC_ACTIVE, bold.clone()));
    style.text_styles = ts;

    // --- spacing ---
    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(8.0, 6.0);
    s.button_padding = Vec2::new(12.0, 5.0);
    s.window_margin = Margin::same(14);
    s.menu_margin = Margin::same(6);
    s.indent = 16.0;
    s.interact_size = Vec2::new(36.0, 28.0);
    s.slider_width = 140.0;
    s.slider_rail_height = 4.0;
    s.combo_width = 160.0;
    s.text_edit_width = 220.0;
    s.icon_width = 16.0;
    s.icon_width_inner = 9.0;
    s.icon_spacing = 6.0;
    s.tooltip_width = 320.0;
    s.menu_width = 200.0;
    s.combo_height = 280.0;
    let mut scroll = ScrollStyle::floating();
    scroll.bar_width = 8.0;
    scroll.floating_width = 3.0;
    scroll.floating_allocated_width = 0.0;
    scroll.handle_min_length = 24.0;
    scroll.bar_inner_margin = 2.0;
    scroll.bar_outer_margin = 2.0;
    scroll.foreground_color = true;
    scroll.dormant_background_opacity = 0.0;
    scroll.active_background_opacity = 0.0;
    scroll.interact_background_opacity = 0.15;
    scroll.dormant_handle_opacity = 0.0;
    scroll.active_handle_opacity = 0.45;
    scroll.interact_handle_opacity = 0.75;
    s.scroll = scroll;

    // --- interaction ---
    style.interaction.tooltip_delay = 0.35;
    style.interaction.selectable_labels = false;
    style.animation_time = 0.12;

    // --- visuals ---
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let md = CornerRadius::same(radius::MD);
    v.override_text_color = None;
    v.weak_text_color = Some(p.text_muted);
    v.hyperlink_color = p.accent_fg;
    v.faint_bg_color = blend(p.surface, p.text, 0.025);
    v.extreme_bg_color = p.input_bg;
    v.text_edit_bg_color = Some(p.input_bg);
    v.code_bg_color = p.elevated;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.panel_fill = p.bg;
    v.window_fill = p.elevated;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(radius::LG);
    v.menu_corner_radius = CornerRadius::same(radius::MD);
    v.window_shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: p.shadow,
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: p.shadow,
    };
    v.window_highlight_topmost = false;
    v.interact_cursor = Some(CursorIcon::PointingHand);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    v.button_frame = true;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.resize_corner_size = 10.0;
    v.clip_rect_margin = 3.0;
    v.image_loading_spinners = true;

    v.selection.bg_fill = Color32::from_rgba_unmultiplied(
        p.accent.r(),
        p.accent.g(),
        p.accent.b(),
        if p.dark { 70 } else { 60 },
    );
    v.selection.stroke = Stroke::new(1.0, p.text);

    v.text_cursor.stroke = Stroke::new(2.0, p.accent);

    let w = &mut v.widgets;
    // labels, separators, frames
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = md;
    w.noninteractive.expansion = 0.0;
    // buttons, checkboxes, slider rails at rest
    w.inactive.bg_fill = p.pressed;
    w.inactive.weak_bg_fill = p.elevated;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = md;
    w.inactive.expansion = 0.0;
    w.hovered.bg_fill = p.border_strong;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = md;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.accent;
    w.active.weak_bg_fill = p.pressed;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = md;
    w.active.expansion = 0.0;
    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = md;
    w.open.expansion = 0.0;

    style.visuals = v;
    style
}

// ---------------------------------------------------------------------------------------
// Color helpers
// ---------------------------------------------------------------------------------------

pub const fn hex_color(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Linear blend `a -> b` by `t` (opaque result, alpha of `a`).
pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), a.a())
}

pub fn lighten(c: Color32, t: f32) -> Color32 {
    blend(c, Color32::WHITE, t)
}

pub fn darken(c: Color32, t: f32) -> Color32 {
    blend(c, Color32::BLACK, t)
}

/// Same color with a new alpha (`0.0..=1.0`), unmultiplied semantics.
pub fn with_alpha(c: Color32, alpha: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, (a as f32 * alpha.clamp(0.0, 1.0)).round() as u8)
}

/// Relative luminance (sRGB approximation, 0..1).
pub fn luminance(c: Color32) -> f32 {
    let f = |x: u8| {
        let v = x as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
}

/// Readable text color on top of `c`.
pub fn on_color(c: Color32) -> Color32 {
    if luminance(c) > 0.36 {
        hex_color(0x0B0D10)
    } else {
        Color32::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_does_not_panic_and_sets_both_themes() {
        let ctx = Context::default();
        for mode in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
            for accent in Accent::ALL {
                apply(&ctx, mode, accent);
            }
        }
        // Run a frame with the styles applied, using every named style.
        let _ = ctx.run_ui(Default::default(), |ui| {
            for name in [
                styles::CAPTION,
                styles::BODY_STRONG,
                styles::TITLE,
                styles::DISPLAY,
                styles::LYRIC,
                styles::LYRIC_ACTIVE,
            ] {
                ui.label(RichText::new("テスト Test").text_style(style_name(name)));
            }
            let p = palette(ui);
            assert_eq!(p.accent, Accent::Green.color(p.dark));
        });
        let s = current_state(&ctx).unwrap();
        assert_eq!(s.mode, ThemeMode::Light);
    }

    #[test]
    fn accent_text_is_readable() {
        for dark in [true, false] {
            for a in Accent::ALL {
                let p = Palette::new(dark, a);
                assert_ne!(p.accent, p.accent_text);
            }
        }
        assert_eq!(Accent::from_key("rose"), Some(Accent::Rose));
    }
}
