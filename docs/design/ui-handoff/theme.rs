//! StarScript theme — starter built from tokens.json.
//!
//! Written against egui 0.31 (`CornerRadius`, `Shadow { offset: [i8; 2], blur: u8, spread: u8, .. }`).
//! On older egui, rename `corner_radius` → `rounding` and adjust `Shadow` fields.
//! Treat this as a starting point: custom widgets (tabs, tool dock, layer rows,
//! scrub fields) should read colours from `Palette` rather than from `Visuals`.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Style, TextStyle, Visuals};

#[derive(Clone, Copy, Debug)]
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
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

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
};

pub const LIGHT: Palette = Palette {
    bg_window: hex(0xE9E2D0),
    bg_chrome: hex(0xE2D9C3),
    bg_panel: hex(0xF4EEE0),
    bg_raised: hex(0xFBF7EC),
    bg_field: hex(0xFFFCF4),
    bg_canvas: hex(0xD6CDB8),
    bg_tool_section: hex(0xF1E8D2),
    tab_active: hex(0xF4EEE0),
    tab_hover: hex(0xEAE2CF),
    tab_divider: hex(0xD9CEB6),
    border_subtle: hex(0xD9CEB6),
    border_strong: hex(0xBFB298),
    text_primary: hex(0x231E15),
    text_secondary: hex(0x5A5040),
    text_tertiary: hex(0x6E6452),
    gold: hex(0x8A6414),
    gold_text: hex(0x7A5810),
    gold_subtle: hex(0xF1E2B8),
    gold_ring: hex(0xC9A55A),
    select: hex(0x2F78B8),
    clip: hex(0x2F6A9E),
    warn: hex(0x8A5A0E),
    danger: hex(0xA63A2A),
    ok: hex(0x2F7A3C),
};

/// Shadows from tokens.json `elevation`.
pub fn shadow_hud() -> Shadow { Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(115) } }
pub fn shadow_popover() -> Shadow { Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(128) } }
pub fn shadow_dialog() -> Shadow { Shadow { offset: [0, 24], blur: 60, spread: 0, color: Color32::from_black_alpha(153) } }

pub fn apply(ctx: &egui::Context, p: &Palette, dark: bool) {
    let mut v = if dark { Visuals::dark() } else { Visuals::light() };
    v.override_text_color = Some(p.text_primary);
    v.window_fill = p.bg_panel;
    v.panel_fill = p.bg_panel;
    v.faint_bg_color = p.bg_raised;
    v.extreme_bg_color = p.bg_field;
    v.window_stroke = Stroke::new(1.0, p.border_strong);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = shadow_dialog();
    v.popup_shadow = shadow_popover();
    v.hyperlink_color = p.gold_text;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.danger;
    v.selection.bg_fill = p.gold_subtle;
    v.selection.stroke = Stroke::new(1.0, p.gold_text);

    let r = CornerRadius::same(6);
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

    w.hovered.bg_fill = hex(0x30353D);
    w.hovered.weak_bg_fill = hex(0x30353D);
    w.hovered.bg_stroke = Stroke::new(1.0, hex(0x4A505B));
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

    let mut style: Style = (*ctx.style()).clone();
    style.visuals = v;
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size = egui::vec2(24.0, 22.0);
    style.spacing.window_margin = Margin::same(16);
    style.text_styles = [
        (TextStyle::Heading, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(11.0, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_style(style);
}
