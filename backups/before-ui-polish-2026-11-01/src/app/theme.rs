//! Colors, fonts and widget styling, built from the theme in Preferences.
//!
//! Nothing in the 3D scene reads from here except selection highlights, so a theme
//! can never change how a rig is lit.

use std::sync::RwLock;

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2};

use crate::settings::{Rgb, ThemeSettings};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

// Status colors mean the same thing in every theme.
pub const WARNING: Color32 = rgb(0xE9, 0xB9, 0x4A);
pub const ERROR: Color32 = rgb(0xEF, 0x5B, 0x5B);

/// Built-in accents: name, accent, and a lighter accent for hover states. Coral is
/// Kaye's brand color.
pub const PRESETS: [(&str, Rgb, Rgb); 6] = [
    ("Coral", [0xF3, 0x6B, 0x70], [0xFF, 0xA8, 0xA3]),
    ("Ocean", [0x4F, 0xA3, 0xE8], [0x9C, 0xCB, 0xF3]),
    ("Mint", [0x3F, 0xC7, 0xA0], [0x90, 0xE2, 0xCA]),
    ("Amber", [0xE9, 0xA2, 0x3B], [0xF4, 0xCB, 0x8C]),
    ("Violet", [0x9B, 0x7B, 0xEA], [0xC7, 0xB4, 0xF4]),
    ("Graphite", [0xC4, 0xC4, 0xC4], [0xE6, 0xE6, 0xE6]),
];

#[derive(Clone, Copy)]
struct Colors {
    accent: Color32,
    accent_soft: Color32,
    window: Color32,
    panel_deep: Color32,
    panel: Color32,
    row_alt: Color32,
    header: Color32,
    field: Color32,
    field_hover: Color32,
    field_active: Color32,
    text: Color32,
    text_dim: Color32,
    text_faint: Color32,
    selected_row: Color32,
    opacity: f32,
    tooltips: bool,
}

const fn opaque(c: Rgb) -> Color32 {
    rgb(c[0], c[1], c[2])
}

static COLORS: RwLock<Colors> = RwLock::new(Colors {
    accent: opaque(PRESETS[0].1),
    accent_soft: opaque(PRESETS[0].2),
    window: rgb(0x14, 0x14, 0x14),
    panel_deep: rgb(0x1A, 0x1A, 0x1A),
    panel: rgb(0x20, 0x20, 0x20),
    row_alt: rgb(0x25, 0x25, 0x25),
    header: rgb(0x2B, 0x2B, 0x2B),
    field: rgb(0x38, 0x38, 0x38),
    field_hover: rgb(0x46, 0x46, 0x46),
    field_active: rgb(0x54, 0x54, 0x54),
    text: rgb(0xF2, 0xF2, 0xF2),
    text_dim: rgb(0xB4, 0xB4, 0xB4),
    text_faint: rgb(0x8C, 0x8C, 0x8C),
    selected_row: rgb(0x60, 0x3A, 0x3C),
    opacity: 1.0,
    tooltips: true,
});

fn colors() -> Colors {
    *COLORS.read().unwrap()
}

/// A surface panels are filled with, which an image behind the UI can show through.
fn surface(c: Color32, opacity: f32) -> Color32 {
    if opacity >= 0.999 {
        c
    } else {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (opacity * 255.0) as u8)
    }
}

pub fn accent() -> Color32 {
    colors().accent
}
pub fn accent_soft() -> Color32 {
    colors().accent_soft
}
pub fn selected_row() -> Color32 {
    colors().selected_row
}
pub fn window() -> Color32 {
    colors().window
}
pub fn border() -> Color32 {
    colors().window
}
pub fn panel_deep() -> Color32 {
    let c = colors();
    surface(c.panel_deep, c.opacity)
}
pub fn panel() -> Color32 {
    let c = colors();
    surface(c.panel, c.opacity)
}
pub fn row_alt() -> Color32 {
    let c = colors();
    surface(c.row_alt, c.opacity)
}
pub fn header() -> Color32 {
    let c = colors();
    surface(c.header, c.opacity)
}
pub fn field() -> Color32 {
    colors().field
}
pub fn field_hover() -> Color32 {
    colors().field_hover
}
pub fn field_active() -> Color32 {
    colors().field_active
}
pub fn ink() -> Color32 {
    colors().text
}
pub fn text() -> Color32 {
    colors().text
}
pub fn text_dim() -> Color32 {
    colors().text_dim
}
pub fn text_faint() -> Color32 {
    colors().text_faint
}
pub fn tooltips() -> bool {
    colors().tooltips
}

/// Text and icons on a full accent fill, chosen so any accent stays readable.
pub fn on_accent() -> Color32 {
    let c = accent();
    let luma = 0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32;
    if luma > 100.0 { rgb(0x14, 0x14, 0x14) } else { rgb(0xF2, 0xF2, 0xF2) }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    rgb(channel(a.r(), b.r()), channel(a.g(), b.g()), channel(a.b(), b.b()))
}

/// Rebuilds every color from the theme settings, deriving the in-between shades.
pub fn apply_settings(ctx: &egui::Context, theme: &ThemeSettings, tooltips: bool) {
    let panel_color = opaque(theme.panel);
    let field_color = opaque(theme.field);
    let text_color = opaque(theme.text);
    let accent_color = opaque(theme.accent);
    let contrast = if panel_color.r() as u32 + panel_color.g() as u32 + panel_color.b() as u32 > 380 {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    let c = Colors {
        accent: accent_color,
        accent_soft: opaque(theme.accent_soft),
        window: mix(panel_color, Color32::BLACK, 0.36),
        panel_deep: mix(panel_color, Color32::BLACK, 0.18),
        panel: panel_color,
        row_alt: mix(panel_color, contrast, 0.022),
        header: opaque(theme.header),
        field: field_color,
        field_hover: mix(field_color, contrast, 0.09),
        field_active: mix(field_color, contrast, 0.17),
        text: text_color,
        text_dim: mix(text_color, panel_color, 0.26),
        text_faint: mix(text_color, panel_color, 0.42),
        selected_row: mix(panel_color, accent_color, 0.34),
        opacity: theme.panel_opacity.clamp(0.0, 1.0),
        tooltips,
    };
    *COLORS.write().unwrap() = c;

    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.panel_fill = surface(c.panel, c.opacity);
        v.window_fill = c.panel;
        v.extreme_bg_color = c.field;
        v.faint_bg_color = c.row_alt;
        v.override_text_color = Some(c.text);
        v.window_stroke = Stroke::new(1.0, c.window);
        v.hyperlink_color = c.accent_soft;
        v.selection.bg_fill = c.selected_row;
        v.selection.stroke = Stroke::new(1.0, c.accent);
        v.text_cursor.stroke = Stroke::new(1.5, c.accent_soft);

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = c.panel;
        w.noninteractive.weak_bg_fill = c.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, c.window);
        w.noninteractive.fg_stroke = Stroke::new(1.0, c.text_dim);
        w.inactive.bg_fill = c.field;
        w.inactive.weak_bg_fill = c.field;
        w.inactive.fg_stroke = Stroke::new(1.0, c.text);
        w.hovered.bg_fill = c.field_hover;
        w.hovered.weak_bg_fill = c.field_hover;
        w.hovered.fg_stroke = Stroke::new(1.0, c.text);
        w.active.bg_fill = c.field_active;
        w.active.weak_bg_fill = c.field_active;
        w.active.fg_stroke = Stroke::new(1.0, c.text);
        w.open.bg_fill = c.field_hover;
        w.open.weak_bg_fill = c.field_hover;
    });
}

/// Sets up everything that doesn't change with the theme.
pub fn apply(ctx: &egui::Context) {
    load_system_font(ctx);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(12.5, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(4.0, 2.0);
        s.button_padding = Vec2::new(6.0, 2.0);
        s.interact_size = Vec2::new(36.0, 20.0);
        s.window_margin = Margin::same(10);
        s.menu_margin = Margin::same(3);
        s.indent = 12.0;
        s.icon_width = 12.0;
        s.icon_width_inner = 7.0;
        s.icon_spacing = 5.0;
        s.scroll.bar_width = 6.0;
        s.scroll.floating = true;

        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.window_corner_radius = CornerRadius::same(3);
        v.menu_corner_radius = CornerRadius::same(3);
        v.window_shadow = Shadow::NONE;
        v.popup_shadow = Shadow::NONE;

        let radius = CornerRadius::same(2);
        let w = &mut v.widgets;
        w.noninteractive.corner_radius = radius;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.corner_radius = radius;
        w.hovered.bg_stroke = Stroke::NONE;
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;
        w.active.bg_stroke = Stroke::NONE;
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;
        w.open.bg_stroke = Stroke::NONE;
        w.open.corner_radius = radius;
    });
}

/// Segoe UI reads better than egui's bundled font at small sizes. Kaye still
/// works with the bundled font when it is missing.
fn load_system_font(ctx: &egui::Context) {
    let Ok(bytes) = std::fs::read("C:/Windows/Fonts/segoeui.ttf") else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "segoe".to_owned(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.insert(0, "segoe".to_owned());
    }
    ctx.set_fonts(fonts);
}
