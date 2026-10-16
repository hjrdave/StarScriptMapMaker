//! The app theme, built from `docs/design/ui-handoff/tokens.json`.
//!
//! [`Palette`] holds every colour token for dark and light. Custom widgets read colours from
//! the palette ([`palette`] picks the one for the current theme), not from `Visuals`.
//! [`install`] applies the palette to egui's `Style` and `Visuals` so stock widgets match.
//! Dark is the default; a light system theme is followed. There is no in-app switch.
//!
//! Fonts: Inter is the first `Proportional` font and JetBrains Mono the first `Monospace`
//! one ([`font_definitions`]); egui's built-in fonts stay behind them as the fallback for
//! glyphs they lack. Every text style takes its family from [`ui_family`] and [`mono_family`].

use eframe::egui::{
    self, Color32, Context, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Style,
    TextStyle, Theme, ThemePreference, Visuals,
};

/// Every colour token of one theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg_window: Color32,
    pub bg_chrome: Color32,
    pub bg_panel: Color32,
    pub bg_raised: Color32,
    pub bg_field: Color32,
    pub bg_canvas: Color32,
    pub bg_tool_section: Color32,
    pub tab_active: Color32,
    pub tab_hover: Color32,
    pub tab_divider: Color32,
    pub border_subtle: Color32,
    pub border_strong: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_tertiary: Color32,
    pub gold: Color32,
    pub gold_text: Color32,
    pub gold_subtle: Color32,
    pub gold_ring: Color32,
    pub select: Color32,
    pub clip: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub ok: Color32,
    /// Stock widget hover fill and stroke. Not in `tokens.json`: the dark values come from the
    /// design's `theme.rs`, the light ones are derived (see [`LIGHT_DERIVED`]).
    pub hover_fill: Color32,
    pub hover_stroke: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Palette {
    pub const DARK: Palette = Palette {
        bg_window: hex(0x14161A),
        bg_chrome: hex(0x17191E),
        bg_panel: hex(0x1B1E23),
        bg_raised: hex(0x262A31),
        bg_field: hex(0x111317),
        bg_canvas: hex(0x0F1114),
        bg_tool_section: hex(0x1F1D17),
        tab_active: hex(0x23272E),
        tab_hover: hex(0x1D2025),
        tab_divider: hex(0x2A2E35),
        border_subtle: hex(0x272B32),
        border_strong: hex(0x3B414B),
        text_primary: hex(0xECE6D8),
        text_secondary: hex(0xB4AE9F),
        text_tertiary: hex(0x9D9788),
        gold: hex(0xD4A84A),
        gold_text: hex(0xF0CF7A),
        gold_subtle: hex(0x3A311C),
        gold_ring: hex(0x8A6C2C),
        select: hex(0x4FA3E0),
        clip: hex(0x7FB6E6),
        warn: hex(0xF2B65A),
        danger: hex(0xF2A093),
        ok: hex(0x8FD49A),
        hover_fill: hex(0x30353D),
        hover_stroke: hex(0x4A505B),
    };

    pub const LIGHT: Palette = Palette {
        bg_window: hex(0xE9E2D0),
        bg_chrome: hex(0xE2D9C3),
        bg_panel: hex(0xF4EEE0),
        bg_raised: hex(0xFBF7EC),
        bg_field: hex(0xFFFCF4),
        bg_canvas: hex(0xD6CDB8),
        border_subtle: hex(0xD9CEB6),
        border_strong: hex(0xBFB298),
        text_primary: hex(0x231E15),
        text_secondary: hex(0x5A5040),
        text_tertiary: hex(0x6E6452),
        gold: hex(0x8A6414),
        gold_text: hex(0x7A5810),
        gold_subtle: hex(0xF1E2B8),
        select: hex(0x2F78B8),
        clip: hex(0x2F6A9E),
        warn: hex(0x8A5A0E),
        danger: hex(0xA63A2A),
        ok: hex(0x2F7A3C),
        // Derived, not from the design: see LIGHT_DERIVED.
        bg_tool_section: hex(0xF1E8D2),
        tab_active: hex(0xF4EEE0),
        tab_hover: hex(0xEBE4D2),
        tab_divider: hex(0xD9CEB6),
        gold_ring: hex(0x8A6414),
        hover_fill: hex(0xEDE5D0),
        hover_stroke: hex(0xA89B80),
    };
}

/// Light values that `tokens.json` does not give; derived from the light tokens that exist
/// (decision on #157), not taken from the design. Checked by a test.
///
/// - `bg_tool_section`: a slightly warmer tint of `bg_panel`.
/// - `tab_active`: `bg_panel`.
/// - `tab_hover`: halfway between `bg_chrome` and `bg_panel`.
/// - `tab_divider`: `border_subtle`.
/// - `gold_ring`: `gold` (kept at 4.5:1 or better on the light panels).
/// - `hover_fill`: a step darker than `bg_raised`; `hover_stroke`: a step darker than
///   `border_strong`.
pub const LIGHT_DERIVED: &[&str] = &[
    "bg_tool_section",
    "tab_active",
    "tab_hover",
    "tab_divider",
    "gold_ring",
    "hover_fill",
    "hover_stroke",
];

/// The palette for `theme`.
pub fn palette_for(theme: Theme) -> &'static Palette {
    match theme {
        Theme::Dark => &Palette::DARK,
        Theme::Light => &Palette::LIGHT,
    }
}

/// The palette of the theme `ctx` is showing now.
pub fn palette(ctx: &Context) -> &'static Palette {
    palette_for(ctx.theme())
}

// Type scale (px). Caption is the minimum size.
pub const TITLE: f32 = 15.0;
pub const HEADING: f32 = 13.0;
pub const BODY: f32 = 12.0;
pub const CAPTION: f32 = 11.0;
pub const MONO: f32 = 11.0;
pub const MIN_TEXT: f32 = CAPTION;

/// Spacing scale (px).
pub const SPACING: [f32; 9] = [2.0, 4.0, 6.0, 8.0, 12.0, 16.0, 20.0, 24.0, 32.0];

// Corner radii (px).
pub const RADIUS_CHIP: u8 = 3;
pub const RADIUS_FIELD: u8 = 4;
pub const RADIUS_BUTTON: u8 = 6;
pub const RADIUS_POPOVER: u8 = 8;
pub const RADIUS_DOCK: u8 = 10;
pub const RADIUS_DIALOG: u8 = 12;

/// Elevation e1: HUD, tooltip.
pub fn shadow_e1() -> Shadow {
    Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(115),
    }
}

/// Elevation e2: popover, menu, tool dock.
pub fn shadow_e2() -> Shadow {
    Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(128),
    }
}

/// Elevation e3: dialog.
pub fn shadow_e3() -> Shadow {
    Shadow {
        offset: [0, 24],
        blur: 60,
        spread: 0,
        color: Color32::from_black_alpha(153),
    }
}

/// The flat modal scrim, rgba(8,9,11,0.55).
pub const SCRIM: Color32 = Color32::from_rgba_unmultiplied_const(8, 9, 11, 140);

/// Layout metrics (px) from `tokens.json`.
pub mod metrics {
    pub const TITLE_BAR_WITH_TABS: f32 = 38.0;
    pub const TRAFFIC_LIGHTS_INSET: f32 = 84.0;
    pub const HOME_TAB: f32 = 44.0;
    pub const MAP_TAB_WIDTH: f32 = 220.0;
    pub const MAP_TAB_MIN: f32 = 120.0;
    pub const NEW_TAB_BUTTON: f32 = 40.0;
    pub const STATUS_BAR: f32 = 24.0;
    pub const PANEL_HEADER: f32 = 34.0;
    pub const LAYER_ROW: f32 = 26.0;
    /// Field height range.
    pub const FIELD: [f32; 2] = [22.0, 26.0];
    pub const LAYERS_PANEL_DEFAULT: f32 = 232.0;
    pub const LAYERS_PANEL_MIN: f32 = 200.0;
    pub const LAYERS_PANEL_MAX: f32 = 360.0;
    pub const LAYERS_PANEL_RAIL: f32 = 40.0;
    pub const PROPERTIES_PANEL_DEFAULT: f32 = 288.0;
    pub const PROPERTIES_PANEL_MIN: f32 = 260.0;
    pub const PROPERTIES_PANEL_MAX: f32 = 360.0;
    /// Tool button size, width then height.
    pub const TOOL_BUTTON: [f32; 2] = [32.0, 30.0];
    pub const TOOL_DOCK_BOTTOM_OFFSET: f32 = 40.0;
    pub const MIN_WINDOW: [f32; 2] = [1024.0, 640.0];
}

/// The UI font family: `Proportional`, which starts with Inter.
pub fn ui_family() -> FontFamily {
    FontFamily::Proportional
}

/// The monospace font family: `Monospace`, which starts with JetBrains Mono.
pub fn mono_family() -> FontFamily {
    FontFamily::Monospace
}

/// egui's fonts with Inter and JetBrains Mono put first in their families. The built-in fonts
/// stay after them, so glyphs the new fonts lack still draw.
pub fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for (font, family) in ssmap_fonts::UI_FONTS
        .iter()
        .zip([ui_family(), mono_family()])
    {
        fonts.font_data.insert(
            font.family.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(font.data)),
        );
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, font.family.to_owned());
    }
    fonts
}

/// The title text style (15 px), for dialog and window titles.
pub fn title_style() -> TextStyle {
    TextStyle::Name("Title".into())
}

/// The caption text style (11 px, the minimum).
pub fn caption_style() -> TextStyle {
    TextStyle::Name("Caption".into())
}

/// Installs the theme: dark by default, light when the system theme is light.
pub fn install(ctx: &Context) {
    ctx.set_fonts(font_definitions());
    ctx.set_theme(ThemePreference::System);
    // Used when the system theme is unknown.
    ctx.options_mut(|o| o.fallback_theme = Theme::Dark);
    // ⌘+ and ⌘− zoom the Canvas, not the whole UI.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    for theme in [Theme::Dark, Theme::Light] {
        ctx.set_style_of(theme, style_for(theme));
    }
}

/// The egui `Style` for `theme`, with the palette applied.
pub fn style_for(theme: Theme) -> Style {
    let p = palette_for(theme);
    let mut v = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    v.override_text_color = None;
    v.window_fill = p.bg_panel;
    v.panel_fill = p.bg_panel;
    v.faint_bg_color = p.bg_raised;
    v.extreme_bg_color = p.bg_field;
    v.code_bg_color = p.bg_field;
    v.window_stroke = Stroke::new(1.0, p.border_strong);
    v.window_corner_radius = CornerRadius::same(RADIUS_DIALOG);
    v.menu_corner_radius = CornerRadius::same(RADIUS_POPOVER);
    v.window_shadow = shadow_e3();
    v.popup_shadow = shadow_e2();
    v.hyperlink_color = p.gold_text;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.danger;
    v.selection.bg_fill = p.gold_subtle;
    v.selection.stroke = Stroke::new(1.0, p.gold_text);

    let r = CornerRadius::same(RADIUS_BUTTON);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg_panel;
    w.noninteractive.weak_bg_fill = p.bg_panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border_subtle);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_secondary);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.bg_raised;
    w.inactive.weak_bg_fill = p.bg_raised;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border_strong);
    w.inactive.fg_stroke = Stroke::new(1.5, p.text_primary);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = p.hover_fill;
    w.hovered.weak_bg_fill = p.hover_fill;
    w.hovered.bg_stroke = Stroke::new(1.0, p.hover_stroke);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text_primary);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.gold_subtle;
    w.active.weak_bg_fill = p.gold_subtle;
    w.active.bg_stroke = Stroke::new(1.0, p.gold_ring);
    w.active.fg_stroke = Stroke::new(1.5, p.gold_text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open = w.active;

    let mut style = Style {
        visuals: v,
        ..Style::default()
    };
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size = egui::vec2(24.0, 22.0);
    style.spacing.window_margin = Margin::same(16);
    style.text_styles = [
        (TextStyle::Heading, FontId::new(HEADING, ui_family())),
        (TextStyle::Body, FontId::new(BODY, ui_family())),
        (TextStyle::Button, FontId::new(BODY, ui_family())),
        (TextStyle::Small, FontId::new(CAPTION, ui_family())),
        (TextStyle::Monospace, FontId::new(MONO, mono_family())),
        (title_style(), FontId::new(TITLE, ui_family())),
        (caption_style(), FontId::new(CAPTION, ui_family())),
    ]
    .into();
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKENS: &str = include_str!("../../../docs/design/ui-handoff/tokens.json");

    fn parse(hex: &str) -> Color32 {
        let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
        super::hex(n)
    }

    /// `(token name in tokens.json, palette colour)`.
    fn fields(p: &Palette) -> Vec<(&'static str, Color32)> {
        vec![
            ("bg.window", p.bg_window),
            ("bg.chrome", p.bg_chrome),
            ("bg.panel", p.bg_panel),
            ("bg.raised", p.bg_raised),
            ("bg.field", p.bg_field),
            ("bg.canvas", p.bg_canvas),
            ("bg.toolSection", p.bg_tool_section),
            ("tab.active", p.tab_active),
            ("tab.hover", p.tab_hover),
            ("tab.divider", p.tab_divider),
            ("border.subtle", p.border_subtle),
            ("border.strong", p.border_strong),
            ("text.primary", p.text_primary),
            ("text.secondary", p.text_secondary),
            ("text.tertiary", p.text_tertiary),
            ("accent.gold", p.gold),
            ("accent.goldText", p.gold_text),
            ("accent.goldSubtle", p.gold_subtle),
            ("accent.goldRing", p.gold_ring),
            ("overlay.select", p.select),
            ("overlay.clip", p.clip),
            ("status.warn", p.warn),
            ("status.danger", p.danger),
            ("status.ok", p.ok),
        ]
    }

    #[test]
    fn palettes_match_tokens_json() {
        let json: serde_json::Value = serde_json::from_str(TOKENS).unwrap();
        for (mode, p) in [("dark", &Palette::DARK), ("light", &Palette::LIGHT)] {
            let tokens = json["color"][mode].as_object().unwrap();
            let mut checked = 0;
            for (name, colour) in fields(p) {
                // The light tokens the design leaves out are derived, see LIGHT_DERIVED.
                if let Some(value) = tokens.get(name) {
                    assert_eq!(colour, parse(value.as_str().unwrap()), "{mode} {name}");
                    checked += 1;
                }
            }
            assert_eq!(checked, tokens.len(), "{mode}: every token is in Palette");
        }
    }

    #[test]
    fn derived_light_values_follow_their_rules() {
        let l = &Palette::LIGHT;
        assert_eq!(l.tab_active, l.bg_panel);
        assert_eq!(l.tab_divider, l.border_subtle);
        assert_eq!(l.gold_ring, l.gold);
        let mid = |a: Color32, b: Color32| {
            let m = |x: u8, y: u8| (u16::from(x) + u16::from(y)).div_ceil(2) as u8;
            Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
        };
        assert_eq!(l.tab_hover, mid(l.bg_chrome, l.bg_panel));
        // Warmer than the panel: less blue.
        assert!(l.bg_tool_section.b() < l.bg_panel.b());
        assert_eq!(LIGHT_DERIVED.len(), 7);
    }

    fn luminance(c: Color32) -> f64 {
        let f = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn text_contrast_is_at_least_4_5_on_the_panels() {
        for p in [&Palette::DARK, &Palette::LIGHT] {
            for bg in [p.bg_panel, p.bg_raised] {
                for fg in [
                    p.text_primary,
                    p.text_secondary,
                    p.text_tertiary,
                    p.gold,
                    p.gold_text,
                ] {
                    assert!(contrast(fg, bg) >= 4.5, "{fg:?} on {bg:?}");
                }
            }
        }
        // Figures quoted in Foundations.html, which rounds them loosely.
        let d = &Palette::DARK;
        for (fg, quoted) in [
            (d.text_primary, 13.4),
            (d.text_secondary, 7.7),
            (d.text_tertiary, 5.8),
            (d.gold_text, 11.3),
        ] {
            let ratio = contrast(fg, d.bg_panel);
            assert!((ratio - quoted).abs() < 0.6, "{fg:?}: {ratio} vs {quoted}");
        }
    }

    #[test]
    fn no_text_style_is_below_the_minimum() {
        for theme in [Theme::Dark, Theme::Light] {
            let style = style_for(theme);
            assert_eq!(style.text_styles.len(), 7);
            for (name, font) in &style.text_styles {
                assert!(font.size >= MIN_TEXT, "{name:?} is {} px", font.size);
            }
        }
    }

    #[test]
    fn text_styles_keep_their_sizes() {
        let style = style_for(Theme::Dark);
        let size = |s: &TextStyle| style.text_styles[s].size;
        assert_eq!(size(&title_style()), 15.0);
        assert_eq!(size(&TextStyle::Heading), 13.0);
        assert_eq!(size(&TextStyle::Body), 12.0);
        assert_eq!(size(&TextStyle::Button), 12.0);
        assert_eq!(size(&TextStyle::Small), 11.0);
        assert_eq!(size(&caption_style()), 11.0);
        assert_eq!(size(&TextStyle::Monospace), 11.0);
    }

    #[test]
    fn inter_and_jetbrains_mono_lead_their_families() {
        let fonts = font_definitions();
        let proportional = &fonts.families[&FontFamily::Proportional];
        let monospace = &fonts.families[&FontFamily::Monospace];
        assert_eq!(proportional[0], "Inter");
        assert_eq!(monospace[0], "JetBrains Mono");
        // egui's own fonts remain as the fallback.
        assert!(proportional.len() > 1 && monospace.len() > 1);
        for name in proportional.iter().chain(monospace) {
            assert!(fonts.font_data.contains_key(name), "{name} has no data");
        }
    }

    #[test]
    fn install_sets_the_palette_on_both_styles() {
        let ctx = Context::default();
        install(&ctx);
        for theme in [Theme::Dark, Theme::Light] {
            let style = ctx.style_of(theme);
            let p = palette_for(theme);
            assert_eq!(style.visuals.panel_fill, p.bg_panel);
            assert_eq!(style.visuals.widgets.inactive.bg_fill, p.bg_raised);
        }
    }
}
