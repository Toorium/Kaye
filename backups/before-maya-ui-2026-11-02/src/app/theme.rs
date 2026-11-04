//! Colors, fonts and widget styling, built from the theme in Preferences.
//!
//! Nothing in the 3D scene reads from here except selection highlights, so a theme
//! can never change how a rig is lit.

use std::sync::RwLock;

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2, style::HandleShape};

use crate::settings::{Rgb, ThemeSettings};

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

// Status colors mean the same thing in every theme.
pub const WARNING: Color32 = rgb(0xE9, 0xB9, 0x4A);
pub const ERROR: Color32 = rgb(0xEF, 0x5B, 0x5B);

// What a thing is reads from its icon's color in every theme, as in Blender's outliner.
pub const RIG_ICON: Color32 = rgb(0xF0, 0xA8, 0x60);
pub const BONE_ICON: Color32 = rgb(0x8F, 0xCB, 0x8A);
pub const PART_ICON: Color32 = rgb(0x6C, 0xC4, 0xC4);
pub const CAMERA_ICON: Color32 = rgb(0xE6, 0xC8, 0x6A);
pub const MODIFIER_ICON: Color32 = rgb(0x7F, 0xB3, 0xEA);

/// The Auto Key button while it's recording.
pub const RECORD: Color32 = rgb(0xD9, 0x48, 0x48);
/// Text and icons over the 3D view, whose background doesn't follow the theme.
pub const OVERLAY: Color32 = rgb(0xE6, 0xE6, 0xE6);

/// Corner rounding shared by every control.
pub const RADIUS: u8 = 3;
/// Corner rounding of property sections and menus.
pub const SECTION_RADIUS: u8 = 4;
/// Corner rounding of windows.
pub const WINDOW_RADIUS: u8 = 6;

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
    section: Color32,
    row_alt: Color32,
    header: Color32,
    field: Color32,
    field_hover: Color32,
    field_active: Color32,
    text: Color32,
    text_dim: Color32,
    text_faint: Color32,
    selected_row: Color32,
    active_row: Color32,
    toggled: Color32,
    opacity: f32,
    tooltips: bool,
}

const fn opaque(c: Rgb) -> Color32 {
    rgb(c[0], c[1], c[2])
}

static COLORS: RwLock<Colors> = RwLock::new(Colors {
    accent: opaque(PRESETS[0].1),
    accent_soft: opaque(PRESETS[0].2),
    window: rgb(0x19, 0x19, 0x19),
    panel_deep: rgb(0x22, 0x22, 0x22),
    panel: rgb(0x2B, 0x2B, 0x2B),
    section: rgb(0x36, 0x36, 0x36),
    row_alt: rgb(0x26, 0x26, 0x26),
    header: rgb(0x33, 0x33, 0x33),
    field: rgb(0x4E, 0x4E, 0x4E),
    field_hover: rgb(0x5C, 0x5C, 0x5C),
    field_active: rgb(0x6A, 0x6A, 0x6A),
    text: rgb(0xE6, 0xE6, 0xE6),
    text_dim: rgb(0xB4, 0xB4, 0xB4),
    text_faint: rgb(0x96, 0x96, 0x96),
    selected_row: rgb(0x55, 0x37, 0x38),
    active_row: rgb(0x7A, 0x44, 0x46),
    toggled: rgb(0x8E, 0x5C, 0x5E),
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
/// The active item in a list, brighter than others selected with it.
pub fn active_row() -> Color32 {
    colors().active_row
}
/// A switch that's on. A full accent fill stays reserved for the current tool.
pub fn toggled() -> Color32 {
    colors().toggled
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
pub fn section() -> Color32 {
    let c = colors();
    surface(c.section, c.opacity)
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
/// Whether the theme's text is dark, so tinted fills have to be light to stay readable.
fn dark_text() -> bool {
    let t = colors().text;
    t.r() as u32 + t.g() as u32 + t.b() as u32 <= 380
}
// A field's color says how its value relates to the keys, as in Blender. Each keeps
// at least 4.5:1 against the text.
/// A field on an animated value, on a frame with no key: green.
pub fn animated_field() -> Color32 {
    if dark_text() { rgb(0xA8, 0xD8, 0xAE) } else { rgb(0x3C, 0x6E, 0x44) }
}
/// A field on a frame with a key: yellow.
pub fn keyed_field() -> Color32 {
    if dark_text() { rgb(0xE8, 0xD5, 0x7A) } else { rgb(0x78, 0x66, 0x1E) }
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
/// Faint lines that only give structure: outliner guides and the timeline grid.
pub fn guide() -> Color32 {
    let t = colors().text;
    Color32::from_rgba_unmultiplied(t.r(), t.g(), t.b(), 22)
}
pub fn tooltips() -> bool {
    colors().tooltips
}

/// The font family behind `strong_font`.
const STRONG: &str = "strong";

/// Tahoma Bold for titles, since egui's `strong` only changes a text's color.
pub fn strong_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(STRONG.into()))
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

/// A tinted control's color under the mouse.
pub fn lighter(c: Color32) -> Color32 {
    mix(c, Color32::WHITE, 0.08)
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
    // Light panels need far less darkening to read as a step down.
    let light_theme = contrast == Color32::BLACK;
    let panel_deep = mix(panel_color, Color32::BLACK, if light_theme { 0.07 } else { 0.2 });
    let c = Colors {
        accent: accent_color,
        accent_soft: opaque(theme.accent_soft),
        window: mix(panel_color, Color32::BLACK, if light_theme { 0.16 } else { 0.42 }),
        panel_deep,
        panel: panel_color,
        section: mix(panel_color, contrast, 0.05),
        row_alt: mix(panel_deep, contrast, 0.022),
        header: opaque(theme.header),
        field: field_color,
        field_hover: mix(field_color, contrast, 0.09),
        field_active: mix(field_color, contrast, 0.17),
        text: text_color,
        text_dim: mix(text_color, panel_color, 0.24),
        text_faint: mix(text_color, panel_color, 0.4),
        selected_row: mix(panel_deep, accent_color, 0.26),
        active_row: mix(panel_deep, accent_color, 0.46),
        toggled: mix(field_color, accent_color, 0.36),
        opacity: theme.panel_opacity.clamp(0.0, 1.0),
        tooltips,
    };
    *COLORS.write().unwrap() = c;

    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.panel_fill = surface(c.panel, c.opacity);
        v.window_fill = c.panel;
        // Text boxes sit darker than number fields, which is how Blender tells them apart.
        v.extreme_bg_color = c.panel_deep;
        v.text_edit_bg_color = Some(if light_theme { mix(c.panel, Color32::WHITE, 0.6) } else { c.window });
        v.faint_bg_color = c.row_alt;
        v.override_text_color = Some(c.text);
        v.window_stroke = Stroke::new(1.0, mix(c.panel, contrast, 0.14));
        v.hyperlink_color = c.accent_soft;
        v.selection.bg_fill = c.toggled;
        v.selection.stroke = Stroke::new(1.0, c.accent);
        v.text_cursor.stroke = Stroke::new(1.5, c.accent_soft);

        // A thin dark edge gives controls the raised look they have in every reference tool.
        let edge = Stroke::new(1.0, mix(c.field, Color32::BLACK, 0.3));
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = c.panel;
        w.noninteractive.weak_bg_fill = c.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, c.window);
        w.noninteractive.fg_stroke = Stroke::new(1.0, c.text_dim);
        w.inactive.bg_fill = c.field;
        w.inactive.weak_bg_fill = c.field;
        w.inactive.bg_stroke = edge;
        w.inactive.fg_stroke = Stroke::new(1.0, c.text);
        w.hovered.bg_fill = c.field_hover;
        w.hovered.weak_bg_fill = c.field_hover;
        w.hovered.bg_stroke = edge;
        w.hovered.fg_stroke = Stroke::new(1.0, c.text);
        w.active.bg_fill = c.field_active;
        w.active.weak_bg_fill = c.field_active;
        w.active.bg_stroke = edge;
        w.active.fg_stroke = Stroke::new(1.0, c.text);
        w.open.bg_fill = c.field_hover;
        w.open.weak_bg_fill = c.field_hover;
        w.open.bg_stroke = edge;
    });
}

/// Sets up everything that doesn't change with the theme.
pub fn apply(ctx: &egui::Context) {
    load_system_fonts(ctx);
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
        s.button_padding = Vec2::new(7.0, 3.0);
        s.interact_size = Vec2::new(36.0, 21.0);
        s.window_margin = Margin::same(12);
        s.menu_margin = Margin::same(4);
        s.indent = 12.0;
        s.icon_width = 13.0;
        s.icon_width_inner = 8.0;
        s.icon_spacing = 5.0;
        s.scroll.bar_width = 6.0;
        s.scroll.floating = true;

        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
        v.menu_corner_radius = CornerRadius::same(SECTION_RADIUS);
        // Menus and windows float over the work, and a soft shadow is what says so.
        v.window_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(120) };
        v.popup_shadow = Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(110) };
        // The title bar stays the window's color, whichever window is on top.
        v.window_highlight_topmost = false;
        v.slider_trailing_fill = true;
        v.handle_shape = HandleShape::Rect { aspect_ratio: 0.5 };

        let radius = CornerRadius::same(RADIUS);
        let w = &mut v.widgets;
        w.noninteractive.corner_radius = radius;
        w.inactive.corner_radius = radius;
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;
        w.open.corner_radius = radius;
    });
}

/// Tahoma, chosen by the owner for its desktop-tool look. Kaye still works with the
/// bundled font when a file is missing, and titles fall back to regular.
fn load_system_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |name: &str, path: &str| {
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        // Tahoma's metrics leave it about a pixel high in egui's boxes.
        let tweak = egui::FontTweak { y_offset_factor: 0.06, ..Default::default() };
        fonts.font_data.insert(name.to_owned(), std::sync::Arc::new(egui::FontData::from_owned(bytes).tweak(tweak)));
        true
    };
    let regular = load("tahoma", "C:/Windows/Fonts/tahoma.ttf");
    let bold = load("tahoma bold", "C:/Windows/Fonts/tahomabd.ttf");
    let family = fonts.families.entry(FontFamily::Proportional).or_default();
    if regular {
        family.insert(0, "tahoma".to_owned());
    }
    let mut strong = family.clone();
    if bold {
        strong.insert(0, "tahoma bold".to_owned());
    }
    fonts.families.insert(FontFamily::Name(STRONG.into()), strong);
    ctx.set_fonts(fonts);
}
